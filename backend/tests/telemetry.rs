//! What the service writes to its log and counts in its metrics, read back
//! from a real request (docs/operations.md, "Logs" and "Metrics").
//!
//! Needs PostgreSQL and the connection strings from `.env`, like the other
//! API tests; it gets a database of its own.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use common::App;
use serde_json::{Value, json};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use yuppers_backend::auth::{CodeMessage, CodeSender, SendFuture};
use yuppers_backend::domain::Rules;
use yuppers_backend::metrics::Text;
use yuppers_backend::telemetry::{self, LogFormat};

/// Every test here takes turns. A log subscriber set for one test's thread
/// misses events while other threads are registering the same call sites,
/// so nothing else may run while a test reads back what was logged.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const DATABASE: &str = "yuppers_test_telemetry";

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
        let without_time = match format {
            LogFormat::Text => line.split_once(' ').map_or("", |(_, rest)| rest).to_owned(),
            LogFormat::Json => {
                let mut event: Value = serde_json::from_str(line).unwrap();
                event.as_object_mut().unwrap().remove("timestamp");
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

async fn signed_in_with_log(format: LogFormat) -> (String, Secrets) {
    let _turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    // Everything, at every level, so nothing is missed by being quieter
    // than a deployment might set it.
    let subscriber = telemetry::subscriber(format, EnvFilter::new("trace"), false, move || {
        writer.clone()
    });
    // A test runs on one thread, so this holds for everything it does.
    let _guard = tracing::subscriber::set_default(subscriber);

    let codes = Arc::new(Codes::default());
    let app = App::start_sending(DATABASE, Rules::default(), codes.clone()).await;
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
    let subscriber =
        telemetry::subscriber(LogFormat::Json, EnvFilter::new("trace"), false, move || {
            writer.clone()
        });
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
