//! Signing in, sessions and the account endpoints, exercised through the HTTP
//! router against a real database.
//!
//! Needs PostgreSQL and the connection strings from `.env`, and runs in a
//! database of its own, created fresh for each run. Each test uses
//! identifiers of its own under a reserved test domain and number range, and
//! removes what it created. Each also makes its requests from network
//! addresses of its own, so that the limits per address count its requests
//! and nobody else's; those counts are kept only as keyed hashes and expire.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderName, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::postgres::{PgPool, PgPoolOptions};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;
use yuppers_backend::auth::{AuthRules, CodeMessage, CodeSender, SendFuture, token_hash};
use yuppers_backend::code_consent::{self, CODE_CONSENT_VERSION, phone_hash};
use yuppers_backend::contact;

mod common;
use yuppers_backend::domain::Rules;
use yuppers_backend::http::{self, AppState, Settings, TrustedProxies};

const WEB_ORIGIN: &str = "https://app.test";
const EMAIL_DOMAIN: &str = "auth-test.invalid";
const PHONE_PREFIX: &str = "+1999";

/// Keeps the codes the service "sent", so tests can read them back.
#[derive(Default)]
struct Outbox(Mutex<Vec<(String, String)>>);

impl CodeSender for Outbox {
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

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    /// The body exactly as sent.
    bytes: Vec<u8>,
}

impl Reply {
    fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or("")
    }
}

#[derive(Clone)]
struct App {
    router: Router,
    outbox: Arc<Outbox>,
    owner: PgPool,
    /// The application role's connection string, for running the worker's
    /// jobs as the service does.
    app_url: String,
    /// Where the requests come from: the connection's peer.
    peer: IpAddr,
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
        .connect(url)
        .await
        .unwrap_or_else(|error| panic!("cannot connect to {url}: {error}"))
}

fn email() -> String {
    format!("{}@{EMAIL_DOMAIN}", Uuid::new_v4().simple())
}

fn phone() -> String {
    format!("{PHONE_PREFIX}{:07}", Uuid::new_v4().as_u128() % 10_000_000)
}

/// An address of its own, in the range reserved for documentation.
/// The box beside a phone number ticked, with the wording shown in `language`.
fn sms_consent(language: &str) -> Value {
    json!({ "version": CODE_CONSENT_VERSION, "language": language })
}

fn address() -> IpAddr {
    let random = Uuid::new_v4().as_u128();
    IpAddr::V6(Ipv6Addr::from((0x2001_0db8_u128 << 96) | (random >> 32)))
}

/// A code that is none of `codes`.
fn other_than(codes: &[&str]) -> String {
    (0..)
        .map(|n| format!("{n:06}"))
        .find(|candidate| !codes.contains(&candidate.as_str()))
        .unwrap()
}

impl App {
    async fn start() -> Self {
        // A database of its own, created fresh for each run, with the
        // contact data key stored and installed as `migrate` and the api do.
        let (owner_url, app_url) = common::database("yuppers_test_auth").await;
        let owner = connect(owner_url).await;

        let outbox = Arc::new(Outbox::default());
        let state = AppState {
            db: connect(app_url).await,
            settings: Arc::new(Settings {
                app_secret: b"test-secret-test-secret-test-secret".to_vec(),
                web_origin: WEB_ORIGIN.to_owned(),
                auth: AuthRules::default(),
                rules: Default::default(),
                consent_version: "test".to_owned(),
                proxies: TrustedProxies::none(),
                min_client_versions: Default::default(),
                app_links: Default::default(),
                push_notifications: false,
                build: Default::default(),
                sms_updates: false,
                sms_webhook_token: None,
            }),
            code_sender: outbox.clone(),
            metrics: Default::default(),
        };

        Self {
            router: http::router(state, None),
            outbox,
            owner,
            app_url: app_url.clone(),
            peer: address(),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        headers: &[(axum::http::HeaderName, &str)],
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            // What the listener would know about the connection.
            .extension(ConnectInfo(SocketAddr::new(self.peer, 40000)));
        for (name, value) in headers {
            request = request.header(name, *value);
        }
        let request = match body {
            Some(body) => request
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            headers,
            body,
            bytes: bytes.to_vec(),
        }
    }

    /// The same service, reached from another address.
    fn from(&self, peer: IpAddr) -> Self {
        Self {
            peer,
            ..self.clone()
        }
    }

    async fn post(&self, path: &str, body: Value) -> Reply {
        self.send(Method::POST, path, Some(body), &[]).await
    }

    async fn get_as(&self, token: &str, path: &str) -> Reply {
        self.send(
            Method::GET,
            path,
            None,
            &[(AUTHORIZATION, &format!("Bearer {token}"))],
        )
        .await
    }

    async fn request_code(&self, identifier: &str) -> String {
        // A phone number with the box beside it ticked, as the apps send it.
        let reply = self
            .post(
                "/v1/auth/codes",
                json!({ "identifier": identifier, "sms_consent": sms_consent("en") }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
        self.last_code(identifier)
    }

    /// A proof of `own`, the account's identifier of `channel`, from a code
    /// sent to it: what adding or replacing one takes.
    async fn proof(&self, token: &str, channel: &str, own: &str) -> String {
        let code = self.request_code(own).await;
        let reply = self
            .send(
                Method::POST,
                "/v1/me/identifiers/proof",
                Some(json!({ "channel": channel, "code": code })),
                &[(AUTHORIZATION, &format!("Bearer {token}"))],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        reply.body["proof"].as_str().unwrap().to_owned()
    }

    fn last_code(&self, identifier: &str) -> String {
        let sent = self.outbox.0.lock().unwrap();
        let (_, code) = sent
            .iter()
            .rev()
            .find(|(to, _)| to == identifier)
            .unwrap_or_else(|| panic!("no code was sent to {identifier}"));
        code.clone()
    }

    async fn create_session(&self, identifier: &str, code: &str) -> Reply {
        self.post(
            "/v1/auth/sessions",
            json!({ "identifier": identifier, "code": code, "delivery": "TOKEN", "terms_version": yuppers_backend::terms::TERMS_VERSION }),
        )
        .await
    }

    /// Signs in and returns the session token and account ID.
    async fn sign_in(&self, identifier: &str) -> (String, String) {
        let code = self.request_code(identifier).await;
        let reply = self.create_session(identifier, &code).await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        (
            reply.body["token"].as_str().unwrap().to_owned(),
            reply.body["account"]["id"].as_str().unwrap().to_owned(),
        )
    }

    /// Removes the rows belonging to the identifiers this test used, found
    /// by their blind indexes, as the service stores them.
    async fn finish(self, identifiers: &[&str]) {
        for identifier in identifiers {
            for statement in [
                "DELETE FROM account_session WHERE account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)",
                "DELETE FROM terms_acceptance WHERE account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)",
                "DELETE FROM sms_code_consent WHERE account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)",
                "DELETE FROM account_combine_offer WHERE account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)
                    OR other_account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)",
                "DELETE FROM account_proof WHERE account_id IN
                    (SELECT id FROM account WHERE email_index = $1 OR phone_index = $1)",
                "DELETE FROM account WHERE email_index = $1 OR phone_index = $1",
                "DELETE FROM one_time_code WHERE identifier_index = $1",
            ] {
                sqlx::query(statement)
                    .bind(common::index(identifier))
                    .execute(&self.owner)
                    .await
                    .unwrap();
            }
        }
    }
}

#[tokio::test]
async fn signing_in_creates_an_account_and_a_working_session() {
    let app = App::start().await;
    let email = email();

    let code = app.request_code(&email).await;
    let reply = app.create_session(&email, &code).await;

    assert_eq!(reply.status, StatusCode::OK);
    let account = &reply.body["account"];
    assert_eq!(account["email"], email.as_str());
    assert_eq!(account["phone"], Value::Null);
    assert_eq!(account["display_name"], "");
    assert_eq!(account["language"], "en");
    assert_eq!(account["adult_confirmed"], false);

    let token = reply.body["token"].as_str().unwrap();
    let me = app.get_as(token, "/v1/me").await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["id"], account["id"]);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn signing_in_records_the_terms_accepted_and_says_so_on_the_account() {
    let app = App::start().await;
    let email = email();

    let code = app.request_code(&email).await;
    let reply = app
        .post(
            "/v1/auth/sessions",
            json!({
                "identifier": email, "code": code, "delivery": "TOKEN",
                "terms_version": yuppers_backend::terms::TERMS_VERSION, "language": "es-MX",
            }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let version = yuppers_backend::terms::TERMS_VERSION;
    assert_eq!(reply.body["account"]["terms_version"], version);
    assert!(reply.body["account"]["terms_accepted_at"].is_string());
    let token = reply.body["token"].as_str().unwrap();
    let me = app.get_as(token, "/v1/me").await;
    assert_eq!(me.body["terms_version"], version);
    assert_eq!(
        me.body["terms_accepted_at"],
        reply.body["account"]["terms_accepted_at"]
    );

    let rows = |app: &App| {
        let owner = app.owner.clone();
        let id = reply.body["account"]["id"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .unwrap();
        async move {
            sqlx::query_as::<_, (String, String, Option<uuid::Uuid>)>(
                "SELECT terms_version, language, session_id FROM terms_acceptance
                 WHERE account_id = $1 ORDER BY id",
            )
            .bind(id)
            .fetch_all(&owner)
            .await
            .unwrap()
        }
    };
    let first = rows(&app).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, version);
    assert_eq!(first[0].1, "es");
    assert!(first[0].2.is_some());

    // Each sign-in adds a row; none is changed.
    let (_, _) = app.sign_in(&email).await;
    let second = rows(&app).await;
    assert_eq!(second.len(), 2);
    assert_eq!(second[0], first[0]);
    assert_eq!(second[1].1, "es");

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn signing_in_without_a_terms_version_completes_and_records_nothing() {
    let app = App::start().await;
    let email = email();

    // As a build from before the field existed sends it.
    let code = app.request_code(&email).await;
    let reply = app
        .post(
            "/v1/auth/sessions",
            json!({ "identifier": email, "code": code, "delivery": "TOKEN" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["account"]["terms_version"], Value::Null);
    let id = reply.body["account"]["id"]
        .as_str()
        .unwrap()
        .parse::<uuid::Uuid>()
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM terms_acceptance WHERE account_id = $1")
        .bind(id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_terms_version_the_service_does_not_know_signs_nobody_in() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;

    for version in [json!("2020-01-01"), json!("")] {
        let reply = app
            .post(
                "/v1/auth/sessions",
                json!({ "identifier": email, "code": code, "delivery": "TOKEN", "terms_version": version }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.body["code"], "TERMS_VERSION_UNKNOWN");
    }

    // Nothing was made, and the code was not used up.
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM account WHERE email_index = $1")
        .bind(common::index(&email))
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(accounts, 0);
    let reply = app.create_session(&email, &code).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn signing_in_again_reaches_the_same_account() {
    let app = App::start().await;
    let email = email();

    let (_, first) = app.sign_in(&email).await;
    // Typed differently, same address.
    let typed = format!("  {} ", email.to_uppercase());
    let reply = app
        .post("/v1/auth/codes", json!({ "identifier": typed }))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let reply = app.create_session(&typed, &app.last_code(&email)).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["account"]["id"], first.as_str());

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_phone_number_signs_in_too() {
    let app = App::start().await;
    let phone = phone();

    let code = app.request_code(&phone).await;
    let reply = app.create_session(&phone, &code).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["account"]["phone"], phone.as_str());
    assert_eq!(reply.body["account"]["email"], Value::Null);

    app.finish(&[&phone]).await;
}

#[tokio::test]
async fn a_code_works_once() {
    let app = App::start().await;
    let email = email();

    let code = app.request_code(&email).await;
    assert_eq!(
        app.create_session(&email, &code).await.status,
        StatusCode::OK
    );

    let again = app.create_session(&email, &code).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::UNAUTHORIZED, "INVALID_CODE")
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn five_wrong_guesses_kill_a_code() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;
    let wrong = other_than(&[&code]);

    for _ in 0..5 {
        let reply = app.create_session(&email, &wrong).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNAUTHORIZED, "INVALID_CODE")
        );
    }
    // The right code no longer helps.
    let reply = app.create_session(&email, &code).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNAUTHORIZED, "INVALID_CODE")
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_new_code_leaves_the_earlier_ones_working_and_using_one_uses_all() {
    let app = App::start().await;
    let (ana, ben) = (email(), email());

    // Asking again does not end the code already sent: either works, and
    // whichever is used, the other is spent with it.
    let first = app.request_code(&ana).await;
    let second = app.request_code(&ana).await;
    assert_eq!(
        app.create_session(&ana, &first).await.status,
        StatusCode::OK
    );
    if first != second {
        let reply = app.create_session(&ana, &second).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNAUTHORIZED, "INVALID_CODE")
        );
    }

    let first = app.request_code(&ben).await;
    let second = app.request_code(&ben).await;
    assert_eq!(
        app.create_session(&ben, &second).await.status,
        StatusCode::OK
    );
    if first != second {
        let reply = app.create_session(&ben, &first).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNAUTHORIZED, "INVALID_CODE")
        );
    }

    app.finish(&[&ana, &ben]).await;
}

#[tokio::test]
async fn only_the_newest_three_codes_stay_live() {
    let app = App::start().await;
    let email = email();

    let mut codes = Vec::new();
    for _ in 0..4 {
        codes.push(app.request_code(&email).await);
    }
    // The oldest of four is dead, unless by chance it is also one of the
    // three still live.
    if !codes[1..].contains(&codes[0]) {
        let reply = app.create_session(&email, &codes[0]).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNAUTHORIZED, "INVALID_CODE")
        );
    }
    assert_eq!(
        app.create_session(&email, &codes[1]).await.status,
        StatusCode::OK
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_wrong_guess_counts_against_every_live_code() {
    let app = App::start().await;
    let email = email();
    let first = app.request_code(&email).await;
    let second = app.request_code(&email).await;
    let wrong = other_than(&[&first, &second]);

    for _ in 0..5 {
        let reply = app.create_session(&email, &wrong).await;
        assert_eq!(reply.code(), "INVALID_CODE");
    }
    // Each code has now been guessed at five times, and both are dead.
    for code in [&first, &second] {
        let reply = app.create_session(&email, code).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNAUTHORIZED, "INVALID_CODE")
        );
    }

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn failed_guesses_are_capped_per_identifier_until_the_day_ends() {
    let app = App::start().await;
    let email = email();

    // Nineteen wrong guesses, each at a live code, from four addresses: the
    // cap is the identifier's, wherever the guesses come from.
    for batch in 0..4 {
        let from = app.from(address());
        let code = from.request_code(&email).await;
        for _ in 0..(if batch == 3 { 4 } else { 5 }) {
            let reply = from.create_session(&email, &other_than(&[&code])).await;
            assert_eq!(reply.code(), "INVALID_CODE");
        }
    }
    // A code is still sent, and the twentieth wrong guess is made at it.
    let elsewhere = app.from(address());
    let code = elsewhere.request_code(&email).await;
    let reply = elsewhere
        .create_session(&email, &other_than(&[&code]))
        .await;
    assert_eq!(reply.code(), "INVALID_CODE");

    // Now even the right code is refused, from anywhere, and being refused
    // is not a guess.
    for offered in [code.clone(), other_than(&[&code])] {
        let reply = elsewhere.create_session(&email, &offered).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_GUESSES")
        );
    }
    // And no code is sent that could not work.
    let sent = app.outbox.0.lock().unwrap().len();
    let reply = app
        .from(address())
        .post("/v1/auth/codes", json!({ "identifier": email }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_GUESSES")
    );
    assert_eq!(app.outbox.0.lock().unwrap().len(), sent);

    // Another identifier is not affected.
    let other = self::email();
    let other_code = elsewhere.request_code(&other).await;
    assert_eq!(
        elsewhere.create_session(&other, &other_code).await.status,
        StatusCode::OK
    );

    // When the day ends, the code still live works. Today's counts of failed
    // guesses by identifier that reached the limit are moved back a day: the
    // count is stored only as a keyed hash, and no other test comes near the
    // limit, so this is the count for this test's identifier.
    sqlx::query(
        "UPDATE sign_in_limit SET window_start = window_start - interval '1 day'
         WHERE scope = 'failed-guesses-by-identifier'
           AND window_start = date_trunc('day', now(), 'UTC')
           AND count >= 20",
    )
    .execute(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        elsewhere.create_session(&email, &code).await.status,
        StatusCode::OK
    );

    app.finish(&[&email, &other]).await;
}

#[tokio::test]
async fn code_requests_are_limited_per_address() {
    let app = App::start().await;
    let identifiers: Vec<String> = (0..11).map(|_| email()).collect();

    // Ten different identifiers from one address, each within its own limit.
    for identifier in &identifiers[..10] {
        app.request_code(identifier).await;
    }
    let reply = app
        .post("/v1/auth/codes", json!({ "identifier": identifiers[10] }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS")
    );

    // Someone at another address is not held up.
    app.from(address()).request_code(&identifiers[10]).await;

    let all: Vec<&str> = identifiers.iter().map(String::as_str).collect();
    app.finish(&all).await;
}

#[tokio::test]
async fn code_requests_from_one_ipv6_network_are_counted_together() {
    let app = App::start().await;
    let identifiers: Vec<String> = (0..11).map(|_| email()).collect();
    // A /64 of its own under 2001:db8::/32, and eleven addresses in it.
    let network = (0x2001_0db8_u128 << 96) | (Uuid::new_v4().as_u128() >> 96 << 64);
    let host = |n: u128| IpAddr::V6(Ipv6Addr::from(network | n));

    for (n, identifier) in identifiers[..10].iter().enumerate() {
        app.from(host(n as u128 + 1)).request_code(identifier).await;
    }
    let reply = app
        .from(host(0x12))
        .post("/v1/auth/codes", json!({ "identifier": identifiers[10] }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS")
    );
    // The next /64 is someone else.
    let next = IpAddr::V6(Ipv6Addr::from((network ^ (1u128 << 64)) | 1));
    app.from(next).request_code(&identifiers[10]).await;

    let all: Vec<&str> = identifiers.iter().map(String::as_str).collect();
    app.finish(&all).await;
}

#[tokio::test]
async fn an_ipv4_address_written_as_ipv6_is_the_same_requester() {
    let app = App::start().await;
    let identifiers: Vec<String> = (0..11).map(|_| email()).collect();
    // An address of its own in 240.0.0.0/4, which is reserved.
    let v4 = Ipv4Addr::from(0xf000_0000 | (Uuid::new_v4().as_u128() as u32 >> 4));
    let plain = IpAddr::V4(v4);
    let mapped = IpAddr::V6(v4.to_ipv6_mapped());

    for (n, identifier) in identifiers[..10].iter().enumerate() {
        let from = if n % 2 == 0 { plain } else { mapped };
        app.from(from).request_code(identifier).await;
    }
    for from in [plain, mapped] {
        let reply = app
            .from(from)
            .post("/v1/auth/codes", json!({ "identifier": identifiers[10] }))
            .await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS")
        );
    }

    let all: Vec<&str> = identifiers.iter().map(String::as_str).collect();
    app.finish(&all).await;
}

#[tokio::test]
async fn made_up_identifiers_do_not_use_up_an_address_that_others_share() {
    let app = App::start().await;
    let email = email();
    let code = app.from(address()).request_code(&email).await;

    // Thirty wrong guesses and more from one address, at identifiers that
    // have no code at all: as many people behind one office or carrier
    // address mistyping theirs.
    for _ in 0..35 {
        let reply = app.create_session(&self::email(), "123456").await;
        assert_eq!(reply.code(), "INVALID_CODE");
    }
    // Someone else at that address signs in with their right code.
    assert_eq!(
        app.create_session(&email, &code).await.status,
        StatusCode::OK
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn many_wrong_guesses_from_one_address_never_say_who_is_signing_in() {
    let app = App::start().await;
    let elsewhere = app.from(address());
    // Nine identifiers with a live code each, asked for elsewhere: five wrong
    // guesses kill a code, so forty need eight of them, and the ninth is
    // still live afterwards.
    let mut targets: Vec<(String, String)> = Vec::new();
    for _ in 0..9 {
        let identifier = email();
        let code = elsewhere.request_code(&identifier).await;
        targets.push((identifier, code));
    }
    let email = email();
    let code = elsewhere.request_code(&email).await;

    // Forty wrong guesses at live codes from one address, more than the
    // thirty an hour that an address was once allowed. Each is the same
    // refusal as any other wrong code.
    for (identifier, live) in &targets[..8] {
        for _ in 0..5 {
            let reply = app.create_session(identifier, &other_than(&[live])).await;
            assert_eq!(
                (reply.status, reply.code()),
                (StatusCode::UNAUTHORIZED, "INVALID_CODE")
            );
        }
    }

    // Now a wrong code where a code is live, and one where nothing was ever
    // asked for, get exactly the same answer: nothing tells the guesser that
    // someone asked for a code a moment ago.
    let (target, live) = &targets[8];
    let at_live = app.create_session(target, &other_than(&[live])).await;
    let at_nothing = app.create_session(&self::email(), "123456").await;
    assert_eq!(at_live.status, StatusCode::UNAUTHORIZED);
    assert_eq!(at_live.status, at_nothing.status);
    assert_eq!(at_live.bytes, at_nothing.bytes);
    assert_eq!(
        at_live.headers.get(CONTENT_TYPE),
        at_nothing.headers.get(CONTENT_TYPE)
    );

    // The guess at the live code was still charged to it.
    let charged: i16 =
        sqlx::query_scalar("SELECT failed_attempts FROM one_time_code WHERE identifier_index = $1")
            .bind(common::index(target))
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(charged, 1);
    // ... so four more kill it, and then even its right code is refused.
    for _ in 0..4 {
        let reply = app.create_session(target, &other_than(&[live])).await;
        assert_eq!(reply.code(), "INVALID_CODE");
    }
    assert_eq!(
        app.create_session(target, live).await.code(),
        "INVALID_CODE"
    );

    // From that address the right code for another identifier still works,
    // so people sharing an address cannot lock each other out.
    assert_eq!(
        app.create_session(&email, &code).await.status,
        StatusCode::OK
    );

    let mut all: Vec<&str> = targets
        .iter()
        .map(|(identifier, _)| identifier.as_str())
        .collect();
    all.push(&email);
    app.finish(&all).await;
}

#[tokio::test]
async fn counts_from_windows_long_past_are_forgotten() {
    let app = App::start().await;
    let (old, current) = (*Uuid::new_v4().as_bytes(), *Uuid::new_v4().as_bytes());
    let (old, current) = ([old, old].concat(), [current, current].concat());
    for (subject, age) in [(&old, "3 days"), (&current, "0 days")] {
        sqlx::query(
            "INSERT INTO sign_in_limit (scope, subject, window_start, count)
             VALUES ('code-requests-by-address', $1,
                     date_trunc('hour', now()) - $2::interval, 1)",
        )
        .bind(subject.as_slice())
        .bind(age)
        .execute(&app.owner)
        .await
        .unwrap();
    }

    let service = connect(&app.app_url).await;
    yuppers_backend::auth::purge_sign_in_limits(&service)
        .await
        .unwrap();

    let left: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT subject FROM sign_in_limit WHERE subject = ANY($1)")
            .bind(vec![old.clone(), current.clone()])
            .fetch_all(&app.owner)
            .await
            .unwrap();
    assert_eq!(left, vec![current.clone()]);

    sqlx::query("DELETE FROM sign_in_limit WHERE subject = $1")
        .bind(current.as_slice())
        .execute(&app.owner)
        .await
        .unwrap();
}

/// A code, and with it the address or number it went to, is kept a day at
/// most once it has expired or been used: long enough for the hourly limit
/// per identifier to count it, and no longer.
#[tokio::test]
async fn codes_are_removed_a_day_after_they_are_done_with() {
    let app = App::start().await;
    let (old, used, live) = (email(), email(), email());
    for email in [&old, &used, &live] {
        app.request_code(email).await;
    }
    for (email, change) in [
        (
            &old,
            "created_at = now() - interval '2 days', \
             expires_at = now() - interval '2 days' + interval '10 minutes'",
        ),
        // Younger than the ten minutes after which another test starting
        // meanwhile clears test rows away (`App::start`).
        (
            &used,
            "created_at = now() - interval '5 minutes', consumed_at = now() - interval '5 minutes'",
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE one_time_code SET {change} WHERE identifier_index = $1"
        )))
        .bind(common::index(email))
        .execute(&app.owner)
        .await
        .unwrap();
    }

    let service = connect(&app.app_url).await;
    yuppers_backend::auth::purge_one_time_codes(&service)
        .await
        .unwrap();

    let index = common::index;
    let left: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT identifier_index FROM one_time_code WHERE identifier_index = ANY($1)
         ORDER BY created_at",
    )
    .bind(vec![index(&old), index(&used), index(&live)])
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(left, [index(&used), index(&live)]);

    app.finish(&[&old, &used, &live]).await;
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;

    sqlx::query("UPDATE one_time_code SET expires_at = now() WHERE identifier_index = $1")
        .bind(common::index(&email))
        .execute(&app.owner)
        .await
        .unwrap();

    let reply = app.create_session(&email, &code).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNAUTHORIZED, "INVALID_CODE")
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn code_requests_are_limited_per_identifier() {
    let app = App::start().await;
    let email = email();

    for _ in 0..5 {
        app.request_code(&email).await;
    }
    let reply = app
        .post("/v1/auth/codes", json!({ "identifier": email }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS")
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn bad_input_gets_a_typed_refusal() {
    let app = App::start().await;

    let reply = app
        .post("/v1/auth/codes", json!({ "identifier": "not an address" }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_IDENTIFIER")
    );

    let reply = app
        .post("/v1/auth/codes", json!({ "wrong": "shape" }))
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST")
    );

    let reply = app
        .send(
            Method::POST,
            "/v1/auth/codes",
            None,
            &[(CONTENT_TYPE, "application/json")],
        )
        .await;
    assert_eq!(reply.code(), "INVALID_REQUEST");
}

#[tokio::test]
async fn account_endpoints_need_a_session() {
    let app = App::start().await;

    let none = app.send(Method::GET, "/v1/me", None, &[]).await;
    assert_eq!(
        (none.status, none.code()),
        (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED")
    );

    let garbage = app.get_as("not-a-token", "/v1/me").await;
    assert_eq!(
        (garbage.status, garbage.code()),
        (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED")
    );
}

#[tokio::test]
async fn signing_out_ends_the_session() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;
    let bearer = format!("Bearer {token}");

    let reply = app
        .send(
            Method::DELETE,
            "/v1/auth/session",
            None,
            &[(AUTHORIZATION, &bearer)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn an_expired_session_stops_working() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;

    sqlx::query("UPDATE account_session SET expires_at = now() WHERE token_hash = $1")
        .bind(token_hash(&token).as_slice())
        .execute(&app.owner)
        .await
        .unwrap();
    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    app.finish(&[&email]).await;
}

/// What the service holds about a session: when it ends, when its holder
/// last proved an identifier, and the row version (`xmin`), which changes
/// whenever the row is written.
async fn session_row(app: &App, token: &str) -> (OffsetDateTime, OffsetDateTime, String) {
    sqlx::query_as(
        "SELECT expires_at, authenticated_at, xmin::text FROM account_session
         WHERE token_hash = $1",
    )
    .bind(token_hash(token).as_slice())
    .fetch_one(&app.owner)
    .await
    .unwrap()
}

/// Moves a session back in time: made `made_days_ago` days ago (fractions
/// allowed), and ending `ends_in_days` days from now (negative for the past).
async fn age_session(app: &App, token: &str, made_days_ago: f64, ends_in_days: f64) {
    sqlx::query(
        "UPDATE account_session
         SET created_at = now() - $2 * interval '1 day',
             authenticated_at = now() - $2 * interval '1 day',
             expires_at = now() + $3 * interval '1 day'
         WHERE token_hash = $1",
    )
    .bind(token_hash(token).as_slice())
    .bind(made_days_ago)
    .bind(ends_in_days)
    .execute(&app.owner)
    .await
    .unwrap();
}

/// How far `at` is from `days` days from now, in seconds.
fn off_by(at: OffsetDateTime, days: i64) -> i64 {
    (at - (OffsetDateTime::now_utc() + time::Duration::days(days)))
        .whole_seconds()
        .abs()
}

#[tokio::test]
async fn a_session_in_use_slides_but_is_written_at_most_once_a_day() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;

    // A new session lasts the idle time.
    let (expires, proved, version) = session_row(&app, &token).await;
    assert!(off_by(expires, 30) < 60, "{expires}");

    // Used within its first day, it is not written at all.
    for _ in 0..3 {
        assert_eq!(app.get_as(&token, "/v1/me").await.status, StatusCode::OK);
    }
    assert_eq!(session_row(&app, &token).await, (expires, proved, version));

    // Used two days later, it ends 30 days after that use.
    age_session(&app, &token, 2.0, 28.0).await;
    let (_, proved, version) = session_row(&app, &token).await;
    assert_eq!(app.get_as(&token, "/v1/me").await.status, StatusCode::OK);
    let (renewed, still_proved, renewed_version) = session_row(&app, &token).await;
    assert!(off_by(renewed, 30) < 60, "{renewed}");
    assert_ne!(renewed_version, version, "renewing is one write");
    // When the identifier was last proved, which signing and the staff
    // endpoints check, is not touched.
    assert_eq!(still_proved, proved);

    // Used again the same day: nothing more is written.
    assert_eq!(app.get_as(&token, "/v1/me").await.status, StatusCode::OK);
    assert_eq!(
        session_row(&app, &token).await,
        (renewed, proved, renewed_version)
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_session_left_unused_for_the_idle_time_ends() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;

    // Last used 30 days and a moment ago.
    age_session(&app, &token, 40.0, -0.001).await;
    let before = session_row(&app, &token).await;
    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    // Using it does not bring it back.
    assert_eq!(session_row(&app, &token).await, before);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn constant_use_never_carries_a_session_past_its_absolute_end() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;

    // Made 175 days ago and used every day since: renewed only to 180 days
    // after it was made, five days from now.
    age_session(&app, &token, 175.0, 3.0).await;
    assert_eq!(app.get_as(&token, "/v1/me").await.status, StatusCode::OK);
    let (expires, ..) = session_row(&app, &token).await;
    assert!(off_by(expires, 5) < 60, "{expires}");

    // Once there, using it moves nothing.
    let before = session_row(&app, &token).await;
    assert_eq!(app.get_as(&token, "/v1/me").await.status, StatusCode::OK);
    assert_eq!(session_row(&app, &token).await, before);

    // Past 180 days it is refused, even with time left on it (as when a
    // deployment shortens SESSION_MAX_DAYS), and a new code is needed.
    age_session(&app, &token, 180.01, 10.0).await;
    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    let (fresh, _) = app.sign_in(&email).await;
    assert_eq!(app.get_as(&fresh, "/v1/me").await.status, StatusCode::OK);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn signing_out_ends_a_session_due_for_renewal() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;
    age_session(&app, &token, 2.0, 28.0).await;

    let reply = app
        .send(
            Method::DELETE,
            "/v1/auth/session",
            None,
            &[(AUTHORIZATION, &format!("Bearer {token}"))],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    app.finish(&[&email]).await;
}

/// The `Max-Age` of the session cookie a response sets, if it sets one.
fn cookie_max_age(reply: &Reply) -> Option<i64> {
    let cookies: Vec<&str> = reply
        .headers
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .filter(|value| value.starts_with("yuppers_session="))
        .collect();
    assert!(
        cookies.len() <= 1,
        "one session cookie at most: {cookies:?}"
    );
    let cookie = cookies.first()?;
    for attribute in ["HttpOnly", "SameSite=Lax", "Secure", "Path=/"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    cookie
        .split("; ")
        .find_map(|part| part.strip_prefix("Max-Age="))
        .map(|age| age.parse().unwrap())
}

#[tokio::test]
async fn a_renewed_web_session_sends_its_cookie_again_to_match() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;
    let reply = app
        .send(
            Method::POST,
            "/v1/auth/sessions",
            Some(json!({ "identifier": email, "code": code, "delivery": "COOKIE", "terms_version": yuppers_backend::terms::TERMS_VERSION })),
            &[(ORIGIN, WEB_ORIGIN)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(cookie_max_age(&reply), Some(30 * 86_400));
    let cookie = reply.headers[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let token = cookie.strip_prefix("yuppers_session=").unwrap().to_owned();
    let with_cookie = [(COOKIE, cookie.as_str())];
    let me = || app.send(Method::GET, "/v1/me", None, &with_cookie);

    // Not renewed: no cookie.
    assert_eq!(cookie_max_age(&me().await), None);

    // Renewed: the cookie again, lasting as long as the session now does.
    age_session(&app, &token, 2.0, 28.0).await;
    let renewed = me().await;
    assert_eq!(renewed.status, StatusCode::OK);
    let age = cookie_max_age(&renewed).expect("the cookie is sent again");
    assert!((30 * 86_400 - age).abs() < 60, "{age}");
    assert_eq!(cookie_max_age(&me().await), None, "once a day at most");

    // Near the absolute end, no longer than what is left of it.
    age_session(&app, &token, 170.0, 5.0).await;
    let age = cookie_max_age(&me().await).expect("the cookie is sent again");
    assert!((10 * 86_400 - age).abs() < 60, "{age}");

    // A bearer token never gets a cookie.
    age_session(&app, &token, 2.0, 28.0).await;
    let bearer = app.get_as(&token, "/v1/me").await;
    assert_eq!(bearer.status, StatusCode::OK);
    assert_eq!(cookie_max_age(&bearer), None);

    // Signing out with a session due for renewal clears the cookie, and
    // sends nothing that would put it back.
    age_session(&app, &token, 2.0, 28.0).await;
    let out = app
        .send(
            Method::DELETE,
            "/v1/auth/session",
            None,
            &[(COOKIE, &cookie), (ORIGIN, WEB_ORIGIN)],
        )
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(cookie_max_age(&out), Some(0));
    assert_eq!(me().await.status, StatusCode::UNAUTHORIZED);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_web_session_is_a_cookie_scripts_cannot_read() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;
    let body = json!({ "identifier": email, "code": code, "delivery": "COOKIE", "terms_version": yuppers_backend::terms::TERMS_VERSION });

    // Not from our web app: refused before the code is even looked at.
    let foreign = app
        .send(
            Method::POST,
            "/v1/auth/sessions",
            Some(body.clone()),
            &[(ORIGIN, "https://elsewhere.test")],
        )
        .await;
    assert_eq!(foreign.status, StatusCode::UNAUTHORIZED);

    let reply = app
        .send(
            Method::POST,
            "/v1/auth/sessions",
            Some(body),
            &[(ORIGIN, WEB_ORIGIN)],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(
        reply.body["token"],
        Value::Null,
        "the token is not exposed to the page"
    );

    let set_cookie = reply.headers[SET_COOKIE].to_str().unwrap();
    for attribute in ["HttpOnly", "SameSite=Lax", "Secure", "Path=/"] {
        assert!(set_cookie.contains(attribute), "{set_cookie}");
    }
    let cookie = set_cookie.split(';').next().unwrap().to_owned();

    // Reading needs only the cookie.
    let me = app
        .send(Method::GET, "/v1/me", None, &[(COOKIE, &cookie)])
        .await;
    assert_eq!(me.status, StatusCode::OK);

    // Changing anything also needs to come from our web app.
    let update = json!({ "display_name": "Ana" });
    let forged = app
        .send(
            Method::PATCH,
            "/v1/me",
            Some(update.clone()),
            &[(COOKIE, &cookie)],
        )
        .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    let forged = app
        .send(
            Method::PATCH,
            "/v1/me",
            Some(update.clone()),
            &[(COOKIE, &cookie), (ORIGIN, "https://elsewhere.test")],
        )
        .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    let genuine = app
        .send(
            Method::PATCH,
            "/v1/me",
            Some(update),
            &[(COOKIE, &cookie), (ORIGIN, WEB_ORIGIN)],
        )
        .await;
    assert_eq!(genuine.status, StatusCode::OK);
    assert_eq!(genuine.body["display_name"], "Ana");

    // Signing out clears the cookie.
    let out = app
        .send(
            Method::DELETE,
            "/v1/auth/session",
            None,
            &[(COOKIE, &cookie), (ORIGIN, WEB_ORIGIN)],
        )
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(
        out.headers[SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let me = app
        .send(Method::GET, "/v1/me", None, &[(COOKIE, &cookie)])
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn the_profile_can_be_edited() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;
    let bearer = format!("Bearer {token}");
    let patch = |body: Value| {
        let (app, bearer) = (&app, &bearer);
        async move {
            app.send(
                Method::PATCH,
                "/v1/me",
                Some(body),
                &[(AUTHORIZATION, bearer)],
            )
            .await
        }
    };

    let reply = patch(json!({
        "display_name": "  Ana Ruiz ",
        "language": "es",
        "adult_confirmed": true,
    }))
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["display_name"], "Ana Ruiz");
    assert_eq!(reply.body["language"], "es");
    assert_eq!(reply.body["adult_confirmed"], true);

    // A confirmation of adulthood cannot be taken back, and omitted fields stay.
    let reply = patch(json!({ "adult_confirmed": false })).await;
    assert_eq!(reply.body["adult_confirmed"], true);
    assert_eq!(reply.body["display_name"], "Ana Ruiz");

    for bad in [
        json!({ "display_name": "   " }),
        json!({ "display_name": "x".repeat(101) }),
    ] {
        let reply = patch(bad).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST")
        );
    }
    // A language the product does not speak is refused; a regional variant of
    // one it does falls back to the base language.
    let reply = patch(json!({ "language": "tlh" })).await;
    assert_eq!(reply.code(), "INVALID_REQUEST");
    let reply = patch(json!({ "language": "en-GB" })).await;
    assert_eq!(reply.body["language"], "en");

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn a_second_identifier_can_be_verified_and_then_signs_in() {
    let app = App::start().await;
    let (email, phone) = (email(), phone());
    let (token, account_id) = app.sign_in(&email).await;

    // A session alone adds no way in: first a proof of the email.
    let code = app.request_code(&phone).await;
    let add = |proof: Option<String>| {
        let (app, token, phone, code) = (&app, &token, &phone, &code);
        async move {
            app.send(
                Method::POST,
                "/v1/me/identifiers",
                Some(json!({ "identifier": phone, "code": code, "proof": proof })),
                &[(AUTHORIZATION, &format!("Bearer {token}"))],
            )
            .await
        }
    };
    let reply = add(None).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::CONFLICT, "PROOF_REQUIRED")
    );
    // A made-up proof is refused too, before the code is spent.
    let reply = add(Some("f".repeat(64))).await;
    assert_eq!(reply.code(), "PROOF_REQUIRED");
    let proof = app.proof(&token, "EMAIL", &email).await;
    let reply = add(Some(proof)).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["email"], email.as_str());
    assert_eq!(reply.body["phone"], phone.as_str());

    let (_, by_phone) = app.sign_in(&phone).await;
    assert_eq!(by_phone, account_id);

    app.finish(&[&email, &phone]).await;
}

#[tokio::test]
async fn an_identifier_belongs_to_one_account() {
    let app = App::start().await;
    let (ana, ben, cleo) = (email(), phone(), email());
    app.sign_in(&ana).await;
    let (ben_token, _) = app.sign_in(&ben).await;

    // An account with an email address of its own is asked first for a proof
    // of one of its own, before any code for another is looked at.
    let (cleo_token, _) = app.sign_in(&cleo).await;
    let reply = app
        .send(
            Method::POST,
            "/v1/me/identifiers",
            Some(json!({ "identifier": ana, "code": "000000" })),
            &[(AUTHORIZATION, &format!("Bearer {cleo_token}"))],
        )
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::CONFLICT, "PROOF_REQUIRED")
    );

    // Ben, signed in by number, holds a valid code for Ana's address, but it
    // is already hers. It stays hers: the code only shows he controls both,
    // so he is offered to combine the two, and nothing moves until he does.
    let proof = app.proof(&ben_token, "PHONE", &ben).await;
    let code = app.request_code(&ana).await;
    let reply = app
        .send(
            Method::POST,
            "/v1/me/identifiers",
            Some(json!({ "identifier": ana, "code": code, "proof": proof })),
            &[(AUTHORIZATION, &format!("Bearer {ben_token}"))],
        )
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::CONFLICT, "IDENTIFIER_ON_OTHER_ACCOUNT")
    );
    assert!(reply.body["combine"]["token"].is_string(), "{}", reply.body);

    app.finish(&[&ana, &ben, &cleo]).await;
}

#[tokio::test]
async fn a_suspended_account_is_shut_out() {
    let app = App::start().await;
    let email = email();
    let (token, _) = app.sign_in(&email).await;

    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE email_index = $1")
        .bind(common::index(&email))
        .execute(&app.owner)
        .await
        .unwrap();

    assert_eq!(
        app.get_as(&token, "/v1/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    let code = app.request_code(&email).await;
    let reply = app.create_session(&email, &code).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::FORBIDDEN, "ACCOUNT_SUSPENDED")
    );

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn neither_codes_nor_tokens_are_stored() {
    let app = App::start().await;
    let email = email();
    let code = app.request_code(&email).await;

    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT code_hash FROM one_time_code WHERE identifier_index = $1")
            .bind(common::index(&email))
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(stored.len(), 32);
    assert_ne!(stored, code.as_bytes());
    // Keyed: knowing the code is not enough to reproduce the stored hash.
    assert_ne!(stored, token_hash(&code));

    let reply = app.create_session(&email, &code).await;
    let token = reply.body["token"].as_str().unwrap();
    let hashes: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT s.token_hash FROM account_session s JOIN account a ON a.id = s.account_id
         WHERE a.email_index = $1",
    )
    .bind(common::index(&email))
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(hashes, vec![token_hash(token).to_vec()]);

    app.finish(&[&email]).await;
}

// ---- Consent to a code by text ------------------------------------------------

/// `APP_SECRET` as these tests set it, which keys a number's hash.
const SECRET: &[u8] = b"test-secret-test-secret-test-secret";

/// One record of consent to a code by text, as stored, its number
/// decrypted.
#[derive(Debug, PartialEq, sqlx::FromRow)]
struct CodeConsent {
    /// Read to decrypt the number, which is bound to it; then left out.
    id: i64,
    purpose: String,
    account_id: Option<Uuid>,
    #[sqlx(skip)]
    phone: Option<String>,
    phone_encrypted: Option<Vec<u8>>,
    source: String,
    consent_version: String,
    consent_language: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
}

/// Every consent recorded for a number, oldest first. The encrypted number
/// is decrypted into `phone` and then left out, so that records compare by
/// what they say.
async fn code_consents(app: &App, phone: &str) -> Vec<CodeConsent> {
    let records: Vec<CodeConsent> = sqlx::query_as(
        "SELECT c.id, c.purpose, c.account_id, c.phone_encrypted,
                c.source, c.consent_version, c.consent_language,
                host(n.ip_address) AS ip_address, n.user_agent
         FROM sms_code_consent c
         LEFT JOIN sms_code_consent_network n ON n.consent_id = c.id
         WHERE c.phone_hash = $1
         ORDER BY c.id",
    )
    .bind(phone_hash(SECRET, phone).as_slice())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    records
        .into_iter()
        .map(|record| {
            let field = contact::Field::SMS_CODE_CONSENT_PHONE.row(record.id);
            let phone = record
                .phone_encrypted
                .as_deref()
                .map(|sealed| common::open(field, sealed));
            CodeConsent {
                id: 0,
                phone,
                phone_encrypted: None,
                ..record
            }
        })
        .collect()
}

/// Removes the consents recorded for numbers this test used.
async fn forget_consents(app: &App, phones: &[&str]) {
    for phone in phones {
        sqlx::query("DELETE FROM sms_code_consent WHERE phone_hash = $1")
            .bind(phone_hash(SECRET, phone).as_slice())
            .execute(&app.owner)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_code_by_text_needs_the_box_ticked_and_nothing_is_counted_without_it() {
    let app = App::start().await;
    let phone = phone();

    // A form without the box, or a client from before it: refused, more
    // times than the address may ask for codes in an hour, none of them
    // counted and nothing sent.
    for _ in 0..=AuthRules::default().code_requests_per_address_per_hour {
        let reply = app
            .post("/v1/auth/codes", json!({ "identifier": phone }))
            .await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED")
        );
    }
    // Wording the service no longer knows is no consent to what it sends
    // now; a language it does not speak is a malformed request.
    let outdated = json!({ "version": "2026-10-05", "language": "en" });
    let reply = app
        .post(
            "/v1/auth/codes",
            json!({ "identifier": phone, "sms_consent": outdated }),
        )
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED")
    );
    let reply = app
        .post(
            "/v1/auth/codes",
            json!({ "identifier": phone, "sms_consent": sms_consent("tlh") }),
        )
        .await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST")
    );
    assert!(
        app.outbox
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(to, _)| to != &phone)
    );
    assert!(code_consents(&app, &phone).await.is_empty());

    // Ticked, from the same address, which has asked for nothing yet.
    let code = app.request_code(&phone).await;
    assert_eq!(
        app.create_session(&phone, &code).await.status,
        StatusCode::OK
    );

    forget_consents(&app, &[&phone]).await;
    app.finish(&[&phone]).await;
}

#[tokio::test]
async fn a_code_by_text_is_recorded_with_its_consent() {
    let app = App::start().await;
    let phone = phone();
    let user_agent = "Yuppers/1.4 CFNetwork Darwin";
    let client_version = HeaderName::from_static("x-client-version");

    // The first time, the number is nobody's: kept only as its hash.
    let reply = app
        .send(
            Method::POST,
            "/v1/auth/codes",
            Some(json!({ "identifier": phone, "sms_consent": sms_consent("es-MX") })),
            &[
                (USER_AGENT, user_agent),
                (client_version.clone(), "ios/1.4.0"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    let first = CodeConsent {
        id: 0,
        purpose: "SIGN_IN".to_owned(),
        account_id: None,
        phone: None,
        phone_encrypted: None,
        source: "IOS".to_owned(),
        consent_version: CODE_CONSENT_VERSION.to_owned(),
        consent_language: "es".to_owned(),
        ip_address: Some(app.peer.to_string()),
        user_agent: Some(user_agent.to_owned()),
    };
    assert_eq!(code_consents(&app, &phone).await, [first]);
    let in_full: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sms_code_consent
         WHERE phone_hash = $1 AND phone_encrypted IS NOT NULL",
    )
    .bind(phone_hash(SECRET, &phone).as_slice())
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        in_full, 0,
        "the number itself is nowhere in the record, not even encrypted"
    );

    // Signed up with it, the number is the account's: kept in full with it,
    // encrypted.
    let code = app.last_code(&phone);
    let account: Uuid = app.create_session(&phone, &code).await.body["account"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let reply = app
        .send(
            Method::POST,
            "/v1/auth/codes",
            Some(json!({ "identifier": phone, "sms_consent": sms_consent("en") })),
            &[(client_version, "web/1.4.0")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    let records = code_consents(&app, &phone).await;
    assert_eq!(records.len(), 2);
    let second = &records[1];
    assert_eq!(
        (
            second.purpose.as_str(),
            second.account_id,
            second.phone.as_deref(),
            second.source.as_str(),
            second.consent_language.as_str(),
            second.user_agent.as_deref(),
        ),
        (
            "SIGN_IN",
            Some(account),
            Some(phone.as_str()),
            "WEB",
            "en",
            None
        )
    );

    forget_consents(&app, &[&phone]).await;
    app.finish(&[&phone]).await;
}

#[tokio::test]
async fn a_code_by_email_needs_no_consent_and_records_none() {
    let app = App::start().await;
    let email = email();

    let reply = app
        .post("/v1/auth/codes", json!({ "identifier": email }))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    // Whatever is sent with it is ignored.
    let nonsense = json!({ "version": "?", "language": "?" });
    let reply = app
        .post(
            "/v1/auth/codes",
            json!({ "identifier": email, "sms_consent": nonsense }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
    let code = app.last_code(&email);
    assert_eq!(
        app.create_session(&email, &code).await.status,
        StatusCode::OK
    );
    assert!(code_consents(&app, &email).await.is_empty());

    app.finish(&[&email]).await;
}

#[tokio::test]
async fn checking_a_number_added_to_an_account_records_that_purpose() {
    let app = App::start().await;
    let (email, phone) = (email(), phone());
    let (token, account) = app.sign_in(&email).await;
    let bearer = format!("Bearer {token}");

    let ask = |consent: Option<Value>| {
        let mut body = json!({ "identifier": phone });
        if let Some(consent) = consent {
            body["sms_consent"] = consent;
        }
        let (app, bearer) = (&app, &bearer);
        async move {
            app.send(
                Method::POST,
                "/v1/auth/codes",
                Some(body),
                &[(AUTHORIZATION, bearer.as_str())],
            )
            .await
        }
    };
    let reply = ask(None).await;
    assert_eq!(
        (reply.status, reply.code()),
        (StatusCode::UNPROCESSABLE_ENTITY, "SMS_CONSENT_REQUIRED")
    );
    let reply = ask(Some(sms_consent("en"))).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);

    // The account asking, and the number as a hash only: it is not the
    // account's until the code comes back.
    let records = code_consents(&app, &phone).await;
    let account: Uuid = account.parse().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record.purpose.as_str(),
                record.account_id,
                record.phone.as_deref()
            ))
            .collect::<Vec<_>>(),
        [("VERIFY_NUMBER", Some(account), None)]
    );
    let code = app.last_code(&phone);
    let proof = app.proof(&token, "EMAIL", &email).await;
    let reply = app
        .send(
            Method::POST,
            "/v1/me/identifiers",
            Some(json!({ "identifier": phone, "code": code, "proof": proof })),
            &[(AUTHORIZATION, bearer.as_str())],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);

    forget_consents(&app, &[&phone]).await;
    app.finish(&[&email, &phone]).await;
}

#[tokio::test]
async fn code_consents_are_kept_for_their_retention_and_their_addresses_for_less() {
    let app = App::start().await;
    let (old, recent) = (phone(), phone());
    app.request_code(&old).await;
    app.request_code(&recent).await;
    let rules = Rules::default();

    // One record older than the retention; the other's address and user
    // agent older than a signature's.
    sqlx::query(
        "UPDATE sms_code_consent
         SET created_at = created_at - interval '1 day' - $2 * interval '1 second'
         WHERE phone_hash = $1",
    )
    .bind(phone_hash(SECRET, &old).as_slice())
    .bind(rules.sms_consent_retention.whole_seconds() as f64)
    .execute(&app.owner)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE sms_code_consent_network
         SET recorded_at = recorded_at - interval '1 day' - $2 * interval '1 second'
         WHERE consent_id IN (SELECT id FROM sms_code_consent WHERE phone_hash = $1)",
    )
    .bind(phone_hash(SECRET, &recent).as_slice())
    .bind(rules.network_metadata_retention.whole_seconds() as f64)
    .execute(&app.owner)
    .await
    .unwrap();

    // As the worker runs it, as the service's own role.
    let service = connect(&app.app_url).await;
    code_consent::purge(&service, &rules, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(code_consents(&app, &old).await.is_empty());
    let kept = code_consents(&app, &recent).await;
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (kept[0].ip_address.as_deref(), kept[0].user_agent.as_deref()),
        (None, None)
    );

    forget_consents(&app, &[&old, &recent]).await;
    app.finish(&[&old, &recent]).await;
}
