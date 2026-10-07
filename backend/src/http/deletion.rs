//! Deleting the signed-in account (DESIGN.md §4.1). The work is in
//! `crate::deletion`; these read the request and name the responses.
//!
//! Deleting cannot be undone, so a session is not enough to do it. The
//! person asks for a one-time code, which goes to an address the account
//! already has, and sends it back with the request to delete.

use axum::Json;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;

use super::auth::{client_of, expired_cookie};
use super::extract::{ApiJson, Session};
use super::{AppState, ClientAddress};
use crate::auth::{self, OfferedCode, Requester};
use crate::code_consent::{CodePurpose, CodeRequest, SmsCodeConsent};
use crate::deletion::{self, CodeChannel, DeletionPreview};
use crate::error::{ApiError, ErrorBody};

/// What deleting the account would do to the exchanges it is in. Changes nothing.
#[utoipa::path(
    get,
    path = "/v1/me/deletion",
    responses(
        (status = 200, description = "What would happen", body = DeletionPreview),
        (status = 401, description = "Not signed in", body = ErrorBody)
    )
)]
pub async fn deletion_preview(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<DeletionPreview>, ApiError> {
    Ok(Json(
        deletion::preview(&state.db, session.account_id).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RequestDeletionCode {
    /// Which of the account's own identifiers to send the code to.
    pub channel: CodeChannel,
    /// For `PHONE`, required: the box beside the number was ticked, with the
    /// version and language of the wording shown. Ignored for `EMAIL`.
    pub sms_consent: Option<SmsCodeConsent>,
}

/// Sends a one-time code for deleting the account to its own email address
/// or phone number. The code is good for that and nothing else. Requests are
/// counted against the account, apart from sign-in codes, so nobody asking
/// for sign-in codes for the same address can use them up. A code by text
/// needs `sms_consent`, which is recorded with it.
#[utoipa::path(
    post,
    path = "/v1/me/deletion/codes",
    request_body = RequestDeletionCode,
    responses(
        (status = 204, description = "A code was sent"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 409, description = "The phone number replied STOP (`PHONE_OPTED_OUT`)", body = ErrorBody),
        (status = 422, description = "The account has no such identifier, or a consent in a language not supported (`INVALID_REQUEST`); its phone number is of a country the service does not take (`PHONE_COUNTRY_NOT_SERVED`); or `PHONE` without `sms_consent`, or with wording that is not the current one (`SMS_CONSENT_REQUIRED`)", body = ErrorBody),
        (status = 429, description = "Too many deletion codes requested by this account", body = ErrorBody)
    )
)]
pub async fn request_deletion_code(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RequestDeletionCode>,
) -> Result<StatusCode, ApiError> {
    let identifier = deletion::identifier(&state.db, session.account_id, body.channel).await?;
    let (source, user_agent) = client_of(&headers);
    let request = CodeRequest {
        purpose: CodePurpose::DeleteAccount,
        account: Some(session.account_id),
        consent: body.sms_consent.as_ref(),
        source,
        address,
        user_agent,
    };
    let settings = &state.settings;
    // The identifier is the account's own, so this finds its language.
    let language = auth::language_for(&state.db, &identifier, None).await;
    auth::request_code(
        &state.db,
        &settings.app_secret,
        &settings.auth,
        state.code_sender.as_ref(),
        &identifier,
        Requester::DeleteAccount {
            account: session.account_id,
        },
        &language,
        &request,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAccount {
    /// Where the code was sent.
    pub channel: CodeChannel,
    /// The one-time code sent for deleting the account.
    pub code: String,
}

/// Deletes the signed-in account. Every session ends, its email address and
/// phone number are released, its working data is removed, and each exchange
/// it is still in is ended or put on the way to closing. Agreements the
/// other party signed stay in that party's record. It cannot be undone.
#[utoipa::path(
    post,
    path = "/v1/me/deletion",
    request_body = DeleteAccount,
    responses(
        (status = 204, description = "The account is deleted"),
        (status = 401, description = "Not signed in, or the code is wrong, expired, used up or was sent for something else", body = ErrorBody),
        (status = 422, description = "The account has no such identifier", body = ErrorBody),
        (status = 429, description = "Too many wrong deletion codes from this account today", body = ErrorBody),
        (status = 503, description = "The account was busy and nothing was done, or a code sent by text could not be checked because the provider that made it did not answer; the code still works", body = ErrorBody)
    )
)]
pub async fn delete_account(
    State(state): State<AppState>,
    session: Session,
    ApiJson(body): ApiJson<DeleteAccount>,
) -> Result<Response, ApiError> {
    let settings = &state.settings;
    let identifier = deletion::identifier(&state.db, session.account_id, body.channel).await?;
    // Checked and used up with the deletion itself: if the account is busy
    // and the deletion gives up, the code still works for another try.
    let code = OfferedCode {
        secret: &settings.app_secret,
        rules: &settings.auth,
        identifier: &identifier,
        code: &body.code,
        requester: Requester::DeleteAccount {
            account: session.account_id,
        },
        verifier: state.code_sender.verifier(&identifier),
    };
    deletion::delete_account_with_code(&state.db, &settings.rules, session.account_id, &code)
        .await?;

    // A browser holds the session in a cookie only the service can remove.
    Ok((
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, expired_cookie(settings))],
    )
        .into_response())
}
