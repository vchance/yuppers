//! Text messages: one-time codes for phone numbers routed by identifier to
//! the provider that makes them (Twilio Verify, here an in-process stand-in
//! that keeps what it is asked; `tests/verify.rs` has Verify's own API),
//! in the reader's language, only to the countries served, capped per hour
//! for the whole service and for numbers beginning alike; agreement updates
//! sent to Twilio's Messages API in the shape it takes (against a stand-in
//! in this process); and nothing logged with a whole phone number.
//!
//! The cap counts every text message the database has seen this hour, so
//! the tests here take turns, and each starts with the counts cleared.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use base64::Engine as _;
use common::App;
use serde_json::{Value, json};
use tokio::sync::MutexGuard;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use yuppers_backend::auth::{
    AuthRules, CheckFuture, CodeMessage, CodeSender, CodeVerifier, LogSender, Purpose, SendFuture,
};
use yuppers_backend::domain::identity::Identifier;
use yuppers_backend::metrics::{self, Text};
use yuppers_backend::notifications::sms::{
    CodeRouter, LogSmsSender, PhoneCodes, Sms, SmsSender, TwilioCredential, TwilioSmsSender,
};
use yuppers_backend::notifications::sms_updates::{self, SmsDelivery};
use yuppers_backend::notifications::smtp::{Secret, SmtpSender, SmtpSettings, TlsMode};
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::telemetry::{self, LogFormat};

const DATABASE: &str = "yuppers_test_sms";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A number nobody else in these tests uses.
fn number() -> String {
    format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000)
}

/// The same, beginning `+1888`: another area code.
fn number_elsewhere() -> String {
    format!("+1888{:07}", Uuid::new_v4().as_u128() % 10_000_000)
}

/// Rules with the per-address limit out of the way (every test request
/// comes from one address) and the given cap on text messages. Every
/// [`number`] begins alike, so the cap per prefix is out of the way too.
fn rules(cap: i64) -> AuthRules {
    AuthRules {
        code_requests_per_address_per_hour: 1_000_000,
        sms_codes_per_hour: cap,
        sms_codes_per_prefix_per_hour: 1_000_000,
        ..AuthRules::default()
    }
}

async fn start(cap: i64, sender: Arc<dyn CodeSender>) -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    (open(cap, sender).await, turn)
}

/// [`start`] for a test that already holds its turn.
async fn open(cap: i64, sender: Arc<dyn CodeSender>) -> App {
    open_with(rules(cap), sender).await
}

/// [`open`] with the given rules.
async fn open_with(rules: AuthRules, sender: Arc<dyn CodeSender>) -> App {
    let app = App::start_messaging(DATABASE, rules, sender, false).await;
    sqlx::query("DELETE FROM sign_in_limit WHERE scope LIKE 'sms-%'")
        .execute(&app.owner)
        .await
        .unwrap();
    app
}

async fn ask(app: &App, identifier: &str, language: &str) -> common::Reply {
    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": identifier, "sms_consent": common::sms_consent() })),
        &[("accept-language", language)],
    )
    .await
}

/// The code this stand-in for Twilio Verify sends every time.
const CODE: &str = "424242";

/// Stands in for Twilio Verify: keeps each code it is asked to send, by
/// number, purpose and language, and takes [`CODE`] as the right code.
#[derive(Default)]
struct Phone(Mutex<Vec<(String, Purpose, String)>>);

impl Phone {
    fn sent(&self) -> Vec<(String, Purpose, String)> {
        self.0.lock().unwrap().clone()
    }
}

impl CodeVerifier for Phone {
    fn start<'a>(&'a self, to: &'a str, purpose: Purpose, language: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push((to.to_owned(), purpose, language.to_owned()));
            Ok(())
        })
    }

    fn check<'a>(&'a self, _to: &'a str, _purpose: Purpose, code: &'a str) -> CheckFuture<'a> {
        Box::pin(async move { Ok(code == CODE) })
    }
}

/// Keeps the email codes it is handed.
#[derive(Default)]
struct Mailbox(Mutex<Vec<String>>);

impl CodeSender for Mailbox {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(message.to.as_str().to_owned());
            Ok(())
        })
    }
}

/// `SMS_CODE_DELIVERY=verify`, with `phone` for Twilio Verify.
fn router(phone: &Arc<Phone>, mailbox: &Arc<Mailbox>) -> Arc<CodeRouter> {
    Arc::new(CodeRouter::new(
        mailbox.clone(),
        PhoneCodes::Verify(phone.clone()),
    ))
}

/// The SMS counts for this hour, as the API's metrics show them.
async fn counted(app: &App, cap: i64) -> String {
    let mut text = Text::new();
    metrics::render_sms(&mut text, &app.db, cap).await;
    text.finish()
}

#[tokio::test]
async fn a_phone_number_gets_its_code_by_sms_in_its_language_and_an_email_address_by_email() {
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    let (app, _turn) = start(50, router(&phone, &mailbox)).await;
    let (es, en) = (number(), number());

    assert_eq!(
        ask(&app, &es, "es-MX,es;q=0.9").await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(ask(&app, &en, "en").await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        ask(&app, "ana@example.test", "en").await.status,
        StatusCode::NO_CONTENT
    );

    // Verify is told the language, as a supported tag, and makes the code.
    assert_eq!(
        phone.sent(),
        [
            (es.clone(), Purpose::SignIn, "es".to_owned()),
            (en.clone(), Purpose::SignIn, "en".to_owned()),
        ]
    );
    // The service keeps no hash of a code it never saw, only that one was
    // asked for.
    let stored: Vec<(Option<Vec<u8>>, String)> =
        sqlx::query_as("SELECT code_hash, checked_by FROM one_time_code WHERE identifier = $1")
            .bind(&es)
            .fetch_all(&app.owner)
            .await
            .unwrap();
    assert_eq!(stored, [(None, "TWILIO_VERIFY".to_owned())]);
    assert_eq!(*mailbox.0.lock().unwrap(), ["ana@example.test"]);
    assert!(
        counted(&app, 50)
            .await
            .contains("yuppers_sms_codes_this_hour{result=\"sent\"} 2")
    );
}

/// A code asked for by a signed-in account, which is only ever to add a
/// number to it, and a code to sign in go to the sign-in purpose's Verify
/// service; a code to delete the account to the deletion service's, in the
/// account's language. Each is checked against its own purpose.
#[tokio::test]
async fn each_code_goes_to_its_purposes_verify_service() {
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    let (app, _turn) = start(50, router(&phone, &mailbox)).await;

    for language in ["en", "es"] {
        // Adding a number, from a signed-in account.
        let ana = app.user("Ana").await;
        let added = number();
        let reply = app
            .call(
                Some(&ana),
                Method::POST,
                "/v1/auth/codes",
                Some(json!({ "identifier": added, "sms_consent": common::sms_consent() })),
                &[("accept-language", language)],
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
        assert_eq!(
            phone.sent().last().unwrap(),
            &(added.clone(), Purpose::SignIn, language.to_owned())
        );
        // Its code is a sign-in code: it adds the number.
        let reply = app
            .call(
                Some(&ana),
                Method::POST,
                "/v1/me/identifiers",
                Some(json!({ "identifier": added, "code": CODE })),
                &[],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);

        // Deleting an account by its number, in the account's language.
        sqlx::query("UPDATE account SET language = $2 WHERE id = $1")
            .bind(ana.id)
            .bind(language)
            .execute(&app.owner)
            .await
            .unwrap();
        let reply = app
            .call(
                Some(&ana),
                Method::POST,
                "/v1/me/deletion/codes",
                Some(json!({ "channel": "PHONE", "sms_consent": common::sms_consent() })),
                &[],
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
        assert_eq!(
            phone.sent().last().unwrap(),
            &(added.clone(), Purpose::DeleteAccount, language.to_owned())
        );

        // Signing in, with nobody signed in.
        let signing_in = number();
        assert_eq!(
            ask(&app, &signing_in, language).await.status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            phone.sent().last().unwrap(),
            &(signing_in, Purpose::SignIn, language.to_owned())
        );
    }
    assert_eq!(phone.sent().len(), 6);
    // No email for any of them.
    assert!(mailbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_service_sends_no_more_than_its_hourly_cap_of_text_messages() {
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    let (app, _turn) = start(2, router(&phone, &mailbox)).await;

    for _ in 0..2 {
        assert_eq!(
            ask(&app, &number(), "en").await.status,
            StatusCode::NO_CONTENT
        );
    }
    // A third number, a fourth: the cap is the whole service's, not one
    // number's or one requester's.
    for _ in 0..2 {
        ask(&app, &number(), "en")
            .await
            .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    }
    assert_eq!(phone.sent().len(), 2);
    // Email is not text messages, and not capped by them.
    assert_eq!(
        ask(&app, "ben@example.test", "en").await.status,
        StatusCode::NO_CONTENT
    );

    let page = counted(&app, 2).await;
    for line in [
        "yuppers_sms_codes_hourly_cap 2",
        "yuppers_sms_codes_this_hour{result=\"sent\"} 2",
        "yuppers_sms_codes_this_hour{result=\"refused\"} 2",
        "yuppers_sms_codes_this_hour{result=\"failed\"} 0",
        "yuppers_sms_codes_refused_this_hour{reason=\"hourly_cap\"} 2",
        "yuppers_sms_codes_refused_this_hour{reason=\"prefix_cap\"} 0",
    ] {
        assert!(page.contains(line), "{line} in\n{page}");
    }
}

/// Every count in `sign_in_limit` but the refusals by country, summed.
async fn all_counts(app: &App) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(count), 0)::bigint FROM sign_in_limit
         WHERE scope <> 'sms-refused-country'",
    )
    .fetch_one(&app.owner)
    .await
    .unwrap()
}

async fn codes_for(app: &App, identifier: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM one_time_code WHERE identifier = $1")
        .bind(identifier)
        .fetch_one(&app.owner)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_number_of_a_country_not_served_is_refused_before_anything_is_counted_or_sent() {
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    // The default: +1 only.
    let (app, _turn) = start(50, router(&phone, &mailbox)).await;
    let before = all_counts(&app).await;
    let (uk, mexico, france) = ("+447700900123", "+525512345678", "+33612345678");

    // Signing in.
    ask(&app, uk, "en")
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "PHONE_COUNTRY_NOT_SERVED");

    // Deleting an account whose number was taken before the setting said
    // otherwise.
    let ana = app.user("Ana").await;
    sqlx::query("UPDATE account SET phone = $1 WHERE id = $2")
        .bind(mexico)
        .bind(ana.id)
        .execute(&app.owner)
        .await
        .unwrap();
    app.call(
        Some(&ana),
        Method::POST,
        "/v1/me/deletion/codes",
        Some(json!({ "channel": "PHONE", "sms_consent": common::sms_consent() })),
        &[],
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "PHONE_COUNTRY_NOT_SERVED");

    // Adding a number to an account: refused before any code is checked.
    let ben = app.user("Ben").await;
    app.call(
        Some(&ben),
        Method::POST,
        "/v1/me/identifiers",
        Some(json!({ "identifier": france, "code": "123456" })),
        &[],
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "PHONE_COUNTRY_NOT_SERVED");
    let added: Option<String> = sqlx::query_scalar("SELECT phone FROM account WHERE id = $1")
        .bind(ben.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(added, None);

    // Nothing sent, stored or counted against anyone: not the address, not
    // the account, not the caps.
    assert!(phone.sent().is_empty());
    for number in [uk, mexico, france] {
        assert_eq!(codes_for(&app, number).await, 0, "{number}");
    }
    assert_eq!(all_counts(&app).await, before);
    let page = counted(&app, 50).await;
    for line in [
        "yuppers_sms_codes_this_hour{result=\"sent\"} 0",
        "yuppers_sms_codes_this_hour{result=\"refused\"} 2",
        "yuppers_sms_codes_refused_this_hour{reason=\"country\"} 2",
        "yuppers_sms_codes_refused_this_hour{reason=\"hourly_cap\"} 0",
    ] {
        assert!(page.contains(line), "{line} in\n{page}");
    }

    // A deployment can serve other countries instead.
    let app = open_with(
        AuthRules {
            phone_country_codes: vec!["44".to_owned()],
            ..rules(50)
        },
        router(&phone, &mailbox),
    )
    .await;
    assert_eq!(ask(&app, uk, "en").await.status, StatusCode::NO_CONTENT);
    ask(&app, &number(), "en")
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "PHONE_COUNTRY_NOT_SERVED");
    assert_eq!(phone.sent().len(), 1);
}

#[tokio::test]
async fn numbers_beginning_alike_have_their_own_hourly_cap() {
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    let _turn = TURN.lock().await;
    let app = open_with(
        AuthRules {
            sms_codes_per_prefix_per_hour: 2,
            ..rules(50)
        },
        router(&phone, &mailbox),
    )
    .await;

    for _ in 0..2 {
        assert_eq!(
            ask(&app, &number(), "en").await.status,
            StatusCode::NO_CONTENT
        );
    }
    // A third number in the same area code waits; another area code does
    // not, so whoever uses up one prefix has not turned off phone sign-in
    // for everyone.
    ask(&app, &number(), "en")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    assert_eq!(
        ask(&app, &number_elsewhere(), "en").await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(phone.sent().len(), 3);

    let page = counted(&app, 50).await;
    for line in [
        "yuppers_sms_codes_this_hour{result=\"sent\"} 3",
        "yuppers_sms_codes_this_hour{result=\"refused\"} 1",
        "yuppers_sms_codes_refused_this_hour{reason=\"prefix_cap\"} 1",
        "yuppers_sms_codes_refused_this_hour{reason=\"hourly_cap\"} 0",
    ] {
        assert!(page.contains(line), "{line} in\n{page}");
    }
}

/// An SMTP sender that never connects: what `CODE_DELIVERY=smtp` builds.
fn smtp() -> Arc<SmtpSender> {
    Arc::new(
        SmtpSender::new(
            SmtpSettings {
                host: "127.0.0.1".to_owned(),
                port: 9,
                tls: TlsMode::None,
                credentials: None,
                from: "no-reply@example.test".to_owned(),
                timeout: Duration::from_secs(1),
            },
            Wording::embedded().unwrap(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn with_sms_off_a_code_for_a_phone_number_is_refused_as_before_and_costs_nothing() {
    // CODE_DELIVERY=smtp and SMS_CODE_DELIVERY=off: the SMTP sender alone.
    // It refuses a phone number before it would connect anywhere.
    let (app, _turn) = start(50, smtp()).await;
    ask(&app, &number(), "en")
        .await
        .refused(StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE");
    assert!(
        counted(&app, 50)
            .await
            .contains("yuppers_sms_codes_this_hour{result=\"sent\"} 0")
    );
}

/// With text messages off, agreement updates are neither offered nor
/// queued, even for someone who turned them on while they were sent, and
/// the worker closes whatever was queued before unsent. This binary never
/// says texts are sent (`sms_updates::configure`), as a process with
/// `SMS_DELIVERY=off` does not.
#[tokio::test]
async fn with_sms_off_no_update_text_is_offered_queued_or_sent() {
    let (app, _turn) = start(50, Arc::new(LogSender)).await;
    let meta = app
        .call(None, Method::GET, "/v1/meta", None, &[])
        .await
        .ok();
    assert_eq!(meta["sms_updates"], false);

    let deal = app.active().await;
    let phone = number();
    sqlx::query("UPDATE account SET phone = $2 WHERE id = $1")
        .bind(deal.ben.id)
        .bind(&phone)
        .execute(&app.db)
        .await
        .unwrap();
    let path = format!("/v1/exchanges/{}/sms-updates", deal.exchange);
    let view = app.get(&deal.ben, &path).await.ok();
    assert_eq!(view["available"], false);
    app.call(
        Some(&deal.ben),
        Method::PUT,
        &path,
        Some(json!({ "on": true, "consent": { "version": "2026-10-05", "language": "en" } })),
        &[],
    )
    .await
    .refused(StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE");

    // Turned on while texts were sent: still nothing is queued now.
    let exchange: Uuid = deal.exchange.parse().unwrap();
    sqlx::query("INSERT INTO sms_update (account_id, exchange_id, phone) VALUES ($1, $2, $3)")
        .bind(deal.ben.id)
        .bind(exchange)
        .bind(&phone)
        .execute(&app.db)
        .await
        .unwrap();
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'SMS' AND recipient_account_id = $1",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(queued, 0);

    // One queued before texts were turned off is closed unsent.
    sqlx::query(
        "INSERT INTO outbox (kind, recipient_account_id, exchange_id, payload)
         VALUES ('SMS', $1, $2, '{\"sms\": \"OPT_IN_CONFIRMATION\"}')",
    )
    .bind(deal.ben.id)
    .bind(exchange)
    .execute(&app.db)
    .await
    .unwrap();
    let off = SmsDelivery {
        sender: None,
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: Default::default(),
        auth: AuthRules::default(),
        secret: Vec::new(),
    };
    let delivered =
        sms_updates::deliver_sms_due_until(&app.db, &off, time::OffsetDateTime::now_utc(), || {
            false
        })
        .await
        .unwrap();
    assert!(delivered.dropped >= 1);
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT last_error FROM outbox WHERE kind = 'SMS' AND recipient_account_id = $1",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(reason.as_deref(), Some(sms_updates::SMS_OFF));
}

#[tokio::test]
async fn the_service_says_which_identifiers_it_can_send_codes_to() {
    let _turn = TURN.lock().await;
    let meta = |app: App| async move {
        let meta = app
            .call(None, Method::GET, "/v1/meta", None, &[])
            .await
            .ok();
        (
            meta["sign_in_channels"].clone(),
            meta["sms_country_codes"].clone(),
        )
    };
    let with_sms = |email: Arc<dyn CodeSender>| -> Arc<dyn CodeSender> {
        Arc::new(CodeRouter::new(email, PhoneCodes::Log))
    };
    let with_verify = |email: Arc<dyn CodeSender>| -> Arc<dyn CodeSender> {
        Arc::new(CodeRouter::new(
            email,
            PhoneCodes::Verify(Arc::new(Phone::default())),
        ))
    };

    // CODE_DELIVERY=smtp, SMS_CODE_DELIVERY=off: email only, and no
    // countries, whatever SMS_DELIVERY says.
    assert_eq!(
        meta(open(50, smtp()).await).await,
        (json!(["email"]), json!([]))
    );
    // CODE_DELIVERY=smtp with SMS_CODE_DELIVERY on (log or verify alike).
    for sender in [with_sms(smtp()), with_verify(smtp())] {
        assert_eq!(
            meta(open(50, sender).await).await,
            (json!(["email", "phone"]), json!(["+1"]))
        );
    }
    // CODE_DELIVERY=log: phone codes go to the development log, SMS or not.
    let log: Arc<dyn CodeSender> = Arc::new(LogSender);
    for sender in [log.clone(), with_sms(log)] {
        assert_eq!(
            meta(open(50, sender).await).await,
            (json!(["email", "phone"]), json!(["+1"]))
        );
    }
    // The countries are the deployment's.
    let served = AuthRules {
        phone_country_codes: vec!["1".to_owned(), "52".to_owned()],
        ..rules(50)
    };
    assert_eq!(
        meta(open_with(served, with_sms(smtp())).await).await,
        (json!(["email", "phone"]), json!(["+1", "+52"]))
    );
}

#[tokio::test]
async fn the_service_names_the_address_codes_come_from_where_it_has_one() {
    let _turn = TURN.lock().await;
    let sender = |app: App| async move {
        let meta = app
            .call(None, Method::GET, "/v1/meta", None, &[])
            .await
            .ok();
        meta.as_object().unwrap().get("code_sender").cloned()
    };
    let named = Arc::new(
        SmtpSender::new(
            SmtpSettings {
                host: "127.0.0.1".to_owned(),
                port: 9,
                tls: TlsMode::None,
                credentials: None,
                from: "Yuppers <No-Reply@example.test>".to_owned(),
                timeout: Duration::from_secs(1),
            },
            Wording::embedded().unwrap(),
        )
        .unwrap(),
    );

    // CODE_DELIVERY=smtp: the From address, and only its address, not the
    // display name in front of it.
    assert_eq!(
        sender(open(50, smtp()).await).await,
        Some(json!("no-reply@example.test"))
    );
    assert_eq!(
        sender(open(50, named.clone()).await).await,
        Some(json!("No-Reply@example.test"))
    );
    // With codes by text on too, codes for email addresses still come from
    // it.
    let routed: Arc<dyn CodeSender> = Arc::new(CodeRouter::new(
        named,
        PhoneCodes::Verify(Arc::new(Phone::default())),
    ));
    assert_eq!(
        sender(open(50, routed).await).await,
        Some(json!("No-Reply@example.test"))
    );
    // CODE_DELIVERY=log: no address to name, so the field is left out.
    assert_eq!(sender(open(50, Arc::new(LogSender)).await).await, None);
}

// ---- Twilio, against a stand-in ---------------------------------------------

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct Twilio {
    received: Arc<Mutex<Vec<Received>>>,
    answer: (StatusCode, Value),
}

impl Twilio {
    async fn start(status: StatusCode, body: Value) -> (Self, SocketAddr) {
        let twilio = Self {
            received: Arc::default(),
            answer: (status, body),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(answer).with_state(twilio.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (twilio, addr)
    }
}

async fn answer(
    State(twilio): State<Twilio>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    twilio.received.lock().unwrap().push(Received {
        path: uri.path().to_owned(),
        headers,
        body: String::from_utf8(body.to_vec()).unwrap(),
    });
    (twilio.answer.0, twilio.answer.1.to_string())
}

/// A sender signing in with the account's auth token.
fn twilio(addr: SocketAddr, from: &str) -> TwilioSmsSender {
    TwilioSmsSender::new(
        &format!("http://{addr}"),
        "AC0123456789abcdef".to_owned(),
        TwilioCredential::AuthToken(Secret::new("auth-token-not-for-logs".to_owned())),
        from.to_owned(),
        Duration::from_secs(5),
    )
}

/// Obviously fake SIDs of the shape Twilio gives.
const ACCOUNT_SID: &str = "AC00000000000000000000000000000000";
const KEY_SID: &str = "SK00000000000000000000000000000000";

/// A sender signing in with an API key.
fn twilio_with_key(addr: SocketAddr) -> TwilioSmsSender {
    TwilioSmsSender::new(
        &format!("http://{addr}"),
        ACCOUNT_SID.to_owned(),
        TwilioCredential::ApiKey {
            sid: KEY_SID.to_owned(),
            secret: Secret::new("api-key-secret-not-for-logs".to_owned()),
        },
        "+15550000000".to_owned(),
        Duration::from_secs(5),
    )
}

/// The username and password of an HTTP Basic `Authorization` header.
fn basic(request: &Received) -> (String, String) {
    let value = request.headers["authorization"].to_str().unwrap();
    let encoded = value.strip_prefix("Basic ").expect("HTTP Basic");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let decoded = String::from_utf8(decoded).unwrap();
    let (username, password) = decoded.split_once(':').unwrap();
    (username.to_owned(), password.to_owned())
}

#[tokio::test]
async fn with_an_api_key_twilio_is_sent_the_key_as_basic_auth_and_the_account_in_the_path() {
    let (stand_in, addr) = Twilio::start(
        StatusCode::CREATED,
        json!({ "sid": "SM0123", "status": "queued" }),
    )
    .await;
    twilio_with_key(addr)
        .send(Sms {
            to: "+15551234567",
            text: "Yuppers.app: an agreement you turned on updates for has changed. See it: https://yuppers.app/exchanges/1. Reply STOP to opt out.",
        })
        .await
        .unwrap();
    // And with the auth token, as before: the account SID is the username.
    twilio(addr, "+15550000000")
        .send(Sms {
            to: "+15551234567",
            text: "654321",
        })
        .await
        .unwrap();

    let received = stand_in.received.lock().unwrap().clone();
    assert_eq!(received.len(), 2);
    assert_eq!(
        received[0].path,
        format!("/2010-04-01/Accounts/{ACCOUNT_SID}/Messages.json")
    );
    assert_eq!(
        basic(&received[0]),
        (KEY_SID.to_owned(), "api-key-secret-not-for-logs".to_owned())
    );
    assert_eq!(
        received[1].path,
        "/2010-04-01/Accounts/AC0123456789abcdef/Messages.json"
    );
    assert_eq!(
        basic(&received[1]),
        (
            "AC0123456789abcdef".to_owned(),
            "auth-token-not-for-logs".to_owned()
        )
    );
    // The form is the same either way.
    for request in &received {
        assert!(
            request
                .body
                .starts_with("To=%2B15551234567&From=%2B15550000000&Body=")
        );
    }
}

#[tokio::test]
async fn a_refusal_to_an_api_key_names_neither_the_key_secret_nor_the_number() {
    let (_stand_in, addr) = Twilio::start(
        StatusCode::UNAUTHORIZED,
        json!({ "code": 20003, "message": "Authenticate", "status": 401 }),
    )
    .await;
    let error = twilio_with_key(addr)
        .send(Sms {
            to: "+15551234567",
            text: "123456",
        })
        .await
        .unwrap_err();
    let error = format!("{error:#}");
    assert_eq!(
        error,
        "the SMS provider refused the message (HTTP 401, error 20003)"
    );
    assert!(!error.contains("api-key-secret"));
    assert!(!error.contains("5551234567"));
}

#[tokio::test]
async fn twilio_is_sent_the_message_in_the_shape_its_api_takes() {
    let (stand_in, addr) = Twilio::start(
        StatusCode::CREATED,
        json!({ "sid": "SM0123", "status": "queued" }),
    )
    .await;
    let sender = twilio(addr, "+15550000000");
    let text = "Yuppers.app: hubo un cambio en un acuerdo que sigues. Velo: https://yuppers.app/exchanges/1. Responde STOP para cancelar.";
    sender
        .send(Sms {
            to: "+15551234567",
            text,
        })
        .await
        .unwrap();

    let received = stand_in.received.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    let request = &received[0];
    assert_eq!(
        request.path,
        "/2010-04-01/Accounts/AC0123456789abcdef/Messages.json"
    );
    // HTTP Basic, the account SID and the auth token.
    assert_eq!(
        request.headers["authorization"],
        "Basic QUMwMTIzNDU2Nzg5YWJjZGVmOmF1dGgtdG9rZW4tbm90LWZvci1sb2dz"
    );
    assert_eq!(
        request.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    let form: Vec<(String, String)> = form_urlencoded::parse(request.body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(
        form,
        [
            ("To".to_owned(), "+15551234567".to_owned()),
            ("From".to_owned(), "+15550000000".to_owned()),
            ("Body".to_owned(), text.to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_refusal_from_twilio_is_described_by_its_code_without_the_number() {
    let (_stand_in, addr) = Twilio::start(
        StatusCode::BAD_REQUEST,
        json!({
            "code": 21211,
            "message": "The 'To' number +15551234567 is not a valid phone number.",
            "status": 400,
        }),
    )
    .await;
    let error = twilio(addr, "MG0123")
        .send(Sms {
            to: "+15551234567",
            text: "123456",
        })
        .await
        .unwrap_err();
    let error = format!("{error:#}");
    assert_eq!(
        error,
        "the SMS provider refused the message (HTTP 400, error 21211)"
    );
    assert!(!error.contains("5551234567"));
}

// ---- Logs -------------------------------------------------------------------

/// Everything logged, as bytes.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

#[tokio::test]
async fn no_log_holds_a_whole_phone_number_and_only_the_development_delivery_holds_the_code() {
    let turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber =
        telemetry::subscriber(LogFormat::Text, EnvFilter::new("trace"), false, move || {
            writer.clone()
        });
    let _guard = tracing::subscriber::set_default(subscriber);
    let _turn = turn;

    let number = "+15551234567";
    let digits = "5551234567";

    // The development deliveries, the number masked: an update text
    // (`SMS_DELIVERY=log`), and a code for a phone number
    // (`SMS_CODE_DELIVERY=log`), which is there to be read.
    LogSmsSender
        .send(Sms {
            to: number,
            text: "Yuppers.app: an agreement you turned on updates for has changed. See it: https://yuppers.app/exchanges/111111. Reply STOP to opt out.",
        })
        .await
        .unwrap();
    let phone = Identifier::parse(number).unwrap();
    let codes = CodeRouter::new(Arc::new(Mailbox::default()), PhoneCodes::Log);
    assert!(codes.verifier(&phone).is_none());
    CodeSender::send(
        &codes,
        CodeMessage {
            to: &phone,
            code: "222222",
            purpose: Purpose::SignIn,
            language: "en",
        },
    )
    .await
    .unwrap();
    let development = log.text();
    assert!(development.contains("111111") && development.contains("222222"));
    assert!(development.contains("+1••••••••67"), "{development}");
    assert!(!development.contains(digits), "{development}");
}
