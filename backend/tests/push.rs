//! Push notifications end to end: devices registered and removed through the
//! API, notices queued once per channel, the exact payload sent in each
//! language, and the push service's word on a token no longer valid acted
//! on, against a stand-in for Expo's service in this process.
//!
//! Delivery acts on every queued message in the database, so the tests here
//! take turns, and each starts with an empty outbox and no devices.

mod common;

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use common::{App, Deal, User};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tokio::sync::MutexGuard;
use uuid::Uuid;
use yuppers_backend::auth::AuthRules;
use yuppers_backend::auth::LogSender;
use yuppers_backend::notifications::expo::{ExpoPushSender, send_body};
use yuppers_backend::notifications::outbox::{Delivery, DeliveryRules, deliver_due};
use yuppers_backend::notifications::push::{
    self, PUSH_OFF, PushData, PushDelivered, PushDelivery, PushFuture, PushMessage, PushSender,
    Receipt, ReceiptRules, Ticket,
};
use yuppers_backend::notifications::smtp::Secret;
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::notifications::{Email, EmailSender};

const DATABASE: &str = "yuppers_test_push";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn clear(app: &App) {
    for statement in ["DELETE FROM outbox", "DELETE FROM device"] {
        sqlx::query(statement).execute(&app.db).await.unwrap();
    }
}

/// The guard keeps the other tests waiting until this one is done.
async fn app() -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    let app = App::start_messaging(DATABASE, AuthRules::default(), Arc::new(LogSender), true).await;
    clear(&app).await;
    (app, turn)
}

fn token(name: &str) -> String {
    format!("ExponentPushToken[{name}-{}]", Uuid::new_v4().simple())
}

async fn register(app: &App, user: &User, token: &str, language: &str) -> common::Reply {
    app.call(
        Some(user),
        Method::PUT,
        "/v1/me/devices",
        Some(json!({
            "token": token,
            "platform": "ios",
            "app_version": "0.1.0",
            "language": language,
        })),
        &[],
    )
    .await
}

async fn device_id(app: &App, user: &User, token: &str) -> String {
    let reply = register(app, user, token, "en").await.ok();
    reply["id"].as_str().unwrap().to_owned()
}

async fn devices_of(app: &App, user: &User) -> Vec<String> {
    sqlx::query_scalar("SELECT token FROM device WHERE account_id = $1 ORDER BY created_at")
        .bind(user.id)
        .fetch_all(&app.owner)
        .await
        .unwrap()
}

/// What is queued for an exchange, as `recipient: KIND NOTICE`, in order.
async fn queued(app: &App, deal: &Deal) -> Vec<String> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT recipient_account_id, kind, payload->>'notice' FROM outbox
         WHERE exchange_id = $1 ORDER BY event_sequence, kind, id",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(recipient, kind, notice)| {
            let name = if recipient == deal.ana.id {
                "Ana"
            } else {
                "Ben"
            };
            format!("{name}: {kind} {notice}")
        })
        .collect()
}

async fn active(app: &App) -> Deal {
    let deal = app.active().await;
    clear(app).await;
    deal
}

/// Ana marks the repair delivered, which tells Ben.
async fn claim(app: &App, deal: &Deal) {
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
}

/// Stands in for a push service: keeps what it was handed, and answers as
/// told, one answer per call, then "taken" for everything.
#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Vec<PushMessage>>>,
    answers: Mutex<VecDeque<anyhow::Result<Vec<Ticket>>>>,
}

impl Recorder {
    fn messages(&self) -> Vec<PushMessage> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect()
    }
}

impl PushSender for Recorder {
    fn send<'a>(&'a self, messages: &'a [PushMessage]) -> PushFuture<'a, Vec<Ticket>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(messages.to_vec());
            match self.answers.lock().unwrap().pop_front() {
                Some(answer) => answer,
                None => Ok(messages
                    .iter()
                    .map(|_| Ticket::Accepted { receipt: None })
                    .collect()),
            }
        })
    }

    fn receipts<'a>(&'a self, _ids: &'a [String]) -> PushFuture<'a, HashMap<String, Receipt>> {
        Box::pin(async { Ok(HashMap::new()) })
    }
}

fn delivery(sender: Option<Arc<dyn PushSender>>) -> PushDelivery {
    PushDelivery {
        sender,
        wording: Wording::embedded().unwrap(),
        rules: DeliveryRules::default(),
    }
}

/// A moment by which everything queued so far is due, in whole
/// microseconds. The database keeps time to the microsecond, and a time is
/// cut to it on the way in, not rounded: a retry time stored from a moment
/// with nanoseconds can read back up to a microsecond earlier than that
/// moment plus the wait, and a test comparing the two would fail now and
/// then. From a whole microsecond, what is stored is exactly what was meant.
fn soon() -> OffsetDateTime {
    let at = OffsetDateTime::now_utc() + Duration::seconds(5);
    at.replace_nanosecond(at.microsecond() * 1_000)
        .expect("a whole microsecond is a valid time")
}

async fn push_due(app: &App, delivery: &PushDelivery, at: OffsetDateTime) -> PushDelivered {
    push::deliver_push_due_until(&app.db, delivery, at, || false)
        .await
        .unwrap()
}

// ---- Registering ------------------------------------------------------------

#[tokio::test]
async fn the_service_says_whether_it_sends_push_notifications() {
    let (app, _turn) = app().await;
    let meta = app
        .call(None, Method::GET, "/v1/meta", None, &[])
        .await
        .ok();
    assert_eq!(meta["push_notifications"], true);

    let off = App::start(DATABASE).await;
    let meta = off
        .call(None, Method::GET, "/v1/meta", None, &[])
        .await
        .ok();
    assert_eq!(meta["push_notifications"], false);
}

#[tokio::test]
async fn a_device_is_registered_updated_and_removed_by_its_own_account() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let phone = token("phone");

    let id = device_id(&app, &ana, &phone).await;
    let stored: (Uuid, String, String, String, String) = sqlx::query_as(
        "SELECT account_id, service, platform, app_version, language FROM device WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        stored,
        (
            ana.id,
            "EXPO".to_owned(),
            "ios".to_owned(),
            "0.1.0".to_owned(),
            "en".to_owned()
        )
    );

    // Registering again is the same device, brought up to date.
    let again = register(&app, &ana, &phone, "es-MX").await.ok();
    assert_eq!(again["id"], id.as_str());
    let language: String = sqlx::query_scalar("SELECT language FROM device WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(language, "es");

    // Someone else cannot remove it, and is told nothing about it.
    let reply = app
        .call(
            Some(&ben),
            Method::DELETE,
            &format!("/v1/me/devices/{id}"),
            None,
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(devices_of(&app, &ana).await, std::slice::from_ref(&phone));

    // Its own account can, and again does no harm.
    for _ in 0..2 {
        let reply = app
            .call(
                Some(&ana),
                Method::DELETE,
                &format!("/v1/me/devices/{id}"),
                None,
                &[],
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
    }
    assert!(devices_of(&app, &ana).await.is_empty());

    // Nobody signed in registers nothing.
    let reply = app
        .call(None, Method::PUT, "/v1/me/devices", Some(json!({ "token": token("x"), "platform": "ios", "app_version": "1.0.0", "language": "en" })), &[])
        .await;
    reply.refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
}

#[tokio::test]
async fn a_token_moves_to_another_account_only_once_its_session_has_ended() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let phone = token("phone");
    let id = device_id(&app, &ana, &phone).await;

    // Ben has learned Ana's token. While her session lives, his registering
    // it moves nothing, and his answer is the one a registration gets, so
    // it does not say that the token is someone's. The ID it names is no
    // use to him.
    let reply = register(&app, &ben, &phone, "es").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["id"], id.as_str());
    assert_eq!(devices_of(&app, &ana).await, std::slice::from_ref(&phone));
    assert!(devices_of(&app, &ben).await.is_empty());
    let language: String = sqlx::query_scalar("SELECT language FROM device WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(language, "en", "nothing about the device changed");
    app.call(
        Some(&ben),
        Method::DELETE,
        &format!("/v1/me/devices/{id}"),
        None,
        &[],
    )
    .await;
    assert_eq!(devices_of(&app, &ana).await, std::slice::from_ref(&phone));

    // Another session of Ana's own takes it: the same person, signed in
    // again on the same phone.
    let again = app.user("Ana again").await;
    sqlx::query("UPDATE account_session SET account_id = $1 WHERE account_id = $2")
        .bind(ana.id)
        .bind(again.id)
        .execute(&app.owner)
        .await
        .unwrap();
    assert_eq!(device_id(&app, &again, &phone).await, id);
    let session: Uuid = sqlx::query_scalar("SELECT session_id FROM device WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.owner)
        .await
        .unwrap();

    // Once the session it is registered under has ended (here revoked
    // without the app saying so; signing out removes the device outright),
    // the phone signed into Ben's account takes it.
    sqlx::query("UPDATE account_session SET revoked_at = now() WHERE id = $1")
        .bind(session)
        .execute(&app.owner)
        .await
        .unwrap();
    device_id(&app, &ben, &phone).await;
    assert!(devices_of(&app, &ana).await.is_empty());
    assert_eq!(devices_of(&app, &ben).await, std::slice::from_ref(&phone));

    // And an expired session is ended too.
    sqlx::query(
        "UPDATE account_session SET expires_at = now() - interval '1 second' WHERE account_id = $1",
    )
    .bind(ben.id)
    .execute(&app.owner)
    .await
    .unwrap();
    let carla = app.user("Carla").await;
    device_id(&app, &carla, &phone).await;
    assert!(devices_of(&app, &ben).await.is_empty());
    assert_eq!(devices_of(&app, &carla).await, [phone]);
}

#[tokio::test]
async fn push_tickets_past_their_receipts_are_removed_whether_or_not_receipts_are_read() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let id = device_id(&app, &ana, &token("phone")).await;
    sqlx::query("DELETE FROM push_ticket")
        .execute(&app.owner)
        .await
        .unwrap();
    for (ticket, age) in [("old", 25), ("older", 72), ("fresh", 1)] {
        sqlx::query(
            "INSERT INTO push_ticket (id, device_id, created_at)
             VALUES ($1, $2::uuid, now() - $3 * interval '1 hour')",
        )
        .bind(ticket)
        .bind(&id)
        .bind(f64::from(age))
        .execute(&app.owner)
        .await
        .unwrap();
    }
    // No push sender, and so no receipts: push is off, or the service has
    // not answered for a day.
    let rules = ReceiptRules::default();
    let removed = push::purge_tickets(&app.db, &rules, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(removed, 2);
    let left: Vec<String> = sqlx::query_scalar("SELECT id FROM push_ticket")
        .fetch_all(&app.owner)
        .await
        .unwrap();
    assert_eq!(left, ["fresh"]);
}

#[tokio::test]
async fn only_an_expo_token_platform_version_and_language_are_taken() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let good = json!({
        "token": token("phone"),
        "platform": "android",
        "app_version": "1.2.3",
        "language": "en",
    });
    for (field, wrong) in [
        ("token", json!("fcm-token-123")),
        ("token", json!("ExponentPushToken[]")),
        ("platform", json!("web")),
        ("app_version", json!("latest")),
        ("language", json!("tlh")),
    ] {
        let mut body = good.clone();
        body[field] = wrong.clone();
        app.call(Some(&ana), Method::PUT, "/v1/me/devices", Some(body), &[])
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    }
    app.call(Some(&ana), Method::PUT, "/v1/me/devices", Some(good), &[])
        .await
        .ok();
}

#[tokio::test]
async fn an_account_keeps_its_ten_most_recent_devices() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let tokens: Vec<String> = (0..11).map(|n| token(&format!("d{n}"))).collect();
    for token in &tokens {
        device_id(&app, &ana, token).await;
    }
    let kept = devices_of(&app, &ana).await;
    assert_eq!(kept.len(), 10);
    assert!(!kept.contains(&tokens[0]), "the oldest goes");
}

#[tokio::test]
async fn signing_out_removes_that_sessions_device_and_no_other() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let (phone, tablet) = (token("phone"), token("tablet"));
    device_id(&app, &ana, &phone).await;
    // The tablet is signed in with a session of its own.
    let other = app.user("Ana elsewhere").await;
    sqlx::query("UPDATE account_session SET account_id = $1 WHERE account_id = $2")
        .bind(ana.id)
        .bind(other.id)
        .execute(&app.owner)
        .await
        .unwrap();
    let tablet_id = device_id(&app, &other, &tablet).await;
    let owner: Uuid = sqlx::query_scalar("SELECT account_id FROM device WHERE id = $1::uuid")
        .bind(&tablet_id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(owner, ana.id);

    let reply = app
        .call(Some(&ana), Method::DELETE, "/v1/auth/session", None, &[])
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(devices_of(&app, &ana).await, [tablet]);
}

#[tokio::test]
async fn a_device_whose_session_has_ended_gets_nothing_and_is_removed() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    device_id(&app, &deal.ben, &token("ben")).await;
    sqlx::query(
        "UPDATE account_session SET expires_at = now() - interval '1 second' WHERE account_id = $1",
    )
    .bind(deal.ben.id)
    .execute(&app.owner)
    .await
    .unwrap();

    claim(&app, &deal).await;
    assert_eq!(queued(&app, &deal).await, ["Ben: EMAIL DELIVERY_CLAIMED"]);
    assert_eq!(push::purge_devices(&app.db).await.unwrap(), 1);
    assert!(devices_of(&app, &deal.ben).await.is_empty());
}

// ---- Which channels ---------------------------------------------------------

#[tokio::test]
async fn each_notice_goes_once_per_channel_the_person_can_be_reached_on() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;

    // Ben has an email address and no device: email only.
    claim(&app, &deal).await;
    assert_eq!(queued(&app, &deal).await, ["Ben: EMAIL DELIVERY_CLAIMED"]);
    clear(&app).await;

    // With two devices: email, and one push notice for both of them.
    let (phone, tablet) = (token("phone"), token("tablet"));
    device_id(&app, &deal.ben, &phone).await;
    device_id(&app, &deal.ben, &tablet).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "RETRACT_CLAIM")
        .await
        .ok();
    assert_eq!(
        queued(&app, &deal).await,
        ["Ben: EMAIL CLAIM_RETRACTED", "Ben: PUSH CLAIM_RETRACTED"]
    );

    // Email delivery takes only the email, and push only the push.
    let email = deliver_due(&app.db, &email_delivery(), soon())
        .await
        .unwrap();
    assert_eq!(email.sent, 1);
    let recorder = Arc::new(Recorder::default());
    let pushed = push_due(&app, &delivery(Some(recorder.clone())), soon()).await;
    assert_eq!((pushed.rows.sent, pushed.rows.handled()), (1, 1));
    let mut to: Vec<String> = recorder.messages().into_iter().map(|m| m.to).collect();
    to.sort();
    let mut want = vec![phone, tablet];
    want.sort();
    assert_eq!(to, want, "each device once");

    // Nothing is sent again, however often or late delivery runs.
    for at in [soon(), soon() + Duration::days(3)] {
        assert!(
            push_due(&app, &delivery(Some(recorder.clone())), at)
                .await
                .rows
                .is_empty()
        );
    }
    assert_eq!(recorder.messages().len(), 2);
}

#[tokio::test]
async fn a_phone_only_account_is_told_by_push_or_not_at_all() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    sqlx::query("UPDATE account SET email = NULL, phone = $2 WHERE id = $1")
        .bind(deal.ben.id)
        .bind(format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000))
        .execute(&app.owner)
        .await
        .unwrap();

    claim(&app, &deal).await;
    assert!(
        queued(&app, &deal).await.is_empty(),
        "no channel, no message"
    );

    device_id(&app, &deal.ben, &token("ben")).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "RETRACT_CLAIM")
        .await
        .ok();
    assert_eq!(queued(&app, &deal).await, ["Ben: PUSH CLAIM_RETRACTED"]);
}

// ---- What is sent -----------------------------------------------------------

#[tokio::test]
async fn the_push_payload_is_generic_in_the_recipients_language_and_opens_the_exchange() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    let (ana_token, ben_token) = (token("ana"), token("ben"));
    device_id(&app, &deal.ana, &ana_token).await;
    device_id(&app, &deal.ben, &ben_token).await;
    sqlx::query("UPDATE account SET language = 'es' WHERE id = $1")
        .bind(deal.ben.id)
        .execute(&app.owner)
        .await
        .unwrap();

    // Ben is told of Ana's claim, in Spanish; Ana of Ben's confirmation, in
    // English.
    claim(&app, &deal).await;
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    let recorder = Arc::new(Recorder::default());
    let pushed = push_due(&app, &delivery(Some(recorder.clone())), soon()).await;
    assert_eq!(pushed.rows.sent, 2);

    let url = format!("/exchanges/{}", deal.exchange);
    let mut messages = recorder.messages();
    messages.sort_by(|a, b| a.to.cmp(&b.to));
    let mut want = vec![
        PushMessage {
            to: ben_token.clone(),
            body: "Tu yup tiene novedades.".to_owned(),
            data: PushData { url: url.clone() },
        },
        PushMessage {
            to: ana_token.clone(),
            body: "Your yup has an update.".to_owned(),
            data: PushData { url: url.clone() },
        },
    ];
    want.sort_by(|a, b| a.to.cmp(&b.to));
    assert_eq!(messages, want);

    // Exactly this goes to Expo: nothing from the agreement, no names, no
    // amount, not even the exchange's code.
    let body = send_body(&messages);
    let mut expected = vec![
        json!({
            "to": ben_token, "body": "Tu yup tiene novedades.", "data": { "url": url },
            "sound": "default", "priority": "high", "channelId": "default",
        }),
        json!({
            "to": ana_token, "body": "Your yup has an update.", "data": { "url": url },
            "sound": "default", "priority": "high", "channelId": "default",
        }),
    ];
    expected.sort_by(|a, b| a["to"].as_str().cmp(&b["to"].as_str()));
    assert_eq!(body, Value::Array(expected));
    // What a person could read: everything but the device token and the
    // exchange's link, whose random hex can contain any short string (a
    // token holding "450" once failed this test).
    let mut readable = body.clone();
    for push in readable.as_array_mut().unwrap() {
        let push = push.as_object_mut().unwrap();
        push.remove("to");
        push.remove("data");
    }
    let text = readable.to_string();
    let code: String = sqlx::query_scalar("SELECT display_code FROM exchange WHERE id = $1::uuid")
        .bind(&deal.exchange)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    for secret in ["fence", "Fence", "Ana", "Ben", "450", &code] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
}

#[tokio::test]
async fn a_push_that_fails_is_retried_like_an_email_and_one_never_registered_is_dropped() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    let ben_token = token("ben");
    let device = device_id(&app, &deal.ben, &ben_token).await;
    claim(&app, &deal).await;

    let recorder = Arc::new(Recorder::default());
    recorder
        .answers
        .lock()
        .unwrap()
        .push_back(Err(anyhow::anyhow!(
            "the push service did not answer in time"
        )));
    let push = delivery(Some(recorder.clone()));
    let start = soon();
    let pushed = push_due(&app, &push, start).await;
    assert_eq!((pushed.rows.failed, pushed.rows.sent), (1, 0));
    let (attempts, error, available_at): (i32, Option<String>, OffsetDateTime) =
        sqlx::query_as("SELECT attempts, last_error, available_at FROM outbox WHERE kind = 'PUSH'")
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    assert_eq!(
        error.as_deref(),
        Some("the push service did not answer in time")
    );
    assert!(
        available_at >= start + Duration::minutes(1),
        "{available_at}"
    );

    // Not before its time; then it goes. The service now says the token is
    // no longer registered: the device goes, and the notice with it.
    assert!(
        push_due(&app, &push, start + Duration::seconds(30))
            .await
            .rows
            .is_empty()
    );
    recorder
        .answers
        .lock()
        .unwrap()
        .push_back(Ok(vec![Ticket::Refused(
            push::Refusal::DeviceNotRegistered,
        )]));
    let pushed = push_due(&app, &push, start + Duration::minutes(2)).await;
    assert_eq!((pushed.rows.dropped, pushed.devices_removed), (1, 1));
    let gone: Option<Uuid> = sqlx::query_scalar("SELECT id FROM device WHERE id = $1::uuid")
        .bind(&device)
        .fetch_optional(&app.owner)
        .await
        .unwrap();
    assert_eq!(gone, None);
}

#[tokio::test]
async fn with_push_off_what_was_queued_for_push_is_closed_unsent() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    device_id(&app, &deal.ben, &token("ben")).await;
    claim(&app, &deal).await;

    let pushed = push_due(&app, &delivery(None), soon()).await;
    assert_eq!(pushed.rows.dropped, 1);
    let (completed, error): (bool, Option<String>) = sqlx::query_as(
        "SELECT completed_at IS NOT NULL, last_error FROM outbox WHERE kind = 'PUSH'",
    )
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert!(completed);
    assert_eq!(error.as_deref(), Some(PUSH_OFF));
    // The email is untouched.
    let email: bool =
        sqlx::query_scalar("SELECT completed_at IS NULL FROM outbox WHERE kind = 'EMAIL'")
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert!(email);
}

// ---- Against a stand-in for Expo's service ----------------------------------

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    body: Value,
}

/// What the stand-in answers next, by path.
type Answers = HashMap<String, VecDeque<(StatusCode, Value)>>;

/// A stand-in for Expo's push service on 127.0.0.1: keeps every request and
/// answers each path with what it was given, in order.
#[derive(Clone, Default)]
struct Expo {
    received: Arc<Mutex<Vec<Received>>>,
    answers: Arc<Mutex<Answers>>,
}

impl Expo {
    async fn start() -> (Self, SocketAddr) {
        let expo = Self::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(answer).with_state(expo.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (expo, addr)
    }

    fn will_answer(&self, path: &str, status: StatusCode, body: Value) {
        self.answers
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .push_back((status, body));
    }

    fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

async fn answer(
    State(expo): State<Expo>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    let path = uri.path().to_owned();
    expo.received.lock().unwrap().push(Received {
        path: path.clone(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let next = expo
        .answers
        .lock()
        .unwrap()
        .get_mut(&path)
        .and_then(VecDeque::pop_front);
    match next {
        Some((status, body)) => (status, body.to_string()),
        None => (StatusCode::NOT_FOUND, "{}".to_owned()),
    }
}

const SEND: &str = "/--/api/v2/push/send";
const RECEIPTS: &str = "/--/api/v2/push/getReceipts";

#[tokio::test]
async fn expo_says_a_token_is_invalid_and_the_device_goes_at_once_or_on_its_receipt() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    let (phone, tablet) = (token("phone"), token("tablet"));
    device_id(&app, &deal.ben, &phone).await;
    device_id(&app, &deal.ben, &tablet).await;
    claim(&app, &deal).await;

    let (expo, addr) = Expo::start().await;
    let sender = Arc::new(ExpoPushSender::new(
        &format!("http://{addr}"),
        Some(Secret::new("expo-access-token".to_owned())),
        StdDuration::from_secs(5),
    ));
    // The phone's message is taken, with a receipt to ask for later; the
    // tablet's token is already unknown.
    expo.will_answer(
        SEND,
        StatusCode::OK,
        json!({ "data": [
            { "status": "ok", "id": "ticket-phone" },
            { "status": "error", "message": format!("\"{tablet}\" is not a registered push notification recipient"),
              "details": { "error": "DeviceNotRegistered", "expoPushToken": tablet } },
        ]}),
    );
    let push = delivery(Some(sender.clone()));
    let pushed = push_due(&app, &push, soon()).await;
    assert_eq!((pushed.rows.sent, pushed.devices_removed), (1, 1));
    assert_eq!(
        devices_of(&app, &deal.ben).await,
        std::slice::from_ref(&phone)
    );

    let sent = expo.received();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].path, SEND);
    assert_eq!(
        sent[0].headers["authorization"].to_str().unwrap(),
        "Bearer expo-access-token"
    );
    assert_eq!(sent[0].headers["content-type"], "application/json");
    let url = format!("/exchanges/{}", deal.exchange);
    assert_eq!(
        sent[0].body,
        json!([
            { "to": phone, "body": "Your yup has an update.", "data": { "url": url },
              "sound": "default", "priority": "high", "channelId": "default" },
            { "to": tablet, "body": "Your yup has an update.", "data": { "url": url },
              "sound": "default", "priority": "high", "channelId": "default" },
        ])
    );

    // Too early for its receipt: nothing is asked.
    let rules = ReceiptRules::default();
    let read = push::check_receipts(&app.db, sender.as_ref(), &rules, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(!read.asked);
    assert_eq!(expo.received().len(), 1);

    // Later, the receipt says the phone's token is no longer valid either.
    expo.will_answer(
        RECEIPTS,
        StatusCode::OK,
        json!({ "data": { "ticket-phone": {
            "status": "error", "message": "The recipient device is not registered with FCM.",
            "details": { "error": "DeviceNotRegistered" } } } }),
    );
    let later = OffsetDateTime::now_utc() + Duration::minutes(20);
    let read = push::check_receipts(&app.db, sender.as_ref(), &rules, later)
        .await
        .unwrap();
    assert_eq!((read.asked, read.devices_removed), (true, 1));
    let asked = expo.received();
    assert_eq!(asked[1].path, RECEIPTS);
    assert_eq!(asked[1].body, json!({ "ids": ["ticket-phone"] }));
    assert!(devices_of(&app, &deal.ben).await.is_empty());
    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM push_ticket")
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(tickets, 0);
}

#[tokio::test]
async fn expo_turning_a_batch_away_is_a_failure_to_retry_that_quotes_nothing_it_said() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    let ben = token("ben");
    device_id(&app, &deal.ben, &ben).await;
    claim(&app, &deal).await;

    let (expo, addr) = Expo::start().await;
    let sender = Arc::new(ExpoPushSender::new(
        &format!("http://{addr}"),
        None,
        StdDuration::from_secs(5),
    ));
    expo.will_answer(
        SEND,
        StatusCode::TOO_MANY_REQUESTS,
        json!({ "errors": [{ "code": "RATE_LIMIT_EXCEEDED", "message": format!("slow down for {ben}") }] }),
    );
    let pushed = push_due(&app, &delivery(Some(sender)), soon()).await;
    assert_eq!(pushed.rows.failed, 1);
    let error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM outbox WHERE kind = 'PUSH'")
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(
        error.as_deref(),
        Some("the push service turned the request away for now (HTTP 429, RATE_LIMIT_EXCEEDED)")
    );
    // No access token configured, none sent.
    assert!(!expo.received()[0].headers.contains_key("authorization"));
}

// ---- Shared with the email tests --------------------------------------------

/// Takes every email and keeps none: the email side is the notifications
/// tests' to check.
struct Nowhere;

impl EmailSender for Nowhere {
    fn send<'a>(&'a self, _email: &'a Email) -> yuppers_backend::auth::SendFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

fn email_delivery() -> Delivery {
    Delivery {
        sender: Arc::new(Nowhere),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: DeliveryRules::default(),
    }
}
