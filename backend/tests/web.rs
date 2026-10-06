//! The service serving the web app from the same origin as the API
//! (DESIGN.md §13.5), against a directory laid out as the web build writes
//! it: the app's `index.html`, one entry page per language at
//! `{language}/i/index.html`, the privacy policy's and the terms' page in
//! each language,
//! hashed files under `assets/`, and the public files beside them. Needs no database: nothing here reaches one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, REFERRER_POLICY,
    STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use yuppers_backend::auth::{AuthRules, LogSender};
use yuppers_backend::build_info::BuildInfo;
use yuppers_backend::db;
use yuppers_backend::http::{self, AppLinks, AppState, Settings, TrustedProxies, WebApp};

const HOME: &str = "<!doctype html><html lang=\"en\"><title>Yuppers</title>home</html>";
const EN: &str = "<!doctype html><html lang=\"en\"><title>Invitation</title>en</html>";
const ES: &str = "<!doctype html><html lang=\"es\"><title>Invitación</title>es</html>";
const PRIVACY: &str = "<!doctype html><html lang=\"en\"><title>Privacy policy</title><h2 id=\"text-messages\">Text messages</h2></html>";
const PRIVACY_ES: &str = "<!doctype html><html lang=\"es\"><title>Política de privacidad</title><h2 id=\"text-messages\">Mensajes</h2></html>";
const TERMS: &str = "<!doctype html><html lang=\"en\"><title>Terms and Conditions</title><h2 id=\"text-messages\">Text messages</h2></html>";
const TERMS_ES: &str = "<!doctype html><html lang=\"es\"><title>Términos y condiciones</title><h2 id=\"text-messages\">Mensajes</h2></html>";
const OPT_IN: &str = "<!doctype html><html lang=\"en\"><title>How people opt in to texts from Yuppers.app</title><img src=\"/sms-opt-in/1-sign-in.webp\"></html>";
const OPT_IN_ES: &str = "<!doctype html><html lang=\"es\"><title>Cómo se suscribe la gente</title><img src=\"/sms-opt-in/es/1-sign-in.webp\"></html>";
const PICTURE: &[u8] = b"RIFF\x10\x00\x00\x00WEBPVP8 ";
const SCRIPT: &str = "console.log('hashed')";
const ICON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\"/>";

/// A build in a directory of its own, removed when dropped.
struct Build(PathBuf);

impl Build {
    fn write() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "yuppers-web-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let write = |path: &str, text: &str| {
            let file = directory.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        write("index.html", HOME);
        write("en/i/index.html", EN);
        write("es/i/index.html", ES);
        write("privacy/index.html", PRIVACY);
        write("es/privacy/index.html", PRIVACY_ES);
        write("terms/index.html", TERMS);
        write("es/terms/index.html", TERMS_ES);
        write("sms-opt-in/index.html", OPT_IN);
        write("es/sms-opt-in/index.html", OPT_IN_ES);
        std::fs::write(directory.join("sms-opt-in/1-sign-in.webp"), PICTURE).unwrap();
        write("assets/index-DCaXBRW7.js", SCRIPT);
        write("favicon.svg", ICON);
        Self(directory)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Build {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn service(web_origin: &str, web: Option<WebApp>) -> Router {
    service_with_links(web_origin, web, AppLinks::default())
}

fn service_with_links(web_origin: &str, web: Option<WebApp>, app_links: AppLinks) -> Router {
    service_with_build(web_origin, web, app_links, BuildInfo::default())
}

fn service_with_build(
    web_origin: &str,
    web: Option<WebApp>,
    app_links: AppLinks,
    build: BuildInfo,
) -> Router {
    let state = AppState {
        // Never connected to: nothing here needs a database.
        db: db::pool("postgres://nobody@127.0.0.1:1/nothing").unwrap(),
        settings: Arc::new(Settings {
            app_secret: b"test-secret-test-secret-test-secret".to_vec(),
            web_origin: web_origin.to_owned(),
            auth: AuthRules::default(),
            rules: Default::default(),
            consent_version: "test".to_owned(),
            proxies: TrustedProxies::none(),
            min_client_versions: Default::default(),
            app_links,
            push_notifications: false,
            build,
            sms_updates: false,
            sms_webhook_token: None,
        }),
        code_sender: Arc::new(LogSender),
        metrics: Default::default(),
    };
    http::router(state, web)
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Reply {
    fn header(&self, name: impl axum::http::header::AsHeaderName) -> &str {
        self.headers
            .get(name)
            .map(|value| value.to_str().unwrap())
            .unwrap_or("")
    }
}

async fn fetch(app: &Router, method: Method, path: &str) -> Reply {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, path: &str) -> Reply {
    fetch(app, Method::GET, path).await
}

#[tokio::test]
async fn the_entry_pages_and_the_app_are_served_without_redirects() {
    let build = Build::write();
    let web = WebApp::open(build.path()).unwrap();
    assert_eq!(web.languages(), ["en", "es"]);
    let app = service("http://localhost:8080", Some(web));

    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.header(CONTENT_TYPE).starts_with("text/html"));
    assert_eq!(home.body, HOME);

    // An invitation link, with or without the slash, is that language's page.
    for path in ["/es/i", "/es/i/"] {
        let page = get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(page.header(LOCATION), "", "{path}: no redirect");
        assert_eq!(page.body, ES, "{path}");
    }
    assert_eq!(get(&app, "/en/i").await.body, EN);

    // A route the app handles in the browser opens on the app's own page,
    // and so does a language the build does not have.
    for path in [
        "/exchanges/some-id",
        "/exchanges/some-id/record",
        "/fr/i",
        "/account",
    ] {
        let page = get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(page.body, HOME, "{path}");
    }

    // Files are files.
    let script = get(&app, "/assets/index-DCaXBRW7.js").await;
    assert_eq!(script.status, StatusCode::OK);
    assert!(
        script.header(CONTENT_TYPE).contains("javascript"),
        "{}",
        script.header(CONTENT_TYPE)
    );
    assert_eq!(script.body, SCRIPT);
    let icon = get(&app, "/favicon.svg").await;
    assert!(icon.header(CONTENT_TYPE).starts_with("image/svg+xml"));

    // HEAD works as GET does, without a body.
    let head = fetch(&app, Method::HEAD, "/es/i").await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.body, "");
}

#[tokio::test]
async fn the_privacy_policy_and_the_terms_are_pages_of_their_own_in_each_language_and_need_no_script()
 {
    let build = Build::write();
    let web = WebApp::open(build.path()).unwrap();
    assert_eq!(
        web.legal_pages().collect::<Vec<_>>(),
        [
            "/privacy",
            "/es/privacy",
            "/terms",
            "/es/terms",
            "/sms-opt-in",
            "/es/sms-opt-in"
        ]
    );
    let app = service("https://app.example.test", Some(web));

    for (path, body) in [
        ("/privacy", PRIVACY),
        ("/privacy/", PRIVACY),
        ("/es/privacy", PRIVACY_ES),
        ("/es/privacy/", PRIVACY_ES),
        ("/terms", TERMS),
        ("/terms/", TERMS),
        ("/es/terms", TERMS_ES),
        ("/es/terms/", TERMS_ES),
        // How people opt in to texts, for the carriers' review.
        ("/sms-opt-in", OPT_IN),
        ("/sms-opt-in/", OPT_IN),
        ("/es/sms-opt-in", OPT_IN_ES),
    ] {
        let page = get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(page.header(LOCATION), "", "{path}: no redirect");
        assert!(page.header(CONTENT_TYPE).starts_with("text/html"), "{path}");
        assert_eq!(page.body, body, "{path}");
        // A page like any other: checked each time, and under the same
        // policy, which allows no inline script or style.
        assert_eq!(page.header(CACHE_CONTROL), "no-cache", "{path}");
        assert_eq!(
            page.header(CONTENT_SECURITY_POLICY),
            "default-src 'self'; img-src 'self' data:; object-src 'none'; \
             base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            "{path}"
        );
        assert_eq!(page.header(X_CONTENT_TYPE_OPTIONS), "nosniff", "{path}");
        assert_eq!(page.header(X_FRAME_OPTIONS), "DENY", "{path}");
        assert_eq!(page.header(REFERRER_POLICY), "no-referrer", "{path}");
        assert_eq!(
            page.header(STRICT_TRANSPORT_SECURITY),
            "max-age=31536000",
            "{path}"
        );
    }

    // The default language has no page under its own code, and a language
    // the build does not have has none at all: the app's page answers, and
    // shows the policy itself.
    for path in [
        "/en/privacy",
        "/fr/privacy",
        "/privacy/more",
        "/en/terms",
        "/fr/terms",
    ] {
        let page = get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(page.body, HOME, "{path}");
    }

    let head = fetch(&app, Method::HEAD, "/es/terms").await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.body, "");

    // Its pictures come from this origin, as images.
    let picture = get(&app, "/sms-opt-in/1-sign-in.webp").await;
    assert_eq!(picture.status, StatusCode::OK);
    assert_eq!(picture.header(CONTENT_TYPE), "image/webp");
}

#[tokio::test]
async fn a_build_without_the_documents_pages_still_serves_the_app_there() {
    let build = Build::write();
    for document in ["privacy", "terms", "sms-opt-in"] {
        std::fs::remove_dir_all(build.path().join(document)).unwrap();
        std::fs::remove_dir_all(build.path().join("es").join(document)).unwrap();
    }
    let web = WebApp::open(build.path()).unwrap();
    assert_eq!(web.legal_pages().count(), 0);
    let app = service("http://localhost:8080", Some(web));
    for path in ["/privacy", "/es/privacy", "/terms", "/es/terms"] {
        assert_eq!(get(&app, path).await.body, HOME, "{path}");
    }
}

#[tokio::test]
async fn hashed_assets_are_cached_for_a_year_and_pages_are_not() {
    let build = Build::write();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );

    assert_eq!(
        get(&app, "/assets/index-DCaXBRW7.js")
            .await
            .header(CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
    for path in ["/", "/es/i", "/exchanges/some-id", "/favicon.svg"] {
        assert_eq!(
            get(&app, path).await.header(CACHE_CONTROL),
            "no-cache",
            "{path}"
        );
    }
    // A missing asset is the app's page, which must not be cached as the
    // asset: a later build may have it.
    let missing = get(&app, "/assets/gone-00000000.js").await;
    assert_eq!(missing.body, HOME);
    assert_eq!(missing.header(CACHE_CONTROL), "no-cache");
}

#[tokio::test]
async fn api_paths_keep_precedence_and_are_never_answered_with_a_page() {
    let build = Build::write();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );

    assert_eq!(get(&app, "/healthz").await.status, StatusCode::NO_CONTENT);
    let meta = get(&app, "/v1/meta").await;
    assert_eq!(meta.status, StatusCode::OK);
    assert!(meta.header(CONTENT_TYPE).starts_with("application/json"));

    for path in ["/v1/nothing", "/v1/exchanges/x/y/z", "/v1/", "/v1"] {
        let reply = get(&app, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            !reply.header(CONTENT_TYPE).starts_with("text/html"),
            "{path}: {}",
            reply.header(CONTENT_TYPE)
        );
    }
}

#[tokio::test]
async fn nothing_outside_the_build_is_served() {
    let build = Build::write();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );
    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/assets/../../Cargo.toml",
    ] {
        let reply = get(&app, path).await;
        assert!(!reply.body.contains("[package]"), "{path}");
    }
}

#[tokio::test]
async fn every_response_carries_the_headers_a_signing_page_needs() {
    let build = Build::write();
    let plain = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );
    for path in [
        "/",
        "/es/i",
        "/assets/index-DCaXBRW7.js",
        "/v1/meta",
        "/healthz",
    ] {
        let reply = get(&plain, path).await;
        assert_eq!(reply.header(X_FRAME_OPTIONS), "DENY", "{path}");
        assert_eq!(
            reply.header(CONTENT_SECURITY_POLICY),
            "default-src 'self'; img-src 'self' data:; object-src 'none'; \
             base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            "{path}"
        );
        assert_eq!(reply.header(X_CONTENT_TYPE_OPTIONS), "nosniff", "{path}");
        assert_eq!(reply.header(REFERRER_POLICY), "no-referrer", "{path}");
        // Not over plain HTTP, or a development setup locks itself out.
        assert_eq!(reply.header(STRICT_TRANSPORT_SECURITY), "", "{path}");
    }

    let secure = service(
        "https://app.example.test",
        Some(WebApp::open(build.path()).unwrap()),
    );
    assert_eq!(
        get(&secure, "/").await.header(STRICT_TRANSPORT_SECURITY),
        "max-age=31536000"
    );
    assert_eq!(
        get(&secure, "/v1/meta")
            .await
            .header(STRICT_TRANSPORT_SECURITY),
        "max-age=31536000"
    );
}

#[tokio::test]
async fn api_answers_are_never_stored_and_pages_keep_their_own_caching() {
    let build = Build::write();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );
    // Answered, refused or not found: none of it may be kept.
    for path in ["/v1/meta", "/v1/me", "/v1/nothing", "/v1"] {
        assert_eq!(
            get(&app, path).await.header(CACHE_CONTROL),
            "no-store",
            "{path}"
        );
    }
    assert_eq!(
        fetch(&app, Method::POST, "/v1/exchanges")
            .await
            .header(CACHE_CONTROL),
        "no-store"
    );
    // Pages and assets keep what the web app's own rules say.
    assert_eq!(get(&app, "/").await.header(CACHE_CONTROL), "no-cache");
    assert_eq!(
        get(&app, "/assets/index-DCaXBRW7.js")
            .await
            .header(CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(get(&app, "/v1x").await.header(CACHE_CONTROL), "no-cache");
}

#[tokio::test]
async fn a_path_with_several_leading_slashes_never_redirects_to_another_host() {
    let build = Build::write();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );
    // `//assets/` in a Location header is a link to a host named `assets`.
    for path in [
        "//assets",
        "///assets",
        "//evil.example",
        "/assets",
        "//es",
        "/es",
    ] {
        let reply = get(&app, path).await;
        assert!(
            !reply.status.is_redirection(),
            "{path}: {} to {}",
            reply.status,
            reply.header(LOCATION)
        );
        assert_eq!(reply.header(LOCATION), "", "{path}");
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(reply.body, HOME, "{path}");
    }
    // Read as the one-slash path it means.
    assert_eq!(get(&app, "//es/i").await.body, ES);
    assert_eq!(get(&app, "//assets/index-DCaXBRW7.js").await.body, SCRIPT);
    // An API path is still the API's, and never a page.
    let reply = get(&app, "//v1/meta").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(!reply.header(CONTENT_TYPE).starts_with("text/html"));
}

#[tokio::test]
async fn without_a_web_directory_the_api_stands_alone() {
    let app = service("http://localhost:8080", None);
    assert_eq!(get(&app, "/healthz").await.status, StatusCode::NO_CONTENT);
    assert_eq!(get(&app, "/").await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/es/i").await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/healthz").await.header(X_FRAME_OPTIONS), "DENY");
}

const APPLE: &str = "/.well-known/apple-app-site-association";
const ANDROID: &str = "/.well-known/assetlinks.json";

fn app_links() -> AppLinks {
    AppLinks::new(
        &["ABCDE12345.app.yuppers".to_owned()],
        "app.yuppers",
        &[["ab"; 32].join(":")],
    )
    .unwrap()
}

#[tokio::test]
async fn the_app_link_files_name_the_apps_as_json_without_a_redirect() {
    let build = Build::write();
    let app = service_with_links(
        "https://app.example.test",
        Some(WebApp::open(build.path()).unwrap()),
        app_links(),
    );

    let apple = get(&app, APPLE).await;
    assert_eq!(apple.status, StatusCode::OK);
    assert_eq!(apple.header(CONTENT_TYPE), "application/json");
    assert_eq!(apple.header(LOCATION), "");
    assert_eq!(apple.header(CACHE_CONTROL), "public, max-age=3600");
    let apple: serde_json::Value = serde_json::from_str(&apple.body).unwrap();
    assert_eq!(
        apple,
        serde_json::json!({
            "applinks": {
                "details": [{
                    "appIDs": ["ABCDE12345.app.yuppers"],
                    "components": [
                        { "/": "/*/i", "comment": "An invitation link" },
                        { "/": "/*/i/", "comment": "An invitation link" },
                        {
                            "/": "/exchanges/*",
                            "comment": "A yup, as notification emails link to it",
                        },
                    ],
                }],
            },
        })
    );

    let android = get(&app, ANDROID).await;
    assert_eq!(android.status, StatusCode::OK);
    assert_eq!(android.header(CONTENT_TYPE), "application/json");
    assert_eq!(android.header(LOCATION), "");
    let fingerprint = ["AB"; 32].join(":");
    let android: serde_json::Value = serde_json::from_str(&android.body).unwrap();
    assert_eq!(
        android,
        serde_json::json!([{
            "relation": ["delegate_permission/common.handle_all_urls"],
            "target": {
                "namespace": "android_app",
                "package_name": "app.yuppers",
                "sha256_cert_fingerprints": [fingerprint],
            },
        }])
    );

    // HEAD as GET, and the same files without the web app.
    assert_eq!(
        fetch(&app, Method::HEAD, APPLE).await.status,
        StatusCode::OK
    );
    let alone = service_with_links("https://app.example.test", None, app_links());
    assert_eq!(get(&alone, APPLE).await.status, StatusCode::OK);
    assert_eq!(get(&alone, ANDROID).await.status, StatusCode::OK);
}

#[tokio::test]
async fn unset_app_link_files_are_not_found_and_never_the_apps_page() {
    let build = Build::write();
    let with_web = service(
        "https://app.example.test",
        Some(WebApp::open(build.path()).unwrap()),
    );
    let alone = service("https://app.example.test", None);
    for app in [&with_web, &alone] {
        for path in [APPLE, ANDROID] {
            let reply = get(app, path).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
            assert_ne!(reply.body, HOME, "{path}");
            assert!(
                !reply.header(CONTENT_TYPE).starts_with("text/html"),
                "{path}"
            );
        }
    }

    // One set and the other not: only the one is served.
    let apple_only = service_with_links(
        "https://app.example.test",
        Some(WebApp::open(build.path()).unwrap()),
        AppLinks::new(&["ABCDE12345.app.yuppers".to_owned()], "app.yuppers", &[]).unwrap(),
    );
    assert_eq!(get(&apple_only, APPLE).await.status, StatusCode::OK);
    assert_eq!(
        get(&apple_only, ANDROID).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_directory_that_is_not_a_build_is_refused() {
    let empty = std::env::temp_dir().join(format!(
        "yuppers-not-a-build-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&empty).unwrap();
    assert!(WebApp::open(&empty).is_err());
    assert!(WebApp::open(&empty.join("missing")).is_err());
    std::fs::remove_dir_all(&empty).ok();
}

#[tokio::test]
async fn the_web_app_is_compressed_for_clients_that_ask_and_the_api_never_is() {
    let build = Build::write();
    // Big enough to be worth compressing.
    let big = "console.log('yup');\n".repeat(200);
    std::fs::write(build.path().join("assets/app-Big12345.js"), &big).unwrap();
    let app = service(
        "http://localhost:8080",
        Some(WebApp::open(build.path()).unwrap()),
    );

    let ask = |path: &'static str, encoding: &'static str| {
        let app = app.clone();
        async move {
            let request = Request::builder()
                .uri(path)
                .header("accept-encoding", encoding)
                .body(Body::empty())
                .unwrap();
            app.oneshot(request).await.unwrap()
        }
    };

    for (encoding, expected) in [("br", "br"), ("gzip", "gzip")] {
        let response = ask("/assets/app-Big12345.js", encoding).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-encoding"], expected);
        assert!(
            response.headers()["vary"]
                .to_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains("accept-encoding")
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            bytes.len() < big.len() / 4,
            "{encoding}: {} bytes",
            bytes.len()
        );
    }

    // Without asking, the file comes as it is.
    let plain = get(&app, "/assets/app-Big12345.js").await;
    assert_eq!(plain.header("content-encoding"), "");
    assert_eq!(plain.body, big);

    // API answers are never compressed, whatever the client accepts.
    for path in ["/v1/meta", "/v1/nothing", "/healthz"] {
        let response = ask(path, "br, gzip").await;
        assert!(
            response.headers().get("content-encoding").is_none(),
            "{path} was compressed"
        );
    }
}

/// A build made from a known commit at a known time.
fn known_build() -> BuildInfo {
    BuildInfo::resolve(
        Some("89abcdef0123456789abcdef0123456789abcdef"),
        Some("2026-10-03T12:00:00Z"),
        &|_| None,
    )
}

#[tokio::test]
async fn meta_names_the_commit_and_build_time_when_the_build_says() {
    let app = service_with_build("https://app.test", None, AppLinks::default(), known_build());
    let reply = get(&app, "/v1/meta").await;
    assert_eq!(reply.status, StatusCode::OK);
    let meta: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(meta["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(meta["commit"], "89abcdef0123456789abcdef0123456789abcdef");
    assert_eq!(meta["built_at"], "2026-10-03T12:00:00Z");
    assert_eq!(reply.header("x-yuppers-version"), "89abcde");
}

#[tokio::test]
async fn meta_says_unknown_when_the_build_does_not() {
    let app = service("https://app.test", None);
    let reply = get(&app, "/v1/meta").await;
    let meta: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(meta["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(meta["commit"], "unknown");
    assert!(meta["built_at"].is_null(), "{meta}");
    assert_eq!(reply.header("x-yuppers-version"), "unknown");
}

#[tokio::test]
async fn every_response_names_the_build() {
    let build = Build::write();
    let web = WebApp::open(build.path()).unwrap();
    let app = service_with_build(
        "https://app.test",
        Some(web),
        AppLinks::default(),
        known_build(),
    );
    for path in [
        "/healthz",
        "/",
        "/en/i",
        "/assets/index-DCaXBRW7.js",
        "/v1/nothing",
        "/.well-known/assetlinks.json",
    ] {
        let reply = get(&app, path).await;
        assert_eq!(reply.header("x-yuppers-version"), "89abcde", "{path}");
    }
}
