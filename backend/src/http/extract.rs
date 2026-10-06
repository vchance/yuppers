//! Request extractors that answer with typed error codes.

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequestParts, Request};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use super::AppState;
use crate::auth::token_hash;
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

        let row: Option<(Uuid, Uuid, String, OffsetDateTime)> = sqlx::query_as(
            "SELECT s.id, s.account_id, s.auth_method, s.authenticated_at
             FROM account_session s
             JOIN account a ON a.id = s.account_id
             WHERE s.token_hash = $1
               AND s.revoked_at IS NULL
               AND s.expires_at > now()
               AND a.status = 'ACTIVE'",
        )
        .bind(token_hash(token).as_slice())
        .fetch_optional(&state.db)
        .await?;

        let (id, account_id, auth_method, authenticated_at) =
            row.ok_or(ErrorCode::Unauthenticated)?;
        Ok(Self {
            id,
            account_id,
            auth_method,
            authenticated_at,
        })
    }
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
