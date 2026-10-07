//! Serving the built web app from the same origin as the API (DESIGN.md
//! §13.5).
//!
//! The build is static files: `index.html`, the app's own entry page; one
//! entry page per language at `{language}/i/index.html`, which an invitation
//! link points at; the privacy policy's and the terms' page in each
//! language; hashed files under `assets/`; and whatever else is in the app's public directory. Four
//! rules turn that into a site:
//!
//! * `/{language}/i`, with or without a trailing slash, is answered with that
//!   language's page directly, without a redirect that would change the link
//!   a messaging app previews;
//! * `/{document}` and `/{language}/{document}`, for the privacy policy
//!   (`privacy`), the terms (`terms`) and the page on how people opt in to
//!   texts (`sms-opt-in`, which has no script), with or without a trailing slash,
//!   are answered with that document's static page in that language
//!   (`{document}/index.html`, `{language}/{document}/index.html`): the
//!   app's entry page with the document written into it, so that it reads
//!   without scripts;
//! * a path that is no file is answered with `index.html`, so a route the app
//!   handles in the browser can be opened or reloaded directly, except under
//!   `/assets/`, where it is not found: no route of the app is there, and a
//!   page in place of a script, a style sheet or a source map is only
//!   confusing;
//! * the API's paths are never answered with a page.
//!
//! A path that starts with several slashes is read as if it had one, and a
//! directory is never answered with a redirect: `//assets` redirected to
//! `//assets/` would be a link to a host named `assets`.
//!
//! Hashed assets may be cached for a year; everything else must be checked
//! each time, or a new build would be a page pointing at files that are gone.
//!
//! The token of an invitation link is in the URL fragment, which a browser
//! never sends. Nothing here reads the query string either, and the request
//! span logs the path alone.
//!
//! The same origin also tells iOS and Android which apps may open its
//! invitation links and exchange pages in place of the browser
//! ([`AppLinks`]): two small JSON files under `/.well-known/`, served when a
//! deployment names the apps and absent when it does not, whether or not
//! this service serves the pages.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::error::{ApiError, ErrorCode};
use crate::http::AppState;

/// The pages the build writes in each language that are answered at their
/// own address: the privacy policy and the terms
/// (`apps/web/build/legal-pages.ts`), and the page on how people opt in to
/// texts (`apps/web/build/sms-opt-in.ts`).
const LEGAL_DOCUMENTS: &[&str] = &["privacy", "terms", "sms-opt-in"];

/// Paths that belong to the API, whatever is or is not routed under them.
const API_PATHS: &[&str] = &["/v1", "/healthz", "/readyz"];

/// Whether a path is one of the API's, or below one of them.
fn is_api_path(path: &str) -> bool {
    API_PATHS.iter().any(|api| {
        path.strip_prefix(api)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// A built web app, ready to serve.
#[derive(Clone, Debug)]
pub struct WebApp {
    directory: PathBuf,
    /// The languages that have an entry page, from the build itself.
    languages: Vec<String>,
    /// The privacy policy's and the terms' pages the build has: the address
    /// each answers, such as `/privacy` or `/es/terms`, and its file.
    legal_pages: Vec<(String, String)>,
}

impl WebApp {
    /// Reads the build at `directory`. Fails if it is not one.
    pub fn open(directory: &Path) -> std::io::Result<Self> {
        let index = directory.join("index.html");
        if !index.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "{} has no index.html; is it a web build?",
                    directory.display()
                ),
            ));
        }
        let mut languages: Vec<String> = std::fs::read_dir(directory)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join("i").join("index.html").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        languages.sort();
        // The default language's page at `/{document}`, every other
        // language's under its own code, as the build writes them.
        let mut legal_pages = Vec::new();
        for document in LEGAL_DOCUMENTS {
            if directory.join(document).join("index.html").is_file() {
                legal_pages.push((format!("/{document}"), format!("/{document}/index.html")));
            }
            for language in &languages {
                if directory
                    .join(language)
                    .join(document)
                    .join("index.html")
                    .is_file()
                {
                    legal_pages.push((
                        format!("/{language}/{document}"),
                        format!("/{language}/{document}/index.html"),
                    ));
                }
            }
        }
        Ok(Self {
            directory: directory.to_owned(),
            languages,
            legal_pages,
        })
    }

    /// The addresses of the privacy policy's and the terms' pages, such as
    /// `/privacy`.
    pub fn legal_pages(&self) -> impl Iterator<Item = &str> {
        self.legal_pages.iter().map(|(path, _)| path.as_str())
    }

    /// The file of the privacy policy's or the terms' page that `path` names,
    /// if it names one.
    fn legal_page(&self, path: &str) -> Option<&str> {
        let path = path.strip_suffix('/').unwrap_or(path);
        self.legal_pages
            .iter()
            .find(|(page, _)| page == path)
            .map(|(_, file)| file.as_str())
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// The language whose entry page `path` names, if it names one.
    fn entry_page(&self, path: &str) -> Option<&str> {
        let rest = path.strip_prefix('/')?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let language = rest.strip_suffix("/i")?;
        self.languages
            .iter()
            .find(|known| known.as_str() == language)
            .map(String::as_str)
    }

    /// The service for everything the API does not route.
    pub fn router(self) -> Router {
        // No redirect from a directory to its path with a slash: the entry
        // pages are routed above without one, and a redirect built from the
        // request's own path can point at another host.
        let files = ServeDir::new(&self.directory)
            .append_index_html_on_directories(true)
            .redirect_to_trailing_slash(false)
            .fallback(ServeFile::new(self.directory.join("index.html")));
        // The same files with no page in place of one missing.
        let assets = ServeDir::new(&self.directory)
            .append_index_html_on_directories(false)
            .redirect_to_trailing_slash(false);
        // The web app's own files are compressed here, so the invitation page
        // is small on a phone whatever sits in front of the service. Only
        // these: API responses carry personal data next to what a request
        // sent, which is the shape compression side channels need.
        Router::new()
            .route_service("/assets/{*file}", assets)
            .fallback_service(files)
            .layer(middleware::from_fn(cache_control))
            .layer(CompressionLayer::new().br(true).gzip(true))
            .layer(middleware::from_fn_with_state(self, route))
    }
}

/// `uri` with any run of slashes at the start of its path made one, or
/// `None` if it has no such run.
fn single_leading_slash(uri: &Uri) -> Option<Uri> {
    let path = uri.path();
    if !path.starts_with("//") {
        return None;
    }
    let path = format!("/{}", path.trim_start_matches('/'));
    let path_and_query = match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    };
    path_and_query.parse().ok()
}

/// Keeps API paths away from the pages and sends an entry-page path to its
/// file.
async fn route(
    axum::extract::State(web): axum::extract::State<WebApp>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("//") {
        let Some(uri) = single_leading_slash(request.uri()) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        *request.uri_mut() = uri;
    }
    let path = request.uri().path();
    if is_api_path(path) {
        return ApiError::from(ErrorCode::NotFound).into_response();
    }
    if let Some(language) = web.entry_page(path) {
        let Ok(uri) = format!("/{language}/i/index.html").parse::<Uri>() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        *request.uri_mut() = uri;
    } else if let Some(file) = web.legal_page(path) {
        let Ok(uri) = file.parse::<Uri>() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        *request.uri_mut() = uri;
    }
    next.run(request).await
}

/// Hashed assets for a year, everything else checked every time. A page is
/// never cached, and nor is an asset that is not found: a later build may
/// have it.
async fn cache_control(request: Request, next: Next) -> Response {
    let hashed = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
        let page = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/html"));
        let value = if hashed && !page {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

/// Where Apple's servers look for the apps a domain trusts.
pub const APPLE_APP_SITE_ASSOCIATION_PATH: &str = "/.well-known/apple-app-site-association";

/// Where Android looks for the apps a domain trusts.
pub const ASSET_LINKS_PATH: &str = "/.well-known/assetlinks.json";

/// The Android package of the Yuppers app, unless a deployment names another.
pub const DEFAULT_ANDROID_PACKAGE: &str = "app.yuppers";

/// What the web origin says about the apps that may open its invitation
/// links: universal links on iOS (`apple-app-site-association`) and app
/// links on Android (`assetlinks.json`).
///
/// Each file is written once, when the service starts, from `APPLE_APP_ID`,
/// and from `ANDROID_SHA256_CERT_FINGERPRINTS` with `ANDROID_PACKAGE`. Each
/// is served as `application/json` with no redirect, as both systems require.
/// A file whose settings are unset is not served at all, rather than
/// answered with the web app's page, which a system would read as a broken
/// file.
///
/// Two kinds of link are claimed (DESIGN.md §4.1): invitation links,
/// `/{language}/i`, whose token is in the fragment, which neither file sees
/// or needs; and an exchange's own pages, `/exchanges/{id}` and below, which
/// is where notification emails link to. Every other page of the web app
/// stays in the browser. On Android the app's intent filter says which paths
/// it takes (`apps/mobile/app.config.ts`); the file only names the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppLinks {
    apple: Option<String>,
    android: Option<String>,
}

impl AppLinks {
    /// Builds the files from the apps' identities. `apple_app_ids` are
    /// `{team id}.{bundle id}`, such as `ABCDE12345.app.yuppers`;
    /// `fingerprints` are the SHA-256 fingerprints of the certificates the
    /// Android app is signed with, as colon-separated hex pairs. Either list
    /// may be empty, and its file is then not served.
    pub fn new(
        apple_app_ids: &[String],
        android_package: &str,
        fingerprints: &[String],
    ) -> Result<Self, String> {
        let apple = if apple_app_ids.is_empty() {
            None
        } else {
            if let Some(wrong) = apple_app_ids.iter().find(|id| !is_apple_app_id(id)) {
                return Err(format!(
                    "{wrong} is not an Apple app ID: the ten-character team ID, a dot and the \
                     bundle identifier, such as ABCDE12345.app.yuppers"
                ));
            }
            let document = serde_json::json!({
                "applinks": {
                    "details": [{
                        "appIDs": apple_app_ids,
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
            });
            Some(document.to_string())
        };
        let android = if fingerprints.is_empty() {
            None
        } else {
            if !is_android_package(android_package) {
                return Err(format!("{android_package} is not an Android package name"));
            }
            let fingerprints = fingerprints
                .iter()
                .map(|fingerprint| {
                    normalized_fingerprint(fingerprint).ok_or_else(|| {
                        format!(
                            "{fingerprint} is not a SHA-256 certificate fingerprint: 32 \
                             hexadecimal pairs, separated by colons"
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let document = serde_json::json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": {
                    "namespace": "android_app",
                    "package_name": android_package,
                    "sha256_cert_fingerprints": fingerprints,
                },
            }]);
            Some(document.to_string())
        };
        Ok(Self { apple, android })
    }

    /// Which of the two files are served, for the log at startup.
    pub fn served(&self) -> Vec<&'static str> {
        let mut served = Vec::new();
        if self.apple.is_some() {
            served.push(APPLE_APP_SITE_ASSOCIATION_PATH);
        }
        if self.android.is_some() {
            served.push(ASSET_LINKS_PATH);
        }
        served
    }
}

/// `{team id}.{bundle id}`: ten capital letters or digits, a dot, and a
/// dotted bundle identifier.
fn is_apple_app_id(id: &str) -> bool {
    let Some((team, bundle)) = id.split_once('.') else {
        return false;
    };
    team.len() == 10
        && team
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && bundle.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// Two or more dot-separated parts, each a letter followed by letters,
/// digits or underscores.
fn is_android_package(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            part.bytes()
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

/// A SHA-256 fingerprint as Android writes it, `AB:CD:…`, read from the
/// same with or without colons and in either case. `None` if it is not one.
fn normalized_fingerprint(text: &str) -> Option<String> {
    let hex: String = text.trim().chars().filter(|char| *char != ':').collect();
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_ascii_uppercase();
    let pairs: Vec<&str> = (0..32).map(|pair| &hex[pair * 2..pair * 2 + 2]).collect();
    Some(pairs.join(":"))
}

/// A file from [`AppLinks`], or the API's own "not found" when it is unset.
fn association_file(document: Option<&String>) -> Response {
    match document {
        Some(document) => (
            [
                (CONTENT_TYPE, HeaderValue::from_static("application/json")),
                // Short enough that a corrected file is picked up within the
                // hour; Apple's servers keep a copy of their own regardless.
                (
                    CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=3600"),
                ),
            ],
            document.clone(),
        )
            .into_response(),
        None => ApiError::from(ErrorCode::NotFound).into_response(),
    }
}

/// `GET /.well-known/apple-app-site-association`.
pub async fn apple_app_site_association(State(state): State<AppState>) -> Response {
    association_file(state.settings.app_links.apple.as_ref())
}

/// `GET /.well-known/assetlinks.json`.
pub async fn asset_links(State(state): State<AppState>) -> Response {
    association_file(state.settings.app_links.android.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINGERPRINT: &str = "14:6D:E9:83:C5:73:06:50:D8:EE:B9:95:2F:34:FC:64:\
                               16:A0:83:42:E6:1D:BE:A8:8A:04:96:B2:3F:CF:44:E5";

    #[test]
    fn apple_app_ids_are_a_team_id_and_a_bundle_id() {
        for good in ["ABCDE12345.app.yuppers", "0123456789.app.yuppers-preview"] {
            assert!(is_apple_app_id(good), "{good}");
        }
        for bad in [
            "",
            "app.yuppers",
            "abcde12345.app.yuppers",
            "ABCDE1234.app.yuppers",
            "ABCDE12345.",
            "ABCDE12345.app..yuppers",
            "ABCDE12345.app/yuppers",
            "ABCDE12345 .app.yuppers",
        ] {
            assert!(!is_apple_app_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn android_packages_are_dotted_names() {
        assert!(is_android_package("app.yuppers"));
        assert!(is_android_package("app.yuppers.preview_1"));
        for bad in [
            "",
            "yuppers",
            "app.",
            ".app",
            "app.1yuppers",
            "app.yup-pers",
        ] {
            assert!(!is_android_package(bad), "{bad:?}");
        }
    }

    #[test]
    fn fingerprints_are_written_as_android_writes_them() {
        assert_eq!(
            normalized_fingerprint(FINGERPRINT).as_deref(),
            Some(FINGERPRINT)
        );
        let bare = FINGERPRINT.replace(':', "").to_ascii_lowercase();
        assert_eq!(normalized_fingerprint(&bare).as_deref(), Some(FINGERPRINT));
        let not_hex = FINGERPRINT.replacen("14", "ZZ", 1);
        let too_long = format!("{FINGERPRINT}:00");
        for bad in ["", "14:6D", not_hex.as_str(), too_long.as_str()] {
            assert_eq!(normalized_fingerprint(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn nothing_is_served_unless_named_and_a_wrong_value_is_refused() {
        let none = AppLinks::new(&[], DEFAULT_ANDROID_PACKAGE, &[]).unwrap();
        assert_eq!(none, AppLinks::default());
        assert!(none.served().is_empty());
        let team_missing = ["app.yuppers".to_owned()];
        assert!(AppLinks::new(&team_missing, DEFAULT_ANDROID_PACKAGE, &[]).is_err());
        let short = ["AB:CD".to_owned()];
        assert!(AppLinks::new(&[], DEFAULT_ANDROID_PACKAGE, &short).is_err());
        let fingerprint = [FINGERPRINT.to_owned()];
        assert!(AppLinks::new(&[], "yuppers", &fingerprint).is_err());
        // A package with no fingerprint says nothing, so it is not checked.
        assert!(
            AppLinks::new(&[], "yuppers", &[])
                .unwrap()
                .served()
                .is_empty()
        );
    }

    fn web(languages: &[&str]) -> WebApp {
        WebApp {
            directory: PathBuf::from("/nowhere"),
            languages: languages.iter().map(|code| (*code).to_owned()).collect(),
            legal_pages: vec![
                ("/privacy".to_owned(), "/privacy/index.html".to_owned()),
                ("/terms".to_owned(), "/terms/index.html".to_owned()),
                (
                    "/es/privacy".to_owned(),
                    "/es/privacy/index.html".to_owned(),
                ),
            ],
        }
    }

    #[test]
    fn a_legal_path_names_a_page_the_build_has() {
        let web = web(&["en", "es"]);
        assert_eq!(web.legal_page("/privacy"), Some("/privacy/index.html"));
        assert_eq!(web.legal_page("/privacy/"), Some("/privacy/index.html"));
        assert_eq!(web.legal_page("/terms"), Some("/terms/index.html"));
        assert_eq!(
            web.legal_page("/es/privacy"),
            Some("/es/privacy/index.html")
        );
        assert_eq!(
            web.legal_page("/es/privacy/"),
            Some("/es/privacy/index.html")
        );
        for other in [
            "/en/privacy",
            "/fr/privacy",
            "/privacy/index.html",
            "/privacy//",
            "/privacypolicy",
            "/es/terms",
            "/terms/x",
            "privacy",
        ] {
            assert_eq!(web.legal_page(other), None, "{other:?}");
        }
    }

    #[test]
    fn api_paths_are_the_api_and_everything_below_them() {
        for api in ["/v1", "/v1/", "/v1/meta", "/healthz", "/readyz", "/readyz/"] {
            assert!(is_api_path(api), "{api}");
        }
        for page in [
            "/",
            "/v10",
            "/v1x/meta",
            "/healthzz",
            "/es/i",
            "/exchanges/v1",
        ] {
            assert!(!is_api_path(page), "{page}");
        }
    }

    #[test]
    fn leading_slashes_are_made_one() {
        let one =
            |text: &str| single_leading_slash(&text.parse().unwrap()).map(|uri| uri.to_string());
        assert_eq!(one("//assets").as_deref(), Some("/assets"));
        assert_eq!(
            one("///evil.example/x?a=b").as_deref(),
            Some("/evil.example/x?a=b")
        );
        assert_eq!(one("//").as_deref(), Some("/"));
        assert_eq!(one("/assets"), None);
        assert_eq!(one("/a//b"), None);
    }

    #[test]
    fn an_entry_page_path_names_a_language_the_build_has() {
        let web = web(&["en", "es", "pt-BR"]);
        assert_eq!(web.entry_page("/es/i"), Some("es"));
        assert_eq!(web.entry_page("/es/i/"), Some("es"));
        assert_eq!(web.entry_page("/pt-BR/i"), Some("pt-BR"));
        for other in [
            "/fr/i",
            "/es",
            "/es/i/index.html",
            "/es/invite",
            "/i",
            "/",
            "es/i",
        ] {
            assert_eq!(web.entry_page(other), None, "{other:?}");
        }
    }
}
