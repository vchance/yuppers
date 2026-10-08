//! Request extractors that answer with typed error codes.

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::AppState;
use crate::auth::{AuthRules, token_hash};
use crate::error::{ApiError, ErrorCode};

pub const SESSION_COOKIE: &str = "yuppers_session";

/// Reads a JSON request body into `T`, refusing what the service cannot
/// store or should not accept, with an error code the clients understand.
fn parse<T: DeserializeOwned>(headers: &HeaderMap, bytes: &[u8]) -> Result<T, ApiError> {
    // A form or plain-text post is something another site can make a browser
    // send without asking; JSON is not. Requiring the type keeps it that way.
    let json = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim_start().starts_with("application/json"));
    if !json {
        return Err(ErrorCode::InvalidRequest.into());
    }

    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ErrorCode::InvalidRequest)?;
    if contains_nul(&value) {
        return Err(ErrorCode::InvalidRequest.into());
    }
    serde_json::from_value(value).map_err(|_| ErrorCode::InvalidRequest.into())
}

/// Whether any text in the value holds a NUL character, which JSON allows and
/// the database cannot store.
fn contains_nul(value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(contains_nul),
        Value::Object(map) => map
            .iter()
            .any(|(key, item)| key.contains('\0') || contains_nul(item)),
        _ => false,
    }
}

/// A JSON body.
pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned> FromRequest<AppState> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, ApiError> {
        let headers = request.headers().clone();
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| ApiError::from(ErrorCode::InvalidRequest))?;
        parse(&headers, &bytes).map(Self)
    }
}

/// A JSON body together with a digest of the request it came in, for
/// endpoints that honor an idempotency key: the same key must come with the
/// same request. The digest covers the method and path as well as the body,
/// so a key used on one exchange cannot stand in for another.
pub struct DigestedJson<T> {
    pub body: T,
    pub digest: [u8; 32],
}

impl<T: DeserializeOwned> FromRequest<AppState> for DigestedJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, ApiError> {
        let headers = request.headers().clone();
        let mut digest = Sha256::new();
        digest.update(request.method().as_str().as_bytes());
        digest.update(b" ");
        digest.update(request.uri().path().as_bytes());
        digest.update(b"\n");

        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| ApiError::from(ErrorCode::InvalidRequest))?;
        digest.update(&bytes);
        Ok(Self {
            body: parse(&headers, &bytes)?,
            digest: digest.finalize().into(),
        })
    }
}

/// The signed-in account making the request.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: Uuid,
    pub account_id: Uuid,
    pub auth_method: String,
    pub authenticated_at: OffsetDateTime,
}

impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let (token, from_cookie) = match bearer_token(&parts.headers) {
            Some(token) => (token, false),
            None => (
                cookie(&parts.headers, SESSION_COOKIE).ok_or(ErrorCode::Unauthenticated)?,
                true,
            ),
        };

        // A browser attaches cookies to requests other sites cause it to
        // make. Anything that changes state must come from our own web app.
        if from_cookie && !is_safe(&parts.method) {
            require_web_origin(&parts.headers, &state.settings.web_origin)?;
        }

        let found = check_and_renew(&state.db, token, &state.settings.auth)
            .await?
            .ok_or(ErrorCode::Unauthenticated)?;

        // The browser's cookie must not run out before the session does, so
        // when the session moved, the response carries the cookie again
        // (`refresh_session_cookie`). A bearer token has no expiry of its
        // own on the device; the app keeps it until the service refuses it.
        if from_cookie
            && let Some(seconds_left) = found.renewed_for
            && let Some(renewal) = parts.extensions.get::<CookieRenewal>()
        {
            renewal.set(token.to_owned(), seconds_left);
        }

        Ok(Self {
            id: found.id,
            account_id: found.account_id,
            auth_method: found.auth_method,
            authenticated_at: found.authenticated_at,
        })
    }
}

/// A valid session, as [`check_and_renew`] found it.
#[derive(Debug, sqlx::FromRow)]
struct Found {
    id: Uuid,
    account_id: Uuid,
    auth_method: String,
    authenticated_at: OffsetDateTime,
    /// The whole seconds the session has left, when this request renewed it.
    renewed_for: Option<i64>,
}

/// Finds the live session a token belongs to and renews it, in one
/// statement. A session used with less than [`AuthRules::renew_below`] of its
/// idle time left is moved to end [`AuthRules::session_idle`] after now, but
/// never later than [`AuthRules::session_max`] after the sign-in that made it
/// (`created_at`). Otherwise the `UPDATE` matches no row and nothing is
/// written, so a session in constant use is written about once a day. Once
/// the absolute end is reached the session expires however it is used, and
/// only a new code makes another; that end is checked here as well as
/// written into `expires_at`, so a deployment that shortens
/// `SESSION_MAX_DAYS` ends the sessions already past it at once.
///
/// Only `expires_at` moves. When the holder last proved an identifier
/// (`authenticated_at`), which signing and the staff endpoints require to be
/// recent, stays as it was. A session signed out (`revoked_at`), or of an
/// account no longer active, is neither found nor renewed.
async fn check_and_renew(
    db: &PgPool,
    token: &str,
    rules: &AuthRules,
) -> Result<Option<Found>, sqlx::Error> {
    sqlx::query_as(
        "WITH found AS (
             SELECT s.id, s.account_id, s.auth_method, s.authenticated_at
             FROM account_session s
             JOIN account a ON a.id = s.account_id
             WHERE s.token_hash = $1
               AND s.revoked_at IS NULL
               AND s.expires_at > now()
               AND s.created_at + $3 * interval '1 second' > now()
               AND a.status = 'ACTIVE'
         ),
         renewed AS (
             UPDATE account_session s
             SET expires_at = least(now() + $2 * interval '1 second',
                                    s.created_at + $3 * interval '1 second')
             FROM found
             WHERE s.id = found.id
               AND s.revoked_at IS NULL
               AND s.expires_at < now() + $4 * interval '1 second'
               AND s.expires_at < least(now() + $2 * interval '1 second',
                                        s.created_at + $3 * interval '1 second')
             RETURNING s.id, s.expires_at
         )
         SELECT found.id, found.account_id, found.auth_method, found.authenticated_at,
                floor(extract(epoch FROM renewed.expires_at - now()))::bigint AS renewed_for
         FROM found
         LEFT JOIN renewed ON renewed.id = found.id",
    )
    .bind(token_hash(token).as_slice())
    .bind(rules.session_idle.whole_seconds() as f64)
    .bind(rules.session_max.whole_seconds() as f64)
    .bind(rules.renew_below().whole_seconds() as f64)
    .fetch_optional(db)
    .await
}

/// Where the [`Session`] extractor leaves a renewed cookie session's token
/// and the seconds it has left, for [`refresh_session_cookie`] to send back.
#[derive(Clone, Default)]
pub struct CookieRenewal(Arc<Mutex<Option<(String, i64)>>>);

impl CookieRenewal {
    fn set(&self, token: String, seconds_left: i64) {
        *self.0.lock().unwrap_or_else(|poison| poison.into_inner()) = Some((token, seconds_left));
    }

    fn take(&self) -> Option<(String, i64)> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
}

/// Sends the session cookie again, with a `Max-Age` of what the session now
/// has left, when the request renewed the session its cookie names. A
/// response that sets the cookie itself, as signing out and deleting the
/// account do to clear it, is left as it is.
pub async fn refresh_session_cookie(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let renewal = CookieRenewal::default();
    request.extensions_mut().insert(renewal.clone());
    let mut response = next.run(request).await;
    if let Some((token, seconds_left)) = renewal.take() {
        let prefix = format!("{SESSION_COOKIE}=");
        let sets_it = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .any(|value| value.as_bytes().starts_with(prefix.as_bytes()));
        if !sets_it {
            response.headers_mut().append(
                SET_COOKIE,
                super::auth::session_cookie_for(&state.settings, &token, seconds_left),
            );
        }
    }
    response
}

/// The session, where a request has a valid one, and `None` where it has
/// none or one that no longer works: for an endpoint open to anyone that
/// also says who asked when someone signed in did (`POST /v1/auth/codes`).
impl OptionalFromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, ApiError> {
        match <Self as FromRequestParts<AppState>>::from_request_parts(parts, state).await {
            Ok(session) => Ok(Some(session)),
            Err(error) if error.code == ErrorCode::Unauthenticated => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

pub fn require_web_origin(headers: &HeaderMap, web_origin: &str) -> Result<(), ApiError> {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if origin == Some(web_origin) {
        Ok(())
    } else {
        Err(ErrorCode::Unauthenticated.into())
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}
