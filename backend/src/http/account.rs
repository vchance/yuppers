//! The signed-in account: reading it, editing it, adding, changing and
//! removing its email address and phone number, and combining another
//! account into it (`crate::combine`).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, PgConnection, PgExecutor, PgPool};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::auth::client_of;
use super::exchanges::idempotency_key;
use super::extract::{ApiJson, DigestedJson, Session};
use super::{AppState, ClientAddress};
use crate::auth::{self, CodeCheck, OfferedCode, Requester};
use crate::code_consent::{CodePurpose, CodeRequest, SmsCodeConsent};
use crate::combine::{self, AccountProof, CombineOffer, CombineOffered, InAppNotice, NoticeKind};
use crate::contact::{self, Field, Kind};
use crate::deletion::{self, CodeChannel};
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
    /// Something that happened to the account with no email address to tell
    /// about it: the clients show it once, until it is dismissed
    /// (`dismiss_notice` in `PATCH /v1/me`). Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<AccountNotice>,
    /// The version of the Terms and the Privacy policy accepted at the
    /// latest sign-in (`TERMS_VERSION`). Absent until the account signs in
    /// again after this was first recorded. Only the account's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_version: Option<String>,
    /// When it was accepted, as RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_accepted_at: Option<String>,
}

/// A notice the app shows once on the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountNotice {
    pub kind: InAppNotice,
    /// When it happened, as RFC 3339.
    pub at: String,
}

type AccountRow = (
    Uuid,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    String,
    String,
    Option<OffsetDateTime>,
    Option<String>,
    Option<OffsetDateTime>,
    Option<String>,
    Option<OffsetDateTime>,
);

const ACCOUNT_COLUMNS: &str = "id, email_encrypted, phone_encrypted, display_name, language, \
                               adult_confirmed_at, notice_kind, notice_at, terms_version, terms_accepted_at";

/// The account as its owner sees it: one of the few places its address and
/// number are decrypted (`crate::contact`).
fn from_row(row: AccountRow) -> Result<Account, contact::Unreadable> {
    let (
        id,
        email_encrypted,
        phone_encrypted,
        display_name,
        language,
        adult,
        kind,
        at,
        terms_version,
        terms_accepted_at,
    ) = row;
    let keys = contact::keys();
    let notice = match (kind.as_deref().and_then(InAppNotice::parse), at) {
        (Some(kind), Some(at)) => Some(AccountNotice {
            kind,
            at: crate::exchanges::dto::rfc3339(at),
        }),
        _ => None,
    };
    Ok(Account {
        id: id.to_string(),
        email: keys.reveal(Field::ACCOUNT_EMAIL, email_encrypted.as_deref())?,
        phone: keys.reveal(Field::ACCOUNT_PHONE, phone_encrypted.as_deref())?,
        display_name,
        language,
        adult_confirmed: adult.is_some(),
        notice,
        terms_version,
        terms_accepted_at: terms_accepted_at.map(crate::exchanges::dto::rfc3339),
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
    /// `true` once the account's `notice` has been shown.
    pub dismiss_notice: Option<bool>,
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
             notice_kind = CASE WHEN $5 THEN NULL ELSE notice_kind END,
             notice_at = CASE WHEN $5 THEN NULL ELSE notice_at END
         WHERE id = $1 AND status = 'ACTIVE'",
    )
    .bind(session.account_id)
    .bind(display_name)
    .bind(language)
    .bind(update.adult_confirmed == Some(true))
    .bind(update.dismiss_notice == Some(true))
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

/// What [`attach_in`] did.
pub(crate) enum Attached {
    /// The identifier is the account's now.
    Done,
    /// Another account has it, which the person has now shown they control.
    /// Nothing was changed: the caller rolls back and makes the offer.
    OnOther(Uuid),
}

/// The account as [`attach_in`] reads it: its identifier of the kind,
/// encrypted, both indexes, the kind's first, and its language.
type Before = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>, String);

/// Puts an identifier whose code was just entered on the account, in the
/// caller's transaction, if no other account has it. While the account has
/// an identifier, of either kind, it is added or replaces the one of its
/// kind only with `proof`, a proof of one the account already has
/// ([`combine::issue_proof`]), used up here: a session alone adds no way in
/// and takes none away. A proof from an identifier that came in the last
/// day does not replace an older one ([`combine::FRESH_IDENTIFIER`]). An
/// email address replaced is told by email, a phone number by a notice in
/// the app. Before the code was checked nothing here ran, so nothing said
/// whether the identifier had an account.
pub(crate) async fn attach_in(
    conn: &mut PgConnection,
    account: Uuid,
    identifier: &Identifier,
    proof: Option<&[u8; 32]>,
) -> Result<Attached, ApiError> {
    let kind = Kind::of(identifier);
    // Stored encrypted, with its blind index, which the unique constraint
    // is on (`crate::contact`).
    let (encrypted, index) = kind.account_columns();
    let sealed = contact::keys().sealed(identifier);
    // As it stands, held until the transaction ends. Only while the account
    // is active: an identifier written onto an account deleted at the same
    // moment could never be used again.
    let (_, other_index) = other_kind(kind).account_columns();
    let before: Option<Before> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {encrypted}, {index}, {other_index}, language FROM account
             WHERE id = $1 AND status = 'ACTIVE' FOR NO KEY UPDATE"
    )))
    .bind(account)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((old_sealed, old_index, other, language)) = before else {
        return Err(ErrorCode::Unauthenticated.into());
    };
    let unchanged = old_index.as_deref() == Some(sealed.index.as_slice());
    let replacing = old_index.is_some() && !unchanged;
    // The account's ways in change: a session alone is not enough for that,
    // as for removing one. Used up while the identifier it proves is still
    // the account's; should the identifier turn out to be another
    // account's, the caller rolls back and it is unused again.
    if !unchanged && (old_index.is_some() || other.is_some()) {
        let Some(proof) = proof else {
            return Err(ErrorCode::ProofRequired.into());
        };
        combine::take_proof(conn, account, proof, replacing.then_some(kind)).await?;
    }

    // Twice at most: the other account may give it up in between.
    for _ in 0..2 {
        let mut attempt = conn.begin().await?;
        let result = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE account SET {encrypted} = $2, {index} = $3 WHERE id = $1"
        )))
        .bind(account)
        .bind(&sealed.encrypted)
        .bind(sealed.index.as_slice())
        .execute(&mut *attempt)
        .await;
        match result {
            Ok(_) => attempt.commit().await?,
            Err(error) if is_unique_violation(&error) => {
                attempt.rollback().await?;
                match combine::owner_of(conn, identifier).await? {
                    Some(other) if other != account => return Ok(Attached::OnOther(other)),
                    // Given up a moment ago: try again.
                    _ => continue,
                }
            }
            Err(error) => return Err(error.into()),
        }
        if replacing {
            let old = contact::keys().reveal(Field::account(kind), old_sealed.as_deref())?;
            match (kind, old) {
                (Kind::Email, Some(old)) => {
                    combine::notice_email(conn, account, NoticeKind::EmailChanged, &old, &language)
                        .await?;
                }
                (Kind::Phone, _) => {
                    combine::notice_in_app(conn, account, InAppNotice::PhoneChanged).await?;
                }
                (Kind::Email, None) => {}
            }
        }
        // Text updates were turned on for a number; a new one has not
        // agreed to them, so those for the old number end.
        if kind == Kind::Phone {
            sms_updates::forget_numbers(
                conn,
                account,
                Some(&sealed.index),
                sms_updates::Source::PhoneChanged,
            )
            .await?;
        }
        return Ok(Attached::Done);
    }
    Err(ErrorCode::ServiceUnavailable.into())
}

/// An account's blind indexes: of one kind, then of the other.
type Indexes = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Refuses, before any code is looked at, putting `identifier` on the
/// account without the proof [`attach_in`] will want: none where it needs
/// one, or one it would refuse. Checked again, and used, there.
async fn check_attach_proof(
    db: &PgPool,
    account: Uuid,
    identifier: &Identifier,
    proof: Option<&[u8; 32]>,
) -> Result<(), ApiError> {
    let kind = Kind::of(identifier);
    let (_, index) = kind.account_columns();
    let (_, other_index) = other_kind(kind).account_columns();
    let found: Option<Indexes> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {index}, {other_index} FROM account WHERE id = $1"
    )))
    .bind(account)
    .fetch_optional(db)
    .await?;
    let Some((old, other)) = found else {
        return Err(ErrorCode::Unauthenticated.into());
    };
    let new = contact::keys().index_of(identifier);
    let unchanged = old.as_deref() == Some(new.as_slice());
    if unchanged || (old.is_none() && other.is_none()) {
        return Ok(());
    }
    let Some(proof) = proof else {
        return Err(ErrorCode::ProofRequired.into());
    };
    combine::check_proof(db, account, proof, old.is_some().then_some(kind)).await
}

/// The kind of identifier that is not `kind`.
fn other_kind(kind: Kind) -> Kind {
    match kind {
        Kind::Email => Kind::Phone,
        Kind::Phone => Kind::Email,
    }
}

/// A proof as a request carries it, hashed as stored.
fn proof_hash(proof: Option<&str>) -> Option<[u8; 32]> {
    proof
        .map(str::trim)
        .filter(|proof| !proof.is_empty())
        .map(auth::token_hash)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddIdentifier {
    /// An email address, or a phone number: in international form, or a US
    /// number without its country code as people there write it, ten digits
    /// or 1 and ten (`(856) 548-8780`), which is taken as `+1`.
    pub identifier: String,
    /// The one-time code sent to it.
    pub code: String,
    /// While the account has an identifier, of either kind: a proof of one
    /// it has (`POST /v1/me/identifiers/proof`). Without a good one the
    /// request is refused with `PROOF_REQUIRED` (or `IDENTIFIER_TOO_RECENT`),
    /// before the code is looked at.
    pub proof: Option<String>,
}

/// Verifies an email address or phone number and attaches it to the
/// account, with a proof of one it already has (`proof`): adding one, or
/// replacing the one of the same kind. A proof from an identifier that came
/// in the last 24 hours does not replace an older one. The address replaced
/// is told by email; a number replaced, by a notice in the app. An account
/// with both a verified email and a verified phone can meet the higher risk
/// tier. If the code is right and the identifier belongs to another
/// account, the answer is `IDENTIFIER_ON_OTHER_ACCOUNT` with an offer to
/// combine the two (`POST /v1/me/combine`); until the code is right, an
/// identifier with an account is answered like any other.
#[utoipa::path(
    post,
    path = "/v1/me/identifiers",
    request_body = AddIdentifier,
    responses(
        (status = 200, description = "The updated account", body = Account),
        (status = 401, description = "Not signed in, or the code is wrong", body = ErrorBody),
        (status = 409, description = "The identifier belongs to another account, which the code shows the person controls (`IDENTIFIER_ON_OTHER_ACCOUNT`, with `combine`), or that account cannot be combined into this one (`COMBINE_SUSPENDED`, `COMBINE_REVIEWER`, `COMBINE_SHARED_EXCHANGE`); or the account has an identifier and no good `proof` was given (`PROOF_REQUIRED`), or one from an identifier too new to replace an older one (`IDENTIFIER_TOO_RECENT`)", body = CombineOffered),
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
    let account = session.account_id;
    let identifier = Identifier::parse(&body.identifier)?;
    let proof = proof_hash(body.proof.as_deref());
    let settings = &state.settings;
    // No code can have been sent to such a number, and none is checked: a
    // number the service does not take is not attached to an account.
    settings.auth.check_taken(&identifier)?;
    // Asked before the code is looked at, so that it is not spent on a
    // request that cannot go through: a bogus proof included.
    check_attach_proof(&state.db, account, &identifier, proof.as_ref()).await?;
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
    match attach_in(&mut tx, account, &identifier, proof.as_ref()).await? {
        Attached::Done => tx.commit().await?,
        Attached::OnOther(other) => {
            drop(tx);
            let offer = combine::offer(&state.db, account, other, &identifier).await?;
            return Err(IdentifierRefusal::Combine(Box::new(offer)));
        }
    }
    Ok(Json(load(&state.db, account).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProveIdentifier {
    /// Which of the account's own identifiers the code was sent to.
    pub channel: CodeChannel,
    /// A one-time code sent to it, asked for with `POST /v1/auth/codes`.
    pub code: String,
}

/// Proves, with a code sent to one of the account's own identifiers, that
/// the person signed in controls it: what adding or replacing an email
/// address or phone number needs, directly, from an invitation, or by
/// combining another account into this one (`proof`). Good once, for ten
/// minutes, for this account only, and only while the account still has
/// the identifier proved.
#[utoipa::path(
    post,
    path = "/v1/me/identifiers/proof",
    request_body = ProveIdentifier,
    responses(
        (status = 200, description = "The proof", body = AccountProof),
        (status = 401, description = "Not signed in, or the code is wrong, expired or used up", body = ErrorBody),
        (status = 422, description = "The account has no such identifier", body = ErrorBody),
        (status = 429, description = "Too many wrong codes for that identifier today", body = ErrorBody),
        (status = 503, description = "A code sent by text could not be checked; nothing was counted", body = ErrorBody)
    )
)]
pub async fn prove_identifier(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    ApiJson(body): ApiJson<ProveIdentifier>,
) -> Result<Json<AccountProof>, ApiError> {
    let account = session.account_id;
    let identifier = deletion::identifier(&state.db, account, body.channel).await?;
    let settings = &state.settings;
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
    let mut conn = state.db.acquire().await?;
    let proof =
        combine::issue_proof(&mut conn, account, &contact::keys().index_of(&identifier)).await?;
    Ok(Json(proof))
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
/// way in away, and the person knows they can still sign in; a code to one
/// that came in the last 24 hours does not remove one the account had
/// before it (`IDENTIFIER_TOO_RECENT`). An email
/// address removed is told by email; a phone number removed, by a notice in
/// the app. Removing the phone number ends text updates for every
/// agreement. Invitations named for the address or number removed are left
/// as they are. Removing one the account does not have changes nothing and
/// succeeds, as a repeat finds; so does a repeat with the same
/// `Idempotency-Key`.
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
        (status = 409, description = "It is the account's only identifier (`LAST_IDENTIFIER`), or the one that stays came in the last 24 hours and the one removed is older (`IDENTIFIER_TOO_RECENT`)", body = ErrorBody),
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
        Kind::Email => (current.email.clone(), current.phone.as_ref()),
        Kind::Phone => (current.phone.clone(), current.email.as_ref()),
    };
    // Gone already: what a repeat finds.
    let Some(removing) = removing else {
        return Ok(Json(current));
    };
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
    // A code to one that came in the last day does not take away one the
    // account had before it: asked before the code is looked at.
    let added: Option<(Option<OffsetDateTime>, Option<OffsetDateTime>)> =
        sqlx::query_as("SELECT email_added_at, phone_added_at FROM account WHERE id = $1")
            .bind(account)
            .fetch_optional(&state.db)
            .await?;
    let (email_added, phone_added) = added.unwrap_or_default();
    let (staying_added, removing_added) = match kind {
        Kind::Email => (phone_added, email_added),
        Kind::Phone => (email_added, phone_added),
    };
    if combine::too_recent(staying_added, removing_added) {
        return Err(ErrorCode::IdentifierTooRecent.into());
    }
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
    // Only while the one that stays is still the one the code went to, and
    // the one removed is the one read above.
    let removed = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE account SET {encrypted} = NULL, {index} = NULL
         WHERE id = $1 AND status = 'ACTIVE' AND {other_index} = $2 AND {index} = $3"
    )))
    .bind(account)
    .bind(contact::keys().index_of(&staying).as_slice())
    .bind(
        contact::keys()
            .index_of(&removed_identifier(kind, &removing))
            .as_slice(),
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(ErrorCode::LastIdentifier.into());
    }
    match kind {
        // Told, at the address that is no longer the account's.
        Kind::Email => {
            combine::notice_email(
                &mut tx,
                account,
                NoticeKind::EmailRemoved,
                &removing,
                &current.language,
            )
            .await?;
        }
        Kind::Phone => {
            combine::notice_in_app(&mut tx, account, InAppNotice::PhoneRemoved).await?;
            // The texts for every agreement went to this number.
            sms_updates::forget_numbers(&mut tx, account, None, sms_updates::Source::PhoneRemoved)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(load(&state.db, account).await?))
}

/// The identifier of `kind` with the value `value`.
fn removed_identifier(kind: Kind, value: &str) -> Identifier {
    match kind {
        Kind::Email => Identifier::Email(value.to_owned()),
        Kind::Phone => Identifier::Phone(value.to_owned()),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CombineAccounts {
    /// The token of the offer: `combine.token` in an
    /// `IDENTIFIER_ON_OTHER_ACCOUNT` answer.
    pub token: String,
    /// Where the offer says `proof_required`: a proof of one of this
    /// account's own identifiers (`POST /v1/me/identifiers/proof`).
    pub proof: Option<String>,
}

/// Combines into the signed-in account the account an offer names. It
/// cannot be undone: that account's place in each of its yups, the address
/// or number proved and what else the offer lists move here, and it can no
/// longer be signed in to. Where it replaces this account's own address or
/// number, it needs `proof`. Every email address either account had is
/// told; with none, the app shows a notice. The offer is good once, for ten
/// minutes, from this account only; a repeat with the same
/// `Idempotency-Key` succeeds and changes nothing.
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
        (status = 409, description = "The offer is used, expired or no longer true (`COMBINE_EXPIRED`), the two cannot be combined (`COMBINE_SUSPENDED`, `COMBINE_REVIEWER`, `COMBINE_SHARED_EXCHANGE`), or the offer needs a proof and none good was given (`PROOF_REQUIRED`)", body = ErrorBody),
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
    let proof = body
        .proof
        .as_deref()
        .map(str::trim)
        .filter(|proof| !proof.is_empty());
    combine::combine(
        &state.db,
        &state.settings.rules,
        session.account_id,
        &body.token,
        proof,
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
    /// The email address or phone number the person says the invitation was
    /// sent to, typed in full.
    pub identifier: String,
    /// For a phone number, required: the box beside it was ticked
    /// (`smsCode.verifyNumber`), with the version and language of the
    /// wording shown. Ignored for an email address.
    pub sms_consent: Option<SmsCodeConsent>,
}

/// Counts an attempt at an invitation's address, by the account and by the
/// invitation, before anything is compared or sent.
async fn count_attempt(state: &AppState, account: Uuid, token: &str) -> Result<(), ApiError> {
    auth::count_invitation_address_attempt(
        &state.db,
        &state.settings.app_secret,
        account,
        &auth::token_hash(token.trim()),
    )
    .await
}

/// Sends a one-time code to the email address or phone number an invitation
/// was sent to, for someone signed in with another who wants to add it to
/// their account and open the invitation (`sent_to` in its preview). The
/// person types the address; unless it is the one the invitation was sent
/// to, nothing is sent and the answer is `NOT_INVITED_ADDRESS`, which says
/// nothing more. Requests are counted by account and by invitation
/// (`TOO_MANY_REQUESTS`), besides the usual limits on codes.
#[utoipa::path(
    post,
    path = "/v1/invitations/address/codes",
    request_body = InvitationAddressCode,
    responses(
        (status = 204, description = "A code was sent"),
        (status = 401, description = "Not signed in", body = ErrorBody),
        (status = 403, description = "The invitation was made before its address was kept: only that address can open it (`INVITATION_NOT_FOR_YOU`)", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer (`INVITATION_UNAVAILABLE`)", body = ErrorBody),
        (status = 409, description = "The invitation names nobody, or this account already (`ACTION_NOT_ALLOWED`), or no code could be sent there (`CODE_NOT_SENT`)", body = ErrorBody),
        (status = 422, description = "Not the address the invitation was sent to (`NOT_INVITED_ADDRESS`), not an address at all (`INVALID_IDENTIFIER`), a phone number without `sms_consent` (`SMS_CONSENT_REQUIRED`), or of a country not served (`PHONE_COUNTRY_NOT_SERVED`)", body = ErrorBody),
        (status = 429, description = "Too many codes or attempts asked for", body = ErrorBody)
    )
)]
pub async fn request_invitation_address_code(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    headers: HeaderMap,
    ApiJson(body): ApiJson<InvitationAddressCode>,
) -> Result<StatusCode, ApiError> {
    let account = session.account_id;
    count_attempt(&state, account, &body.token).await?;
    let identifier = Identifier::parse(&body.identifier)?;
    service::invitation_address(&state.db, account, &body.token, &identifier).await?;
    let (source, user_agent) = client_of(&headers);
    let request = CodeRequest {
        purpose: CodePurpose::VerifyNumber,
        account: Some(account),
        consent: body.sms_consent.as_ref(),
        source,
        address,
        user_agent,
    };
    // In the account's own language: the person asking is the one who will
    // read it.
    let language = load(&state.db, account).await?.language;
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
    .await
    .map_err(|error| match error.code {
        // Whether a number replied STOP is not said to someone else.
        ErrorCode::PhoneOptedOut => ErrorCode::CodeNotSent.into(),
        _ => error,
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddInvitationAddress {
    /// The invitation link's token.
    pub token: String,
    /// The address the code was sent to, as typed for it.
    pub identifier: String,
    /// The code sent to the address the invitation was sent to.
    pub code: String,
    /// The account has another address of this kind (`sent_to.replaces`):
    /// `true` replaces it. Without it such a request is refused, before the
    /// code is looked at, with `IDENTIFIER_KIND_TAKEN`.
    #[serde(default)]
    pub replace: bool,
    /// A proof of one of the account's own identifiers
    /// (`POST /v1/me/identifiers/proof`): adding one takes it. Without a
    /// good one the request is refused, before the code is looked at, with
    /// `PROOF_REQUIRED` (or `IDENTIFIER_TOO_RECENT`).
    pub proof: Option<String>,
}

/// Adds to the signed-in account the email address or phone number an
/// invitation was sent to, with the code sent there, and opens the
/// invitation: the account takes the invited party's place, as named, so
/// nobody has to confirm it. Both happen, or neither does. If the address
/// belongs to another account the answer is `IDENTIFIER_ON_OTHER_ACCOUNT`
/// with the offer to combine the two; once combined, claiming the
/// invitation (`POST /v1/invitations/claim`) opens it.
#[utoipa::path(
    post,
    path = "/v1/invitations/address",
    request_body = AddInvitationAddress,
    responses(
        (status = 200, description = "Added, and the invitation opened", body = ExchangeView),
        (status = 401, description = "Not signed in, or the code is wrong", body = ErrorBody),
        (status = 403, description = "The invitation was made before its address was kept (`INVITATION_NOT_FOR_YOU`)", body = ErrorBody),
        (status = 404, description = "The link is not valid, or no longer", body = ErrorBody),
        (status = 409, description = "The address is another account's (`IDENTIFIER_ON_OTHER_ACCOUNT`, with `combine`); the account has another of this kind and `replace` was not given (`IDENTIFIER_KIND_TAKEN`), or no good `proof` (`PROOF_REQUIRED`, `IDENTIFIER_TOO_RECENT`); or the two accounts cannot be combined", body = CombineOffered),
        (status = 422, description = "Not the address the invitation was sent to (`NOT_INVITED_ADDRESS`)", body = ErrorBody),
        (status = 429, description = "Too many wrong codes or attempts", body = ErrorBody),
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
    count_attempt(&state, account, &body.token).await?;
    let identifier = Identifier::parse(&body.identifier)?;
    let replaces =
        service::invitation_address(&state.db, account, &body.token, &identifier).await?;
    let proof = proof_hash(body.proof.as_deref());
    // Asked before the code is looked at, so that it is not spent on a
    // request that cannot go through. One address of each kind per account,
    // and adding or replacing one takes a proof of the account's own.
    if replaces && !body.replace {
        return Err(ErrorCode::IdentifierKindTaken.into());
    }
    let settings = &state.settings;
    settings.auth.check_taken(&identifier)?;
    check_attach_proof(&state.db, account, &identifier, proof.as_ref()).await?;
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
    // Added and claimed in one transaction: an address is not left added by
    // a claim that failed.
    let mut tx = state.db.begin().await?;
    match attach_in(&mut tx, account, &identifier, proof.as_ref()).await? {
        Attached::Done => {}
        Attached::OnOther(other) => {
            drop(tx);
            let offer = combine::offer(&state.db, account, other, &identifier).await?;
            return Err(IdentifierRefusal::Combine(Box::new(offer)));
        }
    }
    // Now named by it: the claim goes through as for anyone the invitation
    // was sent to.
    let view =
        service::claim_in(&mut tx, &settings.rules, account, &body.token, Claim::Take).await?;
    tx.commit().await?;
    Ok(Json(view))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|e| e.code().as_deref() == Some("23505"))
}
