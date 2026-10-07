//! Wallet passes through the API (DESIGN.md §11): who may have one, Apple's
//! pass web service from a device's side, the Google save link, updates
//! reaching devices through the worker, and revocation when an account is
//! deleted. With throwaway credentials made by the test run
//! (`src/wallet/testkit.rs`); nothing reaches Apple or Google: the senders
//! here record what they were given, and stand in for Google's objects.

mod common;
#[path = "../src/wallet/testkit.rs"]
mod testkit;

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{App, Deal, User};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;
use yuppers_backend::domain::Rules;
use yuppers_backend::wallet::apple::push::{PassPush, PushFuture, PushOutcome};
use yuppers_backend::wallet::config::WalletConfig;
use yuppers_backend::wallet::delivery::{UpdateRules, WalletDelivery, deliver_due};
use yuppers_backend::wallet::google::objects::{
    CreateFuture, PatchFuture, PatchOutcome, WalletObjects,
};
use yuppers_backend::wallet::{Wallet, store};

const DB: &str = "yuppers_test_wallet";
const SERVICE: &str = "/v1/wallet/apple";

/// A wallet with the platforms whose settings `keep` names (all, without).
fn wallet(keep: Option<&[&str]>) -> Arc<Wallet> {
    let settings: HashMap<&str, String> = detailed(testkit::settings())
        .into_iter()
        .filter(|(name, _)| {
            keep.is_none_or(|keep| {
                name.starts_with("WALLET_") || keep.iter().any(|k| name.starts_with(k))
            })
        })
        .collect();
    let config = WalletConfig::from_lookup(&|name| settings.get(name).cloned()).unwrap();
    Arc::new(Wallet::new(&config, "https://app.test").unwrap())
}

/// Both platforms, with `recorder` standing in for Google's objects.
fn wallet_using(recorder: &Arc<Recorder>) -> Arc<Wallet> {
    let config = WalletConfig::from_lookup(&|name| settings_table().get(name).cloned()).unwrap();
    Arc::new(
        Wallet::new(&config, "https://app.test")
            .unwrap()
            .with_google_objects(recorder.clone()),
    )
}

fn settings_table() -> HashMap<&'static str, String> {
    detailed(testkit::settings()).into_iter().collect()
}

/// The detailed status line, which most of these tests read: what presses
/// is what changes a face. The default, neutral, has a test of its own
/// (`by_default_a_face_says_in_force_whatever_presses`).
fn detailed(mut settings: Vec<(&'static str, String)>) -> Vec<(&'static str, String)> {
    settings.push(("WALLET_STATUS_ON_FACE", "detailed".to_owned()));
    settings
}

async fn app() -> App {
    app_using(&Arc::new(Recorder::default())).await
}

async fn app_using(recorder: &Arc<Recorder>) -> App {
    App::start_with_wallet(DB, wallet_using(recorder)).await
}

async fn wallet_post(app: &App, user: &User, exchange: &str, platform: &str) -> common::Reply {
    app.call(
        Some(user),
        Method::POST,
        &format!("/v1/exchanges/{exchange}/wallet/{platform}"),
        None,
        &[],
    )
    .await
}

/// The files of a `.pkpass`.
fn unzip(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).unwrap();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            (file.name().to_owned(), contents)
        })
        .collect()
}

fn pass_json(reply: &common::Reply) -> Value {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.headers["content-type"],
        "application/vnd.apple.pkpass"
    );
    // It carries an authentication token: nothing on the way keeps a copy.
    assert_eq!(reply.headers["cache-control"], "no-store");
    serde_json::from_slice(&unzip(&reply.bytes)["pass.json"]).unwrap()
}

/// What the senders were asked to do, standing in for APNs and for
/// Google's objects.
#[derive(Default)]
struct Recorder {
    pushes: Mutex<Vec<String>>,
    /// Updates to objects Google has.
    patches: Mutex<Vec<(String, Value)>>,
    /// The objects Google has, by ID, as last written.
    objects: Mutex<HashMap<String, Value>>,
    /// Classes created.
    classes: Mutex<Vec<String>>,
}

impl PassPush for Recorder {
    fn push<'a>(&'a self, push_token: &'a str) -> PushFuture<'a> {
        Box::pin(async move {
            self.pushes.lock().unwrap().push(push_token.to_owned());
            Ok(PushOutcome::Sent)
        })
    }
}

impl WalletObjects for Recorder {
    fn patch<'a>(&'a self, object_id: &'a str, object: &'a Value) -> PatchFuture<'a> {
        Box::pin(async move {
            let mut objects = self.objects.lock().unwrap();
            let Some(held) = objects.get_mut(object_id) else {
                // As Google answers for an object it does not have: 404.
                return Ok(PatchOutcome::NotSaved);
            };
            *held = object.clone();
            self.patches
                .lock()
                .unwrap()
                .push((object_id.to_owned(), object.clone()));
            Ok(PatchOutcome::Updated)
        })
    }

    fn upsert<'a>(&'a self, class: &'a Value, object: &'a Value) -> CreateFuture<'a> {
        Box::pin(async move {
            self.classes
                .lock()
                .unwrap()
                .push(class["id"].as_str().unwrap().to_owned());
            self.objects
                .lock()
                .unwrap()
                .insert(object["id"].as_str().unwrap().to_owned(), object.clone());
            Ok(())
        })
    }
}

/// Held for the whole of each test that runs the worker. The worker takes
/// every pass that is due in the database, which the tests share; one test's
/// run would otherwise send another test's updates to its own recorder.
static WORKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs the worker's Wallet pass once, with `recorder` as both platforms.
async fn run_worker(app: &App, recorder: &Arc<Recorder>) {
    // The lease of an earlier run in this test may still hold a pass; the
    // run looks from far enough ahead that it has run out.
    let later = OffsetDateTime::now_utc() + time::Duration::minutes(5);
    run_worker_at(app, recorder, later).await;
}

/// The same, as if it were `at`.
async fn run_worker_at(app: &App, recorder: &Arc<Recorder>, at: OffsetDateTime) {
    let delivery = WalletDelivery {
        wallet: wallet_using(recorder),
        apple: Some(recorder.clone()),
        google: Some(recorder.clone()),
        rules: UpdateRules::default(),
    };
    deliver_due(&app.db, &Rules::default(), &delivery, at)
        .await
        .unwrap();
}

async fn pass_status(app: &App, serial: &str) -> (String, bool) {
    sqlx::query_as(
        "SELECT update_status, voided_at IS NOT NULL FROM wallet_pass WHERE external_id = $1",
    )
    .bind(serial)
    .fetch_one(&app.owner)
    .await
    .unwrap()
}

/// A request from a device to the pass web service.
async fn device(
    app: &App,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    headers: &[(&'static str, &str)],
) -> common::Reply {
    let authorization = token.map(|token| format!("ApplePass {token}"));
    let mut all: Vec<(&'static str, &str)> = headers.to_vec();
    if let Some(authorization) = &authorization {
        all.push(("authorization", authorization));
    }
    app.call(None, method, &format!("{SERVICE}{path}"), body, &all)
        .await
}

async fn platforms(app: &App) -> Value {
    let reply = app
        .call(None, Method::GET, "/v1/meta", None, &[])
        .await
        .ok();
    reply["wallet_platforms"].clone()
}

#[tokio::test]
async fn meta_names_only_the_platforms_that_are_configured() {
    let off = App::start(DB).await;
    assert_eq!(platforms(&off).await, json!([]));
    let google = App::start_with_wallet(DB, wallet(Some(&["GOOGLE_"]))).await;
    assert_eq!(platforms(&google).await, json!(["GOOGLE"]));
    let apple = App::start_with_wallet(DB, wallet(Some(&["APPLE_"]))).await;
    assert_eq!(platforms(&apple).await, json!(["APPLE"]));
    assert_eq!(platforms(&app().await).await, json!(["APPLE", "GOOGLE"]));
}

#[tokio::test]
async fn a_platform_that_is_not_configured_says_so() {
    let off = App::start(DB).await;
    let deal = off.active().await;
    for platform in ["apple", "google", "apple/link"] {
        wallet_post(&off, &deal.ben, &deal.exchange, platform)
            .await
            .refused(StatusCode::NOT_FOUND, "WALLET_UNAVAILABLE");
    }
    let apple_only = App::start_with_wallet(DB, wallet(Some(&["APPLE_"]))).await;
    wallet_post(&apple_only, &deal.ben, &deal.exchange, "google")
        .await
        .refused(StatusCode::NOT_FOUND, "WALLET_UNAVAILABLE");
    assert_eq!(
        wallet_post(&apple_only, &deal.ben, &deal.exchange, "apple")
            .await
            .status,
        StatusCode::OK
    );
    // Nor does the pass web service answer.
    let reply = device(
        &off,
        Method::GET,
        &format!("/v1/passes/{}/x", testkit::PASS_TYPE_ID),
        Some("x"),
        None,
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_a_party_gets_a_pass_and_only_once_something_is_agreed() {
    let app = app().await;
    let deal = app.active().await;
    let stranger = app.user("Sam").await;
    for platform in ["apple", "google", "apple/link"] {
        wallet_post(&app, &stranger, &deal.exchange, platform)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
        let reply = app
            .call(
                None,
                Method::POST,
                &format!("/v1/exchanges/{}/wallet/{platform}", deal.exchange),
                None,
                &[],
            )
            .await;
        reply.refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
        wallet_post(&app, &deal.ben, "not-an-id", platform)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    }
    // Nobody tried has a pass row: refusing comes before issuing.
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_pass WHERE account_id = $1")
        .bind(stranger.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    let negotiating = app.negotiating().await;
    wallet_post(&app, &negotiating.ana, &negotiating.exchange, "apple")
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    for user in [&deal.ana, &deal.ben] {
        let json = pass_json(&wallet_post(&app, user, &deal.exchange, "apple").await);
        assert_eq!(json["passTypeIdentifier"], testkit::PASS_TYPE_ID);
        assert_eq!(json["webServiceURL"], "https://app.test/v1/wallet/apple");
        assert_eq!(
            json["generic"]["backFields"][0]["value"],
            format!("https://app.test/exchanges/{}", deal.exchange)
        );
        // Nothing from the agreement is on it (`fence_job` in common).
        let text = json.to_string();
        for secret in ["Ana", "Ben", "Ruiz", "Ortiz", "fence", "40000", "400.00"] {
            assert!(!text.contains(secret), "{secret} is on the pass: {text}");
        }
    }
}

#[tokio::test]
async fn handing_out_a_pass_is_limited_per_hour() {
    let app = app().await;
    let deal = app.active().await;
    for _ in 0..10 {
        assert_eq!(
            wallet_post(&app, &deal.ben, &deal.exchange, "google")
                .await
                .status,
            StatusCode::OK
        );
    }
    wallet_post(&app, &deal.ben, &deal.exchange, "google")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    // Another platform, and another person, count apart.
    assert_eq!(
        wallet_post(&app, &deal.ben, &deal.exchange, "apple")
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        wallet_post(&app, &deal.ana, &deal.exchange, "google")
            .await
            .status,
        StatusCode::OK
    );
}

/// Ben's Apple pass for the deal: its serial number and token.
async fn ben_apple_pass(app: &App, deal: &Deal) -> (String, String) {
    let json = pass_json(&wallet_post(app, &deal.ben, &deal.exchange, "apple").await);
    (
        json["serialNumber"].as_str().unwrap().to_owned(),
        json["authenticationToken"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn the_pass_web_service_from_a_device() {
    let _worker = WORKER.lock().await;
    let app = app().await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    let pass_type = testkit::PASS_TYPE_ID;
    let device_id = "a1b2c3d4e5f6";
    let registration = format!("/v1/devices/{device_id}/registrations/{pass_type}/{serial}");
    let push = json!({ "pushToken": "00ff00ff" });

    // Registering: the pass's token, and only it.
    for wrong in [None, Some("not-the-token"), Some("")] {
        let reply = device(
            &app,
            Method::POST,
            &registration,
            wrong,
            Some(push.clone()),
            &[],
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{wrong:?}");
    }
    let other_type = format!("/v1/devices/{device_id}/registrations/pass.other.type/{serial}");
    let reply = device(
        &app,
        Method::POST,
        &other_type,
        Some(&token),
        Some(push.clone()),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let reply = device(
        &app,
        Method::POST,
        &registration,
        Some(&token),
        Some(push.clone()),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let reply = device(
        &app,
        Method::POST,
        &registration,
        Some(&token),
        Some(push.clone()),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "registered already");
    let reply = device(
        &app,
        Method::POST,
        &registration,
        Some(&token),
        Some(json!({ "pushToken": "not hex" })),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    // Which passes changed: all of them the first time.
    let list = format!("/v1/devices/{device_id}/registrations/{pass_type}");
    let reply = device(&app, Method::GET, &list, None, None, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["serialNumbers"], json!([serial]));
    let tag = reply.body["lastUpdated"].as_str().unwrap().to_owned();
    let reply = device(
        &app,
        Method::GET,
        &format!("/v1/devices/unknown/registrations/{pass_type}"),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    // Another pass type is never ours.
    let reply = device(
        &app,
        Method::GET,
        &format!("/v1/devices/{device_id}/registrations/pass.other.type"),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    // The latest pass, and 304 while it has not changed.
    let latest = format!("/v1/passes/{pass_type}/{serial}");
    let reply = device(&app, Method::GET, &latest, Some(&token), None, &[]).await;
    let first = pass_json(&reply);
    assert_eq!(first["generic"]["primaryFields"][0]["value"], "In force");
    let modified = reply.headers["last-modified"].to_str().unwrap().to_owned();
    let reply = device(
        &app,
        Method::GET,
        &latest,
        Some(&token),
        None,
        &[("if-modified-since", &modified)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_MODIFIED);
    let reply = device(&app, Method::GET, &latest, Some("wrong"), None, &[]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    // Ana marks the repair she owes Ben as delivered. The pass is marked in
    // the same transaction, and the worker pushes to Ben's device.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    assert_eq!(
        pass_status(&app, &serial).await,
        ("PENDING".to_owned(), false)
    );
    let recorder = Arc::new(Recorder::default());
    run_worker(&app, &recorder).await;
    assert_eq!(*recorder.pushes.lock().unwrap(), ["00ff00ff"]);
    assert_eq!(
        pass_status(&app, &serial).await,
        ("CURRENT".to_owned(), false)
    );

    // The device asks what changed since its tag, and fetches it.
    let reply = device(
        &app,
        Method::GET,
        &format!("{list}?passesUpdatedSince={tag}"),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(reply.body["serialNumbers"], json!([serial]));
    let reply = device(
        &app,
        Method::GET,
        &latest,
        Some(&token),
        None,
        &[("if-modified-since", &modified)],
    )
    .await;
    let updated = pass_json(&reply);
    assert_eq!(
        updated["generic"]["primaryFields"][0]["value"],
        "Waiting for you"
    );
    assert_ne!(reply.headers["last-modified"].to_str().unwrap(), modified);

    // A change that leaves the face as it was pushes nothing.
    app.command(
        &deal.ben,
        &deal.exchange,
        json!({ "type": "REQUEST_CLOSE", "note": null }),
    )
    .await
    .ok();
    app.command(
        &deal.ben,
        &deal.exchange,
        json!({ "type": "RETRACT_CLOSE" }),
    )
    .await
    .ok();
    run_worker(&app, &recorder).await;
    assert_eq!(recorder.pushes.lock().unwrap().len(), 1);

    // Unregistering: the device stops hearing about it.
    let reply = device(&app, Method::DELETE, &registration, Some(&token), None, &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let reply = device(&app, Method::GET, &list, None, None, &[]).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    // The device's log is taken and answered.
    let reply = device(
        &app,
        Method::POST,
        "/v1/log",
        None,
        Some(json!({ "logs": ["a line"] })),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn a_download_link_gives_the_pass_without_a_session_once_for_a_while() {
    let app = app().await;
    let deal = app.active().await;
    let link = wallet_post(&app, &deal.ben, &deal.exchange, "apple/link")
        .await
        .ok();
    let url = link["url"].as_str().unwrap();
    assert!(link["expires_at"].is_string());
    let path = url
        .strip_prefix("https://app.test")
        .expect("on the web origin");
    assert!(path.starts_with("/v1/wallet/apple/pass?token="));

    // One character of the token changed.
    let at = path.find("token=").unwrap() + 10;
    let changed = if &path[at..=at] == "A" { "B" } else { "A" };
    let forged = format!("{}{changed}{}", &path[..at], &path[at + 1..]);
    let refused = app.call(None, Method::GET, &forged, None, &[]).await;
    refused.refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    let json = pass_json(&app.call(None, Method::GET, path, None, &[]).await);
    assert_eq!(json["passTypeIdentifier"], testkit::PASS_TYPE_ID);
    // Used once, it is used up, and told so as an unknown link is.
    let again = app.call(None, Method::GET, path, None, &[]).await;
    assert_eq!((again.status, &again.body), (refused.status, &refused.body));

    // An expired link, the same.
    let link = wallet_post(&app, &deal.ben, &deal.exchange, "apple/link")
        .await
        .ok();
    let path = link["url"].as_str().unwrap()["https://app.test".len()..].to_owned();
    // Only this exchange's links: tests running alongside have their own.
    let exchange: Uuid = deal.exchange.parse().unwrap();
    sqlx::query(
        "UPDATE wallet_download_link SET expires_at = now()
         WHERE used_at IS NULL
           AND wallet_pass_id IN (SELECT id FROM wallet_pass WHERE exchange_id = $1)",
    )
    .bind(exchange)
    .execute(&app.owner)
    .await
    .unwrap();
    let expired = app.call(None, Method::GET, &path, None, &[]).await;
    assert_eq!(
        (expired.status, &expired.body),
        (refused.status, &refused.body)
    );
}

#[tokio::test]
async fn handing_out_a_pass_again_rotates_its_token_and_the_latest_three_work() {
    let app = app().await;
    let deal = app.active().await;
    let mut tokens = Vec::new();
    let mut serial = String::new();
    for _ in 0..4 {
        let (found, token) = ben_apple_pass(&app, &deal).await;
        serial = found;
        tokens.push(token);
    }
    let distinct: std::collections::HashSet<&String> = tokens.iter().collect();
    assert_eq!(distinct.len(), 4, "a new token each time");
    let latest = format!("/v1/passes/{}/{serial}", testkit::PASS_TYPE_ID);
    let status = |token: String| {
        let (app, latest) = (&app, &latest);
        async move {
            device(app, Method::GET, latest, Some(&token), None, &[])
                .await
                .status
        }
    };
    assert_eq!(status(tokens[0].clone()).await, StatusCode::UNAUTHORIZED);
    for token in &tokens[1..] {
        assert_eq!(status(token.clone()).await, StatusCode::OK);
    }
    // What a device fetches carries the token it asked with.
    let reply = device(&app, Method::GET, &latest, Some(&tokens[3]), None, &[]).await;
    assert_eq!(pass_json(&reply)["authenticationToken"], tokens[3]);
    // Only hashes are stored.
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wallet_auth_token t JOIN wallet_pass p ON p.id = t.wallet_pass_id
         WHERE p.external_id = $1",
    )
    .bind(&serial)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(stored, 3);
}

async fn register_device(app: &App, serial: &str, token: &str, device_id: &str) -> StatusCode {
    let registration = format!(
        "/v1/devices/{device_id}/registrations/{}/{serial}",
        testkit::PASS_TYPE_ID
    );
    device(
        app,
        Method::POST,
        &registration,
        Some(token),
        Some(json!({ "pushToken": "beef" })),
        &[],
    )
    .await
    .status
}

async fn devices_of(app: &App, serial: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT device_library_id FROM wallet_device_registration r
         JOIN wallet_pass p ON p.id = r.wallet_pass_id
         WHERE p.external_id = $1 ORDER BY device_library_id",
    )
    .bind(serial)
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_pass_on_as_many_devices_as_it_may_be_takes_a_new_one_in_place_of_the_oldest() {
    let app = app().await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    // Twenty devices already, heard from a minute apart; dev-00 longest ago.
    sqlx::query(
        "INSERT INTO wallet_device_registration
             (wallet_pass_id, device_library_id, push_token, updated_at)
         SELECT p.id, 'dev-' || lpad(n::text, 2, '0'), 'beef', now() - (20 - n) * interval '1 minute'
         FROM wallet_pass p, generate_series(0, 19) n WHERE p.external_id = $1",
    )
    .bind(&serial)
    .execute(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        register_device(&app, &serial, &token, "dev-new").await,
        StatusCode::CREATED
    );
    let devices = devices_of(&app, &serial).await;
    assert_eq!(devices.len(), 20);
    assert!(devices.contains(&"dev-new".to_owned()));
    assert!(!devices.contains(&"dev-00".to_owned()), "{devices:?}");
}

#[tokio::test]
async fn new_devices_are_limited_per_pass_per_hour() {
    let app = app().await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    for n in 0..10 {
        assert_eq!(
            register_device(&app, &serial, &token, &format!("dev-{n}")).await,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        register_device(&app, &serial, &token, "dev-10").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // A device registered already is not a new one.
    assert_eq!(
        register_device(&app, &serial, &token, "dev-3").await,
        StatusCode::OK
    );
    // Another pass counts apart.
    let (ana_serial, ana_token) = {
        let json = pass_json(&wallet_post(&app, &deal.ana, &deal.exchange, "apple").await);
        (
            json["serialNumber"].as_str().unwrap().to_owned(),
            json["authenticationToken"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(
        register_device(&app, &ana_serial, &ana_token, "dev-10").await,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn the_device_log_is_small_and_limited_per_address() {
    let app = app().await;
    let log = |body: Value| {
        let app = &app;
        async move {
            device(app, Method::POST, "/v1/log", None, Some(body), &[])
                .await
                .status
        }
    };
    let heavy = json!({ "logs": ["x".repeat(9 * 1024)] });
    assert_eq!(log(heavy).await, StatusCode::PAYLOAD_TOO_LARGE);
    for _ in 0..10 {
        assert_eq!(
            log(json!({ "logs": vec!["a line"; 50] })).await,
            StatusCode::OK
        );
    }
    assert_eq!(
        log(json!({ "logs": ["a line"] })).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// The claims of a JWT whose signature holds under the test service
/// account's key.
fn verified_claims(jwt: &str) -> Value {
    let (signed, signature) = jwt.rsplit_once('.').unwrap();
    UnparsedPublicKey::new(
        &RSA_PKCS1_2048_8192_SHA256,
        &testkit::CREDENTIALS.google_public_der,
    )
    .verify(
        signed.as_bytes(),
        &URL_SAFE_NO_PAD.decode(signature).unwrap(),
    )
    .expect("signed by the service account");
    let claims = signed.split_once('.').unwrap().1;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap()
}

/// The claims of `user`'s save link for the deal.
async fn google_claims(app: &App, user: &User, exchange: &str) -> Value {
    let reply = wallet_post(app, user, exchange, "google").await.ok();
    let url = reply["url"].as_str().unwrap().to_owned();
    let jwt = url
        .strip_prefix("https://pay.google.com/gp/v/save/")
        .expect("a save link");
    verified_claims(jwt)
}

async fn delivered_hash(app: &App, serial: &str) -> Option<Vec<u8>> {
    sqlx::query_scalar("SELECT delivered_hash FROM wallet_pass WHERE external_id = $1")
        .bind(serial)
        .fetch_one(&app.owner)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_google_link_names_only_this_partys_object_made_at_google_first() {
    let _worker = WORKER.lock().await;
    let google = Arc::new(Recorder::default());
    let app = app_using(&google).await;
    let deal = app.active().await;
    let ben = google_claims(&app, &deal.ben, &deal.exchange).await;
    assert_eq!(ben["iss"], testkit::CLIENT_EMAIL);
    assert_eq!(
        (ben["aud"].as_str(), ben["typ"].as_str()),
        (Some("google"), Some("savetowallet"))
    );
    assert_eq!(ben["origins"], json!(["https://app.test"]));
    // The link carries the object's ID and class, and no face.
    let named = &ben["payload"]["genericObjects"][0];
    let id = named["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with(&format!("{}.", testkit::ISSUER_ID)));
    assert_eq!(
        ben["payload"],
        json!({ "genericObjects": [{ "id": id, "classId": named["classId"] }] })
    );
    // The object was made at Google before the link was handed out.
    let object = google.objects.lock().unwrap()[&id].clone();
    assert_eq!(object["classId"], named["classId"]);
    assert_eq!(
        google.classes.lock().unwrap().first(),
        named["classId"].as_str().map(str::to_owned).as_ref()
    );
    assert_eq!(object["state"], "ACTIVE");
    assert_eq!(object["header"]["defaultValue"]["value"], "In force");
    assert_eq!(
        object["linksModuleData"]["uris"][0]["uri"],
        format!("https://app.test/exchanges/{}", deal.exchange)
    );
    let text = object.to_string();
    for secret in ["Ana", "Ben", "Ruiz", "fence", "40000"] {
        assert!(!text.contains(secret), "{secret} is in the object: {text}");
    }

    // The same object every time for Ben; another one for Ana.
    let again = google_claims(&app, &deal.ben, &deal.exchange).await;
    assert_eq!(again["payload"]["genericObjects"][0]["id"], id.as_str());
    let ana = google_claims(&app, &deal.ana, &deal.exchange).await;
    assert_ne!(ana["payload"]["genericObjects"][0]["id"], id.as_str());

    // A change reaches the object through the worker.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    run_worker(&app, &google).await;
    assert_eq!(
        google.objects.lock().unwrap()[&id]["header"]["defaultValue"]["value"],
        "Waiting for you"
    );

    // Google no longer having the object (404): the face is not counted as
    // delivered, so the next update or link carries it.
    let serial = id.rsplit('.').next().unwrap().to_owned();
    let before = delivered_hash(&app, &serial).await;
    google.objects.lock().unwrap().remove(&id);
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    run_worker(&app, &google).await;
    assert_eq!(delivered_hash(&app, &serial).await, before);
    assert_eq!(pass_status(&app, &serial).await.0, "CURRENT");
    // The next link makes it again, with the face as it is now.
    google_claims(&app, &deal.ben, &deal.exchange).await;
    assert_eq!(
        google.objects.lock().unwrap()[&id]["header"]["defaultValue"]["value"],
        "In force"
    );
    assert_ne!(delivered_hash(&app, &serial).await, before);
}

#[tokio::test]
async fn deleting_an_account_revokes_its_passes() {
    let _worker = WORKER.lock().await;
    let google = Arc::new(Recorder::default());
    let app = app_using(&google).await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    let object_id =
        google_claims(&app, &deal.ben, &deal.exchange).await["payload"]["genericObjects"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
    let pass_type = testkit::PASS_TYPE_ID;
    let (ana_serial, _) = {
        let json = pass_json(&wallet_post(&app, &deal.ana, &deal.exchange, "apple").await);
        (json["serialNumber"].as_str().unwrap().to_owned(), ())
    };

    yuppers_backend::deletion::delete_account(&app.db, &app.rules, deal.ben.id)
        .await
        .unwrap();
    assert_eq!(
        pass_status(&app, &serial).await,
        ("PENDING".to_owned(), true)
    );
    // Ana's pass is marked too, by the request to close made in Ben's name;
    // it is hers and stays live.
    assert_eq!(
        pass_status(&app, &ana_serial).await,
        ("PENDING".to_owned(), false)
    );

    run_worker(&app, &google).await;
    // Google's object is made inactive and stripped. It exists at Google,
    // made when the link was, and the link names only it: using the link
    // now saves this inactive object, never a live one.
    let object = google.objects.lock().unwrap()[&object_id].clone();
    assert_eq!(object["state"], "INACTIVE");
    assert_eq!(object["linksModuleData"]["uris"], json!([]));
    assert!(!object.to_string().contains(&deal.exchange));

    // What the web service now gives for the pass is the void face: no
    // reference, no link, nothing of the exchange.
    let reply = device(
        &app,
        Method::GET,
        &format!("/v1/passes/{pass_type}/{serial}"),
        Some(&token),
        None,
        &[],
    )
    .await;
    let json = pass_json(&reply);
    assert_eq!(json["voided"], true);
    let text = json.to_string();
    assert!(
        !text.contains(&deal.exchange) && !text.contains("exchanges/"),
        "{text}"
    );
    assert_eq!(json["generic"]["auxiliaryFields"], json!([]));
    // A void pass takes no device, and is never updated again.
    let registration = format!("/v1/devices/dev-late/registrations/{pass_type}/{serial}");
    let reply = device(
        &app,
        Method::POST,
        &registration,
        Some(&token),
        Some(json!({ "pushToken": "beef" })),
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "ADD_STATEMENT", "note": "Noted." }),
    )
    .await
    .ok();
    assert_eq!(
        pass_status(&app, &serial).await,
        ("CURRENT".to_owned(), true)
    );
}

#[tokio::test]
async fn a_voided_pass_reaches_the_device_by_apples_two_steps_then_its_registration_goes() {
    let _worker = WORKER.lock().await;
    let recorder = Arc::new(Recorder::default());
    let app = app_using(&recorder).await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    let pass_type = testkit::PASS_TYPE_ID;
    let device_id = "dev-two-steps";
    assert_eq!(
        register_device(&app, &serial, &token, device_id).await,
        StatusCode::CREATED
    );

    // The device's first sync: the list, then the pass.
    let list = format!("/v1/devices/{device_id}/registrations/{pass_type}");
    let reply = device(&app, Method::GET, &list, None, None, &[]).await;
    assert_eq!(reply.body["serialNumbers"], json!([serial]));
    let tag = reply.body["lastUpdated"].as_str().unwrap().to_owned();
    let latest = format!("/v1/passes/{pass_type}/{serial}");
    let reply = device(&app, Method::GET, &latest, Some(&token), None, &[]).await;
    assert_eq!(pass_json(&reply)["voided"], Value::Null);
    let modified = reply.headers["last-modified"].to_str().unwrap().to_owned();

    yuppers_backend::deletion::delete_account(&app.db, &app.rules, deal.ben.id)
        .await
        .unwrap();
    run_worker(&app, &recorder).await;
    // The push wakes the device...
    assert!(recorder.pushes.lock().unwrap().contains(&"beef".to_owned()));
    assert_eq!(devices_of(&app, &serial).await, [device_id]);
    // ...which asks what changed since its tag, and is told this pass...
    let reply = device(
        &app,
        Method::GET,
        &format!("{list}?passesUpdatedSince={tag}"),
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["serialNumbers"], json!([serial]));
    // ...and fetches it: the void face.
    let reply = device(
        &app,
        Method::GET,
        &latest,
        Some(&token),
        None,
        &[("if-modified-since", &modified)],
    )
    .await;
    let json = pass_json(&reply);
    assert_eq!(json["voided"], true);
    assert_eq!(
        json["generic"]["primaryFields"][0]["value"],
        "No longer in use"
    );

    // Kept a while for the fetch, then forgotten: nothing more is ever sent.
    run_worker(&app, &recorder).await;
    assert_eq!(devices_of(&app, &serial).await, [device_id]);
    let later = OffsetDateTime::now_utc() + time::Duration::days(2);
    run_worker_at(&app, &recorder, later).await;
    assert!(devices_of(&app, &serial).await.is_empty());
    let reply = device(&app, Method::GET, &list, None, None, &[]).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_device_never_told_of_the_void_is_forgotten_after_a_month() {
    let _worker = WORKER.lock().await;
    let recorder = Arc::new(Recorder::default());
    let app = app_using(&recorder).await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    register_device(&app, &serial, &token, "dev-away").await;
    yuppers_backend::deletion::delete_account(&app.db, &app.rules, deal.ben.id)
        .await
        .unwrap();
    run_worker(&app, &recorder).await;
    let in_a_week = OffsetDateTime::now_utc() + time::Duration::days(7);
    run_worker_at(&app, &recorder, in_a_week).await;
    assert_eq!(devices_of(&app, &serial).await, ["dev-away"]);
    let in_a_month = OffsetDateTime::now_utc() + time::Duration::days(31);
    run_worker_at(&app, &recorder, in_a_month).await;
    assert!(devices_of(&app, &serial).await.is_empty());
}

#[tokio::test]
async fn a_suspended_accounts_passes_are_not_updated() {
    let _worker = WORKER.lock().await;
    let recorder = Arc::new(Recorder::default());
    let app = app_using(&recorder).await;
    let deal = app.active().await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    register_device(&app, &serial, &token, "dev-suspended").await;
    let latest = format!("/v1/passes/{}/{serial}", testkit::PASS_TYPE_ID);
    let reply = device(&app, Method::GET, &latest, Some(&token), None, &[]).await;
    let modified = reply.headers["last-modified"].to_str().unwrap().to_owned();

    // Marked, then suspended before the worker gets to it.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1")
        .bind(deal.ben.id)
        .execute(&app.owner)
        .await
        .unwrap();
    run_worker(&app, &recorder).await;
    assert!(recorder.pushes.lock().unwrap().is_empty());
    assert_eq!(
        pass_status(&app, &serial).await,
        ("CURRENT".to_owned(), false)
    );
    // Later changes do not mark it, and the device is told nothing new.
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "PROPOSE_END", "note": null }),
    )
    .await
    .ok();
    assert_eq!(pass_status(&app, &serial).await.0, "CURRENT");
    let list = format!(
        "/v1/devices/dev-suspended/registrations/{}",
        testkit::PASS_TYPE_ID
    );
    let reply = device(&app, Method::GET, &list, None, None, &[]).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let reply = device(
        &app,
        Method::GET,
        &latest,
        Some(&token),
        None,
        &[("if-modified-since", &modified)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_MODIFIED);
    let reply = device(&app, Method::GET, &latest, Some(&token), None, &[]).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        register_device(&app, &serial, &token, "dev-new").await,
        StatusCode::UNAUTHORIZED
    );
    // It is not voided: that is for deletion.
    assert!(!pass_status(&app, &serial).await.1);
}

/// An agreement in force in `timezone` whose repair, owed by Ana, is due on
/// `due`, with the payment after it.
async fn due_on(app: &App, timezone: &str, due: time::Date) -> Deal {
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let view = app
        .post(&ana, "/v1/exchanges", json!({ "timezone": timezone }))
        .await
        .ok();
    let exchange = view["id"].as_str().unwrap().to_owned();
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());
    let mut terms = common::fence_job(repair, payment);
    terms["contributions"][0]["due"] = json!({ "kind": "DATE", "date": due.to_string() });
    let sent = app.send(&ana, &exchange, terms).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let invitation = sent["invitation_token"].as_str().unwrap().to_owned();
    app.post(
        &ben,
        "/v1/invitations/claim",
        json!({ "token": invitation }),
    )
    .await
    .ok();
    app.command(&ana, &exchange, json!({ "type": "CONFIRM_COUNTERPARTY" }))
        .await
        .ok();
    let view = app
        .command(&ben, &exchange, common::accept(&revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    Deal {
        ana,
        ben,
        exchange,
        repair,
        payment,
        revision,
        invitation,
    }
}

#[tokio::test]
async fn faces_that_change_with_the_date_are_sent_once_a_day_in_the_exchanges_timezone() {
    let _worker = WORKER.lock().await;
    let recorder = Arc::new(Recorder::default());
    let app = app_using(&recorder).await;
    // Kiritimati is fourteen hours ahead of UTC: its day starts at 10:00 UTC
    // the day before.
    let due = OffsetDateTime::now_utc().date() + time::Duration::days(10);
    let deal = due_on(&app, "Pacific/Kiritimati", due).await;
    let (serial, token) = ben_apple_pass(&app, &deal).await;
    register_device(&app, &serial, &token, "dev-dated").await;
    let object_id =
        google_claims(&app, &deal.ben, &deal.exchange).await["payload"]["genericObjects"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
    let status_at_google = || {
        recorder.objects.lock().unwrap()[&object_id]["header"]["defaultValue"]["value"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let at = |date: time::Date, hour: u8| date.with_hms(hour, 0, 0).unwrap().assume_utc();
    let days = time::Duration::days;

    // Four days before, in Kiritimati: in force, as at issue.
    run_worker_at(&app, &recorder, at(due - days(4), 0)).await;
    assert_eq!(status_at_google(), "In force");
    let pushes = recorder.pushes.lock().unwrap().len();
    // The same day again: nothing is even looked at.
    run_worker_at(&app, &recorder, at(due - days(4), 1)).await;
    assert_eq!(pass_status(&app, &serial).await.0, "CURRENT");
    assert_eq!(recorder.pushes.lock().unwrap().len(), pushes);

    // 11:00 UTC three days before is already two days before in Kiritimati:
    // due soon there, though not yet in UTC.
    run_worker_at(&app, &recorder, at(due - days(3), 11)).await;
    assert_eq!(status_at_google(), "Due soon");
    assert_eq!(recorder.pushes.lock().unwrap().len(), pushes + 1);
    let patches = recorder.patches.lock().unwrap().len();
    run_worker_at(&app, &recorder, at(due - days(3), 12)).await;
    assert_eq!(recorder.patches.lock().unwrap().len(), patches);

    // The next day there: still due soon, so nothing is sent.
    run_worker_at(&app, &recorder, at(due - days(2), 11)).await;
    assert_eq!(recorder.patches.lock().unwrap().len(), patches);
    assert_eq!(recorder.pushes.lock().unwrap().len(), pushes + 1);

    // The day after the due date there: overdue.
    run_worker_at(&app, &recorder, at(due, 11)).await;
    assert_eq!(status_at_google(), "Overdue");
    assert_eq!(recorder.pushes.lock().unwrap().len(), pushes + 2);

    // Once delivered, nothing on a date is pending, and no day marks it.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    run_worker_at(&app, &recorder, at(due, 12)).await;
    let pushes = recorder.pushes.lock().unwrap().len();
    run_worker_at(&app, &recorder, at(due + days(1), 12)).await;
    assert_eq!(recorder.pushes.lock().unwrap().len(), pushes);
}

#[tokio::test]
async fn the_store_marks_and_revokes_passes_in_the_callers_transaction() {
    let app = app().await;
    let deal = app.active().await;
    let (serial, _) = ben_apple_pass(&app, &deal).await;
    let exchange: Uuid = deal.exchange.parse().unwrap();
    let mut tx = app.db.begin().await.unwrap();
    store::mark_exchange_changed(&mut tx, exchange)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(pass_status(&app, &serial).await.0, "CURRENT");
    let mut tx = app.db.begin().await.unwrap();
    assert_eq!(
        store::revoke_for_account(&mut tx, deal.ben.id)
            .await
            .unwrap(),
        1
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        pass_status(&app, &serial).await,
        ("CURRENT".to_owned(), false)
    );
}

/// With `WALLET_STATUS_ON_FACE` unset, the face says how the agreement stands
/// and nothing more: no "Due soon", no "Waiting for you", no next due date
/// (DESIGN.md §11, the owner's decision of 3 October 2026).
#[tokio::test]
async fn by_default_a_face_says_in_force_whatever_presses() {
    let _worker = WORKER.lock().await;
    let settings = settings_table_default();
    let config = WalletConfig::from_lookup(&|name| settings.get(name).cloned()).unwrap();
    let app = App::start_with_wallet(
        DB,
        Arc::new(Wallet::new(&config, "https://app.test").unwrap()),
    )
    .await;
    let due = OffsetDateTime::now_utc().date() + time::Duration::days(1);
    let deal = due_on(&app, "UTC", due).await;
    let face = |json: &Value| {
        (
            json["generic"]["primaryFields"][0]["value"].clone(),
            json["generic"]["secondaryFields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["key"] == "next-due"),
        )
    };

    // Due tomorrow: due soon, on a detailed face.
    let json = pass_json(&wallet_post(&app, &deal.ben, &deal.exchange, "apple").await);
    assert_eq!(face(&json), (json!("In force"), false));

    // Delivered to Ben: waiting for him, on a detailed face.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    let json = pass_json(&wallet_post(&app, &deal.ben, &deal.exchange, "apple").await);
    assert_eq!(face(&json), (json!("In force"), false));
}

fn settings_table_default() -> HashMap<&'static str, String> {
    testkit::settings().into_iter().collect()
}
