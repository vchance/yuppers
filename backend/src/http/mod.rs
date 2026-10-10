use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Extension, Router};
use sqlx::PgPool;
use tracing::Instrument;
use utoipa::OpenApi;
use uuid::Uuid;

use crate::auth::{AuthRules, CodeSender, SignInChannel};
use crate::build_info::{self, BuildInfo};
use crate::client_version::{self, MinimumClientVersions};
use crate::domain::Rules;
use crate::error::{ErrorBody, ErrorCode};
use crate::metrics::{self, HttpMetrics};
use crate::otel::{self, SpanExt, SpanKind};
use crate::wallet::{Wallet, WalletPlatform};

pub mod account;
pub mod auth;
pub mod client_address;
pub mod deletion;
pub mod devices;
pub mod exchanges;
pub mod extract;
pub mod health;
pub mod payments;
pub mod record;
pub mod safety;
pub mod sms;
pub mod staff;
pub mod v1;
pub mod wallet;
pub mod web;

pub use client_address::{ClientAddress, TrustedProxies};
pub use web::{AppLinks, WebApp};

/// What the handlers need besides the database.
pub struct Settings {
    pub app_secret: Vec<u8>,
    pub web_origin: String,
    pub auth: AuthRules,
    pub rules: Rules,
    /// The version of the consent wording a signer must have been shown.
    pub consent_version: String,
    /// Which header, if any, names the requester's address.
    pub proxies: TrustedProxies,
    /// The oldest build of each client that may still change anything
    /// (`crate::client_version`). None required unless configured.
    pub min_client_versions: MinimumClientVersions,
    /// Which apps may open the web origin's invitation links
    /// (`web::AppLinks`). None unless configured.
    pub app_links: AppLinks,
    /// Whether push notifications are sent (`PUSH_DELIVERY`), so that the
    /// apps offer them only when they will arrive.
    pub push_notifications: bool,
    /// Which build this is, for `GET /v1/meta` and the `X-Yuppers-Version`
    /// header (`crate::build_info`).
    pub build: BuildInfo,
    /// Whether text messages are sent (`SMS_DELIVERY`), and so whether
    /// agreement updates by text can be turned on (`sms::updates`).
    pub sms_updates: bool,
    /// The auth token that checks Twilio's signature on its requests to
    /// `POST /v1/sms/inbound`. With none, every such request is refused.
    pub sms_webhook_token: Option<crate::notifications::smtp::Secret>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub settings: Arc<Settings>,
    pub code_sender: Arc<dyn CodeSender>,
    /// Request counts and latencies, served when `METRICS_ADDR` is set.
    pub metrics: Arc<HttpMetrics>,
}

/// The whole service: the API, and the web app if there is one to serve.
/// API paths are routed first; the web app answers what is left. No Wallet
/// platform is configured.
pub fn router(state: AppState, web: Option<WebApp>) -> Router {
    let wallet = Arc::new(Wallet::off(&state.settings.web_origin));
    router_with_wallet(state, web, wallet)
}

/// [`router`], issuing Wallet passes for the platforms `wallet` has.
pub fn router_with_wallet(state: AppState, web: Option<WebApp>, wallet: Arc<Wallet>) -> Router {
    // People sign on these pages, so the page must be ours and nobody
    // else's frame. HSTS only when the origin is HTTPS, or a development
    // setup over plain HTTP would be locked out of itself.
    let hsts = state.settings.web_origin.starts_with("https://");
    let api = Router::new()
        .route("/healthz", get(health::live))
        .route("/readyz", get(health::ready))
        // Routed here, not left to the web app, so that an unset file is a
        // plain "not found", never the app's page, with or without WEB_DIR.
        .route(
            web::APPLE_APP_SITE_ASSOCIATION_PATH,
            get(web::apple_app_site_association),
        )
        .route(web::ASSET_LINKS_PATH, get(web::asset_links))
        .nest("/v1", v1::router())
        // A web session renewed by a request gets its cookie sent again.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            extract::refresh_session_cookie,
        ))
        .layer(Extension(wallet))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            client_version::refuse_old_clients,
        ));
    let app = match web {
        Some(web) => api.fallback_service(web.router()),
        None => api,
    };
    app.layer(middleware::from_fn(move |request, next| {
        security_headers(hsts, request, next)
    }))
    // Added with `Router::layer`, so it runs once routing has picked a
    // route, and the route's template is known.
    .layer(middleware::from_fn_with_state(state.clone(), observe))
    .with_state(state)
}

/// The header a request's ID travels in, both ways.
pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The longest request ID taken from a client or proxy.
const REQUEST_ID_MAX: usize = 64;

/// The request's ID: the one a proxy or client sent in `X-Request-Id`, if it
/// is short and harmless, otherwise a new one. Harmless means 1 to 64
/// letters, digits, `-`, `_` or `.`: enough for a UUID or any proxy's own
/// format, and nothing that could forge a log line or carry an email
/// address.
pub fn request_id(headers: &HeaderMap) -> String {
    headers
        .get(&REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .filter(|id| {
            (1..=REQUEST_ID_MAX).contains(&id.len())
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// Gives each request an ID, handles it in a span that names it, writes one
/// line when it is answered, and counts it.
///
/// The span holds the method, the path and the request ID, so every line
/// logged while the request is handled carries them, and the trace and
/// span IDs (`crate::otel`), so a line and the exported trace can be put
/// together. The path only: a query string could carry something a person
/// typed, and never belongs in a log. Nor does anything else from the
/// request or the response: no body, no other header, no token, no cookie.
///
/// Exported, the span is `GET /v1/exchanges/{id}`: the method and the
/// route's template, with the status it answered, never the path itself.
async fn observe(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let id = request_id(request.headers());
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    // A Wallet device's requests name the device in their path; theirs is
    // logged as the route's template, which names nothing.
    let path = match &route {
        Some(route) if route.starts_with(wallet::DEVICE_ROUTES) => route.as_str(),
        _ => request.uri().path(),
    };
    // Every request is a trace of its own: no client or proxy upstream
    // traces, so none is believed about where a trace began.
    let ids = otel::Ids::new();
    let span = tracing::info_span!(
        "request",
        method = %method,
        path = path,
        request_id = %id,
        trace_id = %ids.trace_hex(),
        span_id = %ids.span_hex(),
    );
    let template = route.as_deref().unwrap_or(metrics::UNMATCHED);
    span.otel_name(format!("{method} {template}"));
    span.otel_kind(SpanKind::Server);
    span.otel_attr("http.request.method", method.as_str().to_owned());
    span.otel_attr("http.route", template.to_owned());

    let mut response = next.run(request).instrument(span.clone()).await;

    let elapsed = started.elapsed();
    let status = response.status();
    state
        .metrics
        .observe(&method, route.as_deref(), status, elapsed);
    span.otel_attr("http.response.status_code", status.as_u16());
    if status.is_server_error() {
        span.otel_error();
    }
    // To the microsecond: most requests take less than a millisecond.
    let latency_ms = elapsed.as_micros() as f64 / 1000.0;
    span.in_scope(|| {
        tracing::info!(status = status.as_u16(), latency_ms, "request completed");
    });
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID, value);
    }
    // On every response, so whatever a person or a proxy captured, a page as
    // much as an API call or a health check, says which build answered.
    // Seven characters, and nothing the public repository and `/v1/meta`
    // do not already say.
    if let Ok(value) = HeaderValue::from_str(state.settings.build.short_commit()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(build_info::HEADER), value);
    }
    response
}

/// What a page may load, and from where: only this origin's own scripts,
/// styles, fonts and API, images from here or inline as `data:`, no plugins,
/// no `<base>` and no frame around it. The built web app needs nothing more:
/// it has no inline style, and talks to the API on its own origin. Its one
/// inline script, which applies the appearance chosen on the device before
/// the page paints, is allowed by its hash and nothing else inline is
/// (`apps/web/build/theme-script.ts`, whose test checks the hash here).
/// API responses carry it too, which costs nothing and covers a response
/// some browser decides to render.
pub const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; img-src 'self' data:; \
    object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; \
    script-src 'self' 'sha256-KxR9MhTq1F37YaceB87TcfvvGJe+U71nuBxrnuq+JCQ='";

/// The browser features no page here may use. The web app uses none of
/// them; its one feature, offering an invitation link to the device's
/// share sheet, stays allowed to this origin (`web-share`), and copying to
/// the clipboard needs no permission. Payment options open the payment
/// app by an ordinary link, never the Payment Request API.
pub const PERMISSIONS_POLICY_VALUE: &str = "accelerometer=(), browsing-topics=(), camera=(), \
    display-capture=(), geolocation=(), gyroscope=(), hid=(), magnetometer=(), microphone=(), \
    midi=(), payment=(), serial=(), usb=(), web-share=(self)";

/// HTTPS only, for a year, here and on every subdomain. Every name under
/// `yuppers.app` is HTTPS-only already: the whole `.app` domain is on the
/// browsers' preload list, and the only other web name is `www`, which
/// Render serves over HTTPS as a redirect to the apex. Not submitted for
/// preloading by name: that is hard to undo, and `.app` already is.
pub const STRICT_TRANSPORT_SECURITY_VALUE: &str = "max-age=31536000; includeSubDomains";

async fn security_headers(hsts: bool, request: Request, next: Next) -> Response {
    // The API's answers are about one person, often behind a session: no
    // cache, shared or the browser's own, may keep them, unless a handler
    // says otherwise.
    let api = request.uri().path() == "/v1" || request.uri().path().starts_with("/v1/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if api && !headers.contains_key(CACHE_CONTROL) {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY_VALUE),
    );
    // A page here shares no browsing context with a window of another
    // origin, which could otherwise reach it through `window.opener`.
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if hsts {
        headers.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(STRICT_TRANSPORT_SECURITY_VALUE),
        );
    }
    response
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Yuppers API"),
    paths(
        health::live,
        health::ready,
        v1::meta,
        auth::request_code,
        auth::create_session,
        auth::delete_session,
        account::me,
        account::update_me,
        account::add_identifier,
        account::prove_identifier,
        account::remove_identifier,
        account::combine_accounts,
        account::request_invitation_address_code,
        account::add_invitation_address,
        deletion::deletion_preview,
        deletion::request_deletion_code,
        deletion::delete_account,
        devices::register_device,
        devices::remove_device,
        payments::payment_handles,
        payments::set_payment_handles,
        payments::remove_payment_handles,
        payments::set_payment_handle,
        payments::remove_payment_handle,
        payments::set_payment_options,
        sms::sms_updates,
        sms::set_sms_updates,
        sms::inbound,
        exchanges::create,
        exchanges::list,
        exchanges::get,
        exchanges::save_draft,
        exchanges::send_revision,
        exchanges::run_command,
        exchanges::leave,
        record::history,
        record::record,
        exchanges::reissue_invitation,
        exchanges::invitation_shared,
        exchanges::preview_invitation,
        exchanges::claim_invitation,
        safety::report_exchange,
        safety::report_invitation,
        safety::block_status,
        safety::block,
        safety::unblock,
        safety::blocked_people,
        staff::queue,
        staff::open_report,
        staff::resolve,
        staff::suspensions,
        staff::lift,
        staff::hidden,
        staff::restore,
        wallet::apple_pass,
        wallet::apple_link,
        wallet::google_link,
    ),
    components(schemas(
        ErrorBody,
        ErrorCode,
        MinimumClientVersions,
        SignInChannel,
        WalletPlatform
    ))
)]
struct ApiDoc;

/// The API contract. The `openapi` binary prints it, and the TypeScript client
/// used by web and mobile is generated from that output.
pub fn api_doc() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    // The derive fills these from Cargo metadata this crate does not set.
    doc.info.description = None;
    doc.info.license = None;
    doc
}
