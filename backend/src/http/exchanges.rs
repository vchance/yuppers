//! Exchange and invitation endpoints. The work is in `crate::exchanges`;
//! these read the request and name the responses.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use uuid::Uuid;

use super::extract::{ApiJson, DigestedJson, Session};
use super::{AppState, ClientAddress};
use crate::error::{ApiError, ErrorBody, ErrorCode};
use crate::exchanges::dto::{
    ClaimInvitation, CreateExchange, ExchangeSummary, ExchangeView, InvitationIssued,
    InvitationOptions, InvitationPreview, InvitationToken, RevisionSent, RunCommand, SaveDraft,
    SendRevision,
};
use crate::exchanges::service::{self, Idempotency, RequestOrigin};

/// An exchange the caller cannot see and one that does not exist look the same.
fn exchange_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.parse().map_err(|_| ErrorCode::NotFound.into())
}

/// The longest idempotency key accepted. A key is an opaque token the client
/// made up, such as a UUID; anything much longer is not one.
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 200;

pub(super) fn idempotency_key(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    match value.to_str() {
        Ok("") => Ok(None),
        Ok(key) if key.len() <= IDEMPOTENCY_KEY_MAX_BYTES => Ok(Some(key)),
        _ => Err(ErrorCode::InvalidRequest.into()),
    }
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers.get(USER_AGENT)?.to_str().ok()
}

/// Starts an exchange as a private draft.
#[utoipa::path(
    post,
    path = "/v1/exchanges",
    request_body = CreateExchange,
    responses(
        (status = 200, description = "The new draft", body = ExchangeView),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 422, description = "Unknown timezone", body = ErrorBody),
        (status = 429, description = "Too many exchanges created today", body = ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ApiJson(body): ApiJson<CreateExchange>,
) -> Result<Json<ExchangeView>, ApiError> {
    Ok(Json(
        service::create(&state.db, &state.settings.rules, &session, body).await?,
    ))
}

/// The caller's exchanges, most recently changed first.
#[utoipa::path(
    get,
    path = "/v1/exchanges",
    responses(
        (status = 200, description = "Exchanges the caller is a party to", body = [ExchangeSummary]),
        (status = 401, description = "Not signed in", body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<ExchangeSummary>>, ApiError> {
    Ok(Json(service::list(&state.db, &session).await?))
}

/// One exchange, as its party sees it.
#[utoipa::path(
    get,
    path = "/v1/exchanges/{id}",
    params(("id" = String, Path, description = "Exchange ID")),
    responses(
        (status = 200, description = "The exchange", body = ExchangeView),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such exchange for this account", body = ErrorBody)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<Json<ExchangeView>, ApiError> {
    Ok(Json(
        service::get(
            &state.db,
            &state.settings.rules,
            &session,
            exchange_id(&id)?,
        )
        .await?,
    ))
}

/// Saves the caller's unsent working copy.
#[utoipa::path(
    put,
    path = "/v1/exchanges/{id}/draft",
    params(("id" = String, Path, description = "Exchange ID")),
    request_body = SaveDraft,
    responses(
        (status = 204, description = "Saved"),
        (status = 404, description = "No such exchange for this account", body = ErrorBody),
        (status = 409, description = "The exchange is closed", body = ErrorBody)
    )
)]
pub async fn save_draft(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SaveDraft>,
) -> Result<StatusCode, ApiError> {
    service::save_draft(
        &state.db,
        &state.settings.rules,
        &session,
        exchange_id(&id)?,
        body.body,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a revision, which signs it: a first proposal, a counteroffer, or an
/// amendment to an agreement in force. Send an `Idempotency-Key` header so a
/// retry cannot send it twice.
#[utoipa::path(
    post,
    path = "/v1/exchanges/{id}/revisions",
    params(
        ("id" = String, Path, description = "Exchange ID"),
        ("Idempotency-Key" = Option<String>, Header, description = "Unique per attempt to make this change")
    ),
    request_body = SendRevision,
    responses(
        (status = 200, description = "Sent", body = RevisionSent),
        (status = 404, description = "No such exchange for this account", body = ErrorBody),
        (status = 409, description = "Refused by the rules, or the exchange changed", body = ErrorBody),
        (status = 422, description = "The revision is not valid", body = ErrorBody)
    )
)]
pub async fn send_revision(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    headers: HeaderMap,
    ClientAddress(address): ClientAddress,
    DigestedJson { body, digest }: DigestedJson<SendRevision>,
) -> Result<Json<RevisionSent>, ApiError> {
    let idempotency = Idempotency {
        key: idempotency_key(&headers)?,
        digest,
    };
    let origin = RequestOrigin {
        address,
        user_agent: user_agent(&headers),
    };
    let sent = service::send_revision(
        &state.db,
        &state.settings,
        &session,
        exchange_id(&id)?,
        idempotency,
        origin,
        body,
    )
    .await?;
    Ok(Json(sent))
}

/// Does one thing to an exchange: accept, decline or withdraw a revision,
/// act on a contribution, confirm or reject whoever claimed the invitation,
/// discard a draft, or end or close it.
#[utoipa::path(
    post,
    path = "/v1/exchanges/{id}/commands",
    params(
        ("id" = String, Path, description = "Exchange ID"),
        ("Idempotency-Key" = Option<String>, Header, description = "Unique per attempt to make this change")
    ),
    request_body = RunCommand,
    responses(
        (status = 200, description = "Done; the exchange as it now stands", body = ExchangeView),
        (status = 403, description = "That is for the other party to do", body = ErrorBody),
        (status = 404, description = "No such exchange for this account", body = ErrorBody),
        (status = 409, description = "Refused by the rules, or the exchange changed", body = ErrorBody)
    )
)]
pub async fn run_command(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    headers: HeaderMap,
    ClientAddress(address): ClientAddress,
    DigestedJson { body, digest }: DigestedJson<RunCommand>,
) -> Result<Json<ExchangeView>, ApiError> {
    let idempotency = Idempotency {
        key: idempotency_key(&headers)?,
        digest,
    };
    let origin = RequestOrigin {
        address,
        user_agent: user_agent(&headers),
    };
    let view = service::run_command(
        &state.db,
        &state.settings,
        &session,
        exchange_id(&id)?,
        idempotency,
        origin,
        body,
    )
    .await?;
    Ok(Json(view))
}

/// Gives up the invited party's place. For someone who opened an invitation
/// link and has not been confirmed by the initiator: it is their way out,
/// since they cannot decline. Anything they signed is void, and from then on
/// the exchange does not exist for them.
#[utoipa::path(
    post,
    path = "/v1/exchanges/{id}/leave",
    params(("id" = String, Path, description = "Exchange ID")),
    responses(
        (status = 204, description = "Left"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 403, description = "The initiator cannot leave their own exchange", body = ErrorBody),
        (status = 404, description = "No such exchange for this account", body = ErrorBody),
        (status = 409, description = "The caller has been confirmed, and is a party for good", body = ErrorBody)
    )
)]
pub async fn leave(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::leave(&state.db, &state.settings, &session, exchange_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replaces the invitation link. The previous link stops working.
#[utoipa::path(
    post,
    path = "/v1/exchanges/{id}/invitation",
    params(("id" = String, Path, description = "Exchange ID")),
    request_body = InvitationOptions,
    responses(
        (status = 200, description = "The new link token, shown once", body = InvitationIssued),
        (status = 422, description = "Neither or both of `bound_to` and `for_anyone: true`", body = ErrorBody),
        (status = 403, description = "Only the initiator can invite", body = ErrorBody),
        (status = 409, description = "Someone is in the invited party's place", body = ErrorBody)
    )
)]
pub async fn reissue_invitation(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    ApiJson(options): ApiJson<InvitationOptions>,
) -> Result<Json<InvitationIssued>, ApiError> {
    let issued = service::reissue_invitation(
        &state.db,
        &state.settings.rules,
        &session,
        exchange_id(&id)?,
        Some(options),
    )
    .await?;
    Ok(Json(issued))
}

/// Records that the initiator opened a way to pass the invitation link on:
/// the share sheet, a text message, an email, WhatsApp, a copy or its QR
/// code. Yuppers never sends the link itself, so this is all the service can
/// know about whether it went anywhere, and it says only that the sender
/// opened a way to send it, not that it arrived. Shown back as
/// `invitation_shared_at` on the exchange and in the list. Harmless to
/// repeat: the latest time is kept. A claim counts as a share when none was
/// recorded, so a claimed link always has a time.
#[utoipa::path(
    post,
    path = "/v1/exchanges/{id}/invitation/shared",
    params(("id" = String, Path, description = "Exchange ID")),
    responses(
        (status = 204, description = "Recorded"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 403, description = "Only the initiator has a link to send", body = ErrorBody),
        (status = 404, description = "No such exchange for this account", body = ErrorBody),
        (status = 409, description = "Someone is in the invited party's place, or the exchange is not open", body = ErrorBody)
    )
)]
pub async fn invitation_shared(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service::invitation_shared(&state.db, &session, exchange_id(&id)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Shows the proposal behind an invitation link, to someone signed in. The
/// token travels in the body so it never appears in a URL the service logs.
///
/// Without a session the answer is `UNAUTHENTICATED` whatever the token, so
/// nobody signed out learns anything about any link (DESIGN.md §9): the only
/// answers about a link come from an account, and an account that is blocked
/// gets the dead-link answer.
#[utoipa::path(
    post,
    path = "/v1/invitations/preview",
    request_body = InvitationToken,
    responses(
        (status = 200, description = "The proposal", body = InvitationPreview),
        (status = 401, description = "Not signed in, whatever the token", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer", body = ErrorBody)
    )
)]
pub async fn preview_invitation(
    State(state): State<AppState>,
    session: Session,
    ApiJson(body): ApiJson<InvitationToken>,
) -> Result<Json<InvitationPreview>, ApiError> {
    Ok(Json(
        service::preview_invitation(&state.db, session.account_id, &body.token).await?,
    ))
}

/// Takes the invited party's place in the exchange. With `only_if_yours`,
/// only opens it for the account that already took that place.
#[utoipa::path(
    post,
    path = "/v1/invitations/claim",
    request_body = ClaimInvitation,
    responses(
        (status = 200, description = "Claimed; the exchange as the new party sees it", body = ExchangeView),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 403, description = "The invitation names someone else", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer", body = ErrorBody)
    )
)]
pub async fn claim_invitation(
    State(state): State<AppState>,
    session: Session,
    ApiJson(body): ApiJson<ClaimInvitation>,
) -> Result<Json<ExchangeView>, ApiError> {
    let claim = if body.only_if_yours {
        service::Claim::OnlyIfYours
    } else {
        service::Claim::Take
    };
    let view = service::claim_invitation(
        &state.db,
        &state.settings.rules,
        &session,
        &body.token,
        claim,
    )
    .await?;
    Ok(Json(view))
}
