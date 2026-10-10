//! What the service writes to its log, counts in its metrics and exports to
//! an OpenTelemetry collector, read back from real requests
//! (docs/operations.md, "Logs", "Metrics" and "Telemetry").
//!
//! Needs PostgreSQL and the connection strings from `.env`, like the other
//! API tests; it gets a database of its own.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use common::App;
use serde_json::{Value, json};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use yuppers_backend::auth::{AuthRules, CodeMessage, CodeSender, SendFuture};
use yuppers_backend::funnel::funnel;
use yuppers_backend::metrics::Text;
use yuppers_backend::otel::{self, Config};
use yuppers_backend::telemetry::{self, LogFormat};

/// Every test here takes turns. A log subscriber set for one test's thread
/// misses events while other threads are registering the same call sites,
/// so nothing else may run while a test reads back what was logged.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const DATABASE: &str = "yuppers_test_telemetry";

/// Sign-in rules that never refuse a code: every test here asks from the one
/// test address, and the whole life of two people asks for many.
fn generous() -> AuthRules {
    AuthRules {
        code_requests_per_address_per_hour: 1_000_000,
        ..AuthRules::default()
    }
}

/// Keeps the codes the service "sent", without logging them.
#[derive(Default)]
struct Codes(Mutex<Vec<(String, String)>>);

impl CodeSender for Codes {
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

impl Codes {
    fn last(&self, to: &str) -> String {
        let sent = self.0.lock().unwrap();
        let (_, code) = sent
            .iter()
            .rev()
            .find(|(sent_to, _)| sent_to == to)
            .unwrap();
        code.clone()
    }
}

/// Everything logged, as bytes.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
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

/// What a person sent and was given while signing in, none of which may
/// reach the log.
struct Secrets {
    email: String,
    phone: String,
    codes: Vec<String>,
    token: String,
    cookie: String,
}

/// Signs a new person in twice, once for a token and once for a cookie,
/// and uses both; asks for a code for a phone number; and makes a request
/// with secrets in its query string.
async fn sign_in(app: &App, codes: &Codes) -> Secrets {
    let email = format!("{}@telemetry-test.invalid", Uuid::new_v4().simple());
    let phone = format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000);
    let mut sent = Vec::new();

    let reply = app
        .call(
            None,
            Method::POST,
            "/v1/auth/codes",
            Some(json!({ "identifier": phone, "sms_consent": common::sms_consent() })),
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    sent.push(codes.last(&phone));

    let reply = app
        .call(
            None,
            Method::POST,
            "/v1/auth/codes",
            Some(json!({ "identifier": email })),
            &[("x-request-id", "proxy-abc.123_XYZ")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert_eq!(reply.headers["x-request-id"], "proxy-abc.123_XYZ");
    let code = codes.last(&email);
    sent.push(code.clone());

    let session = app
        .call(
            None,
            Method::POST,
            "/v1/auth/sessions",
            Some(json!({ "identifier": email, "code": code, "delivery": "TOKEN" })),
            &[],
        )
        .await
        .ok();
    let token = session["token"].as_str().unwrap().to_owned();

    app.call(
        None,
        Method::POST,
        "/v1/auth/codes",
        Some(json!({ "identifier": email })),
        &[],
    )
    .await;
    let code = codes.last(&email);
    sent.push(code.clone());
    let reply = app
        .call(
            None,
            Method::POST,
            "/v1/auth/sessions",
            Some(json!({ "identifier": email, "code": code, "delivery": "COOKIE" })),
            &[("origin", "https://app.test")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let set_cookie = reply.headers["set-cookie"].to_str().unwrap();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();

    let bearer = format!("Bearer {token}");
    let me = app
        .call(
            None,
            Method::GET,
            "/v1/me",
            None,
            &[("authorization", &bearer)],
        )
        .await;
    assert_eq!(me.status, StatusCode::OK);
    let me = app
        .call(None, Method::GET, "/v1/me", None, &[("cookie", &cookie)])
        .await;
    assert_eq!(me.status, StatusCode::OK);

    let query = format!("/v1/meta?email={email}&code={code}&token={token}");
    let meta = app.call(None, Method::GET, &query, None, &[]).await;
    assert_eq!(meta.status, StatusCode::OK);

    Secrets {
        email,
        phone,
        codes: sent,
        token,
        cookie: cookie.split_once('=').unwrap().1.to_owned(),
    }
}

/// Whether `text` holds `digits` as a number of its own, not as part of a
/// longer one.
fn holds_number(text: &str, digits: &str) -> bool {
    text.match_indices(digits).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + digits.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// Checks that nothing the person sent or was given is in `log`. Timestamps
/// are taken out first: their fractions of a second are digits that a code
/// could match by chance.
fn assert_nothing_personal(log: &str, secrets: &Secrets, format: LogFormat) {
    let local_part = secrets.email.split('@').next().unwrap();
    for (what, value) in [
        ("email address", secrets.email.as_str()),
        ("email address", local_part),
        ("email domain", "telemetry-test.invalid"),
        ("phone number", secrets.phone.as_str()),
        ("phone number", secrets.phone.trim_start_matches('+')),
        ("session token", secrets.token.as_str()),
        ("cookie", secrets.cookie.as_str()),
    ] {
        assert!(!log.contains(value), "the log holds the {what}:\n{log}");
    }
    for line in log.lines() {
        // Without the timestamp, and without the trace and span IDs, which
        // are random hexadecimal that a code could match by chance too.
        let without_time = match format {
            LogFormat::Text => line
                .split_once(' ')
                .map_or("", |(_, rest)| rest)
                .split(' ')
                .filter(|part| !part.starts_with("trace_id=") && !part.starts_with("span_id="))
                .collect::<Vec<_>>()
                .join(" "),
            LogFormat::Json => {
                let mut event: Value = serde_json::from_str(line).unwrap();
                event.as_object_mut().unwrap().remove("timestamp");
                if let Some(span) = event["span"].as_object_mut() {
                    span.remove("trace_id");
                    span.remove("span_id");
                }
                event.to_string()
            }
        };
        for code in &secrets.codes {
            assert!(
                !holds_number(&without_time, code),
                "the log holds a one-time code:\n{line}"
            );
        }
    }
}

// ---- A collector to export to -------------------------------------------------

/// One request the collector received: the path, the headers and the body.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: HeaderMap,
    body: Value,
}

/// A stand-in for an OTLP/HTTP collector, keeping everything it is sent.
#[derive(Clone, Default)]
struct Collector(Arc<Mutex<Vec<Received>>>);

impl Collector {
    /// Starts listening on a port of its own; the address to export to.
    async fn start() -> (Self, String) {
        let collector = Self::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let app =
            Router::new()
                .fallback(
                    |State(collector): State<Collector>,
                     uri: Uri,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        collector.0.lock().unwrap().push(Received {
                            path: uri.path().to_owned(),
                            headers,
                            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
                        });
                        (StatusCode::OK, "{}")
                    },
                )
                .with_state(collector.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (collector, address)
    }

    fn received(&self) -> Vec<Received> {
        self.0.lock().unwrap().clone()
    }

    /// The bodies posted to `path`.
    fn documents(&self, path: &str) -> Vec<Value> {
        self.received()
            .into_iter()
            .filter(|received| received.path == path)
            .map(|received| received.body)
            .collect()
    }

    /// Every span exported.
    fn spans(&self) -> Vec<Value> {
        self.documents("/v1/traces")
            .iter()
            .flat_map(|document| array(&document["resourceSpans"][0]["scopeSpans"][0]["spans"]))
            .collect()
    }

    /// Every log record exported.
    fn logs(&self) -> Vec<Value> {
        self.documents("/v1/logs")
            .iter()
            .flat_map(|document| array(&document["resourceLogs"][0]["scopeLogs"][0]["logRecords"]))
            .collect()
    }

    /// Every metric exported, by name, the last document's value of each.
    fn metrics(&self) -> Vec<Value> {
        self.documents("/v1/metrics")
            .iter()
            .flat_map(|document| {
                array(&document["resourceMetrics"][0]["scopeMetrics"][0]["metrics"])
            })
            .collect()
    }
}

fn array(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

/// An attribute of a span, record or data point, by key.
fn attribute(of: &Value, key: &str) -> Option<Value> {
    array(&of["attributes"])
        .into_iter()
        .find(|attribute| attribute["key"] == key)
        .map(|attribute| attribute["value"].clone())
}

/// `document` with every timestamp, identifier and measurement taken out,
/// as text, for scanning: they are digits and hexadecimal a code could
/// match by chance (the fraction of a latency, the sum of a histogram).
fn scannable(document: &Value) -> String {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for key in [
                    "timeUnixNano",
                    "startTimeUnixNano",
                    "endTimeUnixNano",
                    "observedTimeUnixNano",
                    "traceId",
                    "spanId",
                    "parentSpanId",
                ] {
                    object.remove(key);
                }
                for key in ["doubleValue", "sum"] {
                    if object.get(key).is_some_and(Value::is_number) {
                        object[key] = json!(0);
                    }
                }
                for value in object.values_mut() {
                    strip(value);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut copy = document.clone();
    strip(&mut copy);
    copy.to_string()
}

/// A configuration for the collector at `address`, as a deployment would
/// write it: a service name, an environment and Cloudflare Access's two
/// headers.
fn exporting_to(address: &str) -> Config {
    Config::new(
        address,
        vec![
            ("service.name".to_owned(), "yuppers-api".to_owned()),
            ("service.namespace".to_owned(), "yuppers".to_owned()),
            ("deployment.environment".to_owned(), "test".to_owned()),
        ],
    )
    .unwrap()
    .with_header("CF-Access-Client-Id", "test-client-id.access")
    .unwrap()
    .with_header("CF-Access-Client-Secret", "test-client-secret-value")
    .unwrap()
    .with_timeout(Duration::from_secs(1))
}

/// Checks that none of `secrets`, nor any of `codes`, is anywhere in what
/// the collector received.
fn assert_nothing_personal_exported(collector: &Collector, secrets: &[&str], codes: &[String]) {
    let received = collector.received();
    assert!(!received.is_empty());
    for document in received {
        let text = scannable(&document.body);
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{} holds {secret:?}:\n{text}",
                document.path
            );
        }
        for code in codes {
            assert!(
                !holds_number(&text, code),
                "{} holds a one-time code:\n{text}",
                document.path
            );
        }
    }
}

async fn signed_in_with_log(format: LogFormat) -> (String, Secrets) {
    let _turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    // Everything, at every level, so nothing is missed by being quieter
    // than a deployment might set it.
    let subscriber = telemetry::subscriber(
        format,
        EnvFilter::new("trace"),
        false,
        move || writer.clone(),
        None,
    );
    // A test runs on one thread, so this holds for everything it does.
    let _guard = tracing::subscriber::set_default(subscriber);

    let codes = Arc::new(Codes::default());
    let app = App::start_messaging(DATABASE, generous(), codes.clone(), false).await;
    let secrets = sign_in(&app, &codes).await;
    (log.text(), secrets)
}

#[tokio::test]
async fn signing_in_logs_one_line_per_request_and_nothing_personal_as_text() {
    let (log, secrets) = signed_in_with_log(LogFormat::Text).await;
    assert_nothing_personal(&log, &secrets, LogFormat::Text);

    let completed: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("request completed"))
        .collect();
    assert_eq!(completed.len(), 8, "{log}");
    let signed_in = completed
        .iter()
        .find(|line| line.contains(r#"path="/v1/auth/sessions""#))
        .unwrap_or_else(|| panic!("{log}"));
    for part in ["method=POST", "request_id=", "status=200", "latency_ms="] {
        assert!(signed_in.contains(part), "{part} in {signed_in}");
    }
    assert!(log.contains("request_id=proxy-abc.123_XYZ"), "{log}");
    // The query string is not part of the path that is logged.
    assert!(log.contains(r#"path="/v1/meta" "#), "{log}");
    assert!(!log.contains("/v1/meta?"), "{log}");
}

#[tokio::test]
async fn signing_in_logs_one_object_per_request_and_nothing_personal_as_json() {
    let (log, secrets) = signed_in_with_log(LogFormat::Json).await;
    assert_nothing_personal(&log, &secrets, LogFormat::Json);

    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completed: Vec<&Value> = events
        .iter()
        .filter(|event| event["message"] == "request completed")
        .collect();
    assert_eq!(completed.len(), 8, "{log}");
    let signed_in = completed
        .iter()
        .find(|event| event["span"]["path"] == "/v1/auth/sessions")
        .unwrap();
    assert_eq!(signed_in["level"], "INFO");
    assert_eq!(signed_in["span"]["method"], "POST");
    assert_eq!(signed_in["status"], 200);
    assert!(signed_in["latency_ms"].as_f64().unwrap() >= 0.0);
    let id = signed_in["span"]["request_id"].as_str().unwrap();
    assert!(Uuid::parse_str(id).is_ok(), "{id}");
    assert!(
        completed
            .iter()
            .any(|event| event["span"]["request_id"] == "proxy-abc.123_XYZ")
    );
    assert!(
        completed
            .iter()
            .any(|event| event["span"]["path"] == "/v1/meta")
    );
}

#[tokio::test]
async fn a_request_id_is_taken_only_if_it_is_short_and_plain() {
    let _turn = TURN.lock().await;
    let app = App::start(DATABASE).await;
    let id_for = async |sent: &str| -> String {
        let reply = app
            .call(
                None,
                Method::GET,
                "/healthz",
                None,
                &[("x-request-id", sent)],
            )
            .await;
        reply.headers["x-request-id"].to_str().unwrap().to_owned()
    };

    for kept in ["a", "0f8b2c9e-1d2a-4b7e-9c55-3d1e6f0a9b21", &"x".repeat(64)] {
        assert_eq!(id_for(kept).await, kept);
    }
    for replaced in [
        "",
        "ana@example.test",
        "has space",
        "line\tbreak",
        "quote\"d",
        &"x".repeat(65),
    ] {
        let id = id_for(replaced).await;
        assert_ne!(id, replaced);
        assert!(Uuid::parse_str(&id).is_ok(), "{id}");
    }

    // None sent: one is made.
    let reply = app.call(None, Method::GET, "/healthz", None, &[]).await;
    let id = reply.headers["x-request-id"].to_str().unwrap();
    assert!(Uuid::parse_str(id).is_ok(), "{id}");
}

#[tokio::test]
async fn requests_are_counted_by_route_template_not_by_path() {
    let _turn = TURN.lock().await;
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    for id in [first, second] {
        app.get(&ana, &format!("/v1/exchanges/{id}")).await;
    }
    let exchange = app.draft(&ana).await;
    app.get(&ana, &format!("/v1/exchanges/{exchange}"))
        .await
        .ok();
    app.call(
        None,
        Method::GET,
        "/v1/meta?email=ana@example.test",
        None,
        &[],
    )
    .await;
    app.call(None, Method::GET, "/no/such/page", None, &[])
        .await;

    let mut text = Text::new();
    app.metrics.render(&mut text);
    let page = text.finish();

    for line in [
        r#"yuppers_http_requests_total{route="/v1/exchanges/{id}",method="GET",status="4xx"} 2"#,
        r#"yuppers_http_requests_total{route="/v1/exchanges/{id}",method="GET",status="2xx"} 1"#,
        r#"yuppers_http_requests_total{route="/v1/exchanges",method="POST",status="2xx"} 1"#,
        r#"yuppers_http_requests_total{route="/v1/meta",method="GET",status="2xx"} 1"#,
        r#"yuppers_http_requests_total{route="unmatched",method="GET",status="4xx"} 1"#,
    ] {
        assert!(page.contains(&format!("{line}\n")), "{line} in\n{page}");
    }
    for raw in [
        first.to_string(),
        second.to_string(),
        exchange,
        "ana@example.test".to_owned(),
        "/no/such/page".to_owned(),
    ] {
        assert!(!page.contains(&raw), "{raw} in\n{page}");
    }
}

#[tokio::test]
async fn payment_options_are_never_logged() {
    let _turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Json,
        EnvFilter::new("trace"),
        false,
        move || writer.clone(),
        None,
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let app = App::start(DATABASE).await;
    let deal = app.active().await;
    let handles = json!({
        "venmo": "logcheck-venmo",
        "cash_app": "LogcheckCash",
        "paypal": "LogcheckPayPal",
        "zelle": "logcheck@zelle.test",
    });
    app.call(
        Some(&deal.ana),
        Method::PUT,
        "/v1/me/payment-handles",
        Some(handles),
        &[],
    )
    .await
    .ok();
    // A refused one too: what was sent is not in the refusal's log either.
    let refused = app
        .call(
            Some(&deal.ana),
            Method::PUT,
            "/v1/me/payment-handles",
            Some(json!({ "venmo": "logcheck venmo!", "zelle": "logcheck@zelle.test" })),
            &[],
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    // One at a time, saved and refused: the path names the app, never the value.
    for value in ["LogcheckPayPal2", "Logcheck PayPal!"] {
        app.call(
            Some(&deal.ana),
            Method::PUT,
            "/v1/me/payment-handles/paypal",
            Some(json!({ "value": value })),
            &[],
        )
        .await;
    }
    app.call(
        Some(&deal.ana),
        Method::PUT,
        &format!("/v1/exchanges/{}/payment-options", deal.exchange),
        Some(json!({ "on": true })),
        &[],
    )
    .await
    .ok();
    let seen = app.view(&deal.ben, &deal.exchange).await;
    assert_eq!(seen["payment_options"]["theirs"]["venmo"], "logcheck-venmo");

    let text = log.text();
    assert!(text.contains("payment-handles"), "{text}");
    for handle in ["logcheck", "Logcheck"] {
        assert!(
            !text.contains(handle),
            "the log holds a payment option:\n{text}"
        );
    }
}

#[tokio::test]
async fn a_payment_option_copied_into_another_account_fails_closed_and_is_logged_without_it() {
    let _turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Json,
        EnvFilter::new("trace"),
        false,
        move || writer.clone(),
        None,
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let app = App::start(DATABASE).await;
    let deal = app.active().await;
    for (who, venmo) in [(&deal.ana, "copied-venmo"), (&deal.ben, "bens-own")] {
        app.call(
            Some(who),
            Method::PUT,
            "/v1/me/payment-handles",
            Some(json!({ "venmo": venmo, "zelle": "copycheck@zelle.test" })),
            &[],
        )
        .await
        .ok();
    }
    // Someone with the database copies Ben's ciphertexts into Ana's row,
    // hoping Ben's money goes to them.
    sqlx::query(
        "UPDATE payment_handle a
         SET venmo_encrypted = b.venmo_encrypted
         FROM payment_handle b
         WHERE a.account_id = $1 AND b.account_id = $2",
    )
    .bind(deal.ana.id)
    .bind(deal.ben.id)
    .execute(&app.owner)
    .await
    .unwrap();
    app.call(
        Some(&deal.ana),
        Method::PUT,
        &format!("/v1/exchanges/{}/payment-options", deal.exchange),
        Some(json!({ "on": true })),
        &[],
    )
    .await
    .ok();

    // Not shown to the payer, nor to its owner; the rest still is.
    let seen = app.view(&deal.ben, &deal.exchange).await;
    assert_eq!(seen["payment_options"]["theirs"]["venmo"], Value::Null);
    assert_eq!(
        seen["payment_options"]["theirs"]["zelle"],
        "copycheck@zelle.test"
    );
    let own = app.get(&deal.ana, "/v1/me/payment-handles").await.ok();
    assert_eq!(own["venmo"], Value::Null);

    let text = log.text();
    assert!(
        text.contains("a stored payment option does not decrypt for its account"),
        "{text}"
    );
    assert!(text.contains("venmo_encrypted"), "{text}");
    for value in ["bens-own", "copied-venmo", "copycheck"] {
        assert!(!text.contains(value), "the log holds {value}:\n{text}");
    }
}

// ---- OpenTelemetry export ------------------------------------------------------

/// A sign-in, a yup from its draft to an agreement in force with the
/// invitation shared and claimed on the way, and payment options saved:
/// everything exported arrives at the collector with the resource and the
/// headers, spans and lines agree on their identifiers, the funnel counts
/// each step, and nothing personal is in any of it.
#[tokio::test]
async fn what_is_exported_carries_the_resource_the_headers_and_nothing_personal() {
    let _turn = TURN.lock().await;
    let (collector, address) = Collector::start().await;
    let (layer, exporter) = otel::start(exporting_to(&address));
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Json,
        EnvFilter::new("trace"),
        false,
        move || writer.clone(),
        Some(layer),
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    let before = funnel().counts();

    let codes = Arc::new(Codes::default());
    let app = App::start_messaging(DATABASE, generous(), codes.clone(), false).await;
    let metrics = app.metrics.clone();
    exporter.metrics_from(Arc::new(move || {
        let metrics = metrics.clone();
        Box::pin(async move {
            let mut text = Text::new();
            metrics.render(&mut text);
            funnel().render(&mut text);
            text
        })
    }));

    let secrets = sign_in(&app, &codes).await;
    // Ana proposes, shares the link, Ben takes it, Ana confirms him, he
    // signs; Ana saves a payment option and shows it.
    let deal = app.negotiating().await;
    let shared = app
        .post(
            &deal.ana,
            &format!("/v1/exchanges/{}/invitation/shared", deal.exchange),
            json!({}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::NO_CONTENT, "{}", shared.body);
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    let view = app
        .command(&deal.ben, &deal.exchange, common::accept(&deal.revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    app.call(
        Some(&deal.ana),
        Method::PUT,
        "/v1/me/payment-handles",
        Some(json!({ "venmo": "exportcheck-venmo", "zelle": "exportcheck@zelle.test" })),
        &[],
    )
    .await
    .ok();
    app.view(&deal.ben, &deal.exchange).await;
    app.call(None, Method::GET, "/no/such/page", None, &[])
        .await;

    // Everything buffered goes out at shutdown, the metrics with it.
    exporter.shutdown().await;

    // Each signal arrived, with the headers on every request.
    let received = collector.received();
    for path in ["/v1/traces", "/v1/logs", "/v1/metrics"] {
        assert!(
            received.iter().any(|request| request.path == path),
            "nothing was posted to {path}: {:?}",
            received.iter().map(|r| r.path.as_str()).collect::<Vec<_>>()
        );
    }
    for request in &received {
        assert_eq!(
            request.headers["cf-access-client-id"],
            "test-client-id.access"
        );
        assert_eq!(
            request.headers["cf-access-client-secret"],
            "test-client-secret-value"
        );
        assert_eq!(request.headers["content-type"], "application/json");
        let resource = &request.body[match request.path.as_str() {
            "/v1/traces" => "resourceSpans",
            "/v1/logs" => "resourceLogs",
            _ => "resourceMetrics",
        }][0]["resource"];
        let attributes: Vec<(String, String)> = array(&resource["attributes"])
            .iter()
            .map(|attribute| {
                (
                    attribute["key"].as_str().unwrap().to_owned(),
                    attribute["value"]["stringValue"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect();
        for expected in [
            ("service.name", "yuppers-api"),
            ("service.namespace", "yuppers"),
            ("deployment.environment", "test"),
        ] {
            assert!(
                attributes.contains(&(expected.0.to_owned(), expected.1.to_owned())),
                "{} lacks {expected:?}: {attributes:?}",
                request.path
            );
        }
    }

    // The request's span: named by the route, with the status, never the
    // path; the line about it is in the same trace and span.
    let spans = collector.spans();
    let signed_in = spans
        .iter()
        .find(|span| span["name"] == "POST /v1/auth/sessions")
        .unwrap_or_else(|| panic!("{spans:#?}"));
    assert_eq!(signed_in["kind"], 2, "a server span");
    assert_eq!(
        attribute(signed_in, "http.response.status_code"),
        Some(json!({ "intValue": "200" }))
    );
    assert_eq!(
        attribute(signed_in, "http.route"),
        Some(json!({ "stringValue": "/v1/auth/sessions" }))
    );
    assert_eq!(
        attribute(signed_in, "http.request.method"),
        Some(json!({ "stringValue": "POST" }))
    );
    assert!(attribute(signed_in, "request_id").is_some());
    assert_eq!(attribute(signed_in, "path"), None);
    assert!(signed_in.get("parentSpanId").is_none());
    assert_eq!(signed_in["traceId"].as_str().unwrap().len(), 32);
    assert!(signed_in.get("status").is_none(), "{signed_in}");
    let by_template = spans
        .iter()
        .find(|span| span["name"] == "GET /v1/exchanges/{id}")
        .unwrap_or_else(|| panic!("{spans:#?}"));
    assert!(
        !spans.iter().any(|span| {
            span["name"]
                .as_str()
                .is_some_and(|name| name.contains(&deal.exchange))
        }),
        "a span names an exchange"
    );
    // The database spans: the view, a child of the request's, and the load
    // within it.
    let view = spans
        .iter()
        .find(|span| {
            span["name"] == "exchange.view" && span["parentSpanId"] == by_template["spanId"]
        })
        .unwrap_or_else(|| panic!("{spans:#?}"));
    let load = spans
        .iter()
        .find(|span| span["name"] == "exchange.load" && span["parentSpanId"] == view["spanId"])
        .unwrap_or_else(|| panic!("{spans:#?}"));
    assert_eq!(load["kind"], 3, "a client span");
    assert_eq!(load["traceId"], by_template["traceId"]);
    assert_eq!(
        attribute(load, "db.system.name"),
        Some(json!({ "stringValue": "postgresql" }))
    );
    let unmatched = spans
        .iter()
        .find(|span| span["name"] == "GET unmatched")
        .unwrap_or_else(|| panic!("{spans:#?}"));
    assert_eq!(
        attribute(unmatched, "http.response.status_code"),
        Some(json!({ "intValue": "404" }))
    );

    let logs = collector.logs();
    let completed = logs
        .iter()
        .find(|record| {
            record["body"]["stringValue"] == "request completed"
                && record["traceId"] == signed_in["traceId"]
        })
        .unwrap_or_else(|| panic!("{logs:#?}"));
    assert_eq!(completed["spanId"], signed_in["spanId"]);
    assert_eq!(completed["severityNumber"], 9);
    assert_eq!(completed["severityText"], "INFO");
    assert_eq!(
        attribute(completed, "status"),
        Some(json!({ "intValue": "200" }))
    );
    assert_eq!(
        attribute(completed, "method"),
        Some(json!({ "stringValue": "POST" }))
    );
    assert_eq!(
        attribute(completed, "request_id"),
        attribute(signed_in, "request_id")
    );
    assert!(attribute(completed, "latency_ms").is_some());
    assert_eq!(attribute(completed, "path"), None);
    // The same line is on standard output, with the same identifiers.
    let line = log
        .text()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| {
            event["message"] == "request completed"
                && event["span"]["trace_id"] == signed_in["traceId"]
        })
        .unwrap_or_else(|| panic!("{}", log.text()));
    assert_eq!(line["span"]["span_id"], signed_in["spanId"]);
    assert_eq!(line["span"]["path"], "/v1/auth/sessions");

    // The metrics: requests by route, and the funnel.
    let metrics = collector.metrics();
    let metric = |name: &str| {
        metrics
            .iter()
            .rev()
            .find(|metric| metric["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no {name} in {metrics:#?}"))
    };
    let requests = metric("yuppers.http.requests");
    assert_eq!(requests["unit"], "{request}");
    assert!(
        array(&requests["sum"]["dataPoints"]).iter().any(|point| {
            attribute(point, "route") == Some(json!({ "stringValue": "/v1/auth/sessions" }))
                && attribute(point, "status") == Some(json!({ "stringValue": "2xx" }))
                && point["asInt"] == "2"
        }),
        "{requests:#?}"
    );
    assert_eq!(
        metric("yuppers.http.request.duration")["histogram"]["aggregationTemporality"],
        2
    );
    let value = |name: &str, label: Option<(&str, &str)>| -> u64 {
        let family = metric(name);
        let points = array(&family["sum"]["dataPoints"]);
        let point = points
            .iter()
            .find(|point| match label {
                Some((key, value)) => {
                    attribute(point, key) == Some(json!({ "stringValue": value }))
                }
                None => true,
            })
            .unwrap_or_else(|| panic!("{family:#?}"));
        point["asInt"].as_str().unwrap().parse().unwrap()
    };
    let after = funnel().counts();
    // One sign-in made an account by email; the people of the deal were
    // made directly. Two codes went by email, one by phone.
    assert_eq!(after.accounts_email, before.accounts_email + 1);
    assert_eq!(after.accounts_phone, before.accounts_phone);
    assert_eq!(after.codes_email, before.codes_email + 2);
    assert_eq!(after.codes_phone, before.codes_phone + 1);
    assert_eq!(after.yups_created, before.yups_created + 1);
    assert_eq!(after.invitations_shared, before.invitations_shared + 1);
    assert_eq!(after.invitations_claimed, before.invitations_claimed + 1);
    assert_eq!(after.agreements_in_force, before.agreements_in_force + 1);
    assert_eq!(after.closed, before.closed);
    assert_eq!(
        value("yuppers.accounts.created", Some(("channel", "email"))),
        after.accounts_email
    );
    assert_eq!(
        value("yuppers.codes.sent", Some(("channel", "phone"))),
        after.codes_phone
    );
    assert_eq!(value("yuppers.yups.created", None), after.yups_created);
    assert_eq!(
        value("yuppers.invitations.shared", None),
        after.invitations_shared
    );
    assert_eq!(
        value("yuppers.invitations.claimed", None),
        after.invitations_claimed
    );
    assert_eq!(
        value("yuppers.agreements.in_force", None),
        after.agreements_in_force
    );
    assert_eq!(
        value("yuppers.agreements.closed", Some(("outcome", "completed"))),
        after.closed[4]
    );
    assert_eq!(
        metric("yuppers.telemetry.dropped")["sum"]["isMonotonic"],
        true
    );

    // Nothing personal, in any signal.
    let local_part = secrets.email.split('@').next().unwrap().to_owned();
    let phone_digits = secrets.phone.trim_start_matches('+').to_owned();
    assert_nothing_personal_exported(
        &collector,
        &[
            secrets.email.as_str(),
            local_part.as_str(),
            "telemetry-test.invalid",
            secrets.phone.as_str(),
            phone_digits.as_str(),
            secrets.token.as_str(),
            secrets.cookie.as_str(),
            deal.invitation.as_str(),
            deal.ana.email.as_str(),
            deal.ben.email.as_str(),
            "exportcheck",
            &deal.exchange,
        ],
        &secrets.codes,
    );
}

/// Two people's whole life with the service, texting included: signing in
/// by email and by phone, an invitation bound to an address, text updates
/// turned on and STOP and START replied, payment options, combining
/// accounts, a deletion by a code sent by text. Nothing of theirs is
/// exported.
#[tokio::test]
async fn a_whole_life_with_texting_exports_nothing_personal() {
    let _turn = TURN.lock().await;
    let (collector, address) = Collector::start().await;
    let (layer, exporter) = otel::start(exporting_to(&address));
    let subscriber = telemetry::subscriber(
        LogFormat::Json,
        EnvFilter::new("trace"),
        false,
        std::io::sink,
        Some(layer),
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    let before = funnel().counts();

    let texting = common::texting::texting(DATABASE).await;
    let secrets = texting.whole_life().await;
    let codes = texting.codes();
    assert!(codes.len() >= 8, "{}", codes.len());
    exporter.shutdown().await;

    let after = funnel().counts();
    assert!(after.accounts_phone > before.accounts_phone, "{after:?}");
    assert!(after.codes_phone > before.codes_phone, "{after:?}");
    let spans = collector.spans();
    assert!(
        spans
            .iter()
            .any(|span| span["name"] == "POST /v1/sms/inbound"),
        "{spans:#?}"
    );
    let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
    assert_nothing_personal_exported(&collector, &secrets, &codes);
}

/// A collector that cannot be reached costs the service nothing it would
/// notice: requests answer as fast, readiness holds, one warning is
/// written, and shutdown returns within its time limit.
#[tokio::test]
async fn an_unreachable_collector_slows_nothing_and_fails_nothing() {
    let _turn = TURN.lock().await;
    // A port nothing listens on.
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", taken.local_addr().unwrap());
    drop(taken);
    let (layer, exporter) = otel::start(exporting_to(&address));
    let log = Log::default();
    let writer = log.clone();
    let subscriber = telemetry::subscriber(
        LogFormat::Text,
        EnvFilter::new("info"),
        false,
        move || writer.clone(),
        Some(layer),
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let app = App::start(DATABASE).await;
    let metrics = app.metrics.clone();
    exporter.metrics_from(Arc::new(move || {
        let metrics = metrics.clone();
        Box::pin(async move {
            let mut text = Text::new();
            metrics.render(&mut text);
            text
        })
    }));
    // Past the first flush, so requests are made while exports fail.
    tokio::time::sleep(otel::FLUSH_EVERY + Duration::from_millis(200)).await;
    let ana = app.user("Ana").await;
    for _ in 0..20 {
        let started = Instant::now();
        let reply = app.call(None, Method::GET, "/healthz", None, &[]).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }
    let ready = app.call(None, Method::GET, "/readyz", None, &[]).await;
    assert_eq!(ready.status, StatusCode::NO_CONTENT, "{}", ready.body);
    let me = app.get(&ana, "/v1/me").await;
    assert_eq!(me.status, StatusCode::OK);

    let started = Instant::now();
    exporter.shutdown().await;
    assert!(
        started.elapsed() < otel::SHUTDOWN_TIMEOUT + Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );

    let text = log.text();
    let warnings = text
        .lines()
        .filter(|line| line.contains("telemetry export failed"))
        .count();
    assert_eq!(warnings, 1, "once, not once per attempt:\n{text}");
    assert!(!text.contains("test-client-secret-value"), "{text}");
    assert!(text.contains("request completed"), "{text}");
}
