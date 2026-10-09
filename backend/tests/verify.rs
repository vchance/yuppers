//! One-time codes for phone numbers through Twilio Verify
//! (`SMS_CODE_DELIVERY=verify`), against a stand-in for Verify's API in this
//! process that behaves as Twilio's documentation says: a verification is
//! started per service and number, asking again within its life resends the
//! same code, a check answers `approved` or `pending`, and an approved,
//! expired or exhausted verification is gone (`404`).
//!
//! Checked here: the requests' shape and credentials; a right code signing
//! in, once; a wrong one charged against the service's own limits as any
//! code is; a code for one purpose refused for another; expiry, ours and
//! Twilio's; the consent box and the STOP refusal still applying; the
//! hourly caps counting Verify's texts with the agreement updates'; a
//! refusal or silence from Twilio; the development log in Verify's place;
//! and agreement updates going on through the Messages API beside it.
//!
//! The caps count every text the database has seen this hour, so the tests
//! here take turns, and each starts with the counts cleared.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use base64::Engine as _;
use common::{App, User};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::sync::MutexGuard;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use yuppers_backend::auth::{
    AuthRules, CodeMessage, CodeSender, CodeVerifier, Purpose, SendFuture, SmsPlace,
};
use yuppers_backend::domain::identity::Identifier;
use yuppers_backend::metrics::{self, Text};
use yuppers_backend::notifications::outbox::DeliveryRules;
use yuppers_backend::notifications::sms::{
    CodeRouter, PhoneCodes, TwilioCredential, TwilioSmsSender,
};
use yuppers_backend::notifications::sms_updates::{
    self, CONSENT_VERSION, DEFAULT_TEXTS_PER_PERSON_PER_DAY, SmsDelivery,
};
use yuppers_backend::notifications::smtp::Secret;
use yuppers_backend::notifications::verify::{TwilioVerify, VerifyServices};
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::telemetry::{self, LogFormat};

const DATABASE: &str = "yuppers_test_verify";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Obviously fake SIDs of the shapes Twilio gives.
const ACCOUNT_SID: &str = "AC00000000000000000000000000000000";
const KEY_SID: &str = "SK00000000000000000000000000000000";
const SIGN_IN: &str = "VA00000000000000000000000000000001";
const DELETION: &str = "VA00000000000000000000000000000002";
const KEY_SECRET: &str = "api-key-secret-not-for-logs";

/// What the service signs its requests with, here: `APP_SECRET` in
/// `tests/common`.
const APP_SECRET: &[u8] = b"test-secret-test-secret-test-secret";

/// The auth token Twilio signs the webhook's requests with, here.
const WEBHOOK_TOKEN: &str = "test-auth-token";

/// A number nobody else in these tests uses.
fn number() -> String {
    format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000)
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

// ---- The stand-in -------------------------------------------------------------

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    form: Vec<(String, String)>,
}

impl Received {
    fn field(&self, name: &str) -> Option<&str> {
        self.form
            .iter()
            .find(|(each, _)| each == name)
            .map(|(_, value)| value.as_str())
    }

    /// The username and password of its HTTP Basic `Authorization`.
    fn basic(&self) -> (String, String) {
        let value = self.headers["authorization"].to_str().unwrap();
        let encoded = value.strip_prefix("Basic ").expect("HTTP Basic");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        let (username, password) = decoded.split_once(':').unwrap();
        (username.to_owned(), password.to_owned())
    }
}

/// A verification pending at the stand-in.
#[derive(Clone, Debug)]
struct Pending {
    code: String,
    failed_checks: u32,
}

#[derive(Default)]
struct Verify {
    received: Mutex<Vec<Received>>,
    /// By service and number.
    pending: Mutex<HashMap<(String, String), Pending>>,
    /// Texts the Messages API was handed: number and text.
    messages: Mutex<Vec<(String, String)>>,
    /// An answer to every request instead of Verify's own, while set.
    refusing: Mutex<Option<(StatusCode, Value)>>,
    /// How long a check takes to be answered, while set.
    slow: Mutex<Option<Duration>>,
}

#[derive(Clone)]
struct StandIn(Arc<Verify>);

impl StandIn {
    async fn start() -> (Self, SocketAddr) {
        let stand_in = Self(Arc::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(answer).with_state(stand_in.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (stand_in, addr)
    }

    fn received(&self) -> Vec<Received> {
        self.0.received.lock().unwrap().clone()
    }

    /// The requests made to `resource` of `service`.
    fn to(&self, service: &str, resource: &str) -> Vec<Received> {
        let path = format!("/v2/Services/{service}/{resource}");
        self.received()
            .into_iter()
            .filter(|request| request.path == path)
            .collect()
    }

    /// The code pending at `service` for `phone`: what the text said.
    fn code(&self, service: &str, phone: &str) -> String {
        self.0.pending.lock().unwrap()[&(service.to_owned(), phone.to_owned())]
            .code
            .clone()
    }

    /// Forgets a verification, as Twilio does once it expires.
    fn expire(&self, service: &str, phone: &str) {
        self.0
            .pending
            .lock()
            .unwrap()
            .remove(&(service.to_owned(), phone.to_owned()));
    }

    fn refuse(&self, answer: Option<(StatusCode, Value)>) {
        *self.0.refusing.lock().unwrap() = answer;
    }

    /// Has every check take `delay` to be answered.
    fn slow_down(&self, delay: Option<Duration>) {
        *self.0.slow.lock().unwrap() = delay;
    }
}

async fn answer(
    State(stand_in): State<StandIn>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    let verify = &stand_in.0;
    let path = uri.path().to_owned();
    let form: Vec<(String, String)> = form_urlencoded::parse(&body).into_owned().collect();
    let request = Received {
        path: path.clone(),
        headers,
        form,
    };
    verify.received.lock().unwrap().push(request.clone());
    let delay = *verify.slow.lock().unwrap();
    if let Some(delay) = delay.filter(|_| path.ends_with("/VerificationCheck")) {
        tokio::time::sleep(delay).await;
    }
    if let Some((status, body)) = verify.refusing.lock().unwrap().clone() {
        return (status, body.to_string());
    }
    let to = request.field("To").unwrap_or_default().to_owned();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["v2", "Services", service, "Verifications"] => {
            let mut pending = verify.pending.lock().unwrap();
            // The same code while one is pending, as Twilio's docs say.
            let code = pending
                .entry((service.to_string(), to.clone()))
                .or_insert_with(|| Pending {
                    code: format!("{:06}", Uuid::new_v4().as_u128() % 1_000_000),
                    failed_checks: 0,
                })
                .code
                .clone();
            assert_eq!(code.len(), 6);
            (
                StatusCode::CREATED,
                json!({ "sid": "VE0", "service_sid": service, "to": to, "channel": "sms", "status": "pending" })
                    .to_string(),
            )
        }
        ["v2", "Services", service, "VerificationCheck"] => {
            let mut pending = verify.pending.lock().unwrap();
            let key = (service.to_string(), to.clone());
            let Some(found) = pending.get_mut(&key) else {
                return (
                    StatusCode::NOT_FOUND,
                    json!({ "code": 20404, "message": "The requested resource was not found", "status": 404 })
                        .to_string(),
                );
            };
            if found.failed_checks >= 5 {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    json!({ "code": 60202, "message": "Max check attempts reached", "status": 429 })
                        .to_string(),
                );
            }
            let status = if request.field("Code") == Some(found.code.as_str()) {
                pending.remove(&key);
                "approved"
            } else {
                found.failed_checks += 1;
                "pending"
            };
            (
                StatusCode::OK,
                json!({ "sid": "VE0", "to": to, "status": status, "valid": status == "approved" })
                    .to_string(),
            )
        }
        ["2010-04-01", "Accounts", _, "Messages.json"] => {
            let text = request.field("Body").unwrap_or_default().to_owned();
            verify.messages.lock().unwrap().push((to, text));
            (
                StatusCode::CREATED,
                json!({ "sid": "SM0", "status": "queued" }).to_string(),
            )
        }
        _ => (StatusCode::NOT_FOUND, "{}".to_owned()),
    }
}

fn credential() -> TwilioCredential {
    TwilioCredential::ApiKey {
        sid: KEY_SID.to_owned(),
        secret: Secret::new(KEY_SECRET.to_owned()),
    }
}

/// Twilio Verify at the stand-in, signing in with `credential`.
fn verify_with(addr: SocketAddr, credential: TwilioCredential) -> TwilioVerify {
    TwilioVerify::new(
        &format!("http://{addr}"),
        ACCOUNT_SID.to_owned(),
        credential,
        VerifyServices {
            sign_in: SIGN_IN.to_owned(),
            delete_account: DELETION.to_owned(),
        },
        Duration::from_secs(5),
    )
}

/// Keeps the email codes it is handed: who to and the code.
#[derive(Default)]
struct Mailbox(Mutex<Vec<(String, String)>>);

impl CodeSender for Mailbox {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push((message.to.as_str().to_owned(), message.code.to_owned()));
            Ok(())
        })
    }
}

/// `SMS_CODE_DELIVERY=verify` against the stand-in, email codes to a mailbox.
fn router(addr: SocketAddr) -> Arc<CodeRouter> {
    Arc::new(CodeRouter::new(
        Arc::new(Mailbox::default()),
        PhoneCodes::Verify(Arc::new(verify_with(addr, credential()))),
    ))
}

struct Test {
    _turn: MutexGuard<'static, ()>,
    app: App,
    twilio: StandIn,
    addr: SocketAddr,
}

async fn start_with(rules: AuthRules) -> Test {
    let turn = TURN.lock().await;
    let (twilio, addr) = StandIn::start().await;
    let app = App::start_texting(DATABASE, rules, router(addr), WEBHOOK_TOKEN).await;
    clear_counts(&app).await;
    Test {
        _turn: turn,
        app,
        twilio,
        addr,
    }
}

async fn start() -> Test {
    start_with(rules(50)).await
}

async fn clear_counts(app: &App) {
    sqlx::query("DELETE FROM sign_in_limit")
        .execute(&app.owner)
        .await
        .unwrap();
}

/// Asks for a sign-in code for `phone`, the box ticked.
async fn ask(app: &App, phone: &str, language: &str) -> common::Reply {
    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": phone, "sms_consent": common::sms_consent() })),
        &[("accept-language", language)],
    )
    .await
}

/// Signs in with `code`, by token.
async fn sign_in(app: &App, phone: &str, code: &str) -> common::Reply {
    app.call(
        None,
        Method::POST,
        "/v1/auth/sessions",
        Some(json!({ "identifier": phone, "code": code, "delivery": "TOKEN" })),
        &[],
    )
    .await
}

/// Asks for a deletion code to the account's number, the box ticked.
async fn ask_deletion(app: &App, user: &User) -> common::Reply {
    app.post(
        user,
        "/v1/me/deletion/codes",
        json!({ "channel": "PHONE", "sms_consent": common::sms_consent() }),
    )
    .await
}

async fn delete_with(app: &App, user: &User, code: &str) -> common::Reply {
    app.post(
        user,
        "/v1/me/deletion",
        json!({ "channel": "PHONE", "code": code }),
    )
    .await
}

/// A signed-in account whose number is `phone`.
async fn user_with_phone(app: &App, phone: &str) -> User {
    let user = app.user("Ana").await;
    common::set_phone(&app.owner, user.id, phone, false).await;
    user
}

/// The wrong guesses counted today against whatever `scope` counts.
async fn guesses(app: &App, scope: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(count), 0)::bigint FROM sign_in_limit
         WHERE scope = $1 AND window_start = date_trunc('day', now(), 'UTC')",
    )
    .bind(scope)
    .fetch_one(&app.owner)
    .await
    .unwrap()
}

/// The stored rows for an identifier: purpose, whether a hash is kept,
/// who checks it, wrong guesses, and whether it is used.
async fn rows(app: &App, identifier: &str) -> Vec<(String, bool, String, i16, bool)> {
    sqlx::query_as(
        "SELECT purpose, code_hash IS NOT NULL, checked_by, failed_attempts,
                consumed_at IS NOT NULL
         FROM one_time_code WHERE identifier_index = $1 ORDER BY created_at",
    )
    .bind(common::index(identifier))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

async fn counted(app: &App, cap: i64) -> String {
    let mut text = Text::new();
    metrics::render_sms(&mut text, &app.db, cap).await;
    text.finish()
}

// ---- The requests ---------------------------------------------------------------

#[tokio::test]
async fn verify_is_sent_requests_in_the_shape_its_api_takes_with_the_accounts_credentials() {
    let Test {
        _turn,
        app,
        twilio,
        addr,
    } = start().await;
    let (en, es) = (number(), number());
    assert_eq!(ask(&app, &en, "en").await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        ask(&app, &es, "es-MX,es;q=0.9").await.status,
        StatusCode::NO_CONTENT
    );

    // Starting a verification: the sign-in service, by text, in the
    // language as a supported tag; HTTP Basic with the API key.
    let started = twilio.to(SIGN_IN, "Verifications");
    assert_eq!(started.len(), 2);
    for (request, phone, language) in [(&started[0], &en, "en"), (&started[1], &es, "es")] {
        assert_eq!(
            request.form,
            [
                ("To".to_owned(), phone.clone()),
                ("Channel".to_owned(), "sms".to_owned()),
                ("Locale".to_owned(), language.to_owned()),
            ]
        );
        assert_eq!(request.basic(), (KEY_SID.to_owned(), KEY_SECRET.to_owned()));
        assert_eq!(
            request.headers["content-type"],
            "application/x-www-form-urlencoded"
        );
    }

    // Checking it: the same service, the number and the code.
    let code = twilio.code(SIGN_IN, &en);
    let reply = sign_in(&app, &en, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let checked = twilio.to(SIGN_IN, "VerificationCheck");
    assert_eq!(checked.len(), 1);
    assert_eq!(
        checked[0].form,
        [
            ("To".to_owned(), en.clone()),
            ("Code".to_owned(), code.clone()),
        ]
    );
    assert_eq!(
        checked[0].basic(),
        (KEY_SID.to_owned(), KEY_SECRET.to_owned())
    );
    // Nothing went to the deletion service, or to the Messages API.
    assert!(twilio.to(DELETION, "Verifications").is_empty());
    assert!(twilio.0.messages.lock().unwrap().is_empty());

    // With the auth token instead, the account SID is the username.
    let token = verify_with(
        addr,
        TwilioCredential::AuthToken(Secret::new("auth-token-not-for-logs".to_owned())),
    );
    token.start(&es, Purpose::SignIn, "es").await.unwrap();
    let last = twilio.received().pop().unwrap();
    assert_eq!(last.path, format!("/v2/Services/{SIGN_IN}/Verifications"));
    assert_eq!(
        last.basic(),
        (ACCOUNT_SID.to_owned(), "auth-token-not-for-logs".to_owned())
    );
}

// ---- Checking codes -----------------------------------------------------------

#[tokio::test]
async fn a_right_code_signs_in_once_and_the_service_keeps_no_hash_of_a_code_it_never_saw() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();
    ask(&app, &phone, "en").await;
    // Asked for, with no hash: Verify made the code.
    assert_eq!(
        rows(&app, &phone).await,
        [(
            "sign-in".to_owned(),
            false,
            "TWILIO_VERIFY".to_owned(),
            0,
            false
        )]
    );
    // Asking again within its life gets the same code, as Twilio resends it;
    // both requests are counted.
    ask(&app, &phone, "en").await;
    assert_eq!(rows(&app, &phone).await.len(), 2);
    let code = twilio.code(SIGN_IN, &phone);

    let reply = sign_in(&app, &phone, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["account"]["phone"], phone);
    // Used: every live request for it, once.
    assert!(rows(&app, &phone).await.iter().all(|row| row.4));
    // Again, refused, without asking Twilio: nothing is live.
    let checks = twilio.to(SIGN_IN, "VerificationCheck").len();
    sign_in(&app, &phone, &code)
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), checks);
    assert_eq!(guesses(&app, "failed-guesses-by-identifier").await, 0);
}

#[tokio::test]
async fn a_wrong_code_is_charged_against_the_services_limits_as_any_code_is() {
    let Test {
        _turn, app, twilio, ..
    } = start_with(AuthRules {
        failed_guesses_per_identifier_per_day: 3,
        ..rules(50)
    })
    .await;
    let phone = number();
    ask(&app, &phone, "en").await;
    let code = twilio.code(SIGN_IN, &phone);
    let wrong = if code == "000000" { "111111" } else { "000000" };

    // Each wrong code is put to Twilio, refused, and counted: against the
    // code and against the number's day.
    for guess in 1..=2 {
        sign_in(&app, &phone, wrong)
            .await
            .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
        assert_eq!(rows(&app, &phone).await[0].3, guess);
        assert_eq!(
            guesses(&app, "failed-guesses-by-identifier").await,
            i64::from(guess)
        );
    }
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), 2);
    // Something Twilio could never have made is wrong without asking it,
    // and charged all the same.
    sign_in(&app, &phone, "not a code")
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), 2);
    assert_eq!(guesses(&app, "failed-guesses-by-identifier").await, 3);

    // The day's wrong guesses used up: even the right code is refused, and
    // Twilio is not asked, nor is a new code sent.
    sign_in(&app, &phone, &code)
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_GUESSES");
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), 2);
    ask(&app, &phone, "en")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_GUESSES");
    assert_eq!(twilio.to(SIGN_IN, "Verifications").len(), 1);
}

#[tokio::test]
async fn a_code_that_had_its_wrong_guesses_is_dead_whatever_twilio_would_say() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();
    ask(&app, &phone, "en").await;
    let code = twilio.code(SIGN_IN, &phone);
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..AuthRules::default().code_max_failed_attempts {
        sign_in(&app, &phone, wrong)
            .await
            .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    }
    sign_in(&app, &phone, &code)
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    // The last was not put to Twilio: nothing was live to check.
    assert_eq!(
        twilio.to(SIGN_IN, "VerificationCheck").len(),
        AuthRules::default().code_max_failed_attempts as usize
    );
}

#[tokio::test]
async fn a_code_for_one_purpose_is_refused_for_another() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();
    let ana = user_with_phone(&app, &phone).await;

    // A sign-in code, offered to delete the account: refused without
    // asking Twilio, since no deletion code is live, and nothing charged.
    ask(&app, &phone, "en").await;
    let sign_in_code = twilio.code(SIGN_IN, &phone);
    delete_with(&app, &ana, &sign_in_code)
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    assert!(twilio.to(DELETION, "VerificationCheck").is_empty());
    assert_eq!(guesses(&app, "failed-guesses-by-account").await, 0);

    // With a deletion code live too, from the deletion service, the
    // sign-in code is put to that service only, which refuses it, and it
    // is charged against the account as a wrong deletion code.
    assert_eq!(
        ask_deletion(&app, &ana).await.status,
        StatusCode::NO_CONTENT
    );
    let deletion_code = twilio.code(DELETION, &phone);
    assert_eq!(twilio.to(DELETION, "Verifications").len(), 1);
    if deletion_code != sign_in_code {
        delete_with(&app, &ana, &sign_in_code)
            .await
            .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
        assert_eq!(twilio.to(DELETION, "VerificationCheck").len(), 1);
        assert_eq!(guesses(&app, "failed-guesses-by-account").await, 1);
        // And the deletion code does not sign in.
        sign_in(&app, &phone, &deletion_code)
            .await
            .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
        assert_eq!(
            twilio.to(SIGN_IN, "VerificationCheck")[0].field("Code"),
            Some(deletion_code.as_str())
        );
    }
    assert!(
        twilio
            .to(SIGN_IN, "VerificationCheck")
            .iter()
            .all(|request| request.field("To") == Some(phone.as_str()))
    );

    // Each still does what it was sent for.
    let reply = sign_in(&app, &phone, &sign_in_code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let reply = delete_with(&app, &ana, &deletion_code).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
}

/// Twilio forgets a verification once it approves it, so a deletion that
/// found the account busy and gave up, with the code still to work for a
/// second try, must recognise that code without Twilio: the approved code's
/// keyed hash is kept with its request until the code is used or expires.
#[tokio::test]
async fn an_approved_deletion_code_still_works_after_a_try_that_did_not_go_through() {
    let Test {
        _turn,
        app,
        twilio,
        addr,
    } = start().await;
    let phone = number();
    let ana = user_with_phone(&app, &phone).await;
    ask_deletion(&app, &ana).await;
    let code = twilio.code(DELETION, &phone);

    // The first try: Twilio approves, and then (here, by not going on) the
    // deletion does not happen.
    let verifier = verify_with(addr, credential());
    let rules = rules(50);
    let offered = yuppers_backend::auth::OfferedCode {
        secret: APP_SECRET,
        rules: &rules,
        identifier: &Identifier::parse(&phone).unwrap(),
        code: &code,
        requester: yuppers_backend::auth::Requester::DeleteAccount { account: ana.id },
        verifier: Some(&verifier),
    };
    offered.consult_verifier(&app.db).await.unwrap();
    assert_eq!(twilio.to(DELETION, "VerificationCheck").len(), 1);
    assert_eq!(
        rows(&app, &phone).await,
        [(
            "delete-account".to_owned(),
            true,
            "TWILIO_VERIFY".to_owned(),
            0,
            false
        )]
    );

    // The second: the same code works, and Twilio, which would now answer
    // 404, is not asked again.
    let reply = delete_with(&app, &ana, &code).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    assert_eq!(twilio.to(DELETION, "VerificationCheck").len(), 1);
}

#[tokio::test]
async fn an_expired_code_is_refused_whichever_side_it_expired_on() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;

    // Ours: refused without asking Twilio, and nothing to charge.
    let phone = number();
    ask(&app, &phone, "en").await;
    let code = twilio.code(SIGN_IN, &phone);
    sqlx::query(
        "UPDATE one_time_code SET expires_at = now() - interval '1 second' WHERE identifier_index = $1",
    )
    .bind(common::index(&phone))
    .execute(&app.owner)
    .await
    .unwrap();
    sign_in(&app, &phone, &code)
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    assert!(twilio.to(SIGN_IN, "VerificationCheck").is_empty());

    // Twilio's, while ours is live (its life is set on its side): Twilio
    // says it is gone, and the code is refused and charged as a wrong one.
    let other = number();
    ask(&app, &other, "en").await;
    let code = twilio.code(SIGN_IN, &other);
    twilio.expire(SIGN_IN, &other);
    sign_in(&app, &other, &code)
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), 1);
    assert_eq!(rows(&app, &other).await[0].3, 1);
}

/// Should Twilio fail to answer, a code is neither taken nor charged: the
/// person can try it again.
#[tokio::test]
async fn a_check_twilio_cannot_answer_is_neither_taken_nor_charged() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();
    ask(&app, &phone, "en").await;
    let code = twilio.code(SIGN_IN, &phone);
    twilio.refuse(Some((
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "code": 20500, "message": "Internal Server Error" }),
    )));
    sign_in(&app, &phone, &code)
        .await
        .refused(StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE");
    assert_eq!(rows(&app, &phone).await[0].3, 0);
    assert_eq!(guesses(&app, "failed-guesses-by-identifier").await, 0);
    twilio.refuse(None);
    let reply = sign_in(&app, &phone, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
}

/// Runs the futures at the same time and gives their outputs in order.
async fn join_all<F: std::future::Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut futures: Vec<_> = futures.map(|future| Some(Box::pin(future))).collect();
    let mut outputs: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (slot, output) in futures.iter_mut().zip(outputs.iter_mut()) {
            if let Some(future) = slot {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(value) => {
                        *output = Some(value);
                        *slot = None;
                    }
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    outputs.into_iter().map(Option::unwrap).collect()
}

/// Waiting for Twilio holds no database connection: guesses at more numbers
/// than the pool has connections leave the service free to answer
/// meanwhile. Each guess has a place in its number's day while it is with
/// Twilio, and counts once in the end.
#[tokio::test]
async fn guesses_waiting_for_twilio_hold_no_database_connection() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let ana = app.user("Ana").await;
    // Twice the test pool's four connections.
    let phones: Vec<String> = (0..8).map(|_| number()).collect();
    for phone in &phones {
        ask(&app, phone, "en").await;
    }
    let wrong: Vec<&str> = phones
        .iter()
        .map(|phone| {
            if twilio.code(SIGN_IN, phone) == "000000" {
                "111111"
            } else {
                "000000"
            }
        })
        .collect();
    twilio.slow_down(Some(Duration::from_millis(1500)));

    let guessing = join_all(
        phones
            .iter()
            .zip(&wrong)
            .map(|(phone, wrong)| sign_in(&app, phone, wrong)),
    );
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let asked = std::time::Instant::now();
        // Needs the database: the session is looked up.
        let me = app.get(&ana, "/v1/me").await;
        assert_eq!(me.status, StatusCode::OK, "{:?}", me.body);
        let waited = asked.elapsed();
        (waited, guesses(&app, "failed-guesses-by-identifier").await)
    };
    let (replies, (waited, while_asking)) = tokio::join!(guessing, meanwhile);
    assert!(
        waited < Duration::from_millis(1000),
        "the service waited {waited:?} for a connection"
    );
    // Each guess with Twilio held a place in its number's day.
    assert_eq!(while_asking, 8);
    for reply in replies {
        reply.refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    }
    // Once answered, each place was given back and the wrong guess counted.
    assert_eq!(guesses(&app, "failed-guesses-by-identifier").await, 8);
    twilio.slow_down(None);
}

/// Guesses at one number at the same moment go to Twilio no more often
/// than the number's day has wrong guesses left.
#[tokio::test]
async fn guesses_at_once_go_to_twilio_no_more_than_the_day_allows() {
    let Test {
        _turn, app, twilio, ..
    } = start_with(AuthRules {
        failed_guesses_per_identifier_per_day: 2,
        ..rules(50)
    })
    .await;
    let phone = number();
    ask(&app, &phone, "en").await;
    let code = twilio.code(SIGN_IN, &phone);
    let wrong = if code == "000000" { "111111" } else { "000000" };
    twilio.slow_down(Some(Duration::from_millis(200)));

    let replies = join_all((0..5).map(|_| sign_in(&app, &phone, wrong))).await;
    let mut codes: Vec<String> = replies
        .iter()
        .map(|reply| reply.code().to_owned())
        .collect();
    codes.sort();
    assert_eq!(
        codes,
        [
            "INVALID_CODE",
            "INVALID_CODE",
            "TOO_MANY_GUESSES",
            "TOO_MANY_GUESSES",
            "TOO_MANY_GUESSES"
        ]
    );
    assert_eq!(twilio.to(SIGN_IN, "VerificationCheck").len(), 2);
    assert_eq!(guesses(&app, "failed-guesses-by-identifier").await, 2);
    twilio.slow_down(None);
}

/// Twilio resends a pending verification and keeps its first expiry, so a
/// request it resends works no longer here than there; once that one has
/// expired, a new code has its full ten minutes.
#[tokio::test]
async fn a_code_resent_works_only_until_the_first_one_expires() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();
    ask(&app, &phone, "en").await;
    // Four minutes on.
    sqlx::query(
        "UPDATE one_time_code SET created_at = created_at - interval '4 minutes',
                                  expires_at = expires_at - interval '4 minutes'
         WHERE identifier_index = $1",
    )
    .bind(common::index(&phone))
    .execute(&app.owner)
    .await
    .unwrap();
    ask(&app, &phone, "en").await;
    let expiries = || async {
        sqlx::query_scalar::<_, OffsetDateTime>(
            "SELECT expires_at FROM one_time_code WHERE identifier_index = $1 ORDER BY created_at",
        )
        .bind(common::index(&phone))
        .fetch_all(&app.owner)
        .await
        .unwrap()
    };
    let resent = expiries().await;
    assert_eq!(resent.len(), 2);
    assert_eq!(resent[1], resent[0], "the same code, until the same time");
    let left = resent[1] - OffsetDateTime::now_utc();
    assert!(
        left < time::Duration::minutes(7) && left > time::Duration::minutes(5),
        "{left}"
    );

    // Once they have expired, here and at Twilio, the next is a new code
    // with ten minutes of its own.
    sqlx::query("UPDATE one_time_code SET expires_at = now() WHERE identifier_index = $1")
        .bind(common::index(&phone))
        .execute(&app.owner)
        .await
        .unwrap();
    twilio.expire(SIGN_IN, &phone);
    ask(&app, &phone, "en").await;
    let left = *expiries().await.last().unwrap() - OffsetDateTime::now_utc();
    assert!(left > time::Duration::minutes(9), "{left}");
}

/// Asking whether a number replied STOP costs a request like any other, so
/// nobody can ask about number after number without limit.
#[tokio::test]
async fn asking_for_a_code_for_a_number_that_replied_stop_is_counted() {
    let Test {
        _turn, app, twilio, ..
    } = start_with(AuthRules {
        code_requests_per_address_per_hour: 3,
        ..rules(50)
    })
    .await;
    let stopped: Vec<String> = (0..4).map(|_| number()).collect();
    for phone in &stopped {
        sqlx::query("INSERT INTO sms_opt_out (phone_index) VALUES ($1)")
            .bind(common::index(phone))
            .execute(&app.owner)
            .await
            .unwrap();
    }
    // Without the box ticked, that is what is said, whatever the number.
    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": stopped[0] })),
        &[],
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED");
    for phone in &stopped[..3] {
        ask(&app, phone, "en")
            .await
            .refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");
    }
    // The address has asked its hour's worth.
    ask(&app, &stopped[3], "en")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    assert!(twilio.received().is_empty());
}

// ---- What still applies -------------------------------------------------------

#[tokio::test]
async fn the_box_and_the_stop_refusal_still_come_before_anything_is_sent() {
    let Test {
        _turn, app, twilio, ..
    } = start().await;
    let phone = number();

    // No box ticked: refused, and Twilio is never asked.
    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": phone })),
        &[],
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED");
    let ana = user_with_phone(&app, &phone).await;
    app.post(&ana, "/v1/me/deletion/codes", json!({ "channel": "PHONE" }))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED");

    // A number that replied STOP to the agreement updates' number: refused
    // codes too, with an answer pointing to email.
    sqlx::query("INSERT INTO sms_opt_out (phone_index) VALUES ($1)")
        .bind(common::index(&phone))
        .execute(&app.owner)
        .await
        .unwrap();
    ask(&app, &phone, "en")
        .await
        .refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");
    ask_deletion(&app, &ana)
        .await
        .refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");

    assert!(twilio.received().is_empty());
    assert!(rows(&app, &phone).await.is_empty());
    // A ticked box is recorded with the code it led to, as before.
    let other = number();
    ask(&app, &other, "en").await;
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sms_code_consent WHERE purpose = 'SIGN_IN' AND phone_hash = $1",
    )
    .bind(yuppers_backend::code_consent::phone_hash(APP_SECRET, &other).as_slice())
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(recorded, 1);
}

#[tokio::test]
async fn verifys_texts_count_against_the_hourly_cap_with_the_agreement_updates() {
    let Test {
        _turn, app, twilio, ..
    } = start_with(rules(2)).await;
    for _ in 0..2 {
        assert_eq!(
            ask(&app, &number(), "en").await.status,
            StatusCode::NO_CONTENT
        );
    }
    ask(&app, &number(), "en")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    assert_eq!(twilio.to(SIGN_IN, "Verifications").len(), 2);
    // An update text finds no place either: they share the cap.
    let mut conn = app.db.acquire().await.unwrap();
    let phone = Identifier::parse(&number()).unwrap();
    assert!(
        SmsPlace::take(&mut conn, APP_SECRET, &rules(2), &phone)
            .await
            .unwrap()
            .is_none()
    );
    let page = counted(&app, 2).await;
    for line in [
        "yuppers_sms_codes_this_hour{result=\"sent\"} 2",
        "yuppers_sms_codes_refused_this_hour{reason=\"hourly_cap\"} 2",
    ] {
        assert!(page.contains(line), "{line} in\n{page}");
    }
}

// ---- Twilio refusing ------------------------------------------------------------

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
async fn a_code_twilio_refuses_to_send_costs_no_place_and_is_logged_by_its_code_alone() {
    let turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Text,
        EnvFilter::new("trace"),
        false,
        move || writer.clone(),
        None,
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    let (twilio, addr) = StandIn::start().await;
    let one = AuthRules {
        sms_codes_per_prefix_per_hour: 1,
        ..rules(1)
    };
    let app = App::start_texting(DATABASE, one, router(addr), WEBHOOK_TOKEN).await;
    clear_counts(&app).await;
    let phone = "+19995550142";
    let digits = "9995550142";

    // Twilio refusing, as it does a landline.
    twilio.refuse(Some((
        StatusCode::BAD_REQUEST,
        json!({ "code": 60205, "message": format!("SMS is not supported by landline phone number {phone}") }),
    )));
    for _ in 0..3 {
        ask(&app, phone, "en")
            .await
            .refused(StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE");
    }
    assert_eq!(twilio.to(SIGN_IN, "Verifications").len(), 3);
    let page = counted(&app, 1).await;
    for line in [
        "yuppers_sms_codes_this_hour{result=\"sent\"} 0",
        "yuppers_sms_codes_this_hour{result=\"failed\"} 3",
    ] {
        assert!(page.contains(line), "{line} in\n{page}");
    }
    let logged = log.text();
    assert!(logged.contains("error 60205"), "{logged}");
    assert!(!logged.contains(digits), "{logged}");
    assert!(!logged.contains("landline"), "{logged}");
    assert!(!logged.contains(KEY_SECRET), "{logged}");

    // The place is still there for a code Twilio takes.
    twilio.refuse(None);
    assert_eq!(
        ask(&app, &number(), "en").await.status,
        StatusCode::NO_CONTENT
    );
    drop(turn);
}

// ---- The development log ------------------------------------------------------

#[tokio::test]
async fn in_development_the_code_is_written_to_the_log_and_counted_as_a_text() {
    let turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Text,
        EnvFilter::new("info"),
        false,
        move || writer.clone(),
        None,
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    // SMS_CODE_DELIVERY=log.
    let codes = Arc::new(CodeRouter::new(
        Arc::new(Mailbox::default()),
        PhoneCodes::Log,
    ));
    let app = App::start_texting(DATABASE, rules(50), codes, WEBHOOK_TOKEN).await;
    clear_counts(&app).await;
    let phone = number();

    assert_eq!(ask(&app, &phone, "es").await.status, StatusCode::NO_CONTENT);
    let logged = log.text();
    let masked = Identifier::parse(&phone).unwrap().masked();
    let line = logged
        .lines()
        .find(|line| {
            line.contains("one-time code (development delivery)") && line.contains(&masked)
        })
        .unwrap_or_else(|| panic!("no code for {masked} in\n{logged}"));
    assert!(!logged.contains(&phone[2..]), "{logged}");
    let code = line
        .split("code=")
        .nth(1)
        .map(|rest| rest.trim_start_matches('"').get(..6).unwrap().to_owned())
        .unwrap();
    // The service made it, so keeps its hash, and counted it as a text.
    assert_eq!(rows(&app, &phone).await[0].2, "SERVICE");
    assert!(
        counted(&app, 50)
            .await
            .contains("yuppers_sms_codes_this_hour{result=\"sent\"} 1")
    );
    let reply = sign_in(&app, &phone, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    drop(turn);
}

// ---- Agreement updates beside it ----------------------------------------------

#[tokio::test]
async fn agreement_updates_go_on_through_the_messages_api_beside_verify() {
    let Test {
        _turn,
        app,
        twilio,
        addr,
    } = start().await;
    sms_updates::configure(true, DEFAULT_TEXTS_PER_PERSON_PER_DAY);
    let deal = app.active().await;
    let phone = number();

    // Ben adds a number with a code from Verify.
    let reply = app
        .call(
            Some(&deal.ben),
            Method::POST,
            "/v1/auth/codes",
            Some(json!({ "identifier": phone, "sms_consent": common::sms_consent() })),
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    let code = twilio.code(SIGN_IN, &phone);
    let proof = common::own_proof(&app.db, deal.ben.id).await;
    app.post(
        &deal.ben,
        "/v1/me/identifiers",
        json!({ "identifier": phone, "code": code, "proof": proof }),
    )
    .await
    .ok();

    // Turns on updates, and Ana marks something delivered.
    let path = format!("/v1/exchanges/{}/sms-updates", deal.exchange);
    app.call(
        Some(&deal.ben),
        Method::PUT,
        &path,
        Some(json!({ "on": true, "consent": { "version": CONSENT_VERSION, "language": "en" } })),
        &[],
    )
    .await
    .ok();
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    // The worker sends them through the Messages API, from the updates'
    // number, with the program's wording.
    let delivery = SmsDelivery {
        sender: Some(Arc::new(TwilioSmsSender::new(
            &format!("http://{addr}"),
            ACCOUNT_SID.to_owned(),
            credential(),
            "+15550000000".to_owned(),
            Duration::from_secs(5),
        ))),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: DeliveryRules::default(),
        auth: rules(50),
        secret: APP_SECRET.to_vec(),
    };
    let delivered =
        sms_updates::deliver_sms_due_until(&app.db, &delivery, OffsetDateTime::now_utc(), || false)
            .await
            .unwrap();
    assert!(delivered.sent >= 2, "{delivered:?}");
    let wording = Wording::embedded().unwrap();
    let texts: Vec<String> = twilio
        .0
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|(to, _)| *to == phone)
        .map(|(_, text)| text.clone())
        .collect();
    assert_eq!(
        texts,
        [
            wording.opt_in_sms("en"),
            wording.update_sms(
                "en",
                &format!("https://app.test/exchanges/{}", deal.exchange)
            ),
        ]
    );
    // The Messages API carried no code, and Verify no update.
    assert!(texts.iter().all(|text| !text.contains(&code)));
    assert_eq!(twilio.to(SIGN_IN, "Verifications").len(), 1);
    // Both counted against the one hourly cap.
    assert!(
        counted(&app, 50)
            .await
            .contains("yuppers_sms_codes_this_hour{result=\"sent\"} 3")
    );
}
