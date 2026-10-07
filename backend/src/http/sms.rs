//! Text updates for an agreement, and the texts people send back
//! (`crate::notifications::sms_updates` says what they are).
//!
//! * `GET` and `PUT /v1/exchanges/{id}/sms-updates`: where the signed-in
//!   party stands on text updates for one agreement, and turning them on
//!   (with the consent wording's version and language) or off.
//! * `POST /v1/sms/inbound`: Twilio's webhook for texts sent to our number.
//!   Only requests bearing Twilio's signature are taken; a STOP or START is
//!   recorded and anything else ignored, and the answer is an empty TwiML
//!   document, so the service itself never replies: Twilio's Advanced
//!   Opt-Out answers HELP, STOP and START (docs/deploy-render.md).

use std::sync::atomic::AtomicU64;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::header::{CONTENT_TYPE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::extract::{ApiJson, Session};
use super::{AppState, ClientAddress, client_address};
use crate::client_version;
use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorBody, ErrorCode};
use crate::languages;
use crate::notifications::sms::twilio_signature_valid;
use crate::notifications::sms_updates::{self, Consent, Keyword, Source};

/// Where the signed-in party stands on text updates for one agreement.
#[derive(Debug, Serialize, ToSchema)]
pub struct SmsUpdates {
    /// Whether updates are on for this agreement, to the account's phone
    /// number as it is now.
    pub on: bool,
    /// Whether they can be turned on here: text messages are sent, and the
    /// agreement has been sent and is not closed. Turning them off is always
    /// possible.
    pub available: bool,
    /// The account's phone number, in international form, or null when it
    /// has none; one must be added (`POST /v1/me/identifiers`) first.
    pub phone: Option<String>,
    /// The number replied STOP to our texts, so nothing is texted to it
    /// until it replies START.
    pub opted_out: bool,
    /// The version of the consent wording a client must show beside the
    /// box, and name when it turns updates on.
    pub consent_version: String,
}

/// What a party ticked, when turning updates on.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SmsConsent {
    /// The version of the consent wording shown, `consent_version`.
    pub version: String,
    /// The language it was shown in, as a language tag.
    pub language: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSmsUpdates {
    /// `true` to turn updates on, `false` to turn them off.
    pub on: bool,
    /// Required to turn them on: the wording shown beside the box.
    pub consent: Option<SmsConsent>,
}

/// The agreement's state, if the account is one of its parties.
async fn party_state(state: &AppState, account: Uuid, exchange: Uuid) -> Result<String, ApiError> {
    let found: Option<String> = sqlx::query_scalar(
        "SELECT e.state FROM exchange e
         JOIN participant p ON p.exchange_id = e.id AND p.account_id = $2
         WHERE e.id = $1",
    )
    .bind(exchange)
    .bind(account)
    .fetch_optional(&state.db)
    .await?;
    // One the caller is not a party to looks like one that does not exist.
    found.ok_or_else(|| ErrorCode::NotFound.into())
}

fn available(state: &AppState, exchange_state: &str) -> bool {
    state.settings.sms_updates && matches!(exchange_state, "NEGOTIATING" | "ACTIVE")
}

async fn view(
    state: &AppState,
    account: Uuid,
    exchange: Uuid,
    exchange_state: &str,
) -> Result<SmsUpdates, ApiError> {
    let mut conn = state.db.acquire().await?;
    let standing = sms_updates::standing(&mut conn, account, exchange).await?;
    Ok(SmsUpdates {
        on: standing.on,
        available: available(state, exchange_state),
        phone: standing.phone,
        opted_out: standing.opted_out,
        consent_version: sms_updates::CONSENT_VERSION.to_owned(),
    })
}

fn exchange_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.parse().map_err(|_| ErrorCode::NotFound.into())
}

/// Where the signed-in party stands on text updates for this agreement.
#[utoipa::path(
    get,
    path = "/v1/exchanges/{id}/sms-updates",
    params(("id" = String, Path, description = "The exchange")),
    responses(
        (status = 200, description = "Where the caller stands", body = SmsUpdates),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such exchange, or the caller is not a party to it", body = ErrorBody)
    )
)]
pub async fn sms_updates(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<Json<SmsUpdates>, ApiError> {
    let exchange = exchange_id(&id)?;
    let exchange_state = party_state(&state, session.account_id, exchange).await?;
    Ok(Json(
        view(&state, session.account_id, exchange, &exchange_state).await?,
    ))
}

/// Turns text updates for this agreement on or off. Turning them on records
/// the consent (the account, the agreement, the number, the time, the
/// wording's version and language, the client, and the request's address
/// and user agent) and queues a confirmation text; turning them off is
/// recorded too. Repeating either changes nothing.
#[utoipa::path(
    put,
    path = "/v1/exchanges/{id}/sms-updates",
    params(("id" = String, Path, description = "The exchange")),
    request_body = SetSmsUpdates,
    responses(
        (status = 200, description = "Where the caller now stands", body = SmsUpdates),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such exchange, or the caller is not a party to it", body = ErrorBody),
        (status = 409, description = "The agreement is a draft or closed, or the account has no phone number (`ACTION_NOT_ALLOWED`); the wording shown is not the current one (`CONSENT_OUTDATED`); the number replied STOP (`PHONE_OPTED_OUT`)", body = ErrorBody),
        (status = 422, description = "No consent given, or a language not supported (`INVALID_REQUEST`); a number of a country not texted (`PHONE_COUNTRY_NOT_SERVED`)", body = ErrorBody),
        (status = 503, description = "Text messages are not sent here", body = ErrorBody)
    )
)]
pub async fn set_sms_updates(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SetSmsUpdates>,
) -> Result<Json<SmsUpdates>, ApiError> {
    let exchange = exchange_id(&id)?;
    let exchange_state = party_state(&state, session.account_id, exchange).await?;
    let source = Source::of_client(
        headers
            .get(client_version::HEADER)
            .and_then(|value| value.to_str().ok()),
    );

    if !body.on {
        let mut conn = state.db.acquire().await?;
        sms_updates::turn_off(&mut conn, session.account_id, exchange, source).await?;
        drop(conn);
        return Ok(Json(
            view(&state, session.account_id, exchange, &exchange_state).await?,
        ));
    }

    if !state.settings.sms_updates {
        return Err(ErrorCode::ServiceUnavailable.into());
    }
    if !available(&state, &exchange_state) {
        return Err(ErrorCode::ActionNotAllowed.into());
    }
    let consent = body.consent.ok_or(ErrorCode::InvalidRequest)?;
    if consent.version != sms_updates::CONSENT_VERSION {
        return Err(ErrorCode::ConsentOutdated.into());
    }
    let language = languages::resolve(&consent.language).ok_or(ErrorCode::InvalidRequest)?;

    let mut tx = state.db.begin().await?;
    // The number as it is now, held while the updates are turned on, so a
    // change of number at the same moment cannot slip between.
    let phone: Option<String> =
        sqlx::query_scalar("SELECT phone FROM account WHERE id = $1 FOR NO KEY UPDATE")
            .bind(session.account_id)
            .fetch_one(&mut *tx)
            .await?;
    let phone = phone.ok_or(ErrorCode::ActionNotAllowed)?;
    let identifier = Identifier::parse(&phone).map_err(|_| ErrorCode::ActionNotAllowed)?;
    state.settings.auth.check_taken(&identifier)?;
    let opted_out: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sms_opt_out WHERE phone = $1)")
            .bind(&phone)
            .fetch_one(&mut *tx)
            .await?;
    if opted_out {
        return Err(ErrorCode::PhoneOptedOut.into());
    }
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let consent = Consent {
        version: &consent.version,
        language,
        source,
        address,
        user_agent,
    };
    sms_updates::turn_on(&mut tx, session.account_id, exchange, &phone, &consent).await?;
    tx.commit().await?;
    Ok(Json(
        view(&state, session.account_id, exchange, &exchange_state).await?,
    ))
}

/// The answer to every request Twilio's signature checks out on: an empty
/// TwiML document, which sends no reply.
const EMPTY_TWIML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response></Response>";

/// The header Twilio signs its requests in.
const SIGNATURE: &str = "x-twilio-signature";

/// The largest request the webhook reads. Twilio's parameters for a text
/// received, a long one with its media links included, take a few
/// kilobytes; anything much larger is not Twilio's, and is refused with 413
/// before it is read in full (`v1::router`).
pub const INBOUND_BODY_LIMIT: usize = 32 * 1024;

/// The most parameters a request to the webhook may carry. Twilio sends a
/// few dozen at most, with ten media items; more is refused unread.
const INBOUND_MAX_PARAMS: usize = 100;

/// How long Twilio's signature is: an HMAC-SHA1.
const SIGNATURE_BYTES: usize = 20;

/// When a refused signature was last logged (`client_address::time_to_warn`):
/// once a minute is enough to be noticed, and once per request would let
/// anyone fill the logs.
static LAST_REFUSAL_LOGGED: AtomicU64 = AtomicU64::new(0);

/// Whether a header could be a signature at all: base64 for exactly
/// [`SIGNATURE_BYTES`]. Checked before the body is parsed.
fn signature_shaped(header: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(header.trim())
        .is_ok_and(|bytes| bytes.len() == SIGNATURE_BYTES)
}

/// Twilio's webhook for a text sent to our number. Refused with 403, and
/// nothing read, unless `X-Twilio-Signature` is Twilio's signature, under
/// the account's auth token, of this URL as Twilio requested it (the web
/// origin and this path) and the posted parameters; refused with 413 when
/// the body is larger than [`INBOUND_BODY_LIMIT`] or carries more than a
/// hundred parameters. A stop keyword puts the number on the opt-out list
/// and turns off every agreement's updates to it; a start keyword takes it
/// off the list. A message already taken (by its `MessageSid`) changes
/// nothing again, so a request posted twice, or replayed, is answered the
/// same but does nothing. Answered at once, with an empty TwiML document:
/// Twilio's Advanced Opt-Out sends the replies.
#[utoipa::path(
    post,
    path = "/v1/sms/inbound",
    request_body(content = String, content_type = "application/x-www-form-urlencoded", description = "Twilio's parameters for a message received: From, Body, OptOutType and the rest"),
    responses(
        (status = 200, description = "Taken: an empty TwiML document", content_type = "text/xml", body = String),
        (status = 403, description = "Not signed by Twilio, or no auth token to check it with"),
        (status = 413, description = "Too large, or too many parameters, to be Twilio's")
    )
)]
pub async fn inbound(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let forbidden = || StatusCode::FORBIDDEN.into_response();
    let Some(token) = &state.settings.sms_webhook_token else {
        return forbidden();
    };
    let Some(signature) = headers
        .get(SIGNATURE)
        .and_then(|value| value.to_str().ok())
        .filter(|signature| signature_shaped(signature))
    else {
        return forbidden();
    };
    // Counted before anything is parsed, so that a request of many small
    // parameters costs no more than its bytes.
    if body.split(|byte| *byte == b'&').count() > INBOUND_MAX_PARAMS {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let params: Vec<(String, String)> = form_urlencoded::parse(&body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    // The address Twilio was given: the public origin, since a proxy in
    // front may have changed the host and scheme this request arrived with.
    let url = match uri.query() {
        Some(query) => format!("{}{}?{query}", state.settings.web_origin, uri.path()),
        None => format!("{}{}", state.settings.web_origin, uri.path()),
    };
    if !twilio_signature_valid(token.expose(), &url, &params, signature) {
        if client_address::time_to_warn(&LAST_REFUSAL_LOGGED, client_address::now_seconds()) {
            tracing::warn!(
                "a request to the SMS webhook was refused: its signature is not Twilio's \
                 (logged at most once a minute)"
            );
        }
        return forbidden();
    }

    let param = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let from = param("From").and_then(|from| Identifier::parse(from).ok());
    let body = param("Body").unwrap_or("");
    let keyword = param("OptOutType")
        .and_then(Keyword::of_opt_out_type)
        .or_else(|| Keyword::of_message(body));
    // HELP is Twilio's to answer, and changes nothing here.
    if let (Some(Identifier::Phone(phone)), Some(keyword @ (Keyword::Stop | Keyword::Start))) =
        (from, keyword)
    {
        // The word itself, as received, for the record; OptOutType when the
        // message was a word Twilio's settings added.
        let word = Keyword::of_message(body)
            .map(|_| body)
            .or(param("OptOutType"))
            .unwrap_or(body);
        let done = async {
            let mut tx = state.db.begin().await?;
            // Twilio sends every message with its SID; one already taken is
            // a retry or a replay, and changes nothing.
            if let Some(sid) = param("MessageSid")
                && !sms_updates::first_receipt(&mut tx, sid).await?
            {
                tracing::info!("a text already taken was posted again: nothing changed");
                return Ok(());
            }
            match keyword {
                Keyword::Stop => {
                    let turned_off = sms_updates::stop(&mut tx, &phone, word).await?;
                    tracing::info!(turned_off, "STOP received: the number gets no more texts");
                }
                _ => {
                    sms_updates::start(&mut tx, &phone, word).await?;
                    tracing::info!("START received: the number may be texted again");
                }
            }
            tx.commit().await
        };
        if let Err(error) = done.await {
            return ApiError::from(error).into_response();
        }
    }
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/xml; charset=utf-8"),
        )],
        EMPTY_TWIML,
    )
        .into_response()
}
