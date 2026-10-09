use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use super::{
    AppState, account, auth, deletion, devices, exchanges, payments, record, safety, sms, staff,
    wallet,
};
use crate::auth::{SignInChannel, sign_in_channels};
use crate::client_version::MinimumClientVersions;
use crate::wallet::{Wallet, WalletPlatform};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/meta", get(meta))
        .route("/auth/codes", post(auth::request_code))
        .route("/auth/sessions", post(auth::create_session))
        .route("/auth/session", delete(auth::delete_session))
        .route("/me", get(account::me).patch(account::update_me))
        .route("/me/identifiers", post(account::add_identifier))
        .route("/me/identifiers/proof", post(account::prove_identifier))
        .route("/me/identifiers/{kind}", delete(account::remove_identifier))
        .route("/me/combine", post(account::combine_accounts))
        .route(
            "/me/deletion",
            get(deletion::deletion_preview).post(deletion::delete_account),
        )
        .route("/me/deletion/codes", post(deletion::request_deletion_code))
        .route("/me/devices", put(devices::register_device))
        .route("/me/devices/{id}", delete(devices::remove_device))
        .route(
            "/me/payment-handles",
            get(payments::payment_handles)
                .put(payments::set_payment_handles)
                .delete(payments::remove_payment_handles),
        )
        .route(
            "/me/payment-handles/{kind}",
            put(payments::set_payment_handle).delete(payments::remove_payment_handle),
        )
        .route("/exchanges", get(exchanges::list).post(exchanges::create))
        .route("/exchanges/{id}", get(exchanges::get))
        .route("/exchanges/{id}/draft", put(exchanges::save_draft))
        .route("/exchanges/{id}/revisions", post(exchanges::send_revision))
        .route("/exchanges/{id}/commands", post(exchanges::run_command))
        .route("/exchanges/{id}/leave", post(exchanges::leave))
        .route("/exchanges/{id}/history", get(record::history))
        .route("/exchanges/{id}/record", get(record::record))
        .route(
            "/exchanges/{id}/invitation",
            post(exchanges::reissue_invitation),
        )
        .route(
            "/exchanges/{id}/invitation/shared",
            post(exchanges::invitation_shared),
        )
        .route(
            "/exchanges/{id}/sms-updates",
            get(sms::sms_updates).put(sms::set_sms_updates),
        )
        .route(
            "/exchanges/{id}/payment-options",
            put(payments::set_payment_options),
        )
        // Twilio's few kilobytes, not the 2 MB every other route may take.
        .route(
            "/sms/inbound",
            post(sms::inbound).layer(DefaultBodyLimit::max(sms::INBOUND_BODY_LIMIT)),
        )
        .route("/invitations/preview", post(exchanges::preview_invitation))
        .route("/invitations/claim", post(exchanges::claim_invitation))
        .route(
            "/invitations/address/codes",
            post(account::request_invitation_address_code),
        )
        .route(
            "/invitations/address",
            post(account::add_invitation_address),
        )
        .merge(safety::routes())
        .merge(staff::routes())
        .merge(wallet::routes())
}

#[derive(Serialize, ToSchema)]
pub struct Meta {
    pub service: String,
    /// The package version, such as `0.1.0`.
    pub version: String,
    /// The git commit the running build was made from, in full, or
    /// `unknown` when the build did not say. Also in every response's
    /// `X-Yuppers-Version` header, shortened to seven characters.
    pub commit: String,
    /// When the running build was made, as an RFC 3339 time in UTC; null
    /// when the build did not say.
    pub built_at: Option<String>,
    /// The oldest build of each client that may still change anything. A
    /// client below its minimum shows that it must be updated; its changes
    /// are refused with `CLIENT_TOO_OLD`. Absent for a client with no minimum.
    pub minimum_client_versions: MinimumClientVersions,
    /// Whether the service sends push notifications. An app offers them, and
    /// registers its device (`PUT /v1/me/devices`), only when it does.
    pub push_notifications: bool,
    /// The wallets a pass can be added to here (DESIGN.md §11). Empty until
    /// a deployment configures one; a client shows no Wallet button then.
    pub wallet_platforms: Vec<WalletPlatform>,
    /// The kinds of identifier this deployment can send a one-time code to
    /// right now, email first: `email` while codes go by email (or to the
    /// development log), `phone` while text messages are sent (or, in
    /// development, phone codes go to the log too). A client asks only for
    /// these when someone signs in or adds an identifier; a code for any
    /// other kind is refused with `SERVICE_UNAVAILABLE`.
    pub sign_in_channels: Vec<SignInChannel>,
    /// The country calling codes, such as `+1`, of the phone numbers codes
    /// can be sent to (`PHONE_COUNTRY_NOT_SERVED` refuses any other). Empty
    /// when `sign_in_channels` has no `phone`.
    pub sms_country_codes: Vec<String>,
    /// The email address sign-in codes come from, such as
    /// `no-reply@yuppers.app`: the address part of `EMAIL_FROM` (or
    /// `SMTP_FROM`), without its display name, whether codes go by Resend or
    /// SMTP. A client tells someone waiting for a code to look for
    /// it, in their spam folder too. Absent where codes are not sent by
    /// email from an address of the service's own (the development log).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_sender: Option<String>,
    /// Whether a party may turn on text updates for an agreement
    /// (`GET`/`PUT /v1/exchanges/{id}/sms-updates`): while text messages are
    /// sent. A client shows no "Text updates" control otherwise.
    pub sms_updates: bool,
}

/// Identifies the service and its build, and says how old a client may be.
#[utoipa::path(get, path = "/v1/meta", responses((status = 200, description = "Service identity", body = Meta)))]
pub async fn meta(
    State(state): State<AppState>,
    Extension(wallet): Extension<Arc<Wallet>>,
) -> Json<Meta> {
    let channels = sign_in_channels(state.code_sender.as_ref());
    Json(Meta {
        service: env!("CARGO_PKG_NAME").to_owned(),
        version: state.settings.build.version.to_owned(),
        commit: state.settings.build.commit().to_owned(),
        built_at: state.settings.build.built_at().map(str::to_owned),
        minimum_client_versions: state.settings.min_client_versions.clone(),
        push_notifications: state.settings.push_notifications,
        wallet_platforms: wallet.platforms(),
        sms_country_codes: if channels.contains(&SignInChannel::Phone) {
            state
                .settings
                .auth
                .phone_country_codes
                .iter()
                .map(|code| format!("+{code}"))
                .collect()
        } else {
            Vec::new()
        },
        sign_in_channels: channels,
        code_sender: state.code_sender.email_sender(),
        sms_updates: state.settings.sms_updates,
    })
}
