//! Delivery by Resend's API (`NOTIFICATION_DELIVERY=resend`,
//! `CODE_DELIVERY=resend`), against a stand-in for `POST /emails` in this
//! process that behaves as Resend's documentation says: it takes a JSON
//! message with a bearer key, answers `200` with the message's `id`, and
//! answers a repeat of an `Idempotency-Key` with its first answer without
//! sending again.
//!
//! Checked here: the request's shape, key, sender and alternatives; the
//! same key on every try of one message, so a retry never sends twice; a
//! `422` given up on at once; `429` and `5xx` retried with the outbox's
//! backoff; a silent API cut off by the timeout; a refused key logged
//! loudly and retried; and the key, the recipient and the message never in
//! a log or an error.
//!
//! Delivery acts on every queued message in the database, so the tests take
//! turns, and each starts with an empty outbox.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use common::App;
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::sync::MutexGuard;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use yuppers_backend::auth::{CodeMessage, CodeSender, Purpose};
use yuppers_backend::domain::identity::Identifier;
use yuppers_backend::notifications::outbox::{Delivery, DeliveryRules, deliver_due};
use yuppers_backend::notifications::resend::{ResendSender, ResendSettings};
use yuppers_backend::notifications::smtp::Secret;
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::notifications::{Email, EmailSender};
use yuppers_backend::telemetry::{self, LogFormat};

const DATABASE: &str = "yuppers_test_resend";
const WEB_ORIGIN: &str = "https://app.test";
const FROM: &str = "Yuppers <no-reply@example.test>";
/// Obviously fake, in the shape Resend gives.
const KEY: &str = "re_test_key_not_for_logs_123";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- The stand-in -------------------------------------------------------------

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    body: Value,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|value| value.to_str().unwrap())
    }

    fn to(&self) -> String {
        self.body["to"][0].as_str().unwrap().to_owned()
    }
}

#[derive(Default)]
struct Resend {
    received: Mutex<Vec<Received>>,
    /// Messages sent: one per key, or per request without one.
    sent: Mutex<Vec<Value>>,
    /// The first answer to each key, and the body it was given.
    keys: Mutex<HashMap<String, (Value, String)>>,
    /// Answers to give before Resend's own, one per request, first first.
    script: Mutex<Vec<(StatusCode, Value)>>,
    /// How long each request takes to be answered, while set.
    slow: Mutex<Option<Duration>>,
}

#[derive(Clone)]
struct StandIn(Arc<Resend>);

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

    fn sent(&self) -> Vec<Value> {
        self.0.sent.lock().unwrap().clone()
    }

    /// Answers the next requests so, in order.
    fn script(&self, answers: Vec<(StatusCode, Value)>) {
        *self.0.script.lock().unwrap() = answers;
    }

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
    let resend = &stand_in.0;
    let delay = *resend.slow.lock().unwrap();
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    let raw = String::from_utf8(body.to_vec()).unwrap();
    let request = Received {
        path: uri.path().to_owned(),
        headers: headers.clone(),
        body: serde_json::from_str(&raw).unwrap_or(Value::Null),
    };
    resend.received.lock().unwrap().push(request.clone());
    {
        let mut script = resend.script.lock().unwrap();
        if !script.is_empty() {
            let (status, body) = script.remove(0);
            return (status, body.to_string());
        }
    }
    if request.path != "/emails" {
        return (
            StatusCode::NOT_FOUND,
            json!({"statusCode": 404, "name": "not_found", "message": "The requested endpoint does not exist."}).to_string(),
        );
    }
    if request.header("authorization") != Some(&format!("Bearer {KEY}")) {
        return (
            StatusCode::FORBIDDEN,
            json!({"statusCode": 403, "name": "validation_error", "message": "API key is invalid"})
                .to_string(),
        );
    }
    if request.header("user-agent").is_none() {
        return (StatusCode::FORBIDDEN, json!({"code": 1010}).to_string());
    }
    let key = request.header("idempotency-key").map(str::to_owned);
    let first = key
        .as_ref()
        .and_then(|key| resend.keys.lock().unwrap().get(key).cloned());
    if let Some((first, first_body)) = first {
        if first_body != raw {
            return (
                StatusCode::CONFLICT,
                json!({"statusCode": 409, "name": "invalid_idempotent_request", "message": "different payload"}).to_string(),
            );
        }
        return (StatusCode::OK, first.to_string());
    }
    let id = json!({ "id": Uuid::new_v4().to_string() });
    resend.sent.lock().unwrap().push(request.body.clone());
    if let Some(key) = key {
        resend.keys.lock().unwrap().insert(key, (id.clone(), raw));
    }
    (StatusCode::OK, id.to_string())
}

// ---- The sender under test ------------------------------------------------------

fn sender_with(addr: SocketAddr, key: &str, timeout: Duration) -> Arc<ResendSender> {
    Arc::new(
        ResendSender::new(
            &format!("http://{addr}"),
            ResendSettings {
                api_key: Secret::new(key.to_owned()),
                from: FROM.to_owned(),
                timeout,
            },
            Wording::embedded().unwrap(),
        )
        .unwrap(),
    )
}

fn sender(addr: SocketAddr) -> Arc<ResendSender> {
    sender_with(addr, KEY, Duration::from_secs(5))
}

fn delivery(sender: Arc<ResendSender>) -> Delivery {
    Delivery {
        sender,
        wording: Wording::embedded().unwrap(),
        web_origin: WEB_ORIGIN.to_owned(),
        rules: DeliveryRules::default(),
    }
}

async fn app() -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    let app = App::start(DATABASE).await;
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
    (app, turn)
}

/// What the outbox holds: (id, attempts, completed, last_error) per row.
async fn outbox(app: &App) -> Vec<(i64, i32, bool, Option<String>)> {
    sqlx::query_as(
        "SELECT id, attempts, completed_at IS NOT NULL, last_error FROM outbox ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

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

    /// A log that takes everything this test's thread writes.
    fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let log = Self::default();
        let writer = log.clone();
        let subscriber =
            telemetry::subscriber(LogFormat::Text, EnvFilter::new("trace"), false, move || {
                writer.clone()
            });
        (log, tracing::subscriber::set_default(subscriber))
    }
}

// ---- Tests ----------------------------------------------------------------------

#[tokio::test]
async fn a_notification_goes_to_resend_in_the_shape_its_api_takes() {
    let (log, _guard) = Log::capture();
    let (app, _turn) = app().await;
    let deal = app.active().await;
    let (resend, addr) = StandIn::start().await;

    let delivered = deliver_due(
        &app.db,
        &delivery(sender(addr)),
        OffsetDateTime::now_utc() + time::Duration::seconds(5),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (4, 0));
    assert!(
        outbox(&app)
            .await
            .iter()
            .all(|row| row.2 && row.3.is_none())
    );

    let received = resend.received();
    assert_eq!(received.len(), 4);
    let ids: Vec<i64> = outbox(&app).await.iter().map(|row| row.0).collect();
    let mut keys = BTreeSet::new();
    for request in &received {
        assert_eq!(request.path, "/emails");
        assert_eq!(
            request.header("authorization"),
            Some(&*format!("Bearer {KEY}"))
        );
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert!(
            request
                .header("user-agent")
                .unwrap()
                .starts_with("yuppers-backend/")
        );
        keys.insert(request.header("idempotency-key").unwrap().to_owned());
        let body = request.body.as_object().unwrap();
        let fields: BTreeSet<&str> = body.keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            BTreeSet::from(["from", "to", "subject", "text", "html"])
        );
        assert_eq!(body["from"], FROM);
        assert_eq!(body["to"].as_array().unwrap().len(), 1);
        assert!(body["text"].as_str().unwrap().contains(WEB_ORIGIN));
        assert!(body["html"].as_str().unwrap().contains("<html"));
    }
    // One key per outbox row.
    let want: BTreeSet<String> = ids
        .iter()
        .map(|id| ResendSender::idempotency_key(*id))
        .collect();
    assert_eq!(keys, want);
    let to: BTreeSet<String> = received.iter().map(Received::to).collect();
    assert_eq!(
        to,
        BTreeSet::from([deal.ana.email.clone(), deal.ben.email.clone()])
    );

    // Logged by Resend's ID, without the address, subject or body.
    let logged = log.text();
    assert_eq!(
        logged.matches("email handed to Resend").count(),
        4,
        "{logged}"
    );
    for request in &received {
        assert!(!logged.contains(&request.to()), "{logged}");
        assert!(!logged.contains(request.body["subject"].as_str().unwrap()));
    }
    assert!(!logged.contains(KEY), "{logged}");
}

#[tokio::test]
async fn a_retry_carries_the_same_key_and_resend_sends_once() {
    let (app, _turn) = app().await;
    let one = Email {
        to: "ana@example.test".to_owned(),
        subject: "A test".to_owned(),
        body: "Nothing to see.".to_owned(),
        html: Some("<p>Nothing to see.</p>".to_owned()),
        reference: 4242,
    };
    let (resend, addr) = StandIn::start().await;
    let direct = sender(addr);

    // Taken, then sent again, as a worker that died before recording it
    // would: the same key, and Resend does not send twice.
    EmailSender::send(&*direct, &one).await.unwrap();
    EmailSender::send(&*direct, &one).await.unwrap();
    let received = resend.received();
    assert_eq!(received.len(), 2);
    assert_eq!(
        received[0].header("idempotency-key"),
        received[1].header("idempotency-key")
    );
    assert_eq!(
        received[0].header("idempotency-key"),
        Some("yuppers-outbox/4242")
    );
    assert_eq!(resend.sent().len(), 1);

    // Through the outbox: Resend down on the first pass, up on the next.
    app.active().await;
    let (resend, addr) = StandIn::start().await;
    resend.script(vec![
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"statusCode": 500, "name": "application_error", "message": "An unexpected error occurred."}),
        );
        4
    ]);
    let now = OffsetDateTime::now_utc();
    let delivered = deliver_due(&app.db, &delivery(sender(addr)), now)
        .await
        .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (0, 4));
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(addr)),
        now + time::Duration::minutes(2),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (4, 0));
    let received = resend.received();
    assert_eq!(received.len(), 8);
    for row in outbox(&app).await {
        let key = ResendSender::idempotency_key(row.0);
        let tries = received
            .iter()
            .filter(|request| request.header("idempotency-key") == Some(&*key))
            .count();
        assert_eq!(tries, 2, "{key}");
        assert_eq!((row.1, row.2), (2, true));
    }
    assert_eq!(resend.sent().len(), 4);
}

#[tokio::test]
async fn an_address_resend_refuses_is_given_up_on_at_once_and_quoted_nowhere() {
    let (log, _guard) = Log::capture();
    let (app, _turn) = app().await;
    let deal = app.active().await;
    let (resend, addr) = StandIn::start().await;
    // Resend's message quotes the address; ours must not.
    let refusals = [&deal.ana.email, &deal.ben.email, &deal.ana.email, &deal.ben.email]
        .into_iter()
        .map(|to| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({"statusCode": 422, "name": "validation_error", "message": format!("Invalid `to` field: {to}")}),
            )
        })
        .collect();
    resend.script(refusals);

    let now = OffsetDateTime::now_utc();
    let rules = DeliveryRules::default();
    let delivered = deliver_due(&app.db, &delivery(sender(addr)), now)
        .await
        .unwrap();
    assert_eq!(
        (delivered.sent, delivered.failed, delivered.given_up),
        (0, 4, 4)
    );
    for (_, attempts, completed, error) in outbox(&app).await {
        assert_eq!((attempts, completed), (rules.max_attempts, false));
        assert_eq!(
            error.as_deref(),
            Some("Resend refused the message (HTTP 422, validation_error)")
        );
    }
    // Not tried again, however long the wait.
    let later = deliver_due(
        &app.db,
        &delivery(sender(addr)),
        now + time::Duration::days(1),
    )
    .await
    .unwrap();
    assert!(later.is_empty());
    assert_eq!(resend.received().len(), 4);

    let logged = log.text();
    assert_eq!(
        logged.matches("notification given up on").count(),
        4,
        "{logged}"
    );
    for person in [&deal.ana, &deal.ben] {
        let (local, domain) = person.email.split_once('@').unwrap();
        for part in [person.email.as_str(), local, domain] {
            assert!(!logged.contains(part), "{part} in the log:\n{logged}");
        }
    }
    assert!(!logged.contains(KEY));
}

#[tokio::test]
async fn too_many_requests_and_server_errors_are_retried_with_the_backoff() {
    let (app, _turn) = app().await;
    app.active().await;
    let (resend, addr) = StandIn::start().await;
    resend.script(vec![
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"statusCode": 429, "name": "rate_limit_exceeded", "message": "Too many requests."}),
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"statusCode": 429, "name": "daily_quota_exceeded", "message": "quota"}),
        ),
        (
            StatusCode::BAD_GATEWAY,
            json!("<html>bad gateway</html>"),
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"statusCode": 503, "name": "service_unavailable", "message": "down"}),
        ),
    ]);
    let now = OffsetDateTime::now_utc();
    let delivered = deliver_due(&app.db, &delivery(sender(addr)), now)
        .await
        .unwrap();
    assert_eq!(
        (delivered.sent, delivered.failed, delivered.given_up),
        (0, 4, 0)
    );
    let errors: BTreeSet<String> = outbox(&app)
        .await
        .into_iter()
        .map(|(_, attempts, completed, error)| {
            assert_eq!((attempts, completed), (1, false));
            error.unwrap()
        })
        .collect();
    assert_eq!(
        errors,
        BTreeSet::from([
            "Resend did not take the message for now (HTTP 429, rate_limit_exceeded)".to_owned(),
            "Resend did not take the message for now (HTTP 429, daily_quota_exceeded)".to_owned(),
            "Resend did not take the message for now (HTTP 502)".to_owned(),
            "Resend did not take the message for now (HTTP 503, service_unavailable)".to_owned(),
        ])
    );

    // Not before the backoff's minute, then all of them.
    let early = deliver_due(
        &app.db,
        &delivery(sender(addr)),
        now + time::Duration::seconds(30),
    )
    .await
    .unwrap();
    assert!(early.is_empty());
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(addr)),
        now + time::Duration::minutes(2),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (4, 0));
    assert_eq!(resend.sent().len(), 4);
}

#[tokio::test]
async fn a_silent_resend_is_cut_off_by_the_timeout_and_retried() {
    let (app, _turn) = app().await;
    app.active().await;
    let (resend, addr) = StandIn::start().await;
    resend.slow_down(Some(Duration::from_secs(10)));

    let started = Instant::now();
    let delivered = deliver_due(
        &app.db,
        &delivery(sender_with(addr, KEY, Duration::from_millis(300))),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(
        (delivered.sent, delivered.failed, delivered.given_up),
        (0, 4, 0)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    for (_, attempts, _, error) in outbox(&app).await {
        assert_eq!(attempts, 1);
        assert_eq!(
            error.as_deref(),
            Some("Resend: the provider did not answer in time")
        );
    }
}

#[tokio::test]
async fn a_refused_key_is_logged_loudly_without_the_key_and_retried() {
    let (log, _guard) = Log::capture();
    let (app, _turn) = app().await;
    app.active().await;
    let (_resend, addr) = StandIn::start().await;
    let wrong = "re_wrong_key_not_for_logs_456";

    let delivered = deliver_due(
        &app.db,
        &delivery(sender_with(addr, wrong, Duration::from_secs(5))),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(
        (delivered.sent, delivered.failed, delivered.given_up),
        (0, 4, 0)
    );
    for (_, attempts, _, error) in outbox(&app).await {
        assert_eq!(attempts, 1);
        assert_eq!(
            error.as_deref(),
            Some("Resend refused the API key or the sender (HTTP 403, validation_error)")
        );
    }
    let logged = log.text();
    assert!(logged.contains("ERROR"), "{logged}");
    assert!(logged.contains("check RESEND_API_KEY"), "{logged}");
    assert!(!logged.contains(wrong), "{logged}");
    assert!(!logged.contains("re_wrong"), "{logged}");
}

#[tokio::test]
async fn a_one_time_code_goes_to_resend_without_a_key_and_never_to_a_phone() {
    let (log, _guard) = Log::capture();
    let _turn = TURN.lock().await;
    let (resend, addr) = StandIn::start().await;
    let sender = sender(addr);
    let wording = Wording::embedded().unwrap();

    let ana = Identifier::parse("ana@example.test").unwrap();
    for _ in 0..2 {
        CodeSender::send(
            &*sender,
            CodeMessage {
                to: &ana,
                code: "123456",
                purpose: Purpose::SignIn,
                language: "es-MX",
            },
        )
        .await
        .unwrap();
    }
    // Each code asked for is its own message.
    assert_eq!(resend.sent().len(), 2);
    let received = resend.received();
    let want = wording.code_email("es", Purpose::SignIn, "123456");
    for request in &received {
        assert_eq!(request.header("idempotency-key"), None);
        assert_eq!(request.body["to"], json!(["ana@example.test"]));
        assert_eq!(request.body["subject"], want.subject);
        assert_eq!(request.body["text"], want.body);
        assert_eq!(request.body["html"], want.html);
    }
    assert_eq!(
        CodeSender::email_sender(&*sender).as_deref(),
        Some("no-reply@example.test")
    );

    let phone = Identifier::parse("+15551234567").unwrap();
    let error = CodeSender::send(
        &*sender,
        CodeMessage {
            to: &phone,
            code: "111111",
            purpose: Purpose::SignIn,
            language: "en",
        },
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("SMS"), "{error:#}");
    assert_eq!(resend.received().len(), 2);

    // A refused code says so by status and name, quoting nothing.
    resend.script(vec![(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"statusCode": 422, "name": "validation_error", "message": "Invalid `to` field: ana@example.test"}),
    )]);
    let error = CodeSender::send(
        &*sender,
        CodeMessage {
            to: &ana,
            code: "654321",
            purpose: Purpose::SignIn,
            language: "en",
        },
    )
    .await
    .unwrap_err();
    let error = format!("{error:#}");
    assert_eq!(
        error,
        "Resend refused the message (HTTP 422, validation_error)"
    );
    let logged = log.text();
    for secret in ["ana@example.test", KEY] {
        assert!(!logged.contains(secret), "{secret} in the log:\n{logged}");
    }
}
