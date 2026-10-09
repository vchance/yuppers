//! The signed-in account: reading it, editing it, adding, changing and
//! removing its email address and phone number, and combining another
//! account into it (`crate::combine`).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::auth::client_of;
use super::exchanges::idempotency_key;
use super::extract::{ApiJson, DigestedJson, Session};
use super::{AppState, ClientAddress};
use crate::auth::{self, CodeCheck, OfferedCode, Requester};
use crate::code_consent::{CodePurpose, CodeRequest, SmsCodeConsent};
use crate::combine::{self, CombineOffer, CombineOffered};
use crate::contact::{self, Field, Kind};
use crate::domain::identity::{Identifier, InvalidIdentifier};
use crate::error::{ApiError, ErrorBody, ErrorCode};
use crate::exchanges::dto::ExchangeView;
use crate::exchanges::service::{self, Claim, Idempotency, already_applied};
use crate::languages;
use crate::notifications::sms_updates;

#[derive(Debug, Serialize, ToSchema)]
pub struct Account {
    pub id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    /// Empty until the person has chosen one.
    pub display_name: String,
    /// A supported language tag, such as `en` or `es`.
    pub language: String,
    /// The holder has confirmed they are 18 or over. Required before signing.
    pub adult_confirmed: bool,
    /// When another account was combined into this one with no email address
    /// on either to tell, as RFC 3339: the clients show it once, until it is
    /// dismissed (`dismiss_combined_notice` in `PATCH /v1/me`). Absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combined_notice: Option<String>,
}

type AccountRow = (
    Uuid,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    String,
    String,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

const ACCOUNT_COLUMNS: &str = "id, email_encrypted, phone_encrypted, display_name, language, \
                               adult_confirmed_at, combined_notice_at";

/// The account as its owner sees it: one of the few places its address and
/// number are decrypted (`crate::contact`).
fn from_row(row: AccountRow) -> Result<Account, contact::Unreadable> {
    let (id, email_encrypted, phone_encrypted, display_name, language, adult, combined) = row;
    let keys = contact::keys();
    Ok(Account {
        id: id.to_string(),
        email: keys.reveal(Field::ACCOUNT_EMAIL, email_encrypted.as_deref())?,
        phone: keys.reveal(Field::ACCOUNT_PHONE, phone_encrypted.as_deref())?,
        display_name,
        language,
        adult_confirmed: adult.is_some(),
        combined_notice: combined.map(crate::exchanges::dto::rfc3339),
    })
}

pub async fn load(db: impl PgExecutor<'_>, id: Uuid) -> Result<Account, ApiError> {
    let row: AccountRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM account WHERE id = $1"
    )))
    .bind(id)
    .fetch_one(db)
    .await?;
    Ok(from_row(row)?)
}

/// The signed-in account.
#[utoipa::path(
    get,
    path = "/v1/me",
    responses(
        (status = 200, description = "The account", body = Account),
        (status = 401, description = "Not signed in", body = ErrorBody)
    )
)]
pub async fn me(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Account>, ApiError> {
    Ok(Json(load(&state.db, session.account_id).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAccount {
    /// 1 to 100 characters.
    pub display_name: Option<String>,
    /// A language tag. A regional tag falls back to its base language; an
    /// unsupported language is refused.
    pub language: Option<String>,
    /// Only `true` is meaningful: a confirmation cannot be taken back.
    pub adult_confirmed: Option<bool>,
    /// `true` once the notice that accounts were combined into this one has
    /// been shown (`combined_notice`).
    pub dismiss_combined_notice: Option<bool>,
}

/// Changes the display name or language, or records that the holder is an adult.
#[utoipa::path(
    patch,
    path = "/v1/me",
    request_body = UpdateAccount,
    responses(
        (status = 200, description = "The updated account", body = Account),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 422, description = "Invalid request", body = ErrorBody)
    )
)]
pub async fn update_me(
    State(state): State<AppState>,
    session: Session,
    ApiJson(update): ApiJson<UpdateAccount>,
) -> Result<Json<Account>, ApiError> {
    let display_name = match update.display_name.as_deref().map(str::trim) {
        Some(name) if name.is_empty() || name.chars().count() > 100 => {
            return Err(ErrorCode::InvalidRequest.into());
        }
        name => name,
    };

    let language = match update.language.as_deref() {
        Some(tag) => Some(languages::resolve(tag).ok_or(ErrorCode::InvalidRequest)?),
        None => None,
    };

    let changed = sqlx::query(
        "UPDATE account
         SET display_name = coalesce($2, display_name),
             language = coalesce($3, language),
             adult_confirmed_at = CASE WHEN $4 THEN coalesce(adult_confirmed_at, now())
                                       ELSE adult_confirmed_at END,
             combined_notice_at = CASE WHEN $5 THEN NULL ELSE combined_notice_at END
         WHERE id = $1 AND status = 'ACTIVE'",
    )
    .bind(session.account_id)
    .bind(display_name)
    .bind(language)
    .bind(update.adult_confirmed == Some(true))
    .bind(update.dismiss_combined_notice == Some(true))
    .execute(&state.db)
    .await?
    .rows_affected();
    // The session was checked a moment ago; the account can have been
    // deleted since, and a deleted account is not edited.
    if changed == 0 {
        return Err(ErrorCode::Unauthenticated.into());
    }

    Ok(Json(load(&state.db, session.account_id).await?))
}

// ---- Email address and phone number -----------------------------------------

/// A refusal of an identifier proved with its code: an error, or, where the
/// identifier is another account's, the offer to combine the two
/// (`IDENTIFIER_ON_OTHER_ACCOUNT`, 409, with `combine`).
#[derive(Debug)]
pub enum IdentifierRefusal {
    Error(ApiError),
    Combine(Box<CombineOffer>),
}

impl IntoResponse for IdentifierRefusal {
    fn into_response(self) -> Response {
        match self {
            IdentifierRefusal::Error(error) => error.into_response(),
            IdentifierRefusal::Combine(offer) => (
                StatusCode::CONFLICT,
                Json(CombineOffered {
                    code: ErrorCode::IdentifierOnOtherAccount,
                    combine: *offer,
                }),
            )
                .into_response(),
        }
    }
}

impl From<ApiError> for IdentifierRefusal {
    fn from(error: ApiError) -> Self {
        IdentifierRefusal::Error(error)
    }
}

impl From<ErrorCode> for IdentifierRefusal {
    fn from(code: ErrorCode) -> Self {
        IdentifierRefusal::Error(code.into())
    }
}

impl From<sqlx::Error> for IdentifierRefusal {
    fn from(error: sqlx::Error) -> Self {
        IdentifierRefusal::Error(error.into())
    }
}

impl From<InvalidIdentifier> for IdentifierRefusal {
    fn from(error: InvalidIdentifier) -> Self {
        IdentifierRefusal::Error(error.into())
    }
}

impl From<contact::Unreadable> for IdentifierRefusal {
    fn from(error: contact::Unreadable) -> Self {
        IdentifierRefusal::Error(error.into())
    }
}

/// Puts an identifier whose code was just entered on the account, replacing
/// the one of its kind, if no other account has it. If one has, the person
/// has now shown that they control both, and is offered to combine them.
/// Before the code was checked nothing here ran, so nothing said whether the
/// identifier had an account.
pub(crate) async fn attach(
    db: &PgPool,
    account: Uuid,
    identifier: &Identifier,
) -> Result<Account, IdentifierRefusal> {
    // Stored encrypted, with its blind index, which the unique constraint
    // is on (`crate::contact`).
    let (encrypted, index) = Kind::of(identifier).account_columns();
    let sealed = contact::keys().sealed(identifier);
    // Twice at most: the other account may give it up in between.
    for _ in 0..2 {
        // Only while the account is active: an identifier written onto an
        // account deleted at the same moment could never be used again.
        let result = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE account SET {encrypted} = $2, {index} = $3
             WHERE id = $1 AND status = 'ACTIVE'"
        )))
        .bind(account)
        .bind(&sealed.encrypted)
        .bind(sealed.index.as_slice())
        .execute(db)
        .await;
        match result {
            Ok(done) if done.rows_affected() == 0 => {
                return Err(ErrorCode::Unauthenticated.into());
            }
            Ok(_) => {
                // Text updates were turned on for a number; a new one has
                // not agreed to them, so those for the old number end.
                if let Identifier::Phone(_) = identifier {
                    let mut conn = db.acquire().await?;
                    sms_updates::forget_numbers(
                        &mut conn,
                        account,
                        Some(&sealed.index),
                        sms_updates::Source::PhoneChanged,
                    )
                    .await?;
                }
                return Ok(load(db, account).await?);
            }
            Err(error) if is_unique_violation(&error) => {
                let mut conn = db.acquire().await?;
                let owner = combine::owner_of(&mut conn, identifier).await?;
                drop(conn);
                match owner {
                    Some(other) if other != account => {
                        let offer = combine::offer(db, account, other, identifier).await?;
                        return Err(IdentifierRefusal::Combine(Box::new(offer)));
                    }
                    // Given up a moment ago: try again.
                    _ => continue,
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(ErrorCode::ServiceUnavailable.into())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddIdentifier {
    /// An email address, or a phone number: in international form, or a US
    /// number without its country code as people there write it, ten digits
    /// or 1 and ten (`(856) 548-8780`), which is taken as `+1`.
    pub identifier: String,
    /// The one-time code sent to it.
    pub code: String,
}

/// Verifies an email address or phone number and attaches it to the
/// account, or replaces the one of the same kind: adding one, or changing
/// it. An account with both a verified email and a verified phone can meet
/// the higher risk tier. If the code is right and the identifier belongs to
/// another account, the answer is `IDENTIFIER_ON_OTHER_ACCOUNT` with an
/// offer to combine the two (`POST /v1/me/combine`); until the code is
/// right, an identifier with an account is answered like any other.
#[utoipa::path(
    post,
    path = "/v1/me/identifiers",
    request_body = AddIdentifier,
    responses(
        (status = 200, description = "The updated account", body = Account),
        (status = 401, description = "Not signed in, or the code is wrong", body = ErrorBody),
        (status = 409, description = "The identifier belongs to another account, which the code shows the person controls (`IDENTIFIER_ON_OTHER_ACCOUNT`, with `combine`), or that account cannot be combined into this one (`COMBINE_SUSPENDED`, `COMBINE_REVIEWER`, `COMBINE_SHARED_EXCHANGE`)", body = CombineOffered),
        (status = 422, description = "Not an email address or phone number (`INVALID_IDENTIFIER`), or a phone number of a country the service does not take (`PHONE_COUNTRY_NOT_SERVED`)", body = ErrorBody),
        (status = 429, description = "Too many wrong codes for this identifier today (`TOO_MANY_GUESSES`), or a wrong code from an address that has offered too many this hour (`TOO_MANY_REQUESTS`)", body = ErrorBody),
        (status = 503, description = "A code sent by text could not be checked, because the provider that made it did not answer; nothing was counted", body = ErrorBody)
    )
)]
pub async fn add_identifier(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    ApiJson(body): ApiJson<AddIdentifier>,
) -> Result<Json<Account>, IdentifierRefusal> {
    let identifier = Identifier::parse(&body.identifier)?;
    let settings = &state.settings;
    // No code can have been sent to such a number, and none is checked: a
    // number the service does not take is not attached to an account.
    settings.auth.check_taken(&identifier)?;
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
    Ok(Json(
        attach(&state.db, session.account_id, &identifier).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RemoveIdentifier {
    /// A one-time code sent to the identifier that stays (the account's
    /// other one), asked for with `POST /v1/auth/codes`.
    pub code: String,
}

/// The kind named in a path: `email` or `phone`.
fn path_kind(text: &str) -> Result<Kind, ApiError> {
    match text {
        "email" => Ok(Kind::Email),
        "phone" => Ok(Kind::Phone),
        _ => Err(ErrorCode::NotFound.into()),
    }
}

/// Removes the account's email address or phone number, keeping the other:
/// an account keeps one to sign in with (`LAST_IDENTIFIER`). Proved with a
/// code sent to the one that stays, so that a session alone cannot take a
/// way in away, and the person knows they can still sign in. Removing the
/// phone number ends text updates for every agreement. Invitations named
/// for the address or number removed are left as they are. Removing one the
/// account does not have changes nothing and succeeds, as a repeat finds;
/// so does a repeat with the same `Idempotency-Key`.
#[utoipa::path(
    delete,
    path = "/v1/me/identifiers/{kind}",
    params(
        ("kind" = String, Path, description = "`email` or `phone`"),
        ("Idempotency-Key" = Option<String>, Header, description = "Unique per attempt to make this change")
    ),
    request_body = RemoveIdentifier,
    responses(
        (status = 200, description = "The updated account", body = Account),
        (status = 401, description = "Not signed in, or the code is wrong, expired or used up", body = ErrorBody),
        (status = 409, description = "It is the account's only identifier (`LAST_IDENTIFIER`)", body = ErrorBody),
        (status = 429, description = "Too many wrong codes for the identifier that stays today", body = ErrorBody),
        (status = 503, description = "A code sent by text could not be checked; nothing was counted", body = ErrorBody)
    )
)]
pub async fn remove_identifier(
    State(state): State<AppState>,
    session: Session,
    Path(kind): Path<String>,
    headers: HeaderMap,
    ClientAddress(address): ClientAddress,
    DigestedJson { body, digest }: DigestedJson<RemoveIdentifier>,
) -> Result<Json<Account>, ApiError> {
    let kind = path_kind(&kind)?;
    let idempotency = Idempotency {
        key: idempotency_key(&headers)?,
        digest,
    };
    let account = session.account_id;
    let current = load(&state.db, account).await?;
    let (removing, staying) = match kind {
        Kind::Email => (current.email.as_ref(), current.phone.as_ref()),
        Kind::Phone => (current.phone.as_ref(), current.email.as_ref()),
    };
    // Gone already: what a repeat finds.
    if removing.is_none() {
        return Ok(Json(current));
    }
    let Some(staying) = staying else {
        return Err(ErrorCode::LastIdentifier.into());
    };
    let (stays, staying) = match kind {
        Kind::Email => (Kind::Phone, Identifier::Phone(staying.clone())),
        Kind::Phone => (Kind::Email, Identifier::Email(staying.clone())),
    };

    let settings = &state.settings;
    let code = OfferedCode {
        secret: &settings.app_secret,
        rules: &settings.auth,
        identifier: &staying,
        code: &body.code,
        requester: Requester::SignIn { address },
        verifier: state.code_sender.verifier(&staying),
    };
    let _turn = code.consult_verifier(&state.db).await?;

    let mut tx = state.db.begin().await?;
    if already_applied(&mut tx, account, &idempotency).await? {
        tx.commit().await?;
        return Ok(Json(load(&state.db, account).await?));
    }
    // Checked and used up with the removal: both happen or neither does.
    if let CodeCheck::Refused(error) = code.check(&mut tx).await? {
        // The wrong guess is counted, as for any code.
        tx.commit().await?;
        return Err(error);
    }
    let (encrypted, index) = kind.account_columns();
    let (_, other_index) = stays.account_columns();
    // Only while the one that stays is still the one the code went to.
    let removed = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE account SET {encrypted} = NULL, {index} = NULL
         WHERE id = $1 AND status = 'ACTIVE' AND {other_index} = $2"
    )))
    .bind(account)
    .bind(contact::keys().index_of(&staying).as_slice())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(ErrorCode::LastIdentifier.into());
    }
    // The texts for every agreement went to this number.
    if kind == Kind::Phone {
        sms_updates::forget_numbers(&mut tx, account, None, sms_updates::Source::PhoneRemoved)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load(&state.db, account).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CombineAccounts {
    /// The token of the offer: `combine.token` in an
    /// `IDENTIFIER_ON_OTHER_ACCOUNT` answer.
    pub token: String,
}

/// Combines into the signed-in account the account an offer names. It
/// cannot be undone: that account's place in each of its yups, the address
/// or number proved and what else the offer lists move here, and it can no
/// longer be signed in to. Every address and number either account had is
/// told. The offer is good once, for ten minutes, from this account only; a
/// repeat with the same `Idempotency-Key` succeeds and changes nothing.
#[utoipa::path(
    post,
    path = "/v1/me/combine",
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Unique per attempt to make this change")
    ),
    request_body = CombineAccounts,
    responses(
        (status = 200, description = "Combined; the account as it now is", body = Account),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 409, description = "The offer is used, expired or no longer true (`COMBINE_EXPIRED`), or the two cannot be combined (`COMBINE_SUSPENDED`, `COMBINE_REVIEWER`, `COMBINE_SHARED_EXCHANGE`)", body = ErrorBody),
        (status = 503, description = "The accounts were busy and nothing was done; try again", body = ErrorBody)
    )
)]
pub async fn combine_accounts(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    DigestedJson { body, digest }: DigestedJson<CombineAccounts>,
) -> Result<Json<Account>, ApiError> {
    let idempotency = Idempotency {
        key: idempotency_key(&headers)?,
        digest,
    };
    combine::combine(
        &state.db,
        &state.settings.rules,
        session.account_id,
        &body.token,
        &idempotency,
    )
    .await?;
    Ok(Json(load(&state.db, session.account_id).await?))
}

// ---- An invitation sent to another address ------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct InvitationAddressCode {
    /// The invitation link's token.
    pub token: String,
    /// For a phone number, required: the box beside it was ticked
    /// (`smsCode.verifyNumber`), with the version and language of the
    /// wording shown. Ignored for an email address.
    pub sms_consent: Option<SmsCodeConsent>,
}

/// Sends a one-time code to the email address or phone number an invitation
/// names, for someone signed in with another who wants to add it to their
/// account and open the invitation (`sent_to` in its preview). The address
/// is never shown in full and never named in the request. The usual limits
/// on codes apply, as for any address.
#[utoipa::path(
    post,
    path = "/v1/invitations/address/codes",
    request_body = InvitationAddressCode,
    responses(
        (status = 204, description = "A code was sent"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 403, description = "The invitation was made before its address was kept: only that address can open it (`INVITATION_NOT_FOR_YOU`)", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer (`INVITATION_UNAVAILABLE`)", body = ErrorBody),
        (status = 409, description = "The invitation names nobody, or this account already (`ACTION_NOT_ALLOWED`), or the number replied STOP (`PHONE_OPTED_OUT`)", body = ErrorBody),
        (status = 422, description = "A phone number without `sms_consent` (`SMS_CONSENT_REQUIRED`), or of a country not served (`PHONE_COUNTRY_NOT_SERVED`)", body = ErrorBody),
        (status = 429, description = "Too many codes asked for", body = ErrorBody)
    )
)]
pub async fn request_invitation_address_code(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    ApiJson(body): ApiJson<InvitationAddressCode>,
) -> Result<StatusCode, ApiError> {
    let (identifier, _) =
        service::invitation_address(&state.db, session.account_id, &body.token).await?;
    let (source, user_agent) = client_of(&headers);
    let request = CodeRequest {
        purpose: CodePurpose::VerifyNumber,
        account: Some(session.account_id),
        consent: body.sms_consent.as_ref(),
        source,
        address,
        user_agent,
    };
    // In the account's own language: the address may be someone's, but the
    // person asking is the one who will read it.
    let language = load(&state.db, session.account_id).await?.language;
    let settings = &state.settings;
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddInvitationAddress {
    /// The invitation link's token.
    pub token: String,
    /// The code sent to the address the invitation names.
    pub code: String,
    /// The account has another address of this kind (`sent_to.replaces`):
    /// `true` replaces it. Without it such a request is refused, before the
    /// code is looked at, with `IDENTIFIER_KIND_TAKEN`.
    #[serde(default)]
    pub replace: bool,
}

/// Adds to the signed-in account the email address or phone number an
/// invitation names, with the code sent there, and opens the invitation: the
/// account takes the invited party's place, as named, so nobody has to
/// confirm it. If the address belongs to another account the answer is
/// `IDENTIFIER_ON_OTHER_ACCOUNT` with the offer to combine the two; once
/// combined, claiming the invitation (`POST /v1/invitations/claim`) opens it.
#[utoipa::path(
    post,
    path = "/v1/invitations/address",
    request_body = AddInvitationAddress,
    responses(
        (status = 200, description = "Added, and the invitation opened", body = ExchangeView),
        (status = 401, description = "Not signed in, or the code is wrong", body = ErrorBody),
        (status = 403, description = "The invitation was made before its address was kept (`INVITATION_NOT_FOR_YOU`)", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer", body = ErrorBody),
        (status = 409, description = "The address is another account's (`IDENTIFIER_ON_OTHER_ACCOUNT`, with `combine`); the account has another of this kind and `replace` was not given (`IDENTIFIER_KIND_TAKEN`); or the two accounts cannot be combined", body = CombineOffered),
        (status = 429, description = "Too many wrong codes", body = ErrorBody),
        (status = 503, description = "A code sent by text could not be checked", body = ErrorBody)
    )
)]
pub async fn add_invitation_address(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    ApiJson(body): ApiJson<AddInvitationAddress>,
) -> Result<Json<ExchangeView>, IdentifierRefusal> {
    let account = session.account_id;
    let (identifier, replaces) =
        service::invitation_address(&state.db, account, &body.token).await?;
    // Asked before the code is looked at, so that it is not spent on a
    // request that cannot go through. One address of each kind per account.
    if replaces && !body.replace {
        return Err(ErrorCode::IdentifierKindTaken.into());
    }
    let settings = &state.settings;
    settings.auth.check_taken(&identifier)?;
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
    attach(&state.db, account, &identifier).await?;
    // Now named by it: the claim goes through as for anyone the invitation
    // was sent to.
    let view = service::claim_invitation(
        &state.db,
        &settings.rules,
        &session,
        &body.token,
        Claim::Take,
    )
    .await?;
    Ok(Json(view))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|e| e.code().as_deref() == Some("23505"))
}
