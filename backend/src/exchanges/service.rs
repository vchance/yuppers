//! What the exchange endpoints do: check who is asking, run the domain rules,
//! and store the result, all in one transaction per request.

use std::collections::HashSet;

use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::dto::{
    Claimant, CommandDto, Consent, CreateExchange, ExchangeSummary, ExchangeView, InvitationIssued,
    InvitationOptions, InvitationPreview, PaymentOptionsView, RevisionSent, RevisionView,
    RunCommand, SendRevision, ViewContext, rfc3339, state_dto,
};
use super::repo::{self, Aggregate, NewRevision};
use crate::auth;
use crate::contact::{self, Field};
use crate::domain::Rules;
use crate::domain::canonical::content_hash;
use crate::domain::contribution::{Action, Status};
use crate::domain::exchange::{self, Actor, Command, Counterparty, Decision, Event, State, decide};
use crate::domain::identity::Identifier;
use crate::domain::invitation;
use crate::domain::revision::{ContributionId, Kind, Revision, RevisionId, Settlement, Slot};
use crate::domain::risk::{Tier, required_tier};
use crate::error::{ApiError, ErrorCode, Redacted};
use crate::http::Settings;
use crate::http::extract::Session;
use crate::languages;
use crate::payments;

/// A request body's digest together with the caller's idempotency key.
pub struct Idempotency<'a> {
    pub key: Option<&'a str>,
    pub digest: [u8; 32],
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

fn is_violation(error: &sqlx::Error, sqlstate: &str) -> bool {
    error
        .as_database_error()
        .is_some_and(|e| e.code().as_deref() == Some(sqlstate))
}

// ---- Shared steps -----------------------------------------------------------

/// Loads an exchange for one of its participants. Anyone else is told it
/// does not exist.
async fn open_for(
    conn: &mut PgConnection,
    id: Uuid,
    account: Uuid,
    lock: bool,
) -> Result<(Aggregate, Slot), ApiError> {
    if lock {
        acting(conn, account).await?;
    }
    let aggregate = repo::load(conn, id, lock)
        .await?
        .ok_or(ErrorCode::NotFound)?;
    let slot = aggregate.slot_of(account).ok_or(ErrorCode::NotFound)?;
    Ok((aggregate, slot))
}

/// Holds the acting account as it is until the transaction ends, and
/// refuses one that is no longer active. A request is matched to its account
/// before its transaction starts; the account may be deleted in between
/// (`crate::deletion`), and what a deleted account's last request did would
/// otherwise outlive it. Taken before the exchange is locked, which is the
/// order the deletion takes the two in.
pub(crate) async fn acting(conn: &mut PgConnection, account: Uuid) -> Result<(), ApiError> {
    let active: Option<bool> =
        sqlx::query_scalar("SELECT status = 'ACTIVE' FROM account WHERE id = $1 FOR SHARE")
            .bind(account)
            .fetch_optional(&mut *conn)
            .await?;
    if active == Some(true) {
        Ok(())
    } else {
        Err(ErrorCode::Unauthenticated.into())
    }
}

/// Records the idempotency key in the same transaction as the change it
/// guards. Returns `true` when this key has already been applied, in which
/// case the caller answers with the current state instead of acting again.
async fn already_applied(
    conn: &mut PgConnection,
    account: Uuid,
    idempotency: &Idempotency<'_>,
) -> Result<bool, ApiError> {
    let Some(key) = idempotency.key else {
        return Ok(false);
    };
    let inserted = sqlx::query(
        "INSERT INTO idempotency_key (account_id, key, request_hash, response_status)
         VALUES ($1, $2, $3, 200)
         ON CONFLICT (account_id, key) DO NOTHING",
    )
    .bind(account)
    .bind(key)
    .bind(idempotency.digest.as_slice())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 1 {
        return Ok(false);
    }

    let stored: Vec<u8> = sqlx::query_scalar(
        "SELECT request_hash FROM idempotency_key WHERE account_id = $1 AND key = $2",
    )
    .bind(account)
    .bind(key)
    .fetch_one(&mut *conn)
    .await?;
    if stored == idempotency.digest {
        Ok(true)
    } else {
        Err(ErrorCode::IdempotencyKeyReused.into())
    }
}

/// Signing needs a named adult and the current consent wording
/// (DESIGN.md §14.1).
async fn require_signer(
    conn: &mut PgConnection,
    session: &Session,
    consent: &Consent,
    settings: &Settings,
) -> Result<&'static str, ApiError> {
    let (display_name, adult): (String, bool) = sqlx::query_as(
        "SELECT display_name, adult_confirmed_at IS NOT NULL FROM account WHERE id = $1",
    )
    .bind(session.account_id)
    .fetch_one(&mut *conn)
    .await?;
    if display_name.is_empty() || !adult {
        return Err(ErrorCode::ProfileIncomplete.into());
    }
    if consent.version != settings.consent_version {
        return Err(ErrorCode::ConsentOutdated.into());
    }
    // The record must name a language the consent wording exists in.
    languages::resolve(&consent.language).ok_or_else(|| ErrorCode::InvalidRequest.into())
}

/// Refuses a party changing one exchange faster than a person would (§9).
/// Without it one party could flood the permanent history, or change the
/// exchange so often that the other can never act on what they last saw.
/// Called with the exchange locked.
async fn within_change_rate(
    conn: &mut PgConnection,
    exchange: Uuid,
    slot: Slot,
    rules: &Rules,
) -> Result<(), ApiError> {
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exchange_event
         WHERE exchange_id = $1 AND actor_slot = $2
           AND occurred_at > now() - interval '1 minute'",
    )
    .bind(exchange)
    .bind(slot.as_str())
    .fetch_one(&mut *conn)
    .await?;
    if recent >= rules.changes_per_minute {
        return Err(ErrorCode::TooManyRequests.into());
    }
    Ok(())
}

/// Sets the participant rows to the names a revision gives the parties.
async fn name_participants(
    conn: &mut PgConnection,
    exchange: Uuid,
    revision: &Revision,
) -> Result<(), sqlx::Error> {
    for (slot, name) in [("A", &revision.party_a), ("B", &revision.party_b)] {
        sqlx::query(
            "UPDATE participant SET display_name = $3 WHERE exchange_id = $1 AND slot = $2",
        )
        .bind(exchange)
        .bind(slot)
        .bind(name)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Where a request came from. Kept with a signature made in it, apart from
/// the signature itself, so it can be purged later (DESIGN.md §8, §14).
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestOrigin<'a> {
    /// The requester's network address, as far as it can be known
    /// (`crate::http::ClientAddress`).
    pub address: Option<std::net::IpAddr>,
    pub user_agent: Option<&'a str>,
}

struct Signature<'a> {
    exchange: Uuid,
    revision: Uuid,
    slot: Slot,
    content_hash: &'a [u8],
    /// The supported language the consent wording was shown in.
    consent_language: &'a str,
    consent_version: &'a str,
    origin: RequestOrigin<'a>,
}

async fn record_signature(
    conn: &mut PgConnection,
    session: &Session,
    signature: Signature<'_>,
    at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let acceptance: Uuid = sqlx::query_scalar(
        "INSERT INTO acceptance
            (exchange_id, revision_id, slot, account_id, content_hash, auth_method,
             authenticated_at, consent_language, consent_version, accepted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id",
    )
    .bind(signature.exchange)
    .bind(signature.revision)
    .bind(signature.slot.as_str())
    .bind(session.account_id)
    .bind(signature.content_hash)
    .bind(&session.auth_method)
    .bind(session.authenticated_at)
    .bind(signature.consent_language)
    .bind(signature.consent_version)
    .bind(at)
    .fetch_one(&mut *conn)
    .await?;

    // Kept apart so it can be purged after 90 days. The address is empty when
    // it cannot be known, which with no trusted proxy header is never.
    sqlx::query(
        "INSERT INTO acceptance_network_metadata (acceptance_id, ip_address, user_agent)
         VALUES ($1, $2::inet, $3)",
    )
    .bind(acceptance)
    .bind(signature.origin.address.map(|address| address.to_string()))
    .bind(signature.origin.user_agent)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn view(
    conn: &mut PgConnection,
    rules: &Rules,
    id: Uuid,
    account: Uuid,
) -> Result<ExchangeView, ApiError> {
    let (aggregate, you) = open_for(conn, id, account, false).await?;

    // Someone still in an open exchange needs to know that the other party
    // has deleted their account: nothing more will come from them. Once it
    // is closed there is nothing left to wait for, and it is not said.
    let closed = matches!(aggregate.exchange.state, State::Closed(_));
    let other_party_left = match aggregate.account_of(you.other()) {
        Some(other) if !closed => deleted(conn, other).await?,
        _ => false,
    };

    // The initiator is shown who claimed the invitation until they confirm.
    // Someone who has since left is not there to be confirmed.
    let claimant = match (you, aggregate.exchange.counterparty, aggregate.accounts[1]) {
        (Slot::A, Counterparty::Claimed, Some(other)) if !other_party_left => {
            type ClaimantRow = (String, Option<Vec<u8>>, Option<Vec<u8>>);
            let (display_name, email_encrypted, phone_encrypted): ClaimantRow = sqlx::query_as(
                "SELECT display_name, email_encrypted, phone_encrypted FROM account WHERE id = $1",
            )
            .bind(other)
            .fetch_one(&mut *conn)
            .await?;
            // Decrypted to be shown masked, and only to the initiator, who
            // must tell whether this is the person they invited.
            let keys = contact::keys();
            let email = keys.reveal(Field::ACCOUNT_EMAIL, email_encrypted.as_deref())?;
            let phone = keys.reveal(Field::ACCOUNT_PHONE, phone_encrypted.as_deref())?;
            let identifier = email
                .map(Identifier::Email)
                .or(phone.map(Identifier::Phone))
                .map(|identifier| identifier.masked())
                .unwrap_or_default();
            Some(Claimant {
                display_name,
                identifier,
            })
        }
        _ => None,
    };

    // With nobody in the invited party's place, the initiator needs to know
    // whether the link they sent can still bring someone in. It cannot once
    // it has been used, and it has been if a claimant was removed or left.
    let waiting_for_a_claim = you == Slot::A
        && aggregate.exchange.state == State::Negotiating
        && aggregate.exchange.counterparty == Counterparty::Unclaimed;
    let invitation_open = if waiting_for_a_claim {
        let open: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM invitation
                            WHERE exchange_id = $1 AND claimed_by IS NULL
                              AND revoked_at IS NULL AND expires_at > $2)",
        )
        .bind(id)
        .bind(now())
        .fetch_one(&mut *conn)
        .await?;
        Some(open)
    } else {
        None
    };

    let draft: Option<Value> = sqlx::query_scalar(
        "SELECT body FROM exchange_draft WHERE exchange_id = $1 AND account_id = $2",
    )
    .bind(id)
    .bind(account)
    .fetch_optional(&mut *conn)
    .await?;

    // When each contribution came to stand as it does: the moment of the
    // event that gave it its status. A party waiting on the other needs to
    // know how long they have been waiting.
    let status_since = sqlx::query_as::<_, (Uuid, OffsetDateTime)>(
        "SELECT c.id, e.occurred_at FROM contribution c
         JOIN exchange_event e ON e.exchange_id = c.exchange_id AND e.sequence = c.last_event_seq
         WHERE c.exchange_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, at)| (ContributionId(id), at))
    .collect();

    // Payment options (`crate::payments`): whether the viewer shows theirs,
    // and the other party's, decrypted only while the viewer owes them money
    // on the agreement in force still to be paid, and only where the other
    // party shows them here.
    // A payment option that changed after the agreement came into force,
    // and recently, is shown with a warning (`payments::CHANGE_WARNING_DAYS`).
    let theirs = match aggregate.account_of(you.other()) {
        Some(payee) if !other_party_left && owes_unpaid_money(&aggregate, you) => {
            let in_force: Option<OffsetDateTime> = sqlx::query_scalar(
                "SELECT min(occurred_at) FROM exchange_event
                 WHERE exchange_id = $1 AND type = 'AGREEMENT_IN_FORCE'",
            )
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
            let since = payments::warn_since(in_force.unwrap_or_else(now), now());
            payments::for_payer(conn, payee, id, since).await?
        }
        _ => None,
    };
    let (theirs, theirs_changed) = match theirs {
        Some((handles, changes)) => (Some(handles), changes),
        None => (None, Default::default()),
    };
    let payment_options = PaymentOptionsView {
        shown: payments::shown(conn, account, id).await?,
        theirs,
        theirs_changed,
    };

    let mut view = ExchangeView::build(
        &aggregate,
        you,
        ViewContext {
            claimant,
            invitation_open,
            other_party_left,
            draft,
            status_since,
            payment_options,
        },
        rules,
    );
    // What a reviewer hid from this account reads as a placeholder.
    if let Some(placeholder) = crate::review::hidden_text(conn, id, account).await? {
        crate::review::hide_in_view(&mut view, &placeholder);
    }
    Ok(view)
}

/// Whether `payer` owes the other party money on the agreement in force
/// that is still to be paid: paid outside Yuppers, and neither marked paid,
/// accepted nor waived. A dispute puts it back to be paid.
fn owes_unpaid_money(aggregate: &Aggregate, payer: Slot) -> bool {
    if aggregate.exchange.state != State::Active {
        return false;
    }
    aggregate
        .in_force
        .iter()
        .flat_map(|record| &record.revision.contributions)
        .any(|contribution| {
            contribution.from == payer
                && matches!(
                    contribution.kind,
                    Kind::Money {
                        settlement: Settlement::OffPlatform,
                        ..
                    }
                )
                && matches!(
                    aggregate.exchange.statuses.get(&contribution.id),
                    Some(Status::Pending | Status::Disputed)
                )
        })
}

/// Whether an account is active: neither suspended nor deleted.
async fn active(conn: &mut PgConnection, account: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT status = 'ACTIVE' FROM account WHERE id = $1")
        .bind(account)
        .fetch_one(&mut *conn)
        .await
}

/// Whether an account has been deleted.
async fn deleted(conn: &mut PgConnection, account: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT status = 'DELETED' FROM account WHERE id = $1")
        .bind(account)
        .fetch_one(&mut *conn)
        .await
}

// ---- Creating, listing, viewing ---------------------------------------------

/// Letters and digits that are hard to confuse when read aloud or copied.
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

fn display_code() -> String {
    let mut code = String::with_capacity(9);
    for position in 0..8 {
        if position == 4 {
            code.push('-');
        }
        let n = getrandom::u32().expect("the operating system provides randomness") as usize;
        code.push(CODE_ALPHABET[n % CODE_ALPHABET.len()] as char);
    }
    code
}

/// The timezone names the database knows, read once per process.
/// `pg_timezone_names` reads the server's whole timezone database each time
/// it is queried, tens of milliseconds of database time, which made creating
/// an exchange the slowest request by far (README, "Load check"). The list
/// changes only when the server's timezone data does.
static TIMEZONES: tokio::sync::OnceCell<HashSet<String>> = tokio::sync::OnceCell::const_new();

async fn known_timezone(conn: &mut PgConnection, name: &str) -> Result<bool, sqlx::Error> {
    let names = TIMEZONES
        .get_or_try_init(|| async {
            let names: Vec<String> = sqlx::query_scalar("SELECT name FROM pg_timezone_names")
                .fetch_all(conn)
                .await?;
            Ok::<_, sqlx::Error>(names.into_iter().collect())
        })
        .await?;
    Ok(names.contains(name))
}

pub async fn create(
    db: &PgPool,
    rules: &Rules,
    session: &Session,
    body: CreateExchange,
) -> Result<ExchangeView, ApiError> {
    let mut tx = db.begin().await?;
    acting(&mut tx, session.account_id).await?;

    // One creation at a time per account, so that counting and inserting
    // cannot be raced past the limit.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(session.account_id)
        .execute(&mut *tx)
        .await?;

    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exchange
         WHERE created_by = $1 AND created_at > now() - interval '1 day'",
    )
    .bind(session.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if recent >= rules.exchanges_per_day {
        return Err(ErrorCode::TooManyRequests.into());
    }

    if !known_timezone(&mut tx, &body.timezone).await? {
        return Err(ErrorCode::InvalidRequest.into());
    }

    // A display code is short, so a clash is possible; try again with another.
    let mut id = None;
    for _ in 0..5 {
        id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO exchange (display_code, timezone, created_by)
             VALUES ($1, $2, $3)
             ON CONFLICT (display_code) DO NOTHING
             RETURNING id",
        )
        .bind(display_code())
        .bind(&body.timezone)
        .bind(session.account_id)
        .fetch_optional(&mut *tx)
        .await?;
        if id.is_some() {
            break;
        }
    }
    let id = id.ok_or(ErrorCode::Internal)?;

    sqlx::query(
        "INSERT INTO participant (exchange_id, slot, account_id, display_name, alias)
         VALUES ($1, 'A', $2, (SELECT display_name FROM account WHERE id = $2), ''),
                ($1, 'B', NULL, '', '')",
    )
    .bind(id)
    .bind(session.account_id)
    .execute(&mut *tx)
    .await?;

    let view = view(&mut tx, rules, id, session.account_id).await?;
    tx.commit().await?;
    Ok(view)
}

/// id, display code, state, closed outcome, closed reason, the caller's slot,
/// the other party's name, last change.
type SummaryRow = (
    Uuid,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    OffsetDateTime,
);

pub async fn list(db: &PgPool, session: &Session) -> Result<Vec<ExchangeSummary>, ApiError> {
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "SELECT e.id, e.display_code, e.state, e.closed_outcome, e.closed_reason,
                    mine.slot, other.display_name, e.updated_at
             FROM participant mine
             JOIN exchange e ON e.id = mine.exchange_id
             JOIN participant other ON other.exchange_id = e.id AND other.slot <> mine.slot
             WHERE mine.account_id = $1
             ORDER BY e.updated_at DESC
             LIMIT 100",
    )
    .bind(session.account_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, display_code, state, outcome, reason, slot, other, updated_at)| {
                let (state, closed_outcome) = state_dto(repo::parse_state(
                    &state,
                    outcome.as_deref(),
                    reason.as_deref(),
                ));
                ExchangeSummary {
                    id,
                    display_code,
                    state,
                    closed_outcome,
                    you: if slot == "A" { Slot::A } else { Slot::B },
                    other_party_name: other,
                    updated_at: rfc3339(updated_at),
                }
            },
        )
        .collect())
}

pub async fn get(
    db: &PgPool,
    rules: &Rules,
    session: &Session,
    id: Uuid,
) -> Result<ExchangeView, ApiError> {
    // One snapshot for the several queries a view takes, so it never shows
    // parts of two different versions.
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    view(&mut tx, rules, id, session.account_id).await
}

/// An exchange as one of its parties sees it, for a caller that is not that
/// party's own request: a Wallet pass being drawn (`crate::wallet`). Anyone
/// who is not a party is told it does not exist, as always.
pub async fn view_for(
    db: &PgPool,
    rules: &Rules,
    id: Uuid,
    account: Uuid,
) -> Result<ExchangeView, ApiError> {
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    view(&mut tx, rules, id, account).await
}

/// Saves the caller's working copy. Private to them and never binding.
pub async fn save_draft(
    db: &PgPool,
    rules: &Rules,
    session: &Session,
    id: Uuid,
    body: Value,
) -> Result<(), ApiError> {
    if body.to_string().len() > rules.limits.draft_bytes {
        return Err(ErrorCode::InvalidRequest.into());
    }
    let mut tx = db.begin().await?;
    acting(&mut tx, session.account_id).await?;
    let (aggregate, _) = open_for(&mut tx, id, session.account_id, false).await?;
    if matches!(aggregate.exchange.state, State::Closed(_)) {
        return Err(ErrorCode::ActionNotAllowed.into());
    }
    sqlx::query(
        "INSERT INTO exchange_draft (exchange_id, account_id, body) VALUES ($1, $2, $3)
         ON CONFLICT (exchange_id, account_id) DO UPDATE SET body = $3, updated_at = now()",
    )
    .bind(id)
    .bind(session.account_id)
    .bind(body)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- Sending a revision -----------------------------------------------------

async fn issue_invitation(
    conn: &mut PgConnection,
    exchange: Uuid,
    options: Option<InvitationOptions>,
    rules: &Rules,
    at: OffsetDateTime,
) -> Result<String, ApiError> {
    let bound = match options.and_then(|options| options.bound_to) {
        Some(text) => Some(Identifier::parse(&text)?),
        None => None,
    };
    // Only the blind index of whom it names is kept: a claim is compared
    // with it, and nothing reads it back (`crate::contact`).
    let index = bound
        .as_ref()
        .map(|identifier| contact::keys().index_of(identifier));
    let (email, phone) = match &bound {
        Some(Identifier::Email(_)) => (index.as_ref(), None),
        Some(Identifier::Phone(_)) => (None, index.as_ref()),
        None => (None, None),
    };

    let token = auth::generate_token();
    sqlx::query(
        "INSERT INTO invitation
             (exchange_id, token_hash, bound_email_index, bound_phone_index, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(exchange)
    .bind(auth::token_hash(&token).as_slice())
    .bind(email.map(<[u8; 32]>::as_slice))
    .bind(phone.map(<[u8; 32]>::as_slice))
    .bind(at + rules.invitation_ttl)
    .execute(&mut *conn)
    .await?;
    Ok(token)
}

pub async fn send_revision(
    db: &PgPool,
    settings: &Settings,
    session: &Session,
    id: Uuid,
    idempotency: Idempotency<'_>,
    origin: RequestOrigin<'_>,
    body: SendRevision,
) -> Result<RevisionSent, ApiError> {
    let mut tx = db.begin().await?;
    let (aggregate, slot) = open_for(&mut tx, id, session.account_id, true).await?;
    // Read once the lock is held, so a request that waited for another is
    // judged, and its events stamped, in the order they were applied.
    let at = now();

    if already_applied(&mut tx, session.account_id, &idempotency).await? {
        // The token was shown once, the first time; it cannot be shown again.
        let exchange = view(&mut tx, &settings.rules, id, session.account_id).await?;
        return Ok(RevisionSent {
            exchange,
            invitation_token: None,
        });
    }
    if body.expected_version != aggregate.version {
        return Err(ErrorCode::VersionConflict.into());
    }
    // Terms written over text the sender cannot see are not theirs to send.
    if crate::review::is_hidden_from(&mut tx, id, session.account_id).await? {
        return Err(ErrorCode::ContentHidden.into());
    }
    within_change_rate(&mut tx, id, slot, &settings.rules).await?;
    let consent_language = require_signer(&mut tx, session, &body.consent, settings).await?;

    let revision = body.terms.into_domain(body.note)?;
    let revision_id = Uuid::new_v4();
    let decision = decide(
        &aggregate.exchange,
        Actor::Party(slot),
        Command::Send {
            id: RevisionId(revision_id),
            revision: revision.clone(),
        },
        at,
        &settings.rules,
    )?;
    let expires_at = decision
        .exchange
        .open
        .as_ref()
        .expect("sending leaves the revision open")
        .expires_at;
    let hash = content_hash(
        aggregate.id,
        &aggregate.currency,
        &aggregate.timezone,
        &revision,
    );

    repo::insert_revision(
        &mut tx,
        NewRevision {
            id: revision_id,
            exchange: &aggregate,
            author: slot,
            expires_at,
            content_hash: hash,
            revision: &revision,
        },
    )
    .await
    .map_err(|error| {
        // A contribution ID that belongs to a different exchange.
        if is_violation(&error, "23503") {
            ApiError::from(ErrorCode::InvalidRevision)
        } else {
            error.into()
        }
    })?;

    // Sending is signing.
    record_signature(
        &mut tx,
        session,
        Signature {
            exchange: id,
            revision: revision_id,
            slot,
            content_hash: &hash,
            consent_language,
            consent_version: &body.consent.version,
            origin,
        },
        at,
    )
    .await?;

    // Until something is agreed, the parties are known by the names in the
    // first proposal. A later revision changes them only by coming into
    // force: a counteroffer or amendment that is declined renames nobody.
    if aggregate.exchange.state == State::Draft {
        name_participants(&mut tx, id, &revision).await?;
    }

    sqlx::query("DELETE FROM exchange_draft WHERE exchange_id = $1 AND account_id = $2")
        .bind(id)
        .bind(session.account_id)
        .execute(&mut *tx)
        .await?;

    repo::persist(&mut tx, &aggregate, &decision, Actor::Party(slot), None, at).await?;

    // The first revision opens the negotiation, which needs someone to invite.
    let invitation_token = if aggregate.exchange.state == State::Draft {
        Some(issue_invitation(&mut tx, id, body.invitation, &settings.rules, at).await?)
    } else {
        None
    };

    let exchange = view(&mut tx, &settings.rules, id, session.account_id).await?;
    tx.commit().await?;
    Ok(RevisionSent {
        exchange,
        invitation_token,
    })
}

// ---- Commands ---------------------------------------------------------------

fn non_empty(note: Option<String>) -> Option<String> {
    note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
}

pub async fn run_command(
    db: &PgPool,
    settings: &Settings,
    session: &Session,
    id: Uuid,
    idempotency: Idempotency<'_>,
    origin: RequestOrigin<'_>,
    body: RunCommand,
) -> Result<ExchangeView, ApiError> {
    let mut tx = db.begin().await?;
    let (aggregate, slot) = open_for(&mut tx, id, session.account_id, true).await?;
    let at = now();

    if already_applied(&mut tx, session.account_id, &idempotency).await? {
        return view(&mut tx, &settings.rules, id, session.account_id).await;
    }
    if body.expected_version != aggregate.version {
        return Err(ErrorCode::VersionConflict.into());
    }
    within_change_rate(&mut tx, id, slot, &settings.rules).await?;

    let mut signing = None;
    let (command, note) = match body.command {
        CommandDto::Accept { revision, consent } => {
            // Nobody signs what they cannot read.
            if crate::review::is_hidden_from(&mut tx, id, session.account_id).await? {
                return Err(ErrorCode::ContentHidden.into());
            }
            // Nobody is bound to someone who can no longer sign in to keep
            // the agreement. A suspension withdraws what the account sent
            // (`crate::review`), and a deletion too (`crate::deletion`), so
            // this is only ever met if an account was suspended some other
            // way. It is checked all the same.
            if let Some(open) = &aggregate.exchange.open
                && let Some(author) = aggregate.account_of(open.author)
                && !active(&mut tx, author).await?
            {
                return Err(ErrorCode::ActionNotAllowed.into());
            }
            let language = require_signer(&mut tx, session, &consent, settings).await?;
            signing = Some((language, consent.version));
            (
                Command::Accept {
                    revision: RevisionId(revision),
                },
                None,
            )
        }
        CommandDto::Decline { revision } => (
            Command::Decline {
                revision: RevisionId(revision),
            },
            None,
        ),
        CommandDto::Withdraw { revision } => (
            Command::Withdraw {
                revision: RevisionId(revision),
            },
            None,
        ),
        CommandDto::Discard => (Command::Discard, None),
        CommandDto::Contribution {
            contribution,
            action,
            note,
        } => {
            let contribution = ContributionId(contribution);
            let note = non_empty(note);
            // A dispute must say why, and a claim after a dispute must say
            // what was done about it.
            let reclaim = action == Action::Claim
                && aggregate.exchange.statuses.get(&contribution) == Some(&Status::Disputed);
            if (action == Action::Dispute || reclaim) && note.is_none() {
                return Err(ErrorCode::InvalidRequest.into());
            }
            (
                Command::Contribution {
                    id: contribution,
                    action,
                },
                note,
            )
        }
        CommandDto::ConfirmCounterparty => {
            // Confirming says "this is who I meant", of someone shown by name
            // and address. A claimant who deletes their account, or whose
            // account is suspended, leaves the exchange as they go
            // (`crate::deletion`, `crate::review`), so nobody deleted or
            // suspended should be found here. It is checked all the same: a
            // signature left behind must never bind the initiator to someone
            // who cannot sign in.
            if let Some(claimant) = aggregate.account_of(Slot::B)
                && !active(&mut tx, claimant).await?
            {
                return Err(ErrorCode::ActionNotAllowed.into());
            }
            (Command::ConfirmCounterparty, None)
        }
        CommandDto::RejectCounterparty => (Command::RejectCounterparty, None),
        CommandDto::ProposeEnd => (Command::ProposeEnd, None),
        CommandDto::AcceptEnd => (Command::AcceptEnd, None),
        CommandDto::CancelEnd => (Command::CancelEnd, None),
        CommandDto::RequestClose { note } => (Command::RequestClose, non_empty(note)),
        CommandDto::RetractClose => (Command::RetractClose, None),
        CommandDto::AddStatement { note } => {
            let note = non_empty(Some(note)).ok_or(ErrorCode::InvalidRequest)?;
            (Command::AddStatement, Some(note))
        }
    };

    // Notes go into the permanent history, so they are bounded like
    // everything else there.
    if note
        .as_ref()
        .is_some_and(|note| note.chars().count() > settings.rules.note_max_chars)
    {
        return Err(ErrorCode::InvalidRequest.into());
    }

    let actor = Actor::Party(slot);
    let decision = decide(&aggregate.exchange, actor, command, at, &settings.rules)?;

    if let Some((consent_language, consent_version)) = &signing {
        let open = aggregate
            .open
            .as_ref()
            .expect("an accepted revision was open");
        record_signature(
            &mut tx,
            session,
            Signature {
                exchange: id,
                revision: open.id,
                slot,
                content_hash: &open.content_hash,
                consent_language,
                consent_version,
                origin,
            },
            at,
        )
        .await?;
    }

    repo::persist(&mut tx, &aggregate, &decision, actor, note.as_deref(), at).await?;

    // A discarded draft's working copy goes with it: it was never sent and
    // is nobody's record of anything.
    if aggregate.exchange.state == State::Draft
        && matches!(decision.exchange.state, State::Closed(_))
    {
        sqlx::query("DELETE FROM exchange_draft WHERE exchange_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // The parties' names and the risk tier follow the agreement in force.
    let came_into_force = decision
        .events
        .iter()
        .any(|event| matches!(event, Event::AgreementInForce { .. }));
    if let (true, Some(in_force)) = (came_into_force, &decision.exchange.in_force) {
        name_participants(&mut tx, id, &in_force.revision).await?;
        // The tier only ever goes up.
        let threshold = settings.rules.tier_one_threshold_minor;
        if required_tier(&in_force.revision, false, threshold) == Tier::One {
            sqlx::query("UPDATE exchange SET risk_tier = greatest(risk_tier, 1) WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }

    let view = view(&mut tx, &settings.rules, id, session.account_id).await?;
    tx.commit().await?;
    Ok(view)
}

/// The caller, having opened an invitation and not yet been confirmed by the
/// initiator, gives up their place (DESIGN.md §8). Anything they signed is
/// void, and the exchange is no longer theirs to see. It is their one way
/// out, since they cannot decline.
///
/// Unlike the other changes this names no version: nothing the exchange has
/// become since the caller last looked is a reason to keep them in it. And
/// it has no reply to repeat: once it has worked, the exchange is gone for
/// the caller, and a second try is told so like anyone else's would be.
pub async fn leave(
    db: &PgPool,
    settings: &Settings,
    session: &Session,
    id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = db.begin().await?;
    let (aggregate, slot) = open_for(&mut tx, id, session.account_id, true).await?;
    let at = now();
    within_change_rate(&mut tx, id, slot, &settings.rules).await?;

    let actor = Actor::Party(slot);
    let decision = decide(
        &aggregate.exchange,
        actor,
        Command::ReleaseClaim,
        at,
        &settings.rules,
    )?;
    repo::persist(&mut tx, &aggregate, &decision, actor, None, at).await?;
    tx.commit().await?;
    Ok(())
}

// ---- Invitations ------------------------------------------------------------

/// Replaces the invitation link, for when it was lost, expired or sent to
/// the wrong person, or when whoever used it was removed or left. The old
/// link stops working.
pub async fn reissue_invitation(
    db: &PgPool,
    rules: &Rules,
    session: &Session,
    id: Uuid,
    options: Option<InvitationOptions>,
) -> Result<InvitationIssued, ApiError> {
    let mut tx = db.begin().await?;
    let (aggregate, slot) = open_for(&mut tx, id, session.account_id, true).await?;
    let at = now();
    if slot != Slot::A {
        return Err(ErrorCode::WrongActor.into());
    }
    if aggregate.exchange.state != State::Negotiating
        || aggregate.exchange.counterparty != Counterparty::Unclaimed
    {
        return Err(ErrorCode::ActionNotAllowed.into());
    }

    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invitation
         WHERE exchange_id = $1 AND created_at > now() - interval '1 day'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if recent >= rules.invitations_per_day {
        return Err(ErrorCode::TooManyRequests.into());
    }

    sqlx::query(
        "UPDATE invitation SET revoked_at = $2
         WHERE exchange_id = $1 AND claimed_by IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    let invitation_token = issue_invitation(&mut tx, id, options, rules, at).await?;

    tx.commit().await?;
    Ok(InvitationIssued { invitation_token })
}

struct InvitationRow {
    id: Uuid,
    exchange: Uuid,
    record: invitation::Invitation,
    claimed_by: Option<Uuid>,
}

/// Everything an answer about an invitation link turns on, as one query
/// reads it for one viewer: the link, whether its exchange still waits for
/// someone to sign its offer, its initiator and whether their account is
/// active, whether the viewer and the initiator have blocked each other, and
/// the viewer's own identifiers.
///
/// The preview, the claim and through them the report on a link each read
/// this first and decide from it alone whether to refuse, and only then load
/// anything more. A made-up token, a dead link, a link whose initiator is
/// suspended and a link from someone the viewer has blocked, or who has
/// blocked them, so cost the same: this one round trip, then the refusal. A
/// block cannot be told from a dead link by how long the answer takes
/// (DESIGN.md §9).
struct Gate {
    invitation: Option<InvitationRow>,
    /// The exchange is negotiating, with an offer waiting to be signed.
    offer_open: bool,
    initiator: Option<Uuid>,
    initiator_active: bool,
    blocked: bool,
    /// The blind indexes of the viewer's email address and phone number.
    viewer_email: Option<[u8; 32]>,
    viewer_phone: Option<[u8; 32]>,
}

/// What [`gate`] reads. Each address or number comes as its blind index.
#[derive(sqlx::FromRow)]
struct GateRow {
    id: Option<Uuid>,
    exchange_id: Option<Uuid>,
    bound_email_index: Option<Vec<u8>>,
    bound_phone_index: Option<Vec<u8>>,
    expires_at: Option<OffsetDateTime>,
    claimed_by: Option<Uuid>,
    revoked_at: Option<OffsetDateTime>,
    offer_open: bool,
    initiator: Option<Uuid>,
    initiator_active: bool,
    blocked: bool,
    viewer_email_index: Option<Vec<u8>>,
    viewer_phone_index: Option<Vec<u8>>,
}

/// A blind index as stored.
fn index_of(stored: Option<Vec<u8>>) -> Option<[u8; 32]> {
    stored.and_then(|index| index.try_into().ok())
}

/// Reads the [`Gate`] for `viewer` and `token`: one query, whatever the
/// token. The row is the viewer's account, so there is always one, with
/// nothing found where the token names no link.
async fn gate(conn: &mut PgConnection, viewer: Uuid, token: &str) -> Result<Gate, ApiError> {
    let row: Option<GateRow> = sqlx::query_as(
        "SELECT i.id, i.exchange_id, i.bound_email_index, i.bound_phone_index, i.expires_at,
                i.claimed_by, i.revoked_at,
                coalesce(e.state = 'NEGOTIATING' AND e.open_revision_id IS NOT NULL, false)
                    AS offer_open,
                initiator.account_id AS initiator,
                coalesce(ia.status = 'ACTIVE', false) AS initiator_active,
                EXISTS (SELECT 1 FROM account_block b
                        WHERE (b.blocker_account_id = v.id
                               AND b.blocked_account_id = initiator.account_id)
                           OR (b.blocker_account_id = initiator.account_id
                               AND b.blocked_account_id = v.id)) AS blocked,
                v.email_index AS viewer_email_index, v.phone_index AS viewer_phone_index
         FROM account v
         LEFT JOIN invitation i ON i.token_hash = $1
         LEFT JOIN exchange e ON e.id = i.exchange_id
         LEFT JOIN participant initiator
           ON initiator.exchange_id = i.exchange_id AND initiator.slot = 'A'
         LEFT JOIN account ia ON ia.id = initiator.account_id
         WHERE v.id = $2",
    )
    .bind(auth::token_hash(token.trim()).as_slice())
    .bind(viewer)
    .fetch_optional(&mut *conn)
    .await?;
    // The viewer's account is gone: deleted a moment ago by another request.
    let row = row.ok_or(ErrorCode::Unauthenticated)?;
    // Compared by blind index (`crate::contact`): nothing is decrypted.
    let bound_to = index_of(row.bound_email_index)
        .map(invitation::Binding::Email)
        .or(index_of(row.bound_phone_index).map(invitation::Binding::Phone));
    let claimed_by = row.claimed_by;
    let invitation = match (row.id, row.exchange_id, row.expires_at) {
        (Some(id), Some(exchange), Some(expires_at)) => Some(InvitationRow {
            id,
            exchange,
            record: invitation::Invitation {
                expires_at,
                claimed: claimed_by.is_some(),
                revoked: row.revoked_at.is_some(),
                bound_to,
            },
            claimed_by,
        }),
        _ => None,
    };
    Ok(Gate {
        invitation,
        offer_open: row.offer_open,
        initiator: row.initiator,
        initiator_active: row.initiator_active,
        blocked: row.blocked,
        viewer_email: index_of(row.viewer_email_index),
        viewer_phone: index_of(row.viewer_phone_index),
    })
}

impl Gate {
    /// The link, if it can show its offer to the viewer: live, its offer
    /// still open, its initiator active, and no block between the two.
    /// Every other case is the same dead link.
    fn showable(&self, at: OffsetDateTime) -> Option<&InvitationRow> {
        let found = self.invitation.as_ref()?;
        let record = &found.record;
        let live = !record.revoked && !record.claimed && at < record.expires_at;
        (live && self.offer_open && self.initiator_active && !self.blocked).then_some(found)
    }
}

/// Locks an invitation row, once its exchange is locked, so that two claims
/// of one link take turns.
async fn lock_invitation(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1 FROM invitation WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// What the holder of an invitation link reads once signed in, before
/// deciding whether to claim it: the proposal itself. Every way a link can be
/// dead gives the same answer, and so does an initiator who is not active
/// (suspended or deleted) and a block between the viewer and the initiator,
/// so the preview and the claim agree for them. Whether to refuse is decided
/// from one query ([`gate`]) before anything else is read, so that the
/// answers also take the same time.
///
/// Reading needs an account: the HTTP layer refuses anyone signed out before
/// the token is looked at. That is what keeps a block from showing. Were the
/// proposal readable signed out, a blocked person could compare it with the
/// dead link they get signed in (DESIGN.md §9, §18 item 13a).
pub async fn preview_invitation(
    db: &PgPool,
    viewer: Uuid,
    token: &str,
) -> Result<InvitationPreview, ApiError> {
    let unavailable = || ApiError::from(ErrorCode::InvitationUnavailable);
    let mut tx = db.begin().await?;
    // The gate and the proposal read from one snapshot, so the proposal is
    // the one the gate let through.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;

    let gate = gate(&mut tx, viewer, token).await?;
    let found = gate.showable(now()).ok_or_else(unavailable)?;

    let aggregate = repo::load(&mut tx, found.exchange, false)
        .await?
        .ok_or_else(unavailable)?;
    let open = match (&aggregate.exchange.state, &aggregate.open) {
        (State::Negotiating, Some(open)) => open,
        _ => return Err(unavailable()),
    };

    Ok(InvitationPreview {
        display_code: aggregate.display_code.clone(),
        expires_at: rfc3339(found.record.expires_at),
        bound: found.record.bound_to.is_some(),
        currency: aggregate.currency.clone(),
        timezone: aggregate.timezone.clone(),
        revision: RevisionView::from_record(open),
    })
}

/// What a claim may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Take the invited party's place, as the person asked to.
    Take,
    /// Only open the exchange if the account already holds the place this
    /// link gave. Never takes it: anything else, a live link included, is
    /// `INVITATION_UNAVAILABLE`. A client asks this when a link turns out
    /// spent, to take the person who used it back to their exchange, without
    /// the risk of claiming something nobody tapped for.
    OnlyIfYours,
}

/// A claim the [`Gate`] lets go on.
#[derive(Clone, Copy)]
enum Passed {
    /// The account used this link before: it may still hold the place.
    Yours(Uuid),
    /// The account may take the place in this exchange.
    Take { exchange: Uuid, pre_bound: bool },
}

/// Whether a claim may go on, decided from the [`Gate`] alone.
fn claim_gate(
    gate: &Gate,
    account: Uuid,
    claim: Claim,
    at: OffsetDateTime,
) -> Result<Passed, ApiError> {
    let unavailable = || ApiError::from(ErrorCode::InvitationUnavailable);
    let found = gate.invitation.as_ref().ok_or_else(unavailable)?;
    // A link whose initiator is suspended or gone is dead to everyone, the
    // person who already used it included.
    if !gate.initiator_active {
        return Err(unavailable());
    }
    // The account's own place, if it still holds it: checked once the
    // exchange is loaded.
    if found.claimed_by == Some(account) {
        return Ok(Passed::Yours(found.exchange));
    }
    if claim == Claim::OnlyIfYours || !gate.offer_open {
        return Err(unavailable());
    }
    let claimant = invitation::Claimant {
        email: gate.viewer_email,
        phone: gate.viewer_phone,
        is_initiator: gate.initiator == Some(account),
        blocked: gate.blocked,
    };
    let passed = invitation::claim(&found.record, &claimant, at).map_err(|refusal| {
        ApiError::from(match refusal {
            invitation::Refusal::OwnInvitation => ErrorCode::ActionNotAllowed,
            invitation::Refusal::BoundToSomeoneElse => ErrorCode::InvitationNotForYou,
            // Dead links, and blocks, all look the same from outside.
            _ => ErrorCode::InvitationUnavailable,
        })
    })?;
    Ok(Passed::Take {
        exchange: found.exchange,
        pre_bound: passed.pre_bound,
    })
}

/// The signed-in account takes the invited party's place in the exchange,
/// or with [`Claim::OnlyIfYours`] only finds the place it already took.
///
/// Whether to refuse is decided from one query ([`gate`]), the same for
/// every token, before the exchange is loaded or locked; what follows runs
/// only for a claim that may go on, and checks again under the lock.
pub async fn claim_invitation(
    db: &PgPool,
    rules: &Rules,
    session: &Session,
    token: &str,
    claim: Claim,
) -> Result<ExchangeView, ApiError> {
    let unavailable = || ApiError::from(ErrorCode::InvitationUnavailable);
    let account = session.account_id;
    let mut tx = db.begin().await?;
    acting(&mut tx, account).await?;

    let exchange = match claim_gate(&gate(&mut tx, account, token).await?, account, claim, now())? {
        Passed::Yours(exchange) | Passed::Take { exchange, .. } => exchange,
    };

    // The exchange first, then the invitation: the order every other change
    // takes them in. Then the gate again, under the lock: a block, a
    // revocation, a suspension or another claim may have come first.
    let aggregate = repo::load(&mut tx, exchange, true)
        .await?
        .ok_or_else(unavailable)?;
    let gate = gate(&mut tx, account, token).await?;
    let found = gate.invitation.as_ref().ok_or_else(unavailable)?;
    if found.exchange != exchange {
        return Err(unavailable());
    }
    lock_invitation(&mut tx, found.id).await?;
    let at = now();

    // Claiming twice with the same account is harmless, for as long as the
    // place is still theirs. For someone since removed from it, the link is
    // spent like any other, and says so the same way.
    let pre_bound = match claim_gate(&gate, account, claim, at)? {
        Passed::Yours(_) if aggregate.accounts[1] == Some(account) => {
            return view(&mut tx, rules, exchange, account).await;
        }
        Passed::Yours(_) => return Err(unavailable()),
        Passed::Take { pre_bound, .. } => pre_bound,
    };

    let actor = Actor::Party(Slot::B);
    let command = Command::ClaimCounterparty { pre_bound };
    let decision: Decision = decide(&aggregate.exchange, actor, command, at, rules).map_err(
        |refusal| match refusal {
            exchange::Refusal::NotAllowed => unavailable(),
            other => other.into(),
        },
    )?;

    sqlx::query("UPDATE participant SET account_id = $2 WHERE exchange_id = $1 AND slot = 'B'")
        .bind(exchange)
        .bind(account)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE invitation SET claimed_by = $2, claimed_at = $3 WHERE id = $1")
        .bind(found.id)
        .bind(account)
        .bind(at)
        .execute(&mut *tx)
        .await?;
    // Stored with the slot now filled, so the claim event records who
    // claimed it. That fact then lives in the permanent history and not only
    // in a row that can change.
    let mut claimed = aggregate.clone();
    claimed.accounts[1] = Some(account);
    repo::persist(&mut tx, &claimed, &decision, actor, None, at).await?;

    let view = view(&mut tx, rules, exchange, account).await?;
    tx.commit().await?;
    Ok(view)
}

// ---- Timers -----------------------------------------------------------------

/// Forgets the network address and user agent recorded with signatures older
/// than the retention period (DESIGN.md §14). The signatures stay. Returns
/// how many records were removed. Called by the worker.
pub async fn purge_network_metadata(
    db: &PgPool,
    rules: &Rules,
    at: OffsetDateTime,
) -> Result<u64, sqlx::Error> {
    let before = at - rules.network_metadata_retention;
    let removed = sqlx::query("DELETE FROM acceptance_network_metadata WHERE recorded_at < $1")
        .bind(before)
        .execute(db)
        .await?
        .rows_affected();
    Ok(removed)
}

/// Runs every timer that has come due: expired revisions, lapsed close
/// requests, and the inactivity prompt and closure. Returns how many
/// exchanges changed. Called by the worker.
pub async fn run_timers(
    db: &PgPool,
    rules: &Rules,
    at: OffsetDateTime,
) -> Result<usize, sqlx::Error> {
    let due: [(&str, Command, OffsetDateTime); 4] = [
        (
            "SELECT e.id FROM exchange e JOIN revision r ON r.id = e.open_revision_id
             WHERE r.expires_at <= $1",
            Command::ExpireRevision,
            at,
        ),
        (
            "SELECT id FROM exchange WHERE close_requested_at <= $1",
            Command::LapseCloseRequest,
            at - rules.close_response_window,
        ),
        (
            // An exchange with a due date still inside the window is not idle
            // yet, however long ago anyone acted (see `Exchange::idle_since`).
            "SELECT e.id FROM exchange e
             WHERE e.state = 'ACTIVE' AND e.inactivity_prompted_at IS NULL
               AND e.last_activity_at <= $1
               AND NOT EXISTS (
                   SELECT 1 FROM contribution_snapshot s
                   WHERE s.revision_id = e.in_force_revision_id
                     AND s.due_date >= ($1 AT TIME ZONE 'UTC')::date)",
            Command::PromptInactivity,
            at - rules.inactivity_prompt_after,
        ),
        (
            "SELECT id FROM exchange WHERE inactivity_prompted_at <= $1",
            Command::CloseInactive,
            at - rules.inactivity_close_after,
        ),
    ];

    let mut changed = 0;
    for (candidates, command, cutoff) in due {
        let ids: Vec<Uuid> = sqlx::query_scalar(candidates)
            .bind(cutoff)
            .fetch_all(db)
            .await?;
        for id in ids {
            // Each exchange on a task of its own: one that fails, or even
            // panics, is logged and passed over, and the timers still run
            // for every other exchange.
            let (db, rules, command) = (db.clone(), rules.clone(), command.clone());
            let outcome =
                tokio::spawn(async move { run_timer(&db, &rules, id, command, at).await }).await;
            match outcome {
                Ok(Ok(true)) => changed += 1,
                Ok(Ok(false)) => {}
                Ok(Err(error)) => {
                    tracing::error!(error = %Redacted(&error), exchange = %id, "a timer failed for one exchange")
                }
                Err(error) => {
                    tracing::error!(%error, exchange = %id, "a timer panicked for one exchange")
                }
            }
        }
    }
    Ok(changed)
}

/// Runs one timer command on one exchange. Returns whether it changed.
async fn run_timer(
    db: &PgPool,
    rules: &Rules,
    id: Uuid,
    command: Command,
    at: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;
    let Some(aggregate) = repo::load(&mut tx, id, true).await? else {
        return Ok(false);
    };
    // The query was only a shortlist; the rules have the last word, and the
    // exchange may have moved on since it was listed.
    let Ok(decision) = decide(&aggregate.exchange, Actor::System, command, at, rules) else {
        return Ok(false);
    };
    repo::persist(&mut tx, &aggregate, &decision, Actor::System, None, at).await?;
    tx.commit().await?;
    Ok(true)
}
