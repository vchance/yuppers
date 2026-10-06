//! "Yuppers.app agreement updates" (`notifications::sms_updates`): turning
//! text updates on and off for an agreement and the record of consent it
//! leaves, the text queued for each status change and what it holds, the
//! daily cap and the hourly caps shared with codes, numbers that replied
//! STOP never texted, Twilio's signed webhook for STOP and START, and what
//! deleting an account or replacing its number does.
//!
//! Update texts are queued only while the process says texts are sent
//! (`sms_updates::configure`), which every test here does; with it off,
//! `tests/sms.rs` checks that nothing is queued. The hourly caps count every
//! text the database has seen, so the tests that send take turns.

mod common;

use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use common::{App, Deal, PEER, User};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;
use yuppers_backend::auth::{AuthRules, CodeMessage, CodeSender, SendFuture};
use yuppers_backend::domain::Rules;
use yuppers_backend::notifications::outbox::{Delivered, DeliveryRules};
use yuppers_backend::notifications::sms::{CodeRouter, Sms, SmsSender, twilio_signature};
use yuppers_backend::notifications::sms_updates::{
    self, CONSENT_VERSION, DEFAULT_TEXTS_PER_PERSON_PER_DAY, SmsDelivery,
};
use yuppers_backend::notifications::wording::Wording;

const DATABASE: &str = "yuppers_test_sms_updates";

/// The auth token Twilio signs the webhook's requests with, here.
const TOKEN: &str = "test-auth-token";

/// Where Twilio is told to post: the service's origin and the path.
const WEBHOOK: &str = "https://app.test/v1/sms/inbound";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A US number nobody else in these tests uses.
fn number() -> String {
    format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000)
}

/// Keeps the text messages it is handed.
#[derive(Default)]
struct Phone(Mutex<Vec<(String, String)>>);

impl Phone {
    fn sent(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap().clone()
    }
}

impl SmsSender for Phone {
    fn send<'a>(&'a self, sms: Sms<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push((sms.to.to_owned(), sms.text.to_owned()));
            Ok(())
        })
    }
}

/// Keeps the codes sent by email: who to and the code.
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

/// The rules: every test request comes from one address, and every
/// [`number`] begins alike, so those limits are out of the way.
fn rules() -> AuthRules {
    AuthRules {
        code_requests_per_address_per_hour: 1_000_000,
        sms_codes_per_prefix_per_hour: 1_000_000,
        ..AuthRules::default()
    }
}

struct Texting {
    /// This test's turn: the tests share one database, whose texts the
    /// worker's pass, the hourly caps and the retention purge all see.
    _turn: tokio::sync::MutexGuard<'static, ()>,
    app: App,
    /// Codes by text message.
    phone: Arc<Phone>,
    /// Codes by email.
    mailbox: Arc<Mailbox>,
}

async fn start() -> Texting {
    let turn = TURN.lock().await;
    sms_updates::configure(true, DEFAULT_TEXTS_PER_PERSON_PER_DAY);
    let (phone, mailbox) = (Arc::new(Phone::default()), Arc::new(Mailbox::default()));
    let router = Arc::new(CodeRouter::new(
        mailbox.clone(),
        phone.clone(),
        Wording::embedded().unwrap(),
    ));
    let app = App::start_texting(DATABASE, rules(), router, TOKEN).await;
    // Nothing an earlier test left behind is counted or sent.
    sqlx::query("DELETE FROM sign_in_limit WHERE scope LIKE 'sms-%'")
        .execute(&app.owner)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE outbox SET completed_at = now() WHERE kind = 'SMS' AND completed_at IS NULL",
    )
    .execute(&app.owner)
    .await
    .unwrap();
    Texting {
        _turn: turn,
        app,
        phone,
        mailbox,
    }
}

/// Gives an account a verified phone number, as adding one would.
async fn give_phone(app: &App, user: &User) -> String {
    let phone = number();
    sqlx::query("UPDATE account SET phone = $2 WHERE id = $1")
        .bind(user.id)
        .bind(&phone)
        .execute(&app.db)
        .await
        .unwrap();
    phone
}

fn path(deal: &Deal) -> String {
    format!("/v1/exchanges/{}/sms-updates", deal.exchange)
}

fn turn_on_body(language: &str) -> Value {
    json!({ "on": true, "consent": { "version": CONSENT_VERSION, "language": language } })
}

async fn set(app: &App, user: &User, deal: &Deal, body: Value) -> common::Reply {
    app.call(
        Some(user),
        Method::PUT,
        &path(deal),
        Some(body),
        &[
            ("x-client-version", "web/1.0.0"),
            ("user-agent", "Mozilla/5.0 (test)"),
        ],
    )
    .await
}

async fn turn_on(app: &App, user: &User, deal: &Deal) -> Value {
    set(app, user, deal, turn_on_body("en")).await.ok()
}

/// The texts queued for an account: event sequence and payload, oldest first.
async fn queued(db: &PgPool, account: Uuid) -> Vec<(Option<i64>, Value)> {
    sqlx::query_as(
        "SELECT event_sequence, payload FROM outbox
         WHERE kind = 'SMS' AND recipient_account_id = $1 ORDER BY id",
    )
    .bind(account)
    .fetch_all(db)
    .await
    .unwrap()
}

/// The consent records of a number: action, account, exchange, source,
/// keyword, version and language.
type Record = (
    String,
    Option<Uuid>,
    Option<Uuid>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn records(db: &PgPool, phone: &str) -> Vec<Record> {
    sqlx::query_as(
        "SELECT action, account_id, exchange_id, source, keyword, consent_version, consent_language
         FROM sms_consent WHERE phone = $1 ORDER BY id",
    )
    .bind(phone)
    .fetch_all(db)
    .await
    .unwrap()
}

fn exchange_uuid(deal: &Deal) -> Uuid {
    deal.exchange.parse().unwrap()
}

/// The worker's side, sending through `phone` under `auth`'s caps.
fn delivery(phone: &Arc<Phone>, auth: AuthRules) -> SmsDelivery {
    SmsDelivery {
        sender: Some(phone.clone()),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: DeliveryRules::default(),
        auth,
        secret: b"test-secret-test-secret-test-secret".to_vec(),
    }
}

async fn deliver(app: &App, delivery: &SmsDelivery) -> Delivered {
    sms_updates::deliver_sms_due_until(&app.db, delivery, OffsetDateTime::now_utc(), || false)
        .await
        .unwrap()
}

// ---- Turning updates on and off -----------------------------------------------

#[tokio::test]
async fn ticking_the_box_records_the_consent_and_queues_the_confirmation() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;
    let phone = give_phone(&app, &deal.ben).await;

    let before = app.get(&deal.ben, &path(&deal)).await.ok();
    assert_eq!(
        before,
        json!({
            "on": false,
            "available": true,
            "phone": phone,
            "opted_out": false,
            "consent_version": CONSENT_VERSION,
        })
    );

    let after = set(&app, &deal.ben, &deal, turn_on_body("es")).await.ok();
    assert_eq!(after["on"], true);

    // The account, the agreement, the number, the time, the wording's
    // version and language, and the client.
    let exchange = exchange_uuid(&deal);
    assert_eq!(
        records(&app.db, &phone).await,
        [(
            "OPT_IN".to_owned(),
            Some(deal.ben.id),
            Some(exchange),
            "WEB".to_owned(),
            None,
            Some(CONSENT_VERSION.to_owned()),
            Some("es".to_owned()),
        )]
    );
    let (at, address, agent): (OffsetDateTime, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT c.created_at, host(n.ip_address), n.user_agent
         FROM sms_consent c JOIN sms_consent_network n ON n.consent_id = c.id
         WHERE c.phone = $1",
    )
    .bind(&phone)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!((OffsetDateTime::now_utc() - at).whole_minutes() < 5);
    assert_eq!(address.as_deref(), Some(PEER.ip().to_string().as_str()));
    assert_eq!(agent.as_deref(), Some("Mozilla/5.0 (test)"));

    // The confirmation, queued with nothing in it but what it is.
    assert_eq!(
        queued(&app.db, deal.ben.id).await,
        [(None, json!({ "sms": "OPT_IN_CONFIRMATION" }))]
    );

    // Ticking it again changes nothing.
    set(&app, &deal.ben, &deal, turn_on_body("es")).await.ok();
    assert_eq!(records(&app.db, &phone).await.len(), 1);
    assert_eq!(queued(&app.db, deal.ben.id).await.len(), 1);

    // Unticking it turns updates off and is recorded beside the opt-in,
    // which stays.
    let off = set(&app, &deal.ben, &deal, json!({ "on": false }))
        .await
        .ok();
    assert_eq!(off["on"], false);
    let kept = records(&app.db, &phone).await;
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].0, "OPT_IN");
    assert_eq!(
        (kept[1].0.as_str(), kept[1].1, kept[1].2, kept[1].3.as_str()),
        ("OPT_OUT", Some(deal.ben.id), Some(exchange), "WEB")
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM sms_update WHERE account_id = $1")
        .bind(deal.ben.id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // Off again: nothing more is recorded.
    set(&app, &deal.ben, &deal, json!({ "on": false }))
        .await
        .ok();
    assert_eq!(records(&app.db, &phone).await.len(), 2);
}

#[tokio::test]
async fn updates_are_turned_on_only_by_a_party_with_a_number_that_may_be_texted() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;

    // No number yet.
    set(&app, &deal.ben, &deal, turn_on_body("en"))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let phone = give_phone(&app, &deal.ben).await;

    // The wording shown must be the current one, in a language there is.
    let mut old = turn_on_body("en");
    old["consent"]["version"] = json!("1999-01-01");
    set(&app, &deal.ben, &deal, old)
        .await
        .refused(StatusCode::CONFLICT, "CONSENT_OUTDATED");
    set(&app, &deal.ben, &deal, json!({ "on": true }))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    set(&app, &deal.ben, &deal, turn_on_body("tlh"))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    // Someone who is not a party sees no such agreement.
    let stranger = app.user("Cleo").await;
    give_phone(&app, &stranger).await;
    set(&app, &stranger, &deal, turn_on_body("en"))
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    app.get(&stranger, &path(&deal))
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    // A number that replied STOP is not texted.
    sqlx::query("INSERT INTO sms_opt_out (phone) VALUES ($1)")
        .bind(&phone)
        .execute(&app.db)
        .await
        .unwrap();
    let view = app.get(&deal.ben, &path(&deal)).await.ok();
    assert_eq!(view["opted_out"], true);
    set(&app, &deal.ben, &deal, turn_on_body("en"))
        .await
        .refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");

    // Nor is a number outside the countries served.
    let abroad = app.user("Dana").await;
    let deal = app.active_between(deal.ana.clone(), abroad).await;
    sqlx::query("UPDATE account SET phone = $2 WHERE id = $1")
        .bind(deal.ben.id)
        .bind(format!(
            "+44770{:07}",
            Uuid::new_v4().as_u128() % 10_000_000
        ))
        .execute(&app.db)
        .await
        .unwrap();
    set(&app, &deal.ben, &deal, turn_on_body("en"))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "PHONE_COUNTRY_NOT_SERVED");

    // A draft has nobody to hear from yet.
    let draft = app.draft(&deal.ana).await;
    give_phone(&app, &deal.ana).await;
    let reply = app
        .call(
            Some(&deal.ana),
            Method::PUT,
            &format!("/v1/exchanges/{draft}/sms-updates"),
            Some(turn_on_body("en")),
            &[],
        )
        .await;
    reply.refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let view = app
        .get(&deal.ana, &format!("/v1/exchanges/{draft}/sms-updates"))
        .await
        .ok();
    assert_eq!(view["available"], false);
}

#[tokio::test]
async fn the_service_says_it_texts_updates() {
    let Texting { _turn, app, .. } = start().await;
    let meta = app
        .call(None, Method::GET, "/v1/meta", None, &[])
        .await
        .ok();
    assert_eq!(meta["sms_updates"], true);
}

// ---- Sending ----------------------------------------------------------------

#[tokio::test]
async fn each_status_change_queues_one_text_that_names_nothing_agreed() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;
    let phone = give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    let confirmation = queued(&app.db, deal.ben.id).await;
    assert_eq!(confirmation.len(), 1);

    // Ana marks the repair delivered: Ben, who turned updates on, is texted;
    // Ana, who did it and has no updates, is not.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    let sequence: i64 =
        sqlx::query_scalar("SELECT max(sequence) FROM exchange_event WHERE exchange_id = $1")
            .bind(exchange_uuid(&deal))
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        queued(&app.db, deal.ben.id).await[1..],
        [(
            Some(sequence),
            json!({ "sms": "UPDATE", "notice": "DELIVERY_CLAIMED" })
        )]
    );
    assert!(queued(&app.db, deal.ana.id).await.is_empty());

    // A statement is not a status change: email and push only.
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "REQUEST_CLOSE", "note": "Done on my side." }),
    )
    .await
    .ok();
    let payloads: Vec<Value> = queued(&app.db, deal.ben.id)
        .await
        .into_iter()
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(
        payloads[1..],
        [
            json!({ "sms": "UPDATE", "notice": "DELIVERY_CLAIMED" }),
            json!({ "sms": "UPDATE", "notice": "CLOSE_REQUESTED" }),
        ]
    );
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "ADD_STATEMENT", "note": "Still waiting." }),
    )
    .await;
    assert_eq!(queued(&app.db, deal.ben.id).await.len(), 3);

    // Nothing queued holds a number, a name, a term, an amount or the code.
    let display_code: String =
        sqlx::query_scalar("SELECT display_code FROM exchange WHERE id = $1")
            .bind(exchange_uuid(&deal))
            .fetch_one(&app.db)
            .await
            .unwrap();
    for (_, payload) in queued(&app.db, deal.ben.id).await {
        let text = payload.to_string();
        for secret in [
            phone.as_str(),
            "Ana",
            "Ben",
            "fence",
            "40000",
            &display_code,
        ] {
            assert!(!text.contains(secret), "{text} holds {secret}");
        }
    }

    // The worker sends them, written now, to the number, in Ben's language.
    let phone_out = Arc::new(Phone::default());
    let delivered = deliver(&app, &delivery(&phone_out, rules())).await;
    assert_eq!((delivered.sent, delivered.dropped), (3, 0));
    let link = format!("https://app.test/exchanges/{}", deal.exchange);
    assert_eq!(
        phone_out.sent(),
        [
            (
                phone.clone(),
                "Yuppers.app: You're signed up for text updates about this agreement, one text \
                 per status change. Msg frequency varies. Msg & data rates may apply. Reply HELP \
                 for help, STOP to opt out."
                    .to_owned()
            ),
            (
                phone.clone(),
                format!(
                    "Yuppers.app: an agreement you turned on updates for has changed. See it: \
                     {link}. Reply STOP to opt out."
                )
            ),
            (
                phone.clone(),
                format!(
                    "Yuppers.app: an agreement you turned on updates for has changed. See it: \
                     {link}. Reply STOP to opt out."
                )
            ),
        ]
    );
    // Counted under the hourly cap, with codes.
    assert_eq!(sent_this_hour(&app).await, 3);
}

async fn sent_this_hour(app: &App) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(count), 0)::bigint FROM sign_in_limit
         WHERE scope = 'sms-sent' AND window_start = date_trunc('hour', now(), 'UTC')",
    )
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_person_is_queued_no_more_than_the_days_cap() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;
    give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    // The confirmation and as many more as make the day's cap.
    for _ in 1..DEFAULT_TEXTS_PER_PERSON_PER_DAY {
        sqlx::query(
            "INSERT INTO outbox (kind, recipient_account_id, exchange_id, payload, completed_at)
             VALUES ('SMS', $1, $2, '{\"sms\": \"UPDATE\", \"notice\": \"REVISION_SENT\"}', now())",
        )
        .bind(deal.ben.id)
        .bind(exchange_uuid(&deal))
        .execute(&app.db)
        .await
        .unwrap();
    }
    assert_eq!(
        queued(&app.db, deal.ben.id).await.len() as i64,
        DEFAULT_TEXTS_PER_PERSON_PER_DAY
    );
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    assert_eq!(
        queued(&app.db, deal.ben.id).await.len() as i64,
        DEFAULT_TEXTS_PER_PERSON_PER_DAY,
        "past the cap, a change is not texted"
    );
    // The email still goes.
    let emails: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE kind = 'EMAIL' AND recipient_account_id = $1
           AND payload->>'notice' = 'DELIVERY_CLAIMED'",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(emails, 1);
}

#[tokio::test]
async fn update_texts_wait_for_the_next_hour_once_the_hourly_cap_is_reached() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;
    give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    let phone = Arc::new(Phone::default());
    let capped = AuthRules {
        sms_codes_per_hour: 1,
        ..rules()
    };
    let delivered = deliver(&app, &delivery(&phone, capped)).await;
    assert_eq!(phone.sent().len(), 1, "one place under the cap");
    assert_eq!(delivered.sent, 1);
    let (available_at, error, attempts): (OffsetDateTime, Option<String>, i32) = sqlx::query_as(
        "SELECT available_at, last_error, attempts FROM outbox
         WHERE kind = 'SMS' AND recipient_account_id = $1 AND completed_at IS NULL",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(available_at > OffsetDateTime::now_utc());
    assert_eq!(available_at.minute(), 0);
    assert_eq!(
        error.as_deref(),
        Some("waiting: the hourly cap on text messages is reached")
    );
    assert_eq!(attempts, 0, "waiting is not a failure");
    let refused: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(count), 0)::bigint FROM sign_in_limit WHERE scope = 'sms-refused'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(refused, 1);
}

#[tokio::test]
async fn a_number_that_replied_stop_is_never_texted() {
    let Texting {
        _turn,
        app,
        phone: codes,
        ..
    } = start().await;
    let deal = app.active().await;
    let phone = give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    assert_eq!(queued(&app.db, deal.ben.id).await.len(), 2);

    // STOP arrives before the worker gets to them.
    inbound(&app, &phone, "STOP", None).await;
    let out = Arc::new(Phone::default());
    let delivered = deliver(&app, &delivery(&out, rules())).await;
    assert!(out.sent().is_empty());
    assert_eq!(delivered.dropped, 2);

    // Nothing more is queued, and no code is sent to it: the answer points
    // to email.
    let view = app.get(&deal.ben, &path(&deal)).await.ok();
    assert_eq!(
        (view["on"].clone(), view["opted_out"].clone()),
        (json!(false), json!(true))
    );
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    assert_eq!(queued(&app.db, deal.ben.id).await.len(), 2);
    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": phone })),
        &[],
    )
    .await
    .refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");
    assert!(codes.sent().is_empty());

    // Should a text be queued all the same (the row written by hand here),
    // the worker drops it.
    sqlx::query("INSERT INTO sms_update (account_id, exchange_id, phone) VALUES ($1, $2, $3)")
        .bind(deal.ben.id)
        .bind(exchange_uuid(&deal))
        .bind(&phone)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO outbox (kind, recipient_account_id, exchange_id, payload)
         VALUES ('SMS', $1, $2, '{\"sms\": \"UPDATE\", \"notice\": \"DELIVERY_CONFIRMED\"}')",
    )
    .bind(deal.ben.id)
    .bind(exchange_uuid(&deal))
    .execute(&app.db)
    .await
    .unwrap();
    let delivered = deliver(&app, &delivery(&out, rules())).await;
    assert_eq!(delivered.dropped, 1);
    assert!(out.sent().is_empty());
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT last_error FROM outbox WHERE kind = 'SMS' AND recipient_account_id = $1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(reason.as_deref(), Some("not sent: the number replied STOP"));
}

// ---- Twilio's webhook ---------------------------------------------------------

/// Posts a message from `from` to the webhook, signed by Twilio, and checks
/// that it is taken with an empty TwiML document.
async fn inbound(app: &App, from: &str, body: &str, opt_out_type: Option<&str>) {
    let mut params = vec![
        ("AccountSid".to_owned(), "AC0123".to_owned()),
        ("MessageSid".to_owned(), "SM0123".to_owned()),
        ("From".to_owned(), from.to_owned()),
        ("To".to_owned(), "+15550000000".to_owned()),
        ("Body".to_owned(), body.to_owned()),
    ];
    if let Some(kind) = opt_out_type {
        params.push(("OptOutType".to_owned(), kind.to_owned()));
    }
    let signature = twilio_signature(TOKEN, WEBHOOK, &params);
    let reply = post_inbound(app, &params, Some(&signature)).await;
    assert_eq!(reply.0, StatusCode::OK);
    assert_eq!(
        reply.1,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response></Response>"
    );
}

async fn post_inbound(
    app: &App,
    params: &[(String, String)],
    signature: Option<&str>,
) -> (StatusCode, String) {
    let form: String = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    let reply = app
        .raw(
            Method::POST,
            "/v1/sms/inbound",
            "application/x-www-form-urlencoded",
            form.into_bytes(),
            signature.map(|signature| ("x-twilio-signature", signature)),
        )
        .await;
    (reply.status, String::from_utf8(reply.bytes).unwrap())
}

#[tokio::test]
async fn the_webhook_takes_only_requests_twilio_signed() {
    let Texting { _turn, app, .. } = start().await;
    let phone = number();
    let params = vec![
        ("From".to_owned(), phone.clone()),
        ("Body".to_owned(), "STOP".to_owned()),
    ];
    let opted_out = || async {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM sms_opt_out WHERE phone = $1)")
            .bind(&phone)
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    // No signature, a signature under another token, of another URL, or of
    // other parameters: refused, and nothing recorded.
    assert_eq!(
        post_inbound(&app, &params, None).await.0,
        StatusCode::FORBIDDEN
    );
    let wrong = [
        twilio_signature("another-token", WEBHOOK, &params),
        twilio_signature(TOKEN, "https://elsewhere.test/v1/sms/inbound", &params),
        twilio_signature(TOKEN, WEBHOOK, &params[..1]),
        "bm90IGEgc2lnbmF0dXJl".to_owned(),
        "%%%".to_owned(),
    ];
    for signature in &wrong {
        assert_eq!(
            post_inbound(&app, &params, Some(signature)).await.0,
            StatusCode::FORBIDDEN,
            "{signature}"
        );
    }
    assert!(!opted_out().await);

    // Signed: taken.
    let signature = twilio_signature(TOKEN, WEBHOOK, &params);
    assert_eq!(
        post_inbound(&app, &params, Some(&signature)).await.0,
        StatusCode::OK
    );
    assert!(opted_out().await);

    // A service with no auth token to check with refuses everything.
    let unchecked = App::start(DATABASE).await;
    let form = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&params)
        .finish();
    let reply = unchecked
        .raw(
            Method::POST,
            "/v1/sms/inbound",
            "application/x-www-form-urlencoded",
            form.into_bytes(),
            Some(("x-twilio-signature", signature.as_str())),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn stop_ends_every_text_to_the_number_and_start_allows_them_again() {
    let Texting { _turn, app, .. } = start().await;
    let first = app.active().await;
    let phone = give_phone(&app, &first.ben).await;
    let second = app
        .active_between(app.user("Eve").await, first.ben.clone())
        .await;
    turn_on(&app, &first.ben, &first).await;
    turn_on(&app, &second.ben, &second).await;

    // Any of the stop words, in any case; here Twilio's own reading of it.
    inbound(&app, &phone, " unsubscribe ", Some("STOP")).await;
    let kept = records(&app.db, &phone).await;
    let actions: Vec<(&str, &str, Option<&str>)> = kept
        .iter()
        .map(|record| (record.0.as_str(), record.3.as_str(), record.4.as_deref()))
        .collect();
    assert_eq!(
        actions,
        [
            ("OPT_IN", "WEB", None),
            ("OPT_IN", "WEB", None),
            ("STOP", "SMS_REPLY", Some("UNSUBSCRIBE")),
            ("OPT_OUT", "SMS_REPLY", None),
            ("OPT_OUT", "SMS_REPLY", None),
        ]
    );
    for deal in [&first, &second] {
        let view = app.get(&deal.ben, &path(deal)).await.ok();
        assert_eq!(
            (view["on"].clone(), view["opted_out"].clone()),
            (json!(false), json!(true))
        );
    }

    // HELP is Twilio's to answer: nothing changes here.
    inbound(&app, &phone, "HELP", Some("HELP")).await;
    // Neither does anything else someone writes.
    inbound(&app, &phone, "who is this?", None).await;
    assert_eq!(records(&app.db, &phone).await.len(), 5);

    // START: texts may go again, but no agreement's updates come back on.
    inbound(&app, &phone, "Start", None).await;
    let view = app.get(&first.ben, &path(&first)).await.ok();
    assert_eq!(
        (view["on"].clone(), view["opted_out"].clone()),
        (json!(false), json!(false))
    );
    let last = records(&app.db, &phone).await.pop().unwrap();
    assert_eq!(
        (last.0.as_str(), last.3.as_str(), last.4.as_deref()),
        ("START", "SMS_REPLY", Some("START"))
    );
    // And the box can be ticked again.
    turn_on(&app, &first.ben, &first).await;

    // Every one of Twilio's stop words, and the two the carriers add.
    for word in [
        "STOPALL", "CANCEL", "END", "QUIT", "OPTOUT", "REVOKE", "stop",
    ] {
        let other = number();
        inbound(&app, &other, word, None).await;
        let stopped: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sms_opt_out WHERE phone = $1)")
                .bind(&other)
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert!(stopped, "{word}");
    }
}

// ---- The account's number -----------------------------------------------------

#[tokio::test]
async fn a_number_is_verified_by_a_code_and_replacing_it_ends_the_old_numbers_updates() {
    let Texting {
        _turn,
        app,
        phone: codes,
        ..
    } = start().await;
    let deal = app.active().await;

    // The way a party with no number adds one: a code by text, then the
    // code back.
    let add = |phone: String| {
        let (app, codes, ben) = (&app, &codes, &deal.ben);
        async move {
            app.call(
                Some(ben),
                Method::POST,
                "/v1/auth/codes",
                Some(json!({ "identifier": phone })),
                &[],
            )
            .await;
            let (to, text) = codes.sent().pop().unwrap();
            assert_eq!(to, phone);
            let code = &text[..6];
            app.post(
                ben,
                "/v1/me/identifiers",
                json!({ "identifier": phone, "code": code }),
            )
            .await
            .ok()
        }
    };
    let first = number();
    assert_eq!(add(first.clone()).await["phone"], first);
    turn_on(&app, &deal.ben, &deal).await;

    let second = number();
    add(second.clone()).await;
    let view = app.get(&deal.ben, &path(&deal)).await.ok();
    assert_eq!(
        (view["on"].clone(), view["phone"].clone()),
        (json!(false), json!(second))
    );
    let kept = records(&app.db, &first).await;
    assert_eq!(
        kept.iter()
            .map(|record| (record.0.as_str(), record.3.as_str()))
            .collect::<Vec<_>>(),
        [("OPT_IN", "WEB"), ("OPT_OUT", "PHONE_CHANGED")]
    );
}

#[tokio::test]
async fn deleting_the_account_removes_its_number_and_updates_and_keeps_the_record() {
    let Texting {
        _turn,
        app,
        mailbox,
        ..
    } = start().await;
    let deal = app.active().await;
    let phone = give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    assert_eq!(queued(&app.db, deal.ben.id).await.len(), 1);

    app.post(
        &deal.ben,
        "/v1/me/deletion/codes",
        json!({ "channel": "EMAIL" }),
    )
    .await;
    let (_, code) = mailbox.0.lock().unwrap().last().cloned().unwrap();
    let reply = app
        .post(
            &deal.ben,
            "/v1/me/deletion",
            json!({ "channel": "EMAIL", "code": code }),
        )
        .await;
    assert!(reply.status.is_success(), "{}", reply.body);

    let (account_phone, updates, queued_texts): (Option<String>, i64, i64) = sqlx::query_as(
        "SELECT (SELECT phone FROM account WHERE id = $1),
                (SELECT count(*) FROM sms_update WHERE account_id = $1),
                (SELECT count(*) FROM outbox WHERE kind = 'SMS' AND recipient_account_id = $1)",
    )
    .bind(deal.ben.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((account_phone, updates, queued_texts), (None, 0, 0));
    // The record of consent stays, as the privacy policy says, with the end
    // of the updates beside it.
    let kept = records(&app.db, &phone).await;
    assert_eq!(
        kept.iter()
            .map(|record| (record.0.as_str(), record.3.as_str()))
            .collect::<Vec<_>>(),
        [("OPT_IN", "WEB"), ("OPT_OUT", "ACCOUNT_DELETED")]
    );
}

// ---- Retention ----------------------------------------------------------------

#[tokio::test]
async fn consent_records_are_kept_for_their_retention_unless_still_in_force() {
    let Texting { _turn, app, .. } = start().await;
    let deal = app.active().await;
    let phone = give_phone(&app, &deal.ben).await;
    turn_on(&app, &deal.ben, &deal).await;
    let rules = Rules::default();
    let later = |days: i64| OffsetDateTime::now_utc() + time::Duration::days(days);

    // After a signature's 90 days, the address and user agent go; the
    // record of an opt-in still in force stays however old.
    sms_updates::purge_consent(&app.db, &rules, later(91))
        .await
        .unwrap();
    let network: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sms_consent_network n JOIN sms_consent c ON c.id = n.consent_id
         WHERE c.phone = $1",
    )
    .bind(&phone)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(network, 0);
    sms_updates::purge_consent(&app.db, &rules, later(365 * 5))
        .await
        .unwrap();
    assert_eq!(records(&app.db, &phone).await.len(), 1);

    // Once updates are off, the whole record goes four years on.
    set(&app, &deal.ben, &deal, json!({ "on": false }))
        .await
        .ok();
    sms_updates::purge_consent(&app.db, &rules, later(365 * 4 - 1))
        .await
        .unwrap();
    assert_eq!(records(&app.db, &phone).await.len(), 2);
    sms_updates::purge_consent(&app.db, &rules, later(365 * 4 + 1))
        .await
        .unwrap();
    assert!(records(&app.db, &phone).await.is_empty());
}
