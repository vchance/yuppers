//! Payment options (`crate::payments`): the signed-in account's own, and
//! whether it shows them on an agreement.
//!
//! * `GET`, `PUT` and `DELETE /v1/me/payment-handles`: reading, saving
//!   (each optional; one left out is removed) and removing them all.
//! * `PUT` and `DELETE /v1/me/payment-handles/{kind}`: saving or removing
//!   one, leaving the others as they are. What both apps use: each option
//!   is added, changed and removed on its own, so two devices changing
//!   different ones never undo each other, and a refusal leaves the rest
//!   untouched. The whole-set `PUT` stays for builds that still send it.
//! * `PUT /v1/exchanges/{id}/payment-options`: showing them to the other
//!   party of one agreement, or not. Off unless turned on.
//!
//! The other party sees them only in the agreement's view
//! (`ExchangeView::payment_options`), and only while they owe money still
//! to be paid. Nothing here is logged: the request bodies hold the names.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiJson, Session};
use crate::error::{ApiError, ErrorBody, ErrorCode};
use crate::payments::{self, PaymentApp, PaymentHandles};

/// One payment option to save, as typed: a Venmo username with or without
/// its `@`, a $Cashtag with or without its `$`, a PayPal.Me name or its
/// link, or for Zelle an email address or a US phone number.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentHandleValue {
    pub value: String,
}

/// The app a path names; any other is not found.
fn path_app(kind: &str) -> Result<PaymentApp, ApiError> {
    PaymentApp::from_path(kind).ok_or_else(|| ErrorCode::NotFound.into())
}

/// Whether the signed-in party shows their payment options on an agreement.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PaymentOptionsShown {
    pub on: bool,
}

/// The signed-in account's payment options.
#[utoipa::path(
    get,
    path = "/v1/me/payment-handles",
    responses(
        (status = 200, description = "What is saved; each absent one is null", body = PaymentHandles),
        (status = 401, description = "Not signed in", body = ErrorBody)
    )
)]
pub async fn payment_handles(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<PaymentHandles>, ApiError> {
    let mut conn = state.db.acquire().await?;
    Ok(Json(payments::load(&mut conn, session.account_id).await?))
}

/// Saves the account's payment options, replacing what was saved: one left
/// out or empty is removed. With none left, they stop being shown on every
/// agreement.
#[utoipa::path(
    put,
    path = "/v1/me/payment-handles",
    request_body = PaymentHandles,
    responses(
        (status = 200, description = "What is now saved, as stored", body = PaymentHandles),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 422, description = "One of them is not a username, $Cashtag, PayPal.Me name, or email address or US phone number for Zelle", body = ErrorBody),
        (status = 429, description = "Too many changes to payment options this hour", body = ErrorBody)
    )
)]
pub async fn set_payment_handles(
    State(state): State<AppState>,
    session: Session,
    ApiJson(body): ApiJson<PaymentHandles>,
) -> Result<Json<PaymentHandles>, ApiError> {
    // Checked before anything is counted or written.
    body.normalized()?;
    let mut tx = state.db.begin().await?;
    let saved = payments::save(&mut tx, session.account_id, &body).await;
    // A refusal for too many changes is counted all the same.
    tx.commit().await?;
    Ok(Json(saved?))
}

/// Removes all of the account's payment options, and stops showing them on
/// every agreement.
#[utoipa::path(
    delete,
    path = "/v1/me/payment-handles",
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 429, description = "Too many changes to payment options this hour", body = ErrorBody)
    )
)]
pub async fn remove_payment_handles(
    State(state): State<AppState>,
    session: Session,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    let removed = payments::remove(&mut tx, session.account_id).await;
    tx.commit().await?;
    removed?;
    Ok(StatusCode::NO_CONTENT)
}

/// Saves one of the account's payment options, adding it or changing it,
/// and leaves the others as they are. It is marked changed only if its
/// value is new, so the payer's warning beside it stays true.
#[utoipa::path(
    put,
    path = "/v1/me/payment-handles/{kind}",
    params(("kind" = String, Path, description = "`venmo`, `cash_app`, `paypal` or `zelle`")),
    request_body = PaymentHandleValue,
    responses(
        (status = 200, description = "What is now saved, all of them, as stored", body = PaymentHandles),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such kind of payment option", body = ErrorBody),
        (status = 422, description = "Not a username, $Cashtag, PayPal.Me name, or email address or US phone number for Zelle; empty is not one either (remove it instead)", body = ErrorBody),
        (status = 429, description = "Too many changes to payment options this hour", body = ErrorBody)
    )
)]
pub async fn set_payment_handle(
    State(state): State<AppState>,
    session: Session,
    Path(kind): Path<String>,
    ApiJson(body): ApiJson<PaymentHandleValue>,
) -> Result<Json<PaymentHandles>, ApiError> {
    let app = path_app(&kind)?;
    let mut tx = state.db.begin().await?;
    let saved = payments::save_one(&mut tx, session.account_id, app, Some(&body.value)).await;
    // A refusal for too many changes is counted all the same.
    tx.commit().await?;
    Ok(Json(saved?))
}

/// Removes one of the account's payment options, leaving the others as they
/// are. Removing the last one stops showing them on every agreement, as
/// removing them all does. Removing one not saved changes nothing.
#[utoipa::path(
    delete,
    path = "/v1/me/payment-handles/{kind}",
    params(("kind" = String, Path, description = "`venmo`, `cash_app`, `paypal` or `zelle`")),
    responses(
        (status = 200, description = "What is still saved, as stored", body = PaymentHandles),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such kind of payment option", body = ErrorBody),
        (status = 429, description = "Too many changes to payment options this hour", body = ErrorBody)
    )
)]
pub async fn remove_payment_handle(
    State(state): State<AppState>,
    session: Session,
    Path(kind): Path<String>,
) -> Result<Json<PaymentHandles>, ApiError> {
    let app = path_app(&kind)?;
    let mut tx = state.db.begin().await?;
    let left = payments::save_one(&mut tx, session.account_id, app, None).await;
    tx.commit().await?;
    Ok(Json(left?))
}

/// Shows the signed-in party's payment options to the other party of this
/// agreement, or stops. Not part of the terms: it changes no version, is
/// not signed and is not in the history. The other party sees them only
/// while they owe money on the agreement in force that is still to be paid.
#[utoipa::path(
    put,
    path = "/v1/exchanges/{id}/payment-options",
    params(("id" = String, Path, description = "The exchange")),
    request_body = PaymentOptionsShown,
    responses(
        (status = 200, description = "Whether they are now shown", body = PaymentOptionsShown),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 404, description = "No such exchange, or the caller is not a party to it", body = ErrorBody),
        (status = 409, description = "To turn them on: the agreement is a draft or closed, or the account has no payment options saved (`ACTION_NOT_ALLOWED`). Turning them off always works", body = ErrorBody),
        (status = 429, description = "Too many changes to payment options this hour", body = ErrorBody)
    )
)]
pub async fn set_payment_options(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PaymentOptionsShown>,
) -> Result<Json<PaymentOptionsShown>, ApiError> {
    let exchange: Uuid = id.parse().map_err(|_| ErrorCode::NotFound)?;
    let mut tx = state.db.begin().await?;
    let found: Option<String> = sqlx::query_scalar(
        "SELECT e.state FROM exchange e
         JOIN participant p ON p.exchange_id = e.id AND p.account_id = $2
         WHERE e.id = $1",
    )
    .bind(exchange)
    .bind(session.account_id)
    .fetch_optional(&mut *tx)
    .await?;
    // One the caller is not a party to looks like one that does not exist.
    let exchange_state = found.ok_or(ErrorCode::NotFound)?;
    if body.on && !matches!(exchange_state.as_str(), "NEGOTIATING" | "ACTIVE") {
        return Err(ErrorCode::ActionNotAllowed.into());
    }
    let done = payments::set_shown(&mut tx, session.account_id, exchange, body.on).await;
    tx.commit().await?;
    done?;
    Ok(Json(PaymentOptionsShown { on: body.on }))
}
