//! Signing in with a one-time code, and signing out.

use axum::Json;
use axum::extract::State;
use axum::http::header::{ACCEPT_LANGUAGE, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::account::{self, Account};
use super::extract::{ApiJson, SESSION_COOKIE, Session, require_web_origin};
use super::{AppState, ClientAddress, Settings};
use crate::auth::{self, Requester};
use crate::client_version;
use crate::code_consent::{CodePurpose, CodeRequest, SmsCodeConsent};
use crate::contact::{self, Kind};
use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorBody, ErrorCode};
use crate::languages;
use crate::notifications::sms_updates::Source;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RequestCode {
    /// An email address, or a phone number: in international form, or a US
    /// number without its country code as people there write it, ten digits
    /// or 1 and ten (`(856) 548-8780`), which is taken as `+1`.
    pub identifier: String,
    /// For a phone number, required: the box beside it was ticked, with the
    /// version and language of the wording shown. Ignored for an email
    /// address.
    pub sms_consent: Option<SmsCodeConsent>,
}

/// The client a request came from, as `X-Client-Version` names it, and its
/// user agent: what a consent is recorded with.
pub(super) fn client_of(headers: &HeaderMap) -> (Source, Option<&str>) {
    let client = headers
        .get(client_version::HEADER)
        .and_then(|value| value.to_str().ok());
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok());
    (Source::of_client(client), user_agent)
}

/// Sends a one-time code to an email address or phone number. Answers the
/// same way whether or not an account exists for it. Codes sent earlier keep
/// working until they expire, up to the newest few. A code for a phone
/// number needs `sms_consent`, which is recorded with it: for signing in,
/// or, from a signed-in account, for checking a number it is adding.
#[utoipa::path(
    post,
    path = "/v1/auth/codes",
    request_body = RequestCode,
    responses(
        (status = 204, description = "A code was sent"),
        (status = 409, description = "The phone number replied STOP (`PHONE_OPTED_OUT`)", body = ErrorBody),
        (status = 422, description = "Not an email address or phone number (`INVALID_IDENTIFIER`), a phone number of a country the service does not take (`PHONE_COUNTRY_NOT_SERVED`), a phone number without `sms_consent`, or with wording that is not the current one (`SMS_CONSENT_REQUIRED`), or a consent in a language not supported (`INVALID_REQUEST`)", body = ErrorBody),
        (status = 429, description = "Too many codes requested for this identifier or from this address (`TOO_MANY_REQUESTS`), or too many wrong codes for this identifier today, so none is sent (`TOO_MANY_GUESSES`)", body = ErrorBody)
    )
)]
pub async fn request_code(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    session: Option<Session>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RequestCode>,
) -> Result<StatusCode, ApiError> {
    let identifier = Identifier::parse(&body.identifier)?;
    let (source, user_agent) = client_of(&headers);
    // Signed in, the only code asked for is one checking a number being
    // added to the account (for agreement updates); signed out, signing in.
    let (purpose, account) = match &session {
        Some(session) => (CodePurpose::VerifyNumber, Some(session.account_id)),
        None => (CodePurpose::SignIn, None),
    };
    let request = CodeRequest {
        purpose,
        account,
        consent: body.sms_consent.as_ref(),
        source,
        address,
        user_agent,
    };
    let settings = &state.settings;
    // There may be no account yet, so the browser's or device's language
    // stands in for a preference.
    let accept_language = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok());
    let language = auth::language_for(&state.db, &identifier, accept_language).await;
    auth::request_code(
        &state.db,
        &settings.app_secret,
        &settings.auth,
        state.code_sender.as_ref(),
        &identifier,
        Requester::SignIn { address },
        &language,
        &request,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// How the client wants to hold the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Delivery {
    /// Web: an HTTP-only cookie the page's scripts cannot read.
    Cookie,
    /// Mobile: a token in the response, kept in the device's secure storage
    /// and sent as `Authorization: Bearer`.
    Token,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSession {
    pub identifier: String,
    /// The one-time code sent to the identifier.
    pub code: String,
    pub delivery: Delivery,
    /// The language the client is showing, as a tag such as `es-MX`. Used only
    /// when this creates the account; an unsupported one becomes the default.
    pub language: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionCreated {
    pub account: Account,
    /// Present for `TOKEN` delivery only.
    pub token: Option<String>,
}

/// Signs in with a one-time code, creating the account on first use.
#[utoipa::path(
    post,
    path = "/v1/auth/sessions",
    request_body = CreateSession,
    responses(
        (status = 200, description = "Signed in", body = SessionCreated),
        (status = 401, description = "The code is wrong, expired or used up", body = ErrorBody),
        (status = 403, description = "The account is suspended", body = ErrorBody),
        (status = 422, description = "Invalid request", body = ErrorBody),
        (status = 429, description = "Too many wrong codes for this identifier today (`TOO_MANY_GUESSES`), or a wrong code from an address that has offered too many this hour (`TOO_MANY_REQUESTS`)", body = ErrorBody),
        (status = 503, description = "A code sent by text could not be checked, because the provider that made it did not answer; nothing was counted", body = ErrorBody)
    )
)]
pub async fn create_session(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateSession>,
) -> Result<Response, ApiError> {
    let settings = &state.settings;
    if body.delivery == Delivery::Cookie {
        // Another site must not be able to sign a browser in to an account
        // of its choosing.
        require_web_origin(&headers, &settings.web_origin)?;
    }

    let identifier = Identifier::parse(&body.identifier)?;
    auth::verify_code(
        &state.db,
        &settings.app_secret,
        &settings.auth,
        state.code_sender.as_ref(),
        &identifier,
        &body.code,
        Requester::SignIn { address },
    )
    .await?;

    let mut tx = state.db.begin().await?;

    let method = match identifier {
        Identifier::Email(_) => "EMAIL_OTP",
        Identifier::Phone(_) => "PHONE_OTP",
    };
    // Found by its blind index, and stored encrypted (`crate::contact`).
    let (encrypted, index) = Kind::of(&identifier).account_columns();
    let sealed = contact::keys().sealed(&identifier);
    let existing: Option<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id, status FROM account WHERE {index} = $1"
    )))
    .bind(sealed.index.as_slice())
    .fetch_optional(&mut *tx)
    .await?;

    let account_id = match existing {
        Some((_, status)) if status != "ACTIVE" => return Err(ErrorCode::AccountSuspended.into()),
        Some((id, _)) => id,
        None => {
            let language = body
                .language
                .as_deref()
                .and_then(languages::resolve)
                .unwrap_or(languages::default());
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "INSERT INTO account ({encrypted}, {index}, display_name, language)
                 VALUES ($1, $2, '', $3)
                 RETURNING id"
            )))
            .bind(&sealed.encrypted)
            .bind(sealed.index.as_slice())
            .bind(language)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    let token = auth::generate_token();
    sqlx::query(
        "INSERT INTO account_session
            (account_id, token_hash, auth_method, authenticated_at, expires_at)
         VALUES ($1, $2, $3, now(), now() + $4 * interval '1 second')",
    )
    .bind(account_id)
    .bind(auth::token_hash(&token).as_slice())
    .bind(method)
    .bind(settings.auth.session_initial().whole_seconds() as f64)
    .execute(&mut *tx)
    .await?;

    let account = account::load(&mut *tx, account_id).await?;
    tx.commit().await?;

    Ok(match body.delivery {
        Delivery::Token => Json(SessionCreated {
            account,
            token: Some(token),
        })
        .into_response(),
        Delivery::Cookie => (
            [(SET_COOKIE, session_cookie(settings, &token))],
            Json(SessionCreated {
                account,
                token: None,
            }),
        )
            .into_response(),
    })
}

/// Signs out: the session stops working everywhere it was held.
#[utoipa::path(
    delete,
    path = "/v1/auth/session",
    responses(
        (status = 204, description = "Signed out"),
        (status = 401, description = "Not signed in", body = ErrorBody)
    )
)]
pub async fn delete_session(
    State(state): State<AppState>,
    session: Session,
) -> Result<Response, ApiError> {
    // The device signed in with this session stops getting notifications
    // for the account (`devices`).
    sqlx::query("DELETE FROM device WHERE session_id = $1")
        .bind(session.id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE account_session SET revoked_at = now() WHERE id = $1")
        .bind(session.id)
        .execute(&state.db)
        .await?;

    Ok((
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, expired_cookie(&state.settings))],
    )
        .into_response())
}

/// The cookie for a new session: it lasts as long as the session does
/// before its first renewal.
fn session_cookie(settings: &Settings, token: &str) -> HeaderValue {
    session_cookie_for(
        settings,
        token,
        settings.auth.session_initial().whole_seconds(),
    )
}

/// The cookie for a session with `seconds_left` to run, sent again each time
/// the session is renewed (`extract::refresh_session_cookie`), so that the
/// browser keeps it exactly as long as the service will take it.
pub(super) fn session_cookie_for(
    settings: &Settings,
    token: &str,
    seconds_left: i64,
) -> HeaderValue {
    cookie(settings, token, seconds_left.max(0))
}

pub(super) fn expired_cookie(settings: &Settings) -> HeaderValue {
    cookie(settings, "", 0)
}

fn cookie(settings: &Settings, value: &str, max_age: i64) -> HeaderValue {
    // `Secure` whenever the web app is served over HTTPS; plain HTTP is for
    // local development only.
    let secure = if settings.web_origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure}"
    ))
    .expect("a hex token and fixed attributes are a valid header value")
}
