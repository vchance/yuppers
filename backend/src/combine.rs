//! Combining two accounts into one (migration 0028; README, "Combining
//! accounts"; DESIGN.md §8, §14).
//!
//! **The proof.** Signed in to one account (A), a person adds an email
//! address or phone number and enters the code sent to it. If the address
//! turns out to belong to another account (B), they have now shown that
//! they control both: A by its session, B by the code. Until that code is
//! right, nothing says whether the address has an account; afterwards the
//! answer is an offer ([`CombineOffer`]): what B holds, what combining would
//! do, and a token, kept as its hash, that combines exactly these two
//! accounts by exactly that address, once, within [`OFFER_TTL`].
//!
//! **What is refused.** An account that is suspended, a reviewer's account
//! as B, and two accounts that are both parties to one yup, on its two
//! sides or one as the other's unconfirmed claimant: one person cannot be
//! both parties to an agreement, and the record of who signed what would
//! stop meaning anything.
//!
//! **What moves from B to A**, in one transaction, with both account rows
//! held in a fixed order ([`merge`]):
//!
//! * its place in every yup (`participant`). What B signed and did stays
//!   as it was written, under B: signatures, events and who held a place
//!   (`slot_holding`) are history. The database ends B's holding as
//!   combined and opens one for A, and a signature B gave under the old
//!   holding still counts (`holding_void_since`). Invitation links B took
//!   and B's unsent working copies go with the place;
//! * the address or number proved. If B has the other kind and A does not,
//!   that too; if A has its own, A keeps it and B's is dropped;
//! * text updates B turned on, when B's number comes with it; otherwise
//!   they end, recorded as any other ending (`sms_consent` is never
//!   rewritten: its opt-ins are found through `merged_into`);
//! * payment options, only when A has none: re-encrypted for A, and marked
//!   changed now, so that whoever owes money on B's agreements is warned
//!   that they changed. When A has its own, B's are dropped and so is
//!   showing them on B's agreements;
//! * Wallet passes; content a reviewer hid from B; blocks B made and blocks
//!   made against B, each ending what was waiting to be signed between the
//!   two, as a new block does. Blocks between A and B are dropped: there is
//!   no one left to block;
//! * notifications still waiting for B, re-addressed to A: they are about
//!   yups that are now A's.
//!
//! **What does not move.** B's sessions are revoked and its devices
//! forgotten, as when an account is suspended: a device is registered under
//! the session it was signed in with, and nothing is pushed to one whose
//! session has ended. Rate-limit counts stay where they are. Reports by and
//! about B, and the review history, stay under B; review reads them through
//! `merged_into`.
//!
//! **Afterwards** B is `MERGED`, with no address, no number, no name and
//! `merged_into` A. Nobody can sign in to it. Every email address either
//! account had, as they were before, is told by email ([`notify`]), with
//! nothing about any yup; nothing is texted. Where neither had an email
//! address, A shows the notice in the app instead, once. The combination is in `account_merge`, and in the deletion log
//! with the account it went into, so that replaying the log after a restore
//! combines the two again ([`replay`]).

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{generate_token, token_hash};
use crate::contact::{self, Field, Kind};
use crate::domain::Rules;
use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorCode};
use crate::exchanges::dto::rfc3339;
use crate::exchanges::service::{Idempotency, already_applied};
use crate::notifications::sms_updates::{self, Source};
use crate::payments::{self, COLUMNS as PAYMENT_COLUMNS};
use crate::{safety, wallet};

/// How long an offer to combine lasts.
pub const OFFER_TTL: Duration = Duration::minutes(10);

/// How long a notice that two accounts were combined, and the address it
/// goes to, is kept, sent or not.
pub const NOTICE_RETENTION: Duration = Duration::days(7);

/// A kind of identifier, as the API names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentifierKind {
    Email,
    Phone,
}

impl IdentifierKind {
    pub fn of(kind: Kind) -> Self {
        match kind {
            Kind::Email => IdentifierKind::Email,
            Kind::Phone => IdentifierKind::Phone,
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            IdentifierKind::Email => Kind::Email,
            IdentifierKind::Phone => Kind::Phone,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            IdentifierKind::Email => "EMAIL",
            IdentifierKind::Phone => "PHONE",
        }
    }

    fn parse(text: &str) -> Self {
        if text == "PHONE" {
            IdentifierKind::Phone
        } else {
            IdentifierKind::Email
        }
    }
}

/// An identifier with most of it hidden, as the screens show one they may
/// not show in full: `j•••@gmail.com`, and a phone number as
/// `packages/shared/src/phone.ts` masks it, `(•••) •••-8780` for a +1 number
/// and `•••8780` for any other.
pub fn masked(identifier: &Identifier) -> String {
    match identifier {
        Identifier::Email(_) => identifier.masked(),
        Identifier::Phone(phone) => {
            let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
            let last = &digits[digits.len().saturating_sub(4)..];
            if phone.starts_with("+1") && digits.len() == 11 {
                format!("(•••) •••-{last}")
            } else {
                format!("•••{last}")
            }
        }
    }
}

/// How many yups the other account is in, by where they stand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct YupCounts {
    /// Drafts never sent.
    pub drafts: i64,
    /// Sent, nothing agreed yet.
    pub negotiating: i64,
    /// Agreements in force.
    pub in_force: i64,
    /// Closed, whatever the outcome.
    pub closed: i64,
}

/// What the other account holds, as the person combining it is shown. Its
/// address and number are masked; the person proved one of them, and can
/// sign in to the account and read both anyway.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct OtherAccount {
    /// Its name; empty if it has none.
    pub display_name: String,
    /// Masked: `a•••@example.com`.
    pub email: Option<String>,
    /// Masked.
    pub phone: Option<String>,
    pub yups: YupCounts,
    /// It has saved payment options.
    pub payment_options: bool,
    /// It has text updates on for at least one agreement.
    pub text_updates: bool,
    /// A device of its gets push notifications.
    pub devices: bool,
}

/// What becomes of each kind of identifier when the two are combined, from
/// the point of view of the account that stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentifierOutcome {
    /// Neither account has one.
    None,
    /// This account keeps its own; the other has none.
    Kept,
    /// The other account's becomes this one's; this one has none.
    Added,
    /// The other account's replaces this one's: the one proved.
    Replaced,
    /// This account keeps its own, and the other account's is dropped.
    TheirsDropped,
}

/// What combining another account into this one would do, and the token
/// that does it (`POST /v1/me/combine`). It cannot be undone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct CombineOffer {
    /// Good once, for [`OFFER_TTL`], from this account only.
    pub token: String,
    /// RFC 3339.
    pub expires_at: String,
    /// The kind of identifier proved, which comes to this account.
    pub proved: IdentifierKind,
    pub other: OtherAccount,
    pub email: IdentifierOutcome,
    pub phone: IdentifierOutcome,
    /// The other account's payment options become this one's, because this
    /// one has none. With options of its own, the other's are dropped.
    pub payment_options_move: bool,
    /// The other account's text updates end, because its number is dropped.
    pub text_updates_end: bool,
}

/// The answer when an address proved belongs to another account: the
/// refusal's code, `IDENTIFIER_ON_OTHER_ACCOUNT`, and the offer.
#[derive(Debug, Serialize, ToSchema)]
pub struct CombineOffered {
    pub code: ErrorCode,
    pub combine: CombineOffer,
}

/// An account as a combination reads it.
#[derive(Clone, Debug, sqlx::FromRow)]
struct Held {
    id: Uuid,
    status: String,
    email_encrypted: Option<Vec<u8>>,
    email_index: Option<Vec<u8>>,
    phone_encrypted: Option<Vec<u8>>,
    phone_index: Option<Vec<u8>>,
    display_name: String,
    language: String,
    adult_confirmed_at: Option<OffsetDateTime>,
}

impl Held {
    fn index(&self, kind: Kind) -> Option<&[u8]> {
        match kind {
            Kind::Email => self.email_index.as_deref(),
            Kind::Phone => self.phone_index.as_deref(),
        }
    }

    fn encrypted(&self, kind: Kind) -> Option<&[u8]> {
        match kind {
            Kind::Email => self.email_encrypted.as_deref(),
            Kind::Phone => self.phone_encrypted.as_deref(),
        }
    }

    /// Its identifier of `kind`, decrypted, if it has one.
    fn identifier(&self, kind: Kind) -> Result<Option<Identifier>, contact::Unreadable> {
        let value = contact::keys().reveal(Field::account(kind), self.encrypted(kind))?;
        Ok(value.map(|value| match kind {
            Kind::Email => Identifier::Email(value),
            Kind::Phone => Identifier::Phone(value),
        }))
    }
}

const HELD_COLUMNS: &str = "id, status, email_encrypted, email_index, phone_encrypted, \
                            phone_index, display_name, language, adult_confirmed_at";

async fn held(conn: &mut PgConnection, account: Uuid) -> Result<Option<Held>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HELD_COLUMNS} FROM account WHERE id = $1"
    )))
    .bind(account)
    .fetch_optional(conn)
    .await
}

/// The account that has `identifier` now, whatever its status.
pub async fn owner_of(
    conn: &mut PgConnection,
    identifier: &Identifier,
) -> Result<Option<Uuid>, sqlx::Error> {
    let (_, index) = Kind::of(identifier).account_columns();
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM account WHERE {index} = $1"
    )))
    .bind(contact::keys().index_of(identifier).as_slice())
    .fetch_optional(conn)
    .await
}

/// What each kind of identifier becomes when `other` is combined into
/// `stays` by an identifier of kind `proved`.
fn outcomes(stays: &Held, other: &Held, proved: Kind) -> [(Kind, IdentifierOutcome); 2] {
    [Kind::Email, Kind::Phone].map(|kind| {
        let (ours, theirs) = (stays.index(kind).is_some(), other.index(kind).is_some());
        let outcome = match (ours, theirs) {
            (false, false) => IdentifierOutcome::None,
            (true, false) => IdentifierOutcome::Kept,
            (false, true) => IdentifierOutcome::Added,
            (true, true) if kind == proved => IdentifierOutcome::Replaced,
            (true, true) => IdentifierOutcome::TheirsDropped,
        };
        (kind, outcome)
    })
}

fn outcome_of(outcomes: &[(Kind, IdentifierOutcome); 2], kind: Kind) -> IdentifierOutcome {
    outcomes
        .iter()
        .find(|(each, _)| *each == kind)
        .map_or(IdentifierOutcome::None, |(_, outcome)| *outcome)
}

/// Why two accounts cannot be combined, if they cannot. Asked when the
/// offer is made and again, under the locks, when it is taken.
async fn refusal(
    conn: &mut PgConnection,
    stays: &Held,
    other: &Held,
) -> Result<Option<ErrorCode>, sqlx::Error> {
    match (stays.status.as_str(), other.status.as_str()) {
        ("ACTIVE", "ACTIVE") => {}
        ("SUSPENDED", _) | (_, "SUSPENDED") => return Ok(Some(ErrorCode::CombineSuspended)),
        ("ACTIVE", _) => return Ok(Some(ErrorCode::CombineExpired)),
        _ => return Ok(Some(ErrorCode::Unauthenticated)),
    }
    let reviewer: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM staff_member WHERE account_id = $1)")
            .bind(other.id)
            .fetch_one(&mut *conn)
            .await?;
    if reviewer {
        return Ok(Some(ErrorCode::CombineReviewer));
    }
    // On the two sides of one yup: one as initiator, the other in the
    // invited place, claimed and confirmed or not.
    let shared: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM participant p
                        JOIN participant q ON q.exchange_id = p.exchange_id
                        WHERE p.account_id = $1 AND q.account_id = $2)",
    )
    .bind(stays.id)
    .bind(other.id)
    .fetch_one(&mut *conn)
    .await?;
    if shared {
        return Ok(Some(ErrorCode::CombineSharedExchange));
    }
    Ok(None)
}

/// Offers to combine `other` into `account`, for an identifier of `other`'s
/// whose code `account` has just entered. Refused with the reason when the
/// two cannot be combined; otherwise the token is stored and returned once.
pub async fn offer(
    db: &PgPool,
    account: Uuid,
    other: Uuid,
    proved: &Identifier,
) -> Result<CombineOffer, ApiError> {
    let mut tx = db.begin().await?;
    let stays = held(&mut tx, account)
        .await?
        .ok_or(ErrorCode::Unauthenticated)?;
    let combined = held(&mut tx, other)
        .await?
        .ok_or(ErrorCode::CombineExpired)?;
    if let Some(code) = refusal(&mut tx, &stays, &combined).await? {
        return Err(code.into());
    }
    let kind = Kind::of(proved);
    let index = contact::keys().index_of(proved);
    // Proved a moment ago; but it may have moved since.
    if combined.index(kind) != Some(index.as_slice()) {
        return Err(ErrorCode::CombineExpired.into());
    }

    let (drafts, negotiating, in_force, closed): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE e.state = 'DRAFT'),
                count(*) FILTER (WHERE e.state = 'NEGOTIATING'),
                count(*) FILTER (WHERE e.state = 'ACTIVE'),
                count(*) FILTER (WHERE e.state = 'CLOSED')
         FROM participant p JOIN exchange e ON e.id = p.exchange_id
         WHERE p.account_id = $1",
    )
    .bind(other)
    .fetch_one(&mut *tx)
    .await?;
    let has_options = |id: Uuid| {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM payment_handle
                            WHERE account_id = $1
                              AND num_nonnulls(venmo_encrypted, cash_app_encrypted,
                                               paypal_encrypted, zelle_encrypted) > 0)",
        )
        .bind(id)
    };
    let their_options = has_options(other).fetch_one(&mut *tx).await?;
    let our_options = has_options(account).fetch_one(&mut *tx).await?;
    let text_updates: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sms_update WHERE account_id = $1)")
            .bind(other)
            .fetch_one(&mut *tx)
            .await?;
    let devices: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM device d JOIN account_session s ON s.id = d.session_id
                        WHERE d.account_id = $1 AND s.revoked_at IS NULL
                          AND s.expires_at > now())",
    )
    .bind(other)
    .fetch_one(&mut *tx)
    .await?;

    let outcomes = outcomes(&stays, &combined, kind);
    let phone_kept = matches!(
        outcome_of(&outcomes, Kind::Phone),
        IdentifierOutcome::Added | IdentifierOutcome::Replaced
    );
    let masked = |kind| -> Result<Option<String>, ApiError> {
        Ok(combined.identifier(kind)?.map(|found| masked(&found)))
    };

    let token = generate_token();
    let at = OffsetDateTime::now_utc();
    let expires_at = at + OFFER_TTL;
    // Offers no longer good for anything go as new ones are made.
    sqlx::query("DELETE FROM account_combine_offer WHERE expires_at < now() - interval '1 day'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO account_combine_offer
             (token_hash, account_id, other_account_id, identifier_kind, identifier_index,
              created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(token_hash(&token).as_slice())
    .bind(account)
    .bind(other)
    .bind(IdentifierKind::of(kind).as_str())
    .bind(index.as_slice())
    .bind(at)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(CombineOffer {
        token,
        expires_at: rfc3339(expires_at),
        proved: IdentifierKind::of(kind),
        other: OtherAccount {
            display_name: combined.display_name.clone(),
            email: masked(Kind::Email)?,
            phone: masked(Kind::Phone)?,
            yups: YupCounts {
                drafts,
                negotiating,
                in_force,
                closed,
            },
            payment_options: their_options,
            text_updates,
            devices,
        },
        email: outcome_of(&outcomes, Kind::Email),
        phone: outcome_of(&outcomes, Kind::Phone),
        payment_options_move: their_options && !our_options,
        text_updates_end: text_updates && !phone_kept,
    })
}

// ---- Combining ---------------------------------------------------------------

/// How often a combination is tried before giving up, and the first wait
/// between tries, which doubles: as for a deletion (`crate::deletion`).
const ATTEMPTS: u32 = 6;
const FIRST_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

/// Who asked for a combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// The person, with an offer.
    Person,
    /// A replay of the deletion log after a restore, at the time it first
    /// happened.
    Replay(OffsetDateTime),
}

impl Origin {
    fn as_str(self) -> &'static str {
        match self {
            Origin::Person => "PERSON",
            Origin::Replay(_) => "REPLAY",
        }
    }
}

enum Attempt {
    Done,
    /// Another transaction held one of the account rows; nothing was done.
    Busy,
}

fn is_retryable(error: &sqlx::Error) -> bool {
    // The row was not free (`NOWAIT`), or the database broke a deadlock with
    // this transaction: tried again from the start.
    error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "55P03" || code == "40P01")
}

/// Combines into `account` the account its offer `token` names, as the
/// person signed in to `account` asked. With an idempotency key, a repeat
/// of a request that went through changes nothing and succeeds.
pub async fn combine(
    db: &PgPool,
    rules: &Rules,
    account: Uuid,
    token: &str,
    idempotency: &Idempotency<'_>,
) -> Result<(), ApiError> {
    let hash = token_hash(token.trim());
    let mut wait = FIRST_WAIT;
    for _ in 0..ATTEMPTS {
        let mut tx = db.begin().await?;
        if already_applied(&mut tx, account, idempotency).await? {
            tx.commit().await?;
            return Ok(());
        }
        let offer: Option<OfferRow> = sqlx::query_as(
            "SELECT account_id, other_account_id, identifier_kind, identifier_index,
                        expires_at, used_at
                 FROM account_combine_offer WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        let Some((owner, other, kind, index, expires_at, used_at)) = offer else {
            return Err(ErrorCode::CombineExpired.into());
        };
        // Bound to the account it was made for: anyone else holding the
        // token is told only that there is no such offer.
        if owner != account || used_at.is_some() || expires_at <= OffsetDateTime::now_utc() {
            return Err(ErrorCode::CombineExpired.into());
        }
        let kind = IdentifierKind::parse(&kind).kind();
        match merge(
            &mut tx,
            rules,
            account,
            other,
            kind,
            Some(&index),
            Origin::Person,
        )
        .await
        {
            Ok(Attempt::Done) => {}
            Ok(Attempt::Busy) => {
                drop(tx);
                tokio::time::sleep(wait).await;
                wait *= 2;
                continue;
            }
            Err(Failure::Refused(code)) => return Err(code.into()),
            Err(Failure::Database(error)) if is_retryable(&error) => {
                drop(tx);
                tokio::time::sleep(wait).await;
                wait *= 2;
                continue;
            }
            Err(Failure::Database(error)) => return Err(error.into()),
        }
        sqlx::query("UPDATE account_combine_offer SET used_at = now() WHERE token_hash = $1")
            .bind(hash.as_slice())
            .execute(&mut *tx)
            .await?;
        match tx.commit().await {
            Ok(()) => return Ok(()),
            Err(error) if is_retryable(&error) => {
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(ErrorCode::ServiceUnavailable.into())
}

/// Why [`merge`] did not combine the two.
enum Failure {
    Refused(ErrorCode),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        Failure::Database(error)
    }
}

impl From<contact::Unreadable> for Failure {
    fn from(error: contact::Unreadable) -> Self {
        Failure::Database(error.into())
    }
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        Failure::Refused(error.code)
    }
}

/// Combines `other` into `stays`, in the caller's transaction, by an
/// identifier of `kind`; with `proved`, only while `other` still has that
/// one. Everything moves or ends as the module says, or nothing does.
async fn merge(
    conn: &mut PgConnection,
    rules: &Rules,
    stays: Uuid,
    other: Uuid,
    kind: Kind,
    proved: Option<&[u8]>,
    source: Origin,
) -> Result<Attempt, Failure> {
    // Both rows, in a fixed order, so that two combinations of the same pair
    // at once, either way round, take turns. This strength of lock leaves
    // alone transactions that merely refer to an account (the other party
    // of a yup queueing a message for it), which may hold an exchange this
    // one needs below; the rows are taken outright, without waiting, only
    // where their addresses change ([`free`]).
    let rows: Vec<Held> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HELD_COLUMNS} FROM account WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE"
    )))
    .bind([stays, other].as_slice())
    .fetch_all(&mut *conn)
    .await?;
    let find = |id: Uuid| rows.iter().find(|row| row.id == id).cloned();
    let (Some(a), Some(b)) = (find(stays), find(other)) else {
        return Err(Failure::Refused(ErrorCode::CombineExpired));
    };
    if let Some(code) = refusal(conn, &a, &b).await? {
        return Err(Failure::Refused(code));
    }
    if let Some(proved) = proved
        && b.index(kind) != Some(proved)
    {
        return Err(Failure::Refused(ErrorCode::CombineExpired));
    }
    let at = match source {
        Origin::Person => OffsetDateTime::now_utc(),
        Origin::Replay(at) => at,
    };

    // B's yups, in order, locked as every other change locks them.
    let exchanges: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM exchange e JOIN participant p ON p.exchange_id = e.id
         WHERE p.account_id = $1 ORDER BY e.id FOR UPDATE OF e",
    )
    .bind(other)
    .fetch_all(&mut *conn)
    .await?;

    // Whom to tell, as both accounts stand before anything changes.
    let mut told: Vec<Identifier> = Vec::new();
    if source == Origin::Person {
        for account in [&a, &b] {
            for kind in [Kind::Email, Kind::Phone] {
                if let Some(found) = account.identifier(kind)? {
                    told.push(found);
                }
            }
        }
    }

    // The addresses change below, which needs both rows outright. Asked once
    // each; if a row is not free, everything is undone and tried again.
    if !free(conn, &[stays, other]).await? {
        return Ok(Attempt::Busy);
    }

    // What A ends with, kind by kind: the one proved is B's; the other is
    // A's own if it has one, else B's.
    let pick = |kind: Kind, theirs_first: bool| -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let ours = (a.encrypted(kind), a.index(kind));
        let theirs = (b.encrypted(kind), b.index(kind));
        let (first, second) = if theirs_first {
            (theirs, ours)
        } else {
            (ours, theirs)
        };
        let chosen = if first.1.is_some() { first } else { second };
        (chosen.0.map(<[u8]>::to_vec), chosen.1.map(<[u8]>::to_vec))
    };
    let (email_encrypted, email_index) = pick(Kind::Email, kind == Kind::Email);
    let (phone_encrypted, phone_index) = pick(Kind::Phone, kind == Kind::Phone);
    let phone_from_b = phone_index.is_some() && phone_index.as_deref() == b.index(Kind::Phone);
    let phone_from_a = phone_index.is_some() && phone_index.as_deref() == a.index(Kind::Phone);

    // B first: its addresses go, which frees them for A. Accounts combined
    // into B before now point at A, so `merged_into` is always one step.
    sqlx::query(
        "UPDATE account
         SET status = 'MERGED', merged_into = $2, merged_at = $3,
             email_encrypted = NULL, email_index = NULL,
             phone_encrypted = NULL, phone_index = NULL, display_name = ''
         WHERE id = $1",
    )
    .bind(other)
    .bind(stays)
    .bind(at)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE account SET merged_into = $2 WHERE merged_into = $1")
        .bind(other)
        .bind(stays)
        .execute(&mut *conn)
        .await?;
    // A keeps its name and age confirmation, and takes B's where it has
    // none: the same person gave both.
    sqlx::query(
        "UPDATE account
         SET email_encrypted = $2, email_index = $3, phone_encrypted = $4, phone_index = $5,
             display_name = CASE WHEN display_name = '' THEN $6 ELSE display_name END,
             adult_confirmed_at = coalesce(adult_confirmed_at, $7)
         WHERE id = $1",
    )
    .bind(stays)
    .bind(&email_encrypted)
    .bind(&email_index)
    .bind(&phone_encrypted)
    .bind(&phone_index)
    .bind(&b.display_name)
    .bind(b.adult_confirmed_at)
    .execute(&mut *conn)
    .await?;

    // Text updates, before the places move: A's own end if its number was
    // replaced; B's move with its number, or end without it.
    if !phone_from_a {
        let keep: Option<[u8; 32]> = phone_index.as_deref().and_then(|i| i.try_into().ok());
        sms_updates::forget_numbers(conn, stays, keep.as_ref(), Source::PhoneChanged).await?;
    }
    if phone_from_b {
        sqlx::query("UPDATE sms_update SET account_id = $2 WHERE account_id = $1")
            .bind(other)
            .bind(stays)
            .execute(&mut *conn)
            .await?;
    } else {
        sms_updates::forget_numbers(conn, other, None, Source::AccountsCombined).await?;
    }

    // B's places, which the database lets pass only to the account B was
    // combined into, ending B's holdings as combined (migration 0028).
    sqlx::query("UPDATE participant SET account_id = $2 WHERE account_id = $1")
        .bind(other)
        .bind(stays)
        .execute(&mut *conn)
        .await?;
    // The links B took, so that A is taken back to those yups as B was.
    sqlx::query("UPDATE invitation SET claimed_by = $2 WHERE claimed_by = $1")
        .bind(other)
        .bind(stays)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE exchange_draft SET account_id = $2 WHERE account_id = $1")
        .bind(other)
        .bind(stays)
        .execute(&mut *conn)
        .await?;

    move_payment_options(conn, stays, other, at).await?;

    sqlx::query("UPDATE wallet_pass SET account_id = $2, updated_at = now() WHERE account_id = $1")
        .bind(other)
        .bind(stays)
        .execute(&mut *conn)
        .await?;
    // What a reviewer hid from B stays hidden from whoever reads as B now.
    sqlx::query(
        "WITH gone AS (DELETE FROM hidden_content WHERE account_id = $1
                       RETURNING exchange_id, report_id, hidden_at)
         INSERT INTO hidden_content (exchange_id, account_id, report_id, hidden_at)
         SELECT exchange_id, $2, report_id, hidden_at FROM gone
         ON CONFLICT DO NOTHING",
    )
    .bind(other)
    .bind(stays)
    .execute(&mut *conn)
    .await?;

    // B's sessions end; its devices were registered under them.
    sqlx::query("DELETE FROM device WHERE account_id = $1")
        .bind(other)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE account_session SET revoked_at = now()
         WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(other)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM idempotency_key WHERE account_id = $1")
        .bind(other)
        .execute(&mut *conn)
        .await?;
    // Messages waiting for B are about yups that are A's now. One the worker
    // is sending at this moment goes as it was.
    sqlx::query(
        "UPDATE outbox SET recipient_account_id = $2
         WHERE id IN (SELECT id FROM outbox
                      WHERE recipient_account_id = $1 AND completed_at IS NULL
                      FOR UPDATE SKIP LOCKED)",
    )
    .bind(other)
    .bind(stays)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM account_combine_offer
         WHERE (account_id = $1 OR other_account_id = $1) AND used_at IS NULL",
    )
    .bind(other)
    .execute(&mut *conn)
    .await?;

    move_blocks(conn, rules, stays, other).await?;

    for exchange in &exchanges {
        wallet::store::mark_exchange_changed(conn, *exchange).await?;
    }

    sqlx::query(
        "INSERT INTO account_merge
             (merged_account_id, into_account_id, identifier_kind, source, merged_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(other)
    .bind(stays)
    .bind(IdentifierKind::of(kind).as_str())
    .bind(source.as_str())
    .bind(at)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO deletion_log (account_id, deleted_at, merged_into, merged_by)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(other)
    .bind(at)
    .bind(stays)
    .bind(IdentifierKind::of(kind).as_str())
    .execute(&mut *conn)
    .await?;

    notify(conn, stays, &a.language, &told).await?;
    Ok(Attempt::Done)
}

/// Takes the rows outright, without waiting: `false` if one is not free.
async fn free(conn: &mut PgConnection, accounts: &[Uuid]) -> Result<bool, sqlx::Error> {
    let taken =
        sqlx::query("SELECT 1 FROM account WHERE id = ANY($1) ORDER BY id FOR UPDATE NOWAIT")
            .bind(accounts)
            .execute(&mut *conn)
            .await;
    match taken {
        Ok(_) => Ok(true),
        Err(error) if is_retryable(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Payment options: A keeps its own. With none, B's become A's, encrypted
/// again for A, marked changed now, and shown on the agreements B showed
/// them on. Otherwise B's are dropped, and so is showing them.
async fn move_payment_options(
    conn: &mut PgConnection,
    stays: Uuid,
    other: Uuid,
    at: OffsetDateTime,
) -> Result<(), Failure> {
    let ours = payments::load(conn, stays).await?;
    let theirs = payments::load(conn, other).await?;
    let shown: Vec<Uuid> =
        sqlx::query_scalar("DELETE FROM payment_offer WHERE account_id = $1 RETURNING exchange_id")
            .bind(other)
            .fetch_all(&mut *conn)
            .await?;
    if ours.is_empty() && !theirs.is_empty() {
        let keys = contact::keys();
        let values = [
            &theirs.venmo,
            &theirs.cash_app,
            &theirs.paypal,
            &theirs.zelle,
        ];
        let sealed: Vec<Option<Vec<u8>>> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_ref()
                    .map(|value| keys.seal(PAYMENT_COLUMNS[index].1.owned_by(stays), value))
            })
            .collect();
        // Changed now, whatever B's said: the payer of an agreement in force
        // is warned that the name they pay changed.
        let changed: Vec<Option<OffsetDateTime>> = values
            .iter()
            .map(|value| value.as_ref().map(|_| at))
            .collect();
        sqlx::query(
            "INSERT INTO payment_handle AS h
                 (account_id, venmo_encrypted, cash_app_encrypted, paypal_encrypted,
                  zelle_encrypted, venmo_changed_at, cash_app_changed_at, paypal_changed_at,
                  zelle_changed_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
             ON CONFLICT (account_id) DO UPDATE SET
                 venmo_encrypted = EXCLUDED.venmo_encrypted,
                 cash_app_encrypted = EXCLUDED.cash_app_encrypted,
                 paypal_encrypted = EXCLUDED.paypal_encrypted,
                 zelle_encrypted = EXCLUDED.zelle_encrypted,
                 venmo_changed_at = EXCLUDED.venmo_changed_at,
                 cash_app_changed_at = EXCLUDED.cash_app_changed_at,
                 paypal_changed_at = EXCLUDED.paypal_changed_at,
                 zelle_changed_at = EXCLUDED.zelle_changed_at,
                 updated_at = now()",
        )
        .bind(stays)
        .bind(&sealed[0])
        .bind(&sealed[1])
        .bind(&sealed[2])
        .bind(&sealed[3])
        .bind(changed[0])
        .bind(changed[1])
        .bind(changed[2])
        .bind(changed[3])
        .execute(&mut *conn)
        .await?;
        for exchange in shown {
            sqlx::query(
                "INSERT INTO payment_offer (exchange_id, account_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(exchange)
            .bind(stays)
            .execute(&mut *conn)
            .await?;
        }
    }
    payments::forget(conn, other).await?;
    Ok(())
}

/// Blocks between the two go. Every other block B made, or that was made
/// against B, is A's now, and ends what was waiting to be signed between
/// the two people, in the blocker's name, as a new block does
/// (`crate::safety`).
async fn move_blocks(
    conn: &mut PgConnection,
    rules: &Rules,
    stays: Uuid,
    other: Uuid,
) -> Result<(), Failure> {
    sqlx::query(
        "DELETE FROM account_block
         WHERE (blocker_account_id = $1 AND blocked_account_id = $2)
            OR (blocker_account_id = $2 AND blocked_account_id = $1)",
    )
    .bind(stays)
    .bind(other)
    .execute(&mut *conn)
    .await?;
    let made: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM account_block WHERE blocker_account_id = $1 RETURNING blocked_account_id",
    )
    .bind(other)
    .fetch_all(&mut *conn)
    .await?;
    let against: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM account_block WHERE blocked_account_id = $1 RETURNING blocker_account_id",
    )
    .bind(other)
    .fetch_all(&mut *conn)
    .await?;
    let mut pairs: Vec<(Uuid, Uuid)> = made.into_iter().map(|them| (stays, them)).collect();
    pairs.extend(against.into_iter().map(|them| (them, stays)));
    for (blocker, blocked) in pairs {
        let added = sqlx::query(
            "INSERT INTO account_block (blocker_account_id, blocked_account_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if added == 1 {
            safety::end_open_proposals(conn, rules, blocker, blocked).await?;
        }
    }
    Ok(())
}

/// Queues the notice that two accounts were combined, by email, to every
/// email address either had. It says that, and nothing about any yup. Each
/// address is kept encrypted, bound to its row, until the worker has sent
/// it ([`purge`]). Phone numbers are not texted: the SMS program covers
/// agreement updates only. Where neither account had an email address,
/// nothing goes outside, and the combined account shows the notice in the
/// app instead, once (`combined_notice_at`, dismissed through `PATCH /v1/me`).
async fn notify(
    conn: &mut PgConnection,
    account: Uuid,
    language: &str,
    told: &[Identifier],
) -> Result<(), sqlx::Error> {
    let keys = contact::keys();
    let emails: Vec<&str> = told
        .iter()
        .filter_map(|identifier| match identifier {
            Identifier::Email(email) => Some(email.as_str()),
            Identifier::Phone(_) => None,
        })
        .collect();
    if emails.is_empty() {
        sqlx::query("UPDATE account SET combined_notice_at = now() WHERE id = $1")
            .bind(account)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    for email in emails {
        let id: i64 = sqlx::query_scalar("SELECT nextval('combine_notice_id_seq')")
            .fetch_one(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO combine_notice (id, account_id, email_encrypted, language)
             OVERRIDING SYSTEM VALUE VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(account)
        .bind(keys.seal(Field::COMBINE_NOTICE_EMAIL.row(id), email))
        .bind(language)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO outbox (kind, recipient_account_id, payload) VALUES ('EMAIL', $1, $2)",
        )
        .bind(account)
        .bind(json!({ NOTICE_PAYLOAD: id }))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// A notice as stored: its address, encrypted, and the language.
type NoticeRow = (Vec<u8>, String);

/// An offer as stored: for whom, which account, by which kind and index,
/// until when, and when it was used.
type OfferRow = (
    Uuid,
    Uuid,
    String,
    Vec<u8>,
    OffsetDateTime,
    Option<OffsetDateTime>,
);

/// The key of an outbox payload that names a notice: `{"combine_notice": 7}`.
pub const NOTICE_PAYLOAD: &str = "combine_notice";

/// Where a queued notice goes and in which language: an email address,
/// decrypted to send to it. `None` once it is gone.
pub async fn notice_destination(
    conn: &mut PgConnection,
    id: i64,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row: Option<NoticeRow> =
        sqlx::query_as("SELECT email_encrypted, language FROM combine_notice WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    let Some((sealed, language)) = row else {
        return Ok(None);
    };
    let email = contact::keys().open(Field::COMBINE_NOTICE_EMAIL.row(id), &sealed)?;
    Ok(Some((email, language)))
}

/// Removes notices older than [`NOTICE_RETENTION`], sent or not, with the
/// addresses they held, and offers to combine that ended more than a day
/// ago. Returns how many rows went. Called by the worker.
pub async fn purge(db: &PgPool) -> Result<u64, sqlx::Error> {
    let notices = sqlx::query(
        "DELETE FROM combine_notice WHERE created_at < now() - $1 * interval '1 second'",
    )
    .bind(NOTICE_RETENTION.whole_seconds() as f64)
    .execute(db)
    .await?
    .rows_affected();
    let offers = sqlx::query(
        "DELETE FROM account_combine_offer WHERE expires_at < now() - interval '1 day'",
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(notices + offers)
}

// ---- Replaying ----------------------------------------------------------------

/// What [`replay`] found and did for one combined account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replayed {
    /// It was live in this database, and is now combined again.
    Combined,
    /// It was combined here already.
    AlreadyCombined,
    /// This database never held it.
    NotHere,
    /// The account it was combined into is not in this database (it was
    /// made after the backup): it is left as it is, the person's account
    /// as it stood then.
    TargetNotHere,
    /// This database contradicts the line: the account is deleted or
    /// suspended here, the one it went into is not live, or the two could
    /// not be combined. Nothing was done.
    Contradicted,
}

/// Combines again, in a database restored from a backup, an account the
/// deletion log says was combined into `into` at `at` by an identifier of
/// `kind`. Through the same code as the person's combination, so every rule
/// runs again; nobody is told again.
pub async fn replay(
    db: &PgPool,
    rules: &Rules,
    account: Uuid,
    into: Uuid,
    kind: IdentifierKind,
    at: OffsetDateTime,
) -> Result<Replayed, ApiError> {
    let found: Option<(String, Option<Uuid>)> =
        sqlx::query_as("SELECT status, merged_into FROM account WHERE id = $1")
            .bind(account)
            .fetch_optional(db)
            .await?;
    match found
        .as_ref()
        .map(|(status, into)| (status.as_str(), *into))
    {
        None => return Ok(Replayed::NotHere),
        Some(("MERGED", _)) => return Ok(Replayed::AlreadyCombined),
        Some(("ACTIVE", _)) => {}
        Some(_) => return Ok(Replayed::Contradicted),
    }
    // Where `into` itself went since, if anywhere.
    let target: Option<(String, Option<Uuid>)> =
        sqlx::query_as("SELECT status, merged_into FROM account WHERE id = $1")
            .bind(into)
            .fetch_optional(db)
            .await?;
    let into = match target {
        None => return Ok(Replayed::TargetNotHere),
        Some((status, _)) if status == "ACTIVE" => into,
        Some((status, Some(root))) if status == "MERGED" => root,
        Some(_) => return Ok(Replayed::Contradicted),
    };
    let mut wait = FIRST_WAIT;
    for _ in 0..ATTEMPTS {
        let mut tx = db.begin().await?;
        match merge(
            &mut tx,
            rules,
            into,
            account,
            kind.kind(),
            None,
            Origin::Replay(at),
        )
        .await
        {
            Ok(Attempt::Done) => match tx.commit().await {
                Ok(()) => return Ok(Replayed::Combined),
                Err(error) if is_retryable(&error) => {}
                Err(error) => return Err(error.into()),
            },
            Ok(Attempt::Busy) => {}
            Err(Failure::Refused(_)) => return Ok(Replayed::Contradicted),
            Err(Failure::Database(error)) if is_retryable(&error) => {}
            Err(Failure::Database(error)) => return Err(error.into()),
        }
        tokio::time::sleep(wait).await;
        wait *= 2;
    }
    Err(ErrorCode::ServiceUnavailable.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: bool, phone: bool) -> Held {
        Held {
            id: Uuid::new_v4(),
            status: "ACTIVE".into(),
            email_encrypted: email.then(|| vec![1]),
            email_index: email.then(|| vec![1; 32]),
            phone_encrypted: phone.then(|| vec![2]),
            phone_index: phone.then(|| vec![2; 32]),
            display_name: String::new(),
            language: "en".into(),
            adult_confirmed_at: None,
        }
    }

    #[test]
    fn the_identifier_proved_comes_and_the_other_kind_comes_only_where_there_is_none() {
        use IdentifierOutcome::*;
        // A has a phone; B has an email (proved) and a phone.
        let found = outcomes(&account(false, true), &account(true, true), Kind::Email);
        assert_eq!(found, [(Kind::Email, Added), (Kind::Phone, TheirsDropped)]);
        // A has both; B's email is proved: it replaces A's.
        let found = outcomes(&account(true, true), &account(true, false), Kind::Email);
        assert_eq!(found, [(Kind::Email, Replaced), (Kind::Phone, Kept)]);
        // A has an email; B has a phone (proved) and an email.
        let found = outcomes(&account(true, false), &account(true, true), Kind::Phone);
        assert_eq!(found, [(Kind::Email, TheirsDropped), (Kind::Phone, Added)]);
        // Neither has a phone.
        let found = outcomes(&account(true, false), &account(true, false), Kind::Email);
        assert_eq!(found, [(Kind::Email, Replaced), (Kind::Phone, None)]);
    }
}
