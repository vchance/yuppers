//! Reporting an exchange and blocking the other party (DESIGN.md §9).
//!
//! A report is stored for review and does nothing else: the person reported
//! is never told, and the reporter learns only that it was received.
//!
//! A block is between two accounts, made and undone through an exchange the
//! two share, so that no client ever handles an account identifier. What it
//! does:
//!
//! * Neither of the two can claim an invitation from the other, in either
//!   direction, for as long as it stands. That check is in the claim itself
//!   (`exchanges::service::claim_invitation`) and looks like any dead link.
//! * Whatever was still waiting to be signed between them when it was made is
//!   ended in the blocker's name: a revision they sent is withdrawn, one sent
//!   to them is declined. In a negotiation that closes the exchange; on an
//!   agreement in force it drops the proposed amendment. Otherwise the person
//!   blocked could sign an offer the blocker left open and bind them after
//!   the block.
//! * A blocker who had opened the other's invitation and not yet been
//!   confirmed leaves that exchange instead (§8): they cannot decline, and
//!   leaving voids whatever they signed there, so that the person they
//!   blocked cannot bind them by confirming them afterwards. The offer stays
//!   open for someone else. The block itself outlasts the exchange they
//!   shared: it is listed, and can be lifted, through the exchange they left.
//! * An agreement already in force is left alone, and so is everything either
//!   party can do in it afterwards: every exchange must be able to end
//!   (§3, invariant 5).
//!
//! The person blocked is told nothing. What they can see is a withdrawal, a
//! refusal or a departure like any other, and an invitation link that no
//! longer works.

use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth;
use crate::domain::Rules;
use crate::domain::exchange::{Actor, Command, Counterparty, decide};
use crate::domain::revision::Slot;
use crate::error::{ApiError, ErrorCode};
use crate::exchanges::dto::rfc3339;
use crate::exchanges::repo;
use crate::exchanges::service;

// ---- Limits -----------------------------------------------------------------
//
// Placeholders, like the other numbers of §9. Reports are free text into a
// table that staff read, so the tool for reporting abuse is bounded like
// everything else a stranger can write to.

/// Longest details a report may carry, in characters.
pub const REPORT_DETAILS_MAX_CHARS: usize = 2000;
/// Reports one account may file in a day, over all exchanges, invitation
/// links included. Every report has an account behind it: reporting the
/// proposal behind a link needs signing in, as reading it does.
pub const REPORTS_PER_ACCOUNT_PER_DAY: i64 = 10;
/// How many blocked people the list returns.
const BLOCK_LIST_MAX: i64 = 200;

// ---- Shapes -----------------------------------------------------------------

/// Why an exchange is being reported. A fixed list, so that a reviewer can
/// tell what kind of harm a report is about before reading it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportReason {
    /// Harassment, threats or abuse.
    Harassment,
    /// A trade that is unlawful or that the content policy prohibits.
    ProhibitedTrade,
    /// A scam, or an attempt to defraud.
    Scam,
    /// Someone pretending to be someone else.
    Impersonation,
    /// Someone under 18 is involved.
    Underage,
    /// Spam, or a proposal from a stranger that nobody asked for.
    Unwanted,
    /// Anything else. Needs details.
    Other,
}

impl ReportReason {
    /// The form stored in the `report` table.
    pub fn as_str(self) -> &'static str {
        match self {
            ReportReason::Harassment => "HARASSMENT",
            ReportReason::ProhibitedTrade => "PROHIBITED_TRADE",
            ReportReason::Scam => "SCAM",
            ReportReason::Impersonation => "IMPERSONATION",
            ReportReason::Underage => "UNDERAGE",
            ReportReason::Unwanted => "UNWANTED",
            ReportReason::Other => "OTHER",
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewReport {
    pub reason: ReportReason,
    /// What the reporter wants a reviewer to know. Optional, except with the
    /// reason `OTHER`. At most 2,000 characters.
    pub details: Option<String>,
}

/// A report on the proposal behind an invitation link, from someone signed
/// in who holds the link. The token is their proof of having received it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NewInvitationReport {
    pub token: String,
    pub reason: ReportReason,
    /// As for a report on an exchange.
    pub details: Option<String>,
}

/// Whether the caller has blocked the other party of an exchange. Says
/// nothing about whether the other party has blocked the caller.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlockStatus {
    pub blocked: bool,
    /// The other party's name as the exchange writes it. An exchange closed
    /// before anything was agreed shows no terms to read it from.
    pub name: String,
}

/// Someone the caller has blocked, named as an exchange the two share names
/// them. The exchange is also how to unblock them.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlockedPerson {
    pub exchange_id: Uuid,
    pub display_code: String,
    /// Their name as written in that exchange.
    pub name: String,
    /// RFC 3339.
    pub blocked_at: String,
    /// Set when the caller is no longer in that exchange and cannot open it:
    /// they had opened its invitation and left before being confirmed. It
    /// still names the person, and unblocking through it still works.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<bool>,
}

// ---- Shared steps -----------------------------------------------------------

/// The other side of an exchange, as one of its parties may know it.
struct OtherSide {
    /// Nobody, until someone has claimed the invitation.
    account: Option<Uuid>,
    /// Their name as the exchange writes it.
    name: String,
}

/// The other side of an exchange, for one of its parties. To anyone else the
/// exchange does not exist: one question answers both "is there such an
/// exchange" and "is the caller in it", so the two cannot be told apart.
async fn as_party(
    conn: &mut PgConnection,
    exchange: Uuid,
    account: Uuid,
) -> Result<OtherSide, ApiError> {
    let found: Option<(Option<Uuid>, String)> = sqlx::query_as(
        "SELECT theirs.account_id, theirs.display_name
         FROM participant mine
         JOIN participant theirs
           ON theirs.exchange_id = mine.exchange_id AND theirs.slot <> mine.slot
         WHERE mine.exchange_id = $1 AND mine.account_id = $2",
    )
    .bind(exchange)
    .bind(account)
    .fetch_optional(&mut *conn)
    .await?;
    let (account, name) = found.ok_or(ErrorCode::NotFound)?;
    Ok(OtherSide { account, name })
}

/// The other party's account. With nobody on the other side there is no one
/// to report or to block.
async fn other_party(
    conn: &mut PgConnection,
    exchange: Uuid,
    account: Uuid,
) -> Result<Uuid, ApiError> {
    let other = as_party(conn, exchange, account).await?;
    other
        .account
        .ok_or_else(|| ErrorCode::ActionNotAllowed.into())
}

// ---- Reports ----------------------------------------------------------------

/// The reason, and the details trimmed, with nothing where there were none.
fn checked(
    reason: ReportReason,
    details: Option<String>,
) -> Result<(ReportReason, Option<String>), ApiError> {
    let details = details
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let too_long = details
        .as_ref()
        .is_some_and(|text| text.chars().count() > REPORT_DETAILS_MAX_CHARS);
    // "Something else" with nothing said is a report nobody can act on.
    let unexplained = reason == ReportReason::Other && details.is_none();
    if too_long || unexplained {
        return Err(ErrorCode::InvalidRequest.into());
    }
    Ok((reason, details))
}

struct Report<'a> {
    /// The account reporting. There always is one: reporting through an
    /// invitation link needs signing in too.
    reporter: Uuid,
    exchange: Uuid,
    /// The party the report is about.
    subject: Uuid,
    reason: ReportReason,
    details: Option<&'a str>,
}

/// Stores a report, unless it repeats one that is still waiting for review
/// or the reporter has reached the day's limit.
///
/// A person's second report on an exchange, while their first is open, is
/// answered like the first and stores nothing: reporting again must not make
/// the queue longer or the matter look worse.
async fn file(conn: &mut PgConnection, report: Report<'_>) -> Result<(), ApiError> {
    // One at a time per reporter, so that looking for an earlier report,
    // counting and inserting cannot be raced past the limit.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("report by {}", report.reporter))
        .execute(&mut *conn)
        .await?;

    let (repeated, recent): (bool, i64) = sqlx::query_as(
        "SELECT
            EXISTS (SELECT 1 FROM report
                    WHERE reporter_account_id = $1 AND subject_exchange_id = $2
                      AND status = 'OPEN'),
            (SELECT count(*) FROM report
             WHERE reporter_account_id = $1
               AND created_at > now() - interval '1 day')",
    )
    .bind(report.reporter)
    .bind(report.exchange)
    .fetch_one(&mut *conn)
    .await?;
    if repeated {
        return Ok(());
    }
    if recent >= REPORTS_PER_ACCOUNT_PER_DAY {
        return Err(ErrorCode::TooManyRequests.into());
    }

    sqlx::query(
        "INSERT INTO report
            (reporter_account_id, subject_exchange_id, subject_account_id, reason, details)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(report.reporter)
    .bind(report.exchange)
    .bind(report.subject)
    .bind(report.reason.as_str())
    .bind(report.details)
    .execute(&mut *conn)
    .await?;
    // Every reviewer hears that a report is waiting, and nothing more.
    crate::review::alert_staff(conn).await?;
    Ok(())
}

/// A party reports the exchange, and with it the other party.
pub async fn report_exchange(
    db: &PgPool,
    reporter: Uuid,
    exchange: Uuid,
    body: NewReport,
) -> Result<(), ApiError> {
    let mut tx = db.begin().await?;
    service::acting(&mut tx, reporter).await?;
    let subject = other_party(&mut tx, exchange, reporter).await?;
    let (reason, details) = checked(body.reason, body.details)?;
    file(
        &mut tx,
        Report {
            reporter,
            exchange,
            subject,
            reason,
            details: details.as_deref(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Someone signed in, holding an invitation link, reports the proposal it
/// shows them, and with it the person who sent it.
pub async fn report_invitation(
    db: &PgPool,
    reporter: Uuid,
    body: NewInvitationReport,
) -> Result<(), ApiError> {
    // Exactly the preview's test of the link, by running it: a link that
    // shows nothing can report nothing, and says so the same way.
    service::preview_invitation(db, reporter, &body.token).await?;

    let mut tx = db.begin().await?;
    service::acting(&mut tx, reporter).await?;
    let found: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT i.exchange_id, initiator.account_id
         FROM invitation i
         JOIN participant initiator
           ON initiator.exchange_id = i.exchange_id AND initiator.slot = 'A'
         WHERE i.token_hash = $1",
    )
    .bind(auth::token_hash(body.token.trim()).as_slice())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((exchange, Some(subject))) = found else {
        return Err(ErrorCode::InvitationUnavailable.into());
    };
    // Opening one's own link and reporting oneself is not a report.
    if reporter == subject {
        return Err(ErrorCode::ActionNotAllowed.into());
    }

    let (reason, details) = checked(body.reason, body.details)?;
    file(
        &mut tx,
        Report {
            reporter,
            exchange,
            subject,
            reason,
            details: details.as_deref(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- Blocks -----------------------------------------------------------------

/// Whether the caller has blocked the other party of this exchange.
pub async fn block_status(
    db: &PgPool,
    account: Uuid,
    exchange: Uuid,
) -> Result<BlockStatus, ApiError> {
    let mut conn = db.acquire().await?;
    let other = as_party(&mut conn, exchange, account).await?;
    let blocked = match other.account {
        Some(other) => {
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM account_block
                                WHERE blocker_account_id = $1 AND blocked_account_id = $2)",
            )
            .bind(account)
            .bind(other)
            .fetch_one(&mut *conn)
            .await?
        }
        None => false,
    };
    Ok(BlockStatus {
        blocked,
        name: other.name,
    })
}

/// Blocks the other party of this exchange. Blocking someone already blocked
/// changes nothing.
pub async fn block(
    db: &PgPool,
    rules: &Rules,
    blocker: Uuid,
    exchange: Uuid,
) -> Result<(), ApiError> {
    let mut tx = db.begin().await?;
    // Holds the account against its own deletion for the length of this
    // transaction, so a block cannot be made by an account that is gone.
    service::acting(&mut tx, blocker).await?;
    let blocked = other_party(&mut tx, exchange, blocker).await?;

    let added = sqlx::query(
        "INSERT INTO account_block (blocker_account_id, blocked_account_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(blocker)
    .bind(blocked)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Only a new block ends what was open. A repeat of the request must not
    // decline something sent since.
    if added == 1 {
        end_open_proposals(&mut tx, rules, blocker, blocked).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Ends, in the blocker's name, every revision still waiting to be signed
/// between the two: one the blocker sent is withdrawn, one sent to them is
/// declined. Where the blocker is in the exchange only as a claimant the
/// other has not confirmed, they leave it instead. Runs in the transaction
/// that stores the block.
pub(crate) async fn end_open_proposals(
    conn: &mut PgConnection,
    rules: &Rules,
    blocker: Uuid,
    blocked: Uuid,
) -> Result<(), ApiError> {
    // The exchanges the two share, and those of the blocker's that nobody has
    // joined yet: the person blocked may be claiming one at this moment. A
    // claim holds the lock on its exchange, so taking that lock below settles
    // the order. Either the claim finished first and is found here, or it
    // comes after and meets the block.
    //
    // Always in the same order, so two people blocking each other at once
    // cannot each hold a lock the other is waiting for.
    let candidates: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id
         FROM exchange e
         JOIN participant mine ON mine.exchange_id = e.id AND mine.account_id = $1
         JOIN participant theirs ON theirs.exchange_id = e.id AND theirs.slot <> mine.slot
         WHERE e.state IN ('NEGOTIATING', 'ACTIVE')
           AND (theirs.account_id = $2 OR theirs.account_id IS NULL)
         ORDER BY e.id",
    )
    .bind(blocker)
    .bind(blocked)
    .fetch_all(&mut *conn)
    .await?;

    for id in candidates {
        let Some(aggregate) = repo::load(conn, id, true).await? else {
            continue;
        };
        let at = OffsetDateTime::now_utc();
        let Some(slot) = aggregate.slot_of(blocker) else {
            continue;
        };
        if aggregate.account_of(slot.other()) != Some(blocked) {
            continue;
        }
        let mut commands = Vec::new();
        // A blocker the initiator has not confirmed can neither decline nor
        // take a signature back. They leave, which undoes both: whatever
        // they signed is void and the exchange is no longer theirs.
        if slot == Slot::B && aggregate.exchange.counterparty == Counterparty::Claimed {
            commands.push(Command::ReleaseClaim);
        }
        if let Some(open) = &aggregate.exchange.open {
            let revision = open.id;
            commands.push(if open.author == slot {
                Command::Withdraw { revision }
            } else {
                Command::Decline { revision }
            });
        }

        let actor = Actor::Party(slot);
        // The rules decide, as for any command, and the first they allow is
        // what happens. Where they allow none, the exchange is left as it
        // stands.
        let decision = commands
            .into_iter()
            .find_map(|command| decide(&aggregate.exchange, actor, command, at, rules).ok());
        if let Some(decision) = decision {
            repo::persist(conn, &aggregate, &decision, actor, None, at).await?;
        }
    }
    Ok(())
}

/// Removes the caller's block on the other party of this exchange. Nothing
/// that the block ended comes back.
///
/// A block made by a claimant the initiator had not confirmed took them out
/// of the exchange (see the top of this file), and it must not be left
/// standing with no way to lift it. So someone who once held the invited
/// party's place can still lift, through that exchange, a block they have
/// on its initiator. That is all it answers them: with no such block there
/// is, for them as for any stranger, no such exchange. A place held by an
/// account since combined into the caller's counts as the caller's own: the
/// block moved with it (`crate::combine`).
pub async fn unblock(db: &PgPool, blocker: Uuid, exchange: Uuid) -> Result<(), ApiError> {
    let mut tx = db.begin().await?;
    service::acting(&mut tx, blocker).await?;
    let party = match as_party(&mut tx, exchange, blocker).await {
        Ok(other) => Some(other),
        Err(error) if error.code == ErrorCode::NotFound => None,
        Err(error) => return Err(error),
    };
    let Some(other) = party else {
        let lifted = sqlx::query(
            "DELETE FROM account_block b
             USING slot_holding mine, participant theirs
             WHERE mine.exchange_id = $1
               AND (mine.account_id = $2
                    OR mine.account_id IN (SELECT id FROM account WHERE merged_into = $2))
               AND theirs.exchange_id = mine.exchange_id AND theirs.slot <> mine.slot
               AND b.blocker_account_id = $2
               AND b.blocked_account_id = theirs.account_id",
        )
        .bind(exchange)
        .bind(blocker)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if lifted == 0 {
            return Err(ErrorCode::NotFound.into());
        }
        tx.commit().await?;
        return Ok(());
    };

    let blocked = other.account.ok_or(ErrorCode::ActionNotAllowed)?;
    sqlx::query(
        "DELETE FROM account_block WHERE blocker_account_id = $1 AND blocked_account_id = $2",
    )
    .bind(blocker)
    .bind(blocked)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The people the caller has blocked, most recently blocked first.
///
/// A block is stored between two accounts and does not say which exchange it
/// was made through, so each person is shown by the most recent exchange the
/// two share. An exchange the caller is still a party to comes before one
/// they only once held a place in, which is all there is to show for a block
/// that took them out of it; there the other party is named as the last
/// proposal they saw named them, since what the exchange has called them
/// since is none of their business. Places held by accounts combined into
/// the caller's count as the caller's, as their blocks do.
pub async fn blocked_people(db: &PgPool, blocker: Uuid) -> Result<Vec<BlockedPerson>, ApiError> {
    let rows: Vec<(Uuid, String, String, OffsetDateTime, bool)> = sqlx::query_as(
        "SELECT exchange_id, display_code, name, blocked_at, left_it
         FROM (
             SELECT DISTINCT ON (b.blocked_account_id)
                    e.id AS exchange_id, e.display_code,
                    CASE WHEN mine.ended_at IS NULL THEN theirs.display_name
                         ELSE coalesce((
                             SELECT CASE WHEN theirs.slot = 'A' THEN r.party_a_name
                                         ELSE r.party_b_name END
                             FROM revision r
                             WHERE r.exchange_id = e.id AND r.created_at <= mine.ended_at
                             ORDER BY r.sequence DESC
                             LIMIT 1), '')
                    END AS name,
                    b.created_at AS blocked_at, mine.ended_at IS NOT NULL AS left_it
             FROM account_block b
             JOIN slot_holding mine
               ON mine.account_id = b.blocker_account_id
               OR mine.account_id IN (SELECT id FROM account
                                      WHERE merged_into = b.blocker_account_id)
             JOIN participant theirs
               ON theirs.exchange_id = mine.exchange_id
              AND theirs.slot <> mine.slot
              AND theirs.account_id = b.blocked_account_id
             JOIN exchange e ON e.id = mine.exchange_id
             WHERE b.blocker_account_id = $1
             ORDER BY b.blocked_account_id, mine.ended_at IS NOT NULL, e.created_at DESC, e.id
         ) latest
         ORDER BY blocked_at DESC, exchange_id
         LIMIT $2",
    )
    .bind(blocker)
    .bind(BLOCK_LIST_MAX)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(exchange_id, display_code, name, blocked_at, left)| BlockedPerson {
                exchange_id,
                display_code,
                name,
                blocked_at: rfc3339(blocked_at),
                left: left.then_some(true),
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_are_trimmed_bounded_and_needed_for_other() {
        let (_, details) = checked(ReportReason::Scam, Some("  asked for a deposit  ".into()))
            .ok()
            .unwrap();
        assert_eq!(details.as_deref(), Some("asked for a deposit"));

        // Only spaces is nothing.
        let (_, details) = checked(ReportReason::Scam, Some("   ".into()))
            .ok()
            .unwrap();
        assert_eq!(details, None);
        assert!(checked(ReportReason::Harassment, None).is_ok());

        assert!(checked(ReportReason::Other, None).is_err());
        assert!(checked(ReportReason::Other, Some(" ".into())).is_err());
        assert!(checked(ReportReason::Other, Some("see the terms".into())).is_ok());

        // Counted in characters, not bytes.
        let longest = "é".repeat(REPORT_DETAILS_MAX_CHARS);
        assert!(checked(ReportReason::Scam, Some(longest.clone())).is_ok());
        assert!(checked(ReportReason::Scam, Some(longest + "é")).is_err());
    }

    #[test]
    fn a_reason_is_stored_as_the_api_spells_it() {
        for reason in [
            ReportReason::Harassment,
            ReportReason::ProhibitedTrade,
            ReportReason::Scam,
            ReportReason::Impersonation,
            ReportReason::Underage,
            ReportReason::Unwanted,
            ReportReason::Other,
        ] {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::Value::from(reason.as_str())
            );
        }
    }
}
