//! Staff review of abuse reports (DESIGN.md §9; decided 3 October 2026).
//!
//! Reports wait in a queue, oldest first, each with its age; the decision is
//! that every report is reviewed within a day, every day. A reviewer opens
//! one, reads the reported exchange's history and record, and resolves it:
//! dismissed, the exchange's content hidden from the reported person, the
//! reported person's account suspended, or both of the last two.
//!
//! **Who reviews.** An account named in `staff_member`, which only the
//! owner can write, with the `staff` command and the schema owner's
//! connection (`src/bin/staff.rs`); the service may read it and nothing
//! more. A reviewer signs in like anyone else, with a one-time code, and the
//! staff endpoints also want that code to have been entered recently
//! ([`STAFF_SIGN_IN_MAX_AGE`]). To anyone else every staff path answers
//! "not found".
//!
//! **What a reviewer can read.** The reported exchange's terms and history,
//! as a party's copy of the record holds them, and only while the report is
//! open: a resolved report shows nothing more. Opening a report is itself
//! recorded, like every action.
//!
//! **The audit history.** Every view and every action is a row in
//! `review_event`, which the database refuses to change or remove: who, when,
//! what, about which report, exchange and account, and the reviewer's note.
//! The report row carries its resolution (who, when, the outcome and the
//! note), set once and never changed; anything done about the same matter
//! later, such as lifting a suspension or showing hidden content again, is a
//! new event and leaves the resolution as it was.
//!
//! **Hiding content.** §9: "a reviewer can hide an exchange's content from
//! the other party". In this product a report is about an exchange "and with
//! it the other party" (`crate::safety`), so the other party is the person
//! reported, and that is whose view changes: everything the parties wrote in
//! the exchange (its terms, each item's description and completion criteria,
//! the message sent with a revision, and every note, reason and statement in
//! the history) reads as one placeholder, "Hidden by review" in their
//! language, in the exchange, its history and their copy of the record. They
//! can no longer sign or send terms there (`CONTENT_HIDDEN`), since they
//! cannot read them; declining, withdrawing, delivering and closing work as
//! before, so the exchange can still end (§3, invariant 5). The reporter's
//! view does not change, and nothing stored changes: the record stays whole
//! and a reviewer can show the text again. The point is what the reporter
//! wrote into the exchange (an address, a phone number, anything they would
//! not have shared with someone who turned out to abuse it): the person
//! reported loses it. Names and amounts stay, since the exchange has to
//! remain recognisable and the agreement still has to be ended.
//!
//! **Suspending.** The account's status becomes `SUSPENDED`: every session
//! ends, signing in is refused (`ACCOUNT_SUSPENDED`), nothing is sent to it,
//! and its Wallet passes stop updating. Nothing new can bind it while it is
//! suspended, so in the same transaction: the invitation links it issued that
//! nobody has taken stop working, an offer or amendment it sent that is still
//! waiting to be signed is withdrawn in its name, and where it had opened an
//! invitation and was not yet confirmed it leaves, which voids its signature
//! there. These are ordinary steps through the rules, recorded and notified
//! like any other, and say nothing about a suspension: the other party sees a
//! withdrawal, a departure or a dead link, as after a block (`crate::safety`).
//! Agreements in force are left as they stand; the other party can still end
//! them. A reviewer can lift the suspension again; what it ended stays ended.
//!
//! **Who may not review a report.** A reviewer never handles a report they
//! are part of: one they made, one about them, or one about an exchange in
//! which they hold or once held a place (`slot_holding`, which keeps
//! claimants since removed). To them such a report does not exist: it is
//! not in their queue, and opening or resolving it answers exactly as a
//! report that does not exist, before anything is read or recorded. The
//! same goes for a suspension or hidden content that such a report led to,
//! and for their own account. A reviewer cannot suspend another reviewer
//! (`SUBJECT_IS_REVIEWER`): the owner first takes the reviewer's role away
//! with `staff revoke`, and the report is then reviewed like any other.
//!
//! **The alert.** A new report queues an email to every reviewer through
//! the outbox: it says only that a report is waiting, never what it says,
//! and links to the review screen. A reviewer who has not yet been sent the
//! last one is not sent another.

use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::contact::{self, Field, Kind};
use crate::domain::Rules;
use crate::domain::exchange::{Actor, Command, Counterparty, decide};
use crate::domain::identity::Identifier;
use crate::domain::revision::Slot;
use crate::error::{ApiError, ErrorCode};
use crate::exchanges::dto::{ExchangeView, RevisionTerms, rfc3339};
use crate::exchanges::record::dto::{RecordEvent, RecordRevision, ReviewRecord};
use crate::exchanges::record::{self, Limits, notices};
use crate::exchanges::repo;
use crate::safety::ReportReason;

// ---- Limits -----------------------------------------------------------------
//
// Placeholders, like the other numbers of §9.

/// How soon a report is to be reviewed (§9: within 24 hours, every day). An
/// open report older than this is overdue, and the screen says so.
pub const REVIEW_WITHIN: Duration = Duration::hours(24);
/// How long after entering a one-time code a reviewer may still use the
/// staff endpoints. After that they sign in again.
pub const STAFF_SIGN_IN_MAX_AGE: Duration = Duration::hours(12);
/// Reports one reviewer may open in an hour. Far above what reading takes,
/// and low enough that a stolen session cannot page through every report.
pub const VIEWS_PER_HOUR: i64 = 300;
/// Actions one reviewer may take in an hour.
pub const ACTIONS_PER_HOUR: i64 = 60;
/// Lists one reviewer may read in an hour: the queue, the suspended accounts
/// and the hidden content, together. The review screen reads all three each
/// time it opens and after each action.
pub const LISTS_PER_HOUR: i64 = 600;
/// Longest note a reviewer may write, in characters.
pub const NOTE_MAX_CHARS: usize = 1000;
/// How many entries a list answers with at most.
const LIST_MAX: i64 = 500;
/// How many entries of the audit history a report shows, the latest.
const HISTORY_MAX: i64 = 200;

// ---- Shapes -----------------------------------------------------------------

/// How a reviewer resolved a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewOutcome {
    /// Nothing to act on.
    Dismissed,
    /// What the parties wrote in the exchange is hidden from the reported
    /// person.
    ContentHidden,
    /// The reported person's account is suspended.
    AccountSuspended,
    /// Both of the above.
    ContentHiddenAndAccountSuspended,
}

impl ReviewOutcome {
    const ALL: [ReviewOutcome; 4] = [
        ReviewOutcome::Dismissed,
        ReviewOutcome::ContentHidden,
        ReviewOutcome::AccountSuspended,
        ReviewOutcome::ContentHiddenAndAccountSuspended,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewOutcome::Dismissed => "DISMISSED",
            ReviewOutcome::ContentHidden => "CONTENT_HIDDEN",
            ReviewOutcome::AccountSuspended => "ACCOUNT_SUSPENDED",
            ReviewOutcome::ContentHiddenAndAccountSuspended => {
                "CONTENT_HIDDEN_AND_ACCOUNT_SUSPENDED"
            }
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == text)
    }

    fn hides(self) -> bool {
        matches!(
            self,
            ReviewOutcome::ContentHidden | ReviewOutcome::ContentHiddenAndAccountSuspended
        )
    }

    fn suspends(self) -> bool {
        matches!(
            self,
            ReviewOutcome::AccountSuspended | ReviewOutcome::ContentHiddenAndAccountSuspended
        )
    }
}

/// One entry of the audit history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewAction {
    /// A reviewer opened the report and read the exchange.
    ReportViewed,
    ReportDismissed,
    ContentHidden,
    /// Hidden content shown again.
    ContentRestored,
    AccountSuspended,
    SuspensionLifted,
    /// The owner made the account a reviewer, from the command line.
    StaffGranted,
    /// The owner took that away, from the command line.
    StaffRevoked,
}

impl ReviewAction {
    const ALL: [ReviewAction; 8] = [
        ReviewAction::ReportViewed,
        ReviewAction::ReportDismissed,
        ReviewAction::ContentHidden,
        ReviewAction::ContentRestored,
        ReviewAction::AccountSuspended,
        ReviewAction::SuspensionLifted,
        ReviewAction::StaffGranted,
        ReviewAction::StaffRevoked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewAction::ReportViewed => "REPORT_VIEWED",
            ReviewAction::ReportDismissed => "REPORT_DISMISSED",
            ReviewAction::ContentHidden => "CONTENT_HIDDEN",
            ReviewAction::ContentRestored => "CONTENT_RESTORED",
            ReviewAction::AccountSuspended => "ACCOUNT_SUSPENDED",
            ReviewAction::SuspensionLifted => "SUSPENSION_LIFTED",
            ReviewAction::StaffGranted => "STAFF_GRANTED",
            ReviewAction::StaffRevoked => "STAFF_REVOKED",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.as_str() == text)
    }
}

/// Where an account stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AccountStanding {
    Active,
    Suspended,
    Deleted,
}

impl AccountStanding {
    fn parse(text: &str) -> Self {
        match text {
            "SUSPENDED" => AccountStanding::Suspended,
            "DELETED" => AccountStanding::Deleted,
            _ => AccountStanding::Active,
        }
    }
}

/// Where a report stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportStatus {
    Open,
    Dismissed,
    Actioned,
}

impl ReportStatus {
    fn parse(text: &str) -> Self {
        match text {
            "DISMISSED" => ReportStatus::Dismissed,
            "ACTIONED" => ReportStatus::Actioned,
            _ => ReportStatus::Open,
        }
    }
}

/// An open report, as the queue lists it: enough to choose what to open
/// next, and nothing of what the reporter wrote or who they are. Those are
/// read by opening the report, which is recorded and limited.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueuedReport {
    pub id: Uuid,
    /// RFC 3339.
    pub created_at: String,
    /// Seconds since it was made, when the answer was given.
    pub age_seconds: i64,
    /// Older than the time a report is to be reviewed within.
    pub overdue: bool,
    pub reason: ReportReason,
    /// Whether an account made it. Every report made now has one; it is
    /// false only for a report made through an invitation link without
    /// signing in, before reporting needed an account.
    pub has_reporter: bool,
    pub display_code: Option<String>,
}

/// A report as its page shows it, once opened.
#[derive(Debug, Serialize, ToSchema)]
pub struct OpenedReport {
    pub id: Uuid,
    /// RFC 3339.
    pub created_at: String,
    /// Seconds since it was made, when the answer was given.
    pub age_seconds: i64,
    /// Older than the time a report is to be reviewed within.
    pub overdue: bool,
    pub reason: ReportReason,
    pub details: Option<String>,
    /// Who made it. Every report made now has one; it is absent only for a
    /// report made through an invitation link without signing in, before
    /// reporting needed an account.
    pub reporter_account_id: Option<Uuid>,
    /// The person reported.
    pub subject_account_id: Option<Uuid>,
    pub exchange_id: Option<Uuid>,
    pub display_code: Option<String>,
}

/// The queue of open reports, oldest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReviewQueue {
    /// How soon a report is to be reviewed (DESIGN.md §9).
    pub review_within_hours: i64,
    pub reports: Vec<QueuedReport>,
}

/// One side of a report, as the reviewer is shown it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReviewedAccount {
    pub id: Uuid,
    pub status: AccountStanding,
    /// Their side of the exchange, if they hold one.
    pub party: Option<Slot>,
    /// Their name as the exchange writes it; empty if they hold no side.
    pub name: String,
}

/// Another report about the same exchange.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelatedReport {
    pub id: Uuid,
    pub created_at: String,
    pub reason: ReportReason,
    pub status: ReportStatus,
    pub outcome: Option<ReviewOutcome>,
}

/// One entry of the audit history.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReviewEntry {
    pub id: i64,
    pub action: ReviewAction,
    /// The reviewer; absent for the owner's command line: naming and
    /// removing reviewers, and a suspension lifted by replaying the deletion
    /// log after a restore, whose note says so.
    pub staff_account_id: Option<Uuid>,
    /// RFC 3339.
    pub at: String,
    pub note: Option<String>,
    pub report_id: Option<Uuid>,
    pub exchange_id: Option<Uuid>,
    /// The account acted on.
    pub account_id: Option<Uuid>,
}

/// A report opened for review: what it says, who it is about, the
/// exchange's record, and what review has done about it so far.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportDetail {
    pub report: OpenedReport,
    pub reporter: Option<ReviewedAccount>,
    pub subject: Option<ReviewedAccount>,
    /// Whether the exchange's content is already hidden from the person
    /// reported.
    pub content_hidden: bool,
    /// The reported exchange's record, with nothing hidden.
    pub record: Option<ReviewRecord>,
    /// The other reports about the same exchange, oldest first.
    pub other_reports: Vec<RelatedReport>,
    /// The audit history of this report, its exchange and the person
    /// reported: the latest 200 entries, oldest first.
    pub history: Vec<ReviewEntry>,
}

/// Resolving a report.
#[derive(Debug, Deserialize, ToSchema)]
pub struct Resolution {
    pub outcome: ReviewOutcome,
    /// What the reviewer found, for the audit history. Required for every
    /// outcome but `DISMISSED`. At most 1,000 characters.
    pub note: Option<String>,
}

/// A reviewer's note, required for lifting a suspension.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StaffNote {
    /// At most 1,000 characters.
    pub note: String,
}

/// Showing hidden content again.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreContent {
    pub exchange_id: Uuid,
    pub account_id: Uuid,
    /// At most 1,000 characters.
    pub note: String,
}

/// A suspended account.
#[derive(Debug, Serialize, ToSchema)]
pub struct Suspension {
    pub account_id: Uuid,
    /// The name on the account.
    pub name: String,
    /// When review suspended it, RFC 3339; absent if review did not.
    pub suspended_at: Option<String>,
    pub note: Option<String>,
    pub report_id: Option<Uuid>,
}

/// Content hidden from one account.
#[derive(Debug, Serialize, ToSchema)]
pub struct HiddenContent {
    pub exchange_id: Uuid,
    pub display_code: String,
    pub account_id: Uuid,
    /// Their name as the exchange writes it.
    pub name: String,
    pub hidden_at: String,
    pub report_id: Uuid,
}

// ---- Who reviews ------------------------------------------------------------

/// Whether an account is a reviewer.
pub async fn is_staff(db: &PgPool, account: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM staff_member WHERE account_id = $1)")
        .bind(account)
        .fetch_one(db)
        .await
}

/// Whether a one-time code entered at `authenticated_at` is recent enough
/// for the staff endpoints at `now`.
pub fn signed_in_recently(authenticated_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    now - authenticated_at <= STAFF_SIGN_IN_MAX_AGE
}

// ---- Reading ----------------------------------------------------------------

const REASONS: [ReportReason; 7] = [
    ReportReason::Harassment,
    ReportReason::ProhibitedTrade,
    ReportReason::Scam,
    ReportReason::Impersonation,
    ReportReason::Underage,
    ReportReason::Unwanted,
    ReportReason::Other,
];

fn reason(text: &str) -> ReportReason {
    REASONS
        .into_iter()
        .find(|reason| reason.as_str() == text)
        .unwrap_or(ReportReason::Other)
}

/// Whether the reviewer bound as `staff` (a parameter such as `$2`) takes
/// part in the report aliased `r`: they made it, it is about them, or they
/// hold or once held a place in the exchange it is about. `slot_holding`
/// keeps every account that has held a slot, a claimant since removed
/// included; `participant` is asked as well, for the slot as it stands.
fn involves(staff: &str) -> String {
    format!(
        "(r.reporter_account_id IS NOT DISTINCT FROM {staff}
          OR r.subject_account_id IS NOT DISTINCT FROM {staff}
          OR EXISTS (SELECT 1 FROM slot_holding h
                     WHERE h.exchange_id = r.subject_exchange_id AND h.account_id = {staff})
          OR EXISTS (SELECT 1 FROM participant p
                     WHERE p.exchange_id = r.subject_exchange_id AND p.account_id = {staff}))"
    )
}

type ReportRow = (
    Uuid,
    OffsetDateTime,
    i64,
    String,
    Option<String>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
);

const REPORT_COLUMNS: &str = "r.id, r.created_at,
    EXTRACT(EPOCH FROM now() - r.created_at)::bigint AS age,
    r.reason, r.details, r.reporter_account_id, r.subject_account_id,
    r.subject_exchange_id, e.display_code";

fn opened(row: ReportRow) -> OpenedReport {
    let (id, created_at, age, reason_text, details, reporter, subject, exchange, code) = row;
    OpenedReport {
        id,
        created_at: rfc3339(created_at),
        age_seconds: age,
        overdue: age > REVIEW_WITHIN.whole_seconds(),
        reason: reason(&reason_text),
        details,
        reporter_account_id: reporter,
        subject_account_id: subject,
        exchange_id: exchange,
        display_code: code,
    }
}

/// A report as the queue lists it: ID, made when, age, reason, whether an
/// account made it, and the exchange's code.
type QueueRow = (Uuid, OffsetDateTime, i64, String, bool, Option<String>);

/// The open reports, oldest first, leaving out those the reviewer takes part
/// in.
pub async fn queue(db: &PgPool, staff: Uuid) -> Result<ReviewQueue, ApiError> {
    let mut tx = db.begin().await?;
    within_list_limit(&mut tx, staff).await?;
    let rows: Vec<QueueRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT r.id, r.created_at, EXTRACT(EPOCH FROM now() - r.created_at)::bigint,
                r.reason, r.reporter_account_id IS NOT NULL, e.display_code
         FROM report r LEFT JOIN exchange e ON e.id = r.subject_exchange_id
         WHERE r.status = 'OPEN' AND NOT {}
         ORDER BY r.created_at, r.id
         LIMIT $2",
        involves("$1")
    )))
    .bind(staff)
    .bind(LIST_MAX)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ReviewQueue {
        review_within_hours: REVIEW_WITHIN.whole_hours(),
        reports: rows
            .into_iter()
            .map(
                |(id, created_at, age, reason_text, has_reporter, display_code)| QueuedReport {
                    id,
                    created_at: rfc3339(created_at),
                    age_seconds: age,
                    overdue: age > REVIEW_WITHIN.whole_seconds(),
                    reason: reason(&reason_text),
                    has_reporter,
                    display_code,
                },
            )
            .collect(),
    })
}

/// Holds the reviewer's own counts until the transaction ends, so that
/// counting and adding cannot be raced past a limit by requests made at the
/// same moment.
async fn hold_reviewer(conn: &mut PgConnection, staff: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("staff limits {staff}"))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Refuses a reviewer who has done `limit` of these in the last hour.
async fn within_limit(
    conn: &mut PgConnection,
    staff: Uuid,
    views: bool,
    limit: i64,
) -> Result<(), ApiError> {
    hold_reviewer(conn, staff).await?;
    let done: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM review_event
         WHERE staff_account_id = $1 AND occurred_at > now() - interval '1 hour'
           AND (action = 'REPORT_VIEWED') = $2",
    )
    .bind(staff)
    .bind(views)
    .fetch_one(&mut *conn)
    .await?;
    if done >= limit {
        return Err(ErrorCode::TooManyRequests.into());
    }
    Ok(())
}

/// Counts a list read, and refuses a reviewer who has read
/// [`LISTS_PER_HOUR`] this hour.
async fn within_list_limit(conn: &mut PgConnection, staff: Uuid) -> Result<(), ApiError> {
    hold_reviewer(conn, staff).await?;
    sqlx::query(
        "DELETE FROM staff_list_limit
         WHERE staff_account_id = $1 AND window_start < now() - interval '1 day'",
    )
    .bind(staff)
    .execute(&mut *conn)
    .await?;
    let read: i32 = sqlx::query_scalar(
        "INSERT INTO staff_list_limit (staff_account_id, window_start, count)
         VALUES ($1, date_trunc('hour', now()), 1)
         ON CONFLICT (staff_account_id, window_start)
         DO UPDATE SET count = staff_list_limit.count + 1
         RETURNING count",
    )
    .bind(staff)
    .fetch_one(&mut *conn)
    .await?;
    if i64::from(read) > LISTS_PER_HOUR {
        return Err(ErrorCode::TooManyRequests.into());
    }
    Ok(())
}

/// One side of a report: the account, and the side of the exchange it holds.
async fn reviewed_account(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Option<Uuid>,
) -> Result<ReviewedAccount, sqlx::Error> {
    let status: String = sqlx::query_scalar("SELECT status FROM account WHERE id = $1")
        .bind(account)
        .fetch_one(&mut *conn)
        .await?;
    let side: Option<(String, String)> = match exchange {
        Some(exchange) => {
            sqlx::query_as(
                "SELECT slot, display_name FROM participant
                 WHERE exchange_id = $1 AND account_id = $2",
            )
            .bind(exchange)
            .bind(account)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let (party, name) = match side {
        Some((slot, name)) => (Some(if slot == "A" { Slot::A } else { Slot::B }), name),
        None => (None, String::new()),
    };
    Ok(ReviewedAccount {
        id: account,
        status: AccountStanding::parse(&status),
        party,
        name,
    })
}

async fn record_event(
    conn: &mut PgConnection,
    staff: Option<Uuid>,
    action: ReviewAction,
    report: Option<Uuid>,
    exchange: Option<Uuid>,
    account: Option<Uuid>,
    note: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO review_event
            (staff_account_id, action, report_id, exchange_id, account_id, note)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(staff)
    .bind(action.as_str())
    .bind(report)
    .bind(exchange)
    .bind(account)
    .bind(note)
    .execute(conn)
    .await?;
    Ok(())
}

/// Opens a report for review: refused once it is resolved, and recorded in
/// the audit history. A report the reviewer takes part in is, to them, a
/// report that does not exist: the same answer, and nothing read or
/// recorded.
pub async fn open_report(db: &PgPool, staff: Uuid, report: Uuid) -> Result<ReportDetail, ApiError> {
    let mut tx = db.begin().await?;
    within_limit(&mut tx, staff, true, VIEWS_PER_HOUR).await?;

    let row: Option<(ReportRow, String)> = sqlx::query_as::<
        _,
        (
            Uuid,
            OffsetDateTime,
            i64,
            String,
            Option<String>,
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            Option<String>,
            String,
        ),
    >(sqlx::AssertSqlSafe(format!(
        "SELECT {REPORT_COLUMNS}, r.status
         FROM report r LEFT JOIN exchange e ON e.id = r.subject_exchange_id
         WHERE r.id = $1 AND NOT {}",
        involves("$2")
    )))
    .bind(report)
    .bind(staff)
    .fetch_optional(&mut *tx)
    .await?
    .map(|(a, b, c, d, e, f, g, h, i, status)| ((a, b, c, d, e, f, g, h, i), status));
    let (row, status) = row.ok_or(ErrorCode::NotFound)?;
    // The exchange is the reviewer's to read only while a report about it
    // waits for a decision.
    if status != "OPEN" {
        return Err(ErrorCode::ReportResolved.into());
    }
    let report = opened(row);
    let (exchange, subject_id) = (report.exchange_id, report.subject_account_id);

    record_event(
        &mut tx,
        Some(staff),
        ReviewAction::ReportViewed,
        Some(report.id),
        exchange,
        subject_id,
        None,
    )
    .await?;

    let language: String = sqlx::query_scalar("SELECT language FROM account WHERE id = $1")
        .bind(staff)
        .fetch_one(&mut *tx)
        .await?;
    let record = match exchange {
        Some(exchange) => {
            record::for_review(&mut tx, exchange, &language, &Limits::default()).await?
        }
        None => None,
    };

    let subject = match subject_id {
        Some(account) => Some(reviewed_account(&mut tx, account, exchange).await?),
        None => None,
    };
    let reporter = match report.reporter_account_id {
        Some(account) => Some(reviewed_account(&mut tx, account, exchange).await?),
        None => None,
    };
    let content_hidden = match (exchange, subject_id) {
        (Some(exchange), Some(account)) => is_hidden_from(&mut tx, exchange, account).await?,
        _ => false,
    };

    let others: Vec<(Uuid, OffsetDateTime, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, created_at, reason, status, outcome FROM report
         WHERE subject_exchange_id = $1 AND id <> $2
         ORDER BY created_at, id
         LIMIT $3",
    )
    .bind(exchange)
    .bind(report.id)
    .bind(LIST_MAX)
    .fetch_all(&mut *tx)
    .await?;
    let other_reports = others
        .into_iter()
        .map(
            |(id, created_at, reason_text, status, outcome)| RelatedReport {
                id,
                created_at: rfc3339(created_at),
                reason: reason(&reason_text),
                status: ReportStatus::parse(&status),
                outcome: outcome.as_deref().and_then(ReviewOutcome::parse),
            },
        )
        .collect();

    let history = history(&mut tx, report.id, exchange, subject_id).await?;
    tx.commit().await?;

    Ok(ReportDetail {
        report,
        reporter,
        subject,
        content_hidden,
        record,
        other_reports,
        history,
    })
}

type EntryRow = (
    i64,
    String,
    Option<Uuid>,
    OffsetDateTime,
    Option<String>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
);

/// The latest entries of the audit history about a report, its exchange or
/// the person it reports, oldest first.
async fn history(
    conn: &mut PgConnection,
    report: Uuid,
    exchange: Option<Uuid>,
    account: Option<Uuid>,
) -> Result<Vec<ReviewEntry>, sqlx::Error> {
    let mut rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT id, action, staff_account_id, occurred_at, note, report_id, exchange_id, account_id
         FROM review_event
         WHERE report_id = $1 OR exchange_id = $2 OR account_id = $3
         ORDER BY id DESC
         LIMIT $4",
    )
    .bind(report)
    .bind(exchange)
    .bind(account)
    .bind(HISTORY_MAX)
    .fetch_all(conn)
    .await?;
    rows.reverse();
    Ok(rows
        .into_iter()
        .filter_map(|(id, action, staff, at, note, report, exchange, account)| {
            Some(ReviewEntry {
                id,
                action: ReviewAction::parse(&action)?,
                staff_account_id: staff,
                at: rfc3339(at),
                note,
                report_id: report,
                exchange_id: exchange,
                account_id: account,
            })
        })
        .collect())
}

// ---- Acting -----------------------------------------------------------------

/// A note trimmed and bounded, with nothing where there was none; refused
/// when `required` and empty.
fn checked_note(note: Option<String>, required: bool) -> Result<Option<String>, ApiError> {
    let note = note
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let too_long = note
        .as_ref()
        .is_some_and(|text| text.chars().count() > NOTE_MAX_CHARS);
    if too_long || (required && note.is_none()) {
        return Err(ErrorCode::InvalidRequest.into());
    }
    Ok(note)
}

/// Suspends an active account, in the caller's transaction, which holds the
/// account row: every session ends, the devices that were signed in with
/// them stop getting notifications, and nothing new can bind it. The
/// invitation links it issued that nobody took stop working, and wherever
/// something it sent is still waiting to be signed, it is withdrawn in the
/// account's name; where it had opened an invitation and was not yet
/// confirmed, it leaves, which voids its signature there. Each is an
/// ordinary step through the rules, which the other party sees as such.
///
/// Why withdraw rather than wait: the other party could otherwise sign an
/// offer the suspended person left open, and so make a new agreement with
/// someone who can no longer sign in to keep it, read it or end it (§9).
/// Offers sent to the suspended person are left alone: they cannot accept
/// them while suspended, and each lapses on its own timer.
async fn suspend(conn: &mut PgConnection, rules: &Rules, account: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1 AND status = 'ACTIVE'")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE account_session SET revoked_at = now()
         WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(account)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM device WHERE account_id = $1")
        .bind(account)
        .execute(&mut *conn)
        .await?;

    // Always in the same order, as the deletion takes them, so two of these
    // at once cannot each hold a lock the other is waiting for.
    let open: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM exchange e JOIN participant p ON p.exchange_id = e.id
         WHERE p.account_id = $1 AND e.state IN ('NEGOTIATING', 'ACTIVE')
         ORDER BY e.id",
    )
    .bind(account)
    .fetch_all(&mut *conn)
    .await?;
    for exchange in open {
        let Some(aggregate) = repo::load(conn, exchange, true).await? else {
            continue;
        };
        let Some(slot) = aggregate.slot_of(account) else {
            continue;
        };
        let current = &aggregate.exchange;
        let mut commands = Vec::new();
        if slot == Slot::B && current.counterparty == Counterparty::Claimed {
            commands.push(Command::ReleaseClaim);
        }
        if let Some(open) = &current.open
            && open.author == slot
        {
            commands.push(Command::Withdraw { revision: open.id });
        }
        let at = OffsetDateTime::now_utc();
        let actor = Actor::Party(slot);
        // The rules decide, as for any command; where they allow none, the
        // exchange is left as it stands.
        let decision = commands
            .into_iter()
            .find_map(|command| decide(current, actor, command, at, rules).ok());
        if let Some(decision) = decision {
            repo::persist(conn, &aggregate, &decision, actor, None, at).await?;
        }
    }

    // Lifting the suspension later does not bring these back.
    sqlx::query(
        "UPDATE invitation i SET revoked_at = coalesce(i.revoked_at, now())
         FROM participant p
         WHERE p.exchange_id = i.exchange_id AND p.slot = 'A' AND p.account_id = $1
           AND i.claimed_by IS NULL AND i.revoked_at IS NULL",
    )
    .bind(account)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether an account is a reviewer, in the caller's transaction.
async fn is_reviewer(conn: &mut PgConnection, account: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM staff_member WHERE account_id = $1)")
        .bind(account)
        .fetch_one(conn)
        .await
}

/// Resolves an open report, once. A report the reviewer takes part in is, to
/// them, a report that does not exist.
pub async fn resolve(
    db: &PgPool,
    rules: &Rules,
    staff: Uuid,
    report: Uuid,
    body: Resolution,
) -> Result<(), ApiError> {
    let outcome = body.outcome;
    let note = checked_note(body.note, outcome != ReviewOutcome::Dismissed)?;

    let mut tx = db.begin().await?;
    within_limit(&mut tx, staff, false, ACTIONS_PER_HOUR).await?;

    let found: Option<(String, Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT r.status, r.subject_exchange_id, r.subject_account_id FROM report r
             WHERE r.id = $1 AND NOT {}
             FOR UPDATE OF r",
            involves("$2")
        )))
        .bind(report)
        .bind(staff)
        .fetch_optional(&mut *tx)
        .await?;
    let (status, exchange, subject) = found.ok_or(ErrorCode::NotFound)?;
    if status != "OPEN" {
        return Err(ErrorCode::ReportResolved.into());
    }

    if outcome.suspends() {
        let Some(subject) = subject else {
            return Err(ErrorCode::ActionNotAllowed.into());
        };
        // A reviewer does not decide a report about themselves (already
        // left out above, and checked again here).
        if subject == staff {
            return Err(ErrorCode::NotFound.into());
        }
        // Nor suspend another reviewer: the owner takes the role away first
        // (`staff revoke`), and the report is then reviewed like any other.
        if is_reviewer(&mut tx, subject).await? {
            return Err(ErrorCode::SubjectIsReviewer.into());
        }
    }

    if outcome.hides() {
        let (Some(exchange), Some(subject)) = (exchange, subject) else {
            return Err(ErrorCode::ActionNotAllowed.into());
        };
        sqlx::query(
            "INSERT INTO hidden_content (exchange_id, account_id, report_id) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(exchange)
        .bind(subject)
        .bind(report)
        .execute(&mut *tx)
        .await?;
        record_event(
            &mut tx,
            Some(staff),
            ReviewAction::ContentHidden,
            Some(report),
            Some(exchange),
            Some(subject),
            note.as_deref(),
        )
        .await?;
    }

    if outcome.suspends() {
        let Some(subject) = subject else {
            return Err(ErrorCode::ActionNotAllowed.into());
        };
        // Held to the end. This strength of lock leaves alone transactions
        // that merely refer to the account, such as the other party acting
        // in an exchange the two share and queueing a message for it; they
        // may hold the lock on an exchange this one needs next
        // (`crate::deletion` takes the account the same way). The account's
        // own requests wait here, then find it suspended.
        let standing: String =
            sqlx::query_scalar("SELECT status FROM account WHERE id = $1 FOR NO KEY UPDATE")
                .bind(subject)
                .fetch_one(&mut *tx)
                .await?;
        match AccountStanding::parse(&standing) {
            AccountStanding::Deleted => return Err(ErrorCode::ActionNotAllowed.into()),
            AccountStanding::Active => suspend(&mut tx, rules, subject).await?,
            AccountStanding::Suspended => {}
        }
        record_event(
            &mut tx,
            Some(staff),
            ReviewAction::AccountSuspended,
            Some(report),
            exchange,
            Some(subject),
            note.as_deref(),
        )
        .await?;
    }

    if outcome == ReviewOutcome::Dismissed {
        record_event(
            &mut tx,
            Some(staff),
            ReviewAction::ReportDismissed,
            Some(report),
            exchange,
            subject,
            note.as_deref(),
        )
        .await?;
    }

    sqlx::query(
        "UPDATE report
         SET status = CASE WHEN $2 = 'DISMISSED' THEN 'DISMISSED' ELSE 'ACTIONED' END,
             resolved_at = now(), resolved_by = $3, outcome = $2, resolution_note = $4
         WHERE id = $1",
    )
    .bind(report)
    .bind(outcome.as_str())
    .bind(staff)
    .bind(note.as_deref())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// A suspended account: its ID and name, and the latest suspension's
/// time, note and report.
type SuspensionRow = (
    Uuid,
    String,
    Option<OffsetDateTime>,
    Option<String>,
    Option<Uuid>,
);

/// The latest suspension of the account `a`, as a lateral join: its time,
/// note and report.
const LATEST_SUSPENSION: &str = "LEFT JOIN LATERAL (
         SELECT occurred_at, note, report_id FROM review_event
         WHERE account_id = a.id AND action = 'ACCOUNT_SUSPENDED'
         ORDER BY id DESC
         LIMIT 1
     ) latest ON true
     LEFT JOIN report r ON r.id = latest.report_id";

/// The suspended accounts, most recently suspended first, leaving out the
/// reviewer's own and any that a report they take part in led to.
pub async fn suspensions(db: &PgPool, staff: Uuid) -> Result<Vec<Suspension>, ApiError> {
    let mut tx = db.begin().await?;
    within_list_limit(&mut tx, staff).await?;
    let rows: Vec<SuspensionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT a.id, a.display_name, latest.occurred_at, latest.note, latest.report_id
         FROM account a
         {LATEST_SUSPENSION}
         WHERE a.status = 'SUSPENDED' AND a.id <> $1
           AND (r.id IS NULL OR NOT {})
         ORDER BY latest.occurred_at DESC NULLS LAST, a.id
         LIMIT $2",
        involves("$1")
    )))
    .bind(staff)
    .bind(LIST_MAX)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(account_id, name, at, note, report_id)| Suspension {
            account_id,
            name,
            suspended_at: at.map(rfc3339),
            note,
            report_id,
        })
        .collect())
}

/// Lifts a suspension. The account can sign in again; the sessions it had
/// stay ended, and so does whatever the suspension ended. A reviewer's own
/// suspension, and one that a report they take part in led to, is to them
/// a suspension that does not exist; another reviewer's is the owner's to
/// lift (`SUBJECT_IS_REVIEWER`).
pub async fn lift(
    db: &PgPool,
    staff: Uuid,
    account: Uuid,
    body: StaffNote,
) -> Result<(), ApiError> {
    let Some(note) = checked_note(Some(body.note), true)? else {
        return Err(ErrorCode::InvalidRequest.into());
    };
    let mut tx = db.begin().await?;
    within_limit(&mut tx, staff, false, ACTIONS_PER_HOUR).await?;
    let (mine, suspended): (bool, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT a.id = $2 OR (r.id IS NOT NULL AND {}), a.status = 'SUSPENDED'
         FROM account a
         {LATEST_SUSPENSION}
         WHERE a.id = $1",
        involves("$2")
    )))
    .bind(account)
    .bind(staff)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or((true, false));
    if mine || !suspended {
        return Err(ErrorCode::NotFound.into());
    }
    if is_reviewer(&mut tx, account).await? {
        return Err(ErrorCode::SubjectIsReviewer.into());
    }
    if !lift_suspension(&mut tx, staff, account, &note).await? {
        return Err(ErrorCode::NotFound.into());
    }
    tx.commit().await?;
    Ok(())
}

/// Lifts a suspension in the caller's transaction and records it in the
/// audit history as the reviewer's, with the note. Returns whether the
/// account was suspended. (Replaying the deletion log lifts a suspension as
/// the owner, through the database's own `replay_lift_suspension`,
/// migration 0019: `crate::deletion::replay`.)
async fn lift_suspension(
    conn: &mut PgConnection,
    staff: Uuid,
    account: Uuid,
    note: &str,
) -> Result<bool, sqlx::Error> {
    let lifted =
        sqlx::query("UPDATE account SET status = 'ACTIVE' WHERE id = $1 AND status = 'SUSPENDED'")
            .bind(account)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if lifted == 0 {
        return Ok(false);
    }
    // Tied to the report that led to the suspension, so that both read
    // together in its history.
    let report: Option<Uuid> = sqlx::query_scalar(
        "SELECT report_id FROM review_event
         WHERE account_id = $1 AND action = 'ACCOUNT_SUSPENDED'
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(account)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    record_event(
        conn,
        Some(staff),
        ReviewAction::SuspensionLifted,
        report,
        None,
        Some(account),
        Some(note),
    )
    .await?;
    Ok(true)
}

/// Content hidden by review, most recently hidden first, leaving out what
/// is hidden from the reviewer and what a report they take part in hid.
pub async fn hidden(db: &PgPool, staff: Uuid) -> Result<Vec<HiddenContent>, ApiError> {
    let mut tx = db.begin().await?;
    within_list_limit(&mut tx, staff).await?;
    let rows: Vec<(Uuid, String, Uuid, Option<String>, OffsetDateTime, Uuid)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT h.exchange_id, e.display_code, h.account_id, p.display_name, h.hidden_at,
                    h.report_id
             FROM hidden_content h
             JOIN exchange e ON e.id = h.exchange_id
             JOIN report r ON r.id = h.report_id
             LEFT JOIN participant p
               ON p.exchange_id = h.exchange_id AND p.account_id = h.account_id
             WHERE h.account_id <> $1 AND NOT {}
             ORDER BY h.hidden_at DESC, h.exchange_id
             LIMIT $2",
            involves("$1")
        )))
        .bind(staff)
        .bind(LIST_MAX)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(
            |(exchange_id, display_code, account_id, name, at, report_id)| HiddenContent {
                exchange_id,
                display_code,
                account_id,
                name: name.unwrap_or_default(),
                hidden_at: rfc3339(at),
                report_id,
            },
        )
        .collect())
}

/// Shows hidden content again. Content hidden from the reviewer, or hidden
/// by a report they take part in, is to them content that is not hidden.
pub async fn restore(db: &PgPool, staff: Uuid, body: RestoreContent) -> Result<(), ApiError> {
    let note = checked_note(Some(body.note), true)?;
    let mut tx = db.begin().await?;
    within_limit(&mut tx, staff, false, ACTIONS_PER_HOUR).await?;
    let report: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "DELETE FROM hidden_content h
         USING report r
         WHERE h.exchange_id = $1 AND h.account_id = $2 AND r.id = h.report_id
           AND h.account_id <> $3 AND NOT {}
         RETURNING h.report_id",
        involves("$3")
    )))
    .bind(body.exchange_id)
    .bind(body.account_id)
    .bind(staff)
    .fetch_optional(&mut *tx)
    .await?;
    let report = report.ok_or(ErrorCode::NotFound)?;
    record_event(
        &mut tx,
        Some(staff),
        ReviewAction::ContentRestored,
        Some(report),
        Some(body.exchange_id),
        Some(body.account_id),
        note.as_deref(),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- Hidden content as the parties see it -----------------------------------

/// Whether what the parties wrote in an exchange is hidden from an account.
pub async fn is_hidden_from(
    conn: &mut PgConnection,
    exchange: Uuid,
    account: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM hidden_content WHERE exchange_id = $1 AND account_id = $2)",
    )
    .bind(exchange)
    .bind(account)
    .fetch_one(conn)
    .await
}

/// The placeholder to show an account in place of what the parties wrote
/// in an exchange, in its language, when a reviewer has hidden it from them;
/// nothing otherwise.
pub async fn hidden_text(
    conn: &mut PgConnection,
    exchange: Uuid,
    account: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let language: Option<String> = sqlx::query_scalar(
        "SELECT a.language FROM hidden_content h JOIN account a ON a.id = h.account_id
         WHERE h.exchange_id = $1 AND h.account_id = $2",
    )
    .bind(exchange)
    .bind(account)
    .fetch_optional(conn)
    .await?;
    Ok(language.map(|language| notices::wording(&language).1.hidden().to_owned()))
}

/// Every piece of free text in a revision's terms, in place of the
/// placeholder: the terms, and each contribution's description, completion
/// criteria and unit of quantity. Names stay, by design; amounts, quantities
/// as numbers, dates and kinds are not free text.
fn hide_in_terms(terms: &mut RevisionTerms, placeholder: &str) {
    terms.terms = placeholder.to_owned();
    for contribution in &mut terms.contributions {
        contribution.description = placeholder.to_owned();
        if contribution.completion_criteria.is_some() {
            contribution.completion_criteria = Some(placeholder.to_owned());
        }
        if let Some(quantity) = &mut contribution.quantity
            && quantity.unit.is_some()
        {
            quantity.unit = Some(placeholder.to_owned());
        }
    }
}

/// An exchange's view with what the parties wrote in place of the
/// placeholder, and no working copy.
pub fn hide_in_view(view: &mut ExchangeView, placeholder: &str) {
    for revision in [&mut view.open_revision, &mut view.in_force_revision]
        .into_iter()
        .flatten()
    {
        hide_in_terms(&mut revision.terms, placeholder);
        if revision.note.is_some() {
            revision.note = Some(placeholder.to_owned());
        }
    }
    view.draft = None;
    view.content_hidden = true;
    // Nor are the other party's payment options shown beside it.
    view.payment_options.theirs = None;
    view.payment_options.theirs_changed = Default::default();
}

/// Events with every note and description in place of the placeholder.
pub fn hide_in_events(events: &mut [RecordEvent], placeholder: &str) {
    for event in events {
        if event.note.is_some() {
            event.note = Some(placeholder.to_owned());
        }
        if let Some(contribution) = &mut event.contribution {
            contribution.description = placeholder.to_owned();
        }
    }
}

/// A record's revisions with what was written in them in place of the
/// placeholder.
///
/// The signed document itself is left out (`signed`): with text replaced it
/// would no longer be what was signed, nor hash to `content_hash`, and a
/// copy of the record must not present it as if it were. In its place,
/// `redacted` holds the same document with every piece of free text (the
/// terms, and each contribution's description, completion criteria and unit
/// of quantity) replaced, and says so by its name; the record as a whole
/// says `content_hidden`. Names and amounts stay.
pub fn hide_in_revisions(revisions: &mut [RecordRevision], placeholder: &str) {
    use serde_json::Value;
    for revision in revisions {
        if revision.note.is_some() {
            revision.note = Some(placeholder.to_owned());
        }
        let Some(mut redacted) = revision.signed.take() else {
            continue;
        };
        if let Some(document) = redacted.as_object_mut() {
            document.insert("terms".to_owned(), Value::from(placeholder));
            if let Some(Value::Array(contributions)) = document.get_mut("contributions") {
                for contribution in contributions.iter_mut().filter_map(Value::as_object_mut) {
                    contribution.insert("description".to_owned(), Value::from(placeholder));
                    if contribution
                        .get("completion_criteria")
                        .is_some_and(|criteria| !criteria.is_null())
                    {
                        contribution
                            .insert("completion_criteria".to_owned(), Value::from(placeholder));
                    }
                    if let Some(Value::Object(quantity)) = contribution.get_mut("quantity")
                        && quantity.get("unit").is_some_and(|unit| !unit.is_null())
                    {
                        quantity.insert("unit".to_owned(), Value::from(placeholder));
                    }
                }
            }
        }
        revision.redacted = Some(redacted);
    }
}

// ---- The alert --------------------------------------------------------------

/// The outbox payload of the email that tells a reviewer a report is
/// waiting. It holds nothing about the report.
pub const ALERT_PAYLOAD: &str = "REPORT_RECEIVED";

/// Queues the alert for every reviewer who can be emailed and is not already
/// waiting for one. Runs in the transaction that stores the report.
pub async fn alert_staff(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    // One report at a time, so that two filed at once cannot both find no
    // alert waiting and queue one each. Held until the report is stored.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('staff alert', 0))")
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO outbox (kind, recipient_account_id, payload)
         SELECT 'EMAIL', a.id, jsonb_build_object('staff', $1::text)
         FROM staff_member s
         JOIN account a ON a.id = s.account_id
         WHERE a.status = 'ACTIVE' AND a.email_index IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM outbox o
               WHERE o.recipient_account_id = a.id AND o.kind = 'EMAIL'
                 AND o.completed_at IS NULL AND o.attempts = 0
                 AND o.payload ->> 'staff' = $1)",
    )
    .bind(ALERT_PAYLOAD)
    .execute(conn)
    .await?;
    Ok(())
}

// ---- The owner's command line -----------------------------------------------

/// Why the `staff` command could not do what it was asked.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StaffCommandError {
    #[error("no account has that email address, phone number or ID; sign in once first")]
    NoSuchAccount,
    #[error("that account is suspended or deleted")]
    NotActive,
    #[error("not an email address, phone number or account ID")]
    Unreadable,
}

/// Who a reviewer is, as `staff list` prints them.
#[derive(Debug)]
pub struct Reviewer {
    pub account_id: Uuid,
    /// The account's email address, masked.
    pub email: Option<String>,
    /// The account's phone number, masked.
    pub phone: Option<String>,
    pub name: String,
    pub status: String,
    pub granted_at: OffsetDateTime,
}

/// The account an argument names: an account ID, an email address or a
/// phone number. Locked until the transaction ends.
async fn named_account(
    conn: &mut PgConnection,
    who: &str,
) -> Result<Result<(Uuid, String), StaffCommandError>, sqlx::Error> {
    let found: Option<(Uuid, String)> = if let Ok(id) = who.trim().parse::<Uuid>() {
        sqlx::query_as("SELECT id, status FROM account WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
    } else {
        let Ok(identifier) = Identifier::parse(who) else {
            return Ok(Err(StaffCommandError::Unreadable));
        };
        // Found by its blind index (`crate::contact`).
        let (_, index) = Kind::of(&identifier).account_columns();
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id, status FROM account WHERE {index} = $1 FOR UPDATE"
        )))
        .bind(contact::keys().index_of(&identifier).as_slice())
        .fetch_optional(&mut *conn)
        .await?
    };
    Ok(found.ok_or(StaffCommandError::NoSuchAccount))
}

/// Makes an account a reviewer. Returns whether it was not one already.
/// Needs the schema owner's connection: the service's role cannot.
pub async fn grant(db: &PgPool, who: &str) -> anyhow::Result<bool> {
    let mut tx = db.begin().await?;
    let (account, status) = named_account(&mut tx, who).await??;
    if status != "ACTIVE" {
        return Err(StaffCommandError::NotActive.into());
    }
    let added =
        sqlx::query("INSERT INTO staff_member (account_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(account)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
    if added {
        record_event(
            &mut tx,
            None,
            ReviewAction::StaffGranted,
            None,
            None,
            Some(account),
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(added)
}

/// Stops an account being a reviewer. Returns whether it was one.
pub async fn revoke(db: &PgPool, who: &str) -> anyhow::Result<bool> {
    let mut tx = db.begin().await?;
    let (account, _) = named_account(&mut tx, who).await??;
    let removed = sqlx::query("DELETE FROM staff_member WHERE account_id = $1")
        .bind(account)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    if removed {
        record_event(
            &mut tx,
            None,
            ReviewAction::StaffRevoked,
            None,
            None,
            Some(account),
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(removed)
}

/// A reviewer's account: ID, email and phone (encrypted), name, status,
/// and since when.
type ReviewerRow = (
    Uuid,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    String,
    String,
    OffsetDateTime,
);

/// The reviewers, longest-serving first, for the owner's `staff list`. Each
/// address and number is decrypted to be shown masked (`a•••@example.com`),
/// as the initiator of an exchange is shown a claimant's: enough to tell
/// who a reviewer is, never the address itself (`crate::contact`).
pub async fn reviewers(db: &PgPool) -> Result<Vec<Reviewer>, sqlx::Error> {
    let rows: Vec<ReviewerRow> = sqlx::query_as(
        "SELECT a.id, a.email_encrypted, a.phone_encrypted, a.display_name, a.status,
                s.granted_at
             FROM staff_member s JOIN account a ON a.id = s.account_id
             ORDER BY s.granted_at, a.id",
    )
    .fetch_all(db)
    .await?;
    let keys = contact::keys();
    let masked = |field: Field, sealed: Option<Vec<u8>>| {
        let value = keys.reveal(field, sealed.as_deref())?;
        Ok::<_, contact::Unreadable>(
            value
                .and_then(|value| Identifier::parse(&value).ok())
                .map(|identifier| identifier.masked()),
        )
    };
    rows.into_iter()
        .map(|(account_id, email, phone, name, status, granted_at)| {
            Ok(Reviewer {
                account_id,
                email: masked(Field::ACCOUNT_EMAIL, email)?,
                phone: masked(Field::ACCOUNT_PHONE, phone)?,
                name,
                status,
                granted_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staff_sign_in_is_recent_for_twelve_hours() {
        let now = OffsetDateTime::now_utc();
        assert!(signed_in_recently(now, now));
        assert!(signed_in_recently(now - Duration::hours(12), now));
        assert!(!signed_in_recently(
            now - Duration::hours(12) - Duration::seconds(1),
            now
        ));
    }

    #[test]
    fn a_note_is_trimmed_bounded_and_sometimes_required() {
        assert_eq!(
            checked_note(Some("  spam  ".into()), true)
                .unwrap()
                .as_deref(),
            Some("spam")
        );
        assert_eq!(checked_note(Some("   ".into()), false).unwrap(), None);
        assert!(checked_note(None, true).is_err());
        assert!(checked_note(Some(" ".into()), true).is_err());
        let longest = "é".repeat(NOTE_MAX_CHARS);
        assert!(checked_note(Some(longest.clone()), true).is_ok());
        assert!(checked_note(Some(longest + "é"), true).is_err());
    }

    #[test]
    fn outcomes_and_actions_are_named_as_the_database_names_them() {
        for outcome in ReviewOutcome::ALL {
            assert_eq!(ReviewOutcome::parse(outcome.as_str()), Some(outcome));
            assert_eq!(
                serde_json::to_value(outcome).unwrap(),
                serde_json::Value::from(outcome.as_str())
            );
        }
        for action in ReviewAction::ALL {
            assert_eq!(ReviewAction::parse(action.as_str()), Some(action));
            assert_eq!(
                serde_json::to_value(action).unwrap(),
                serde_json::Value::from(action.as_str())
            );
        }
        for reason_value in REASONS {
            assert_eq!(reason(reason_value.as_str()), reason_value);
        }
    }

    const MARK: &str = "SECRET-1-Main-St";

    #[test]
    fn hiding_leaves_out_the_signed_terms_and_replaces_every_piece_of_free_text() {
        use crate::exchanges::record::dto::{RevisionStanding, RevisionStatus};
        let mut revisions = vec![RecordRevision {
            id: Uuid::nil(),
            sequence: 1,
            answers: None,
            author: Slot::A,
            sent_at: String::new(),
            expires_at: String::new(),
            note: Some(format!("my address is {MARK}")),
            standing: RevisionStanding {
                status: RevisionStatus::Open,
                since: String::new(),
                in_force_at: None,
                replaced_by: None,
            },
            content_hash: "abc".into(),
            signed: Some(serde_json::json!({
                "terms": format!("Meet at {MARK}"),
                "contributions": [
                    { "description": format!("Fix the fence at {MARK}"),
                      "completion_criteria": null, "quantity": null, "amount_minor": null },
                    { "description": "Pay", "completion_criteria": format!("Cash at {MARK}"),
                      "quantity": { "amount": "2", "unit": format!("bags from {MARK}") },
                      "amount_minor": 40000 },
                ],
                "parties": { "A": "Ana", "B": "Ben" },
            })),
            redacted: None,
            signatures: Vec::new(),
            void_signatures: Vec::new(),
        }];
        hide_in_revisions(&mut revisions, "Hidden");
        let text = serde_json::to_string(&revisions[0]).unwrap();
        assert!(!text.contains(MARK), "{text}");
        // The signed document is not shown altered as if it were signed.
        let shown = serde_json::to_value(&revisions[0]).unwrap();
        assert!(shown.get("signed").is_none(), "{shown}");
        assert_eq!(shown["content_hash"], "abc");
        let redacted = &shown["redacted"];
        assert_eq!(redacted["terms"], "Hidden");
        assert_eq!(redacted["contributions"][1]["quantity"]["unit"], "Hidden");
        assert_eq!(
            redacted["contributions"][1]["completion_criteria"],
            "Hidden"
        );
        assert_eq!(
            redacted["contributions"][0]["completion_criteria"],
            serde_json::Value::Null
        );
        // Names and amounts stay: the exchange has to stay recognisable.
        assert_eq!(redacted["parties"]["A"], "Ana");
        assert_eq!(redacted["contributions"][1]["amount_minor"], 40000);
        assert_eq!(redacted["contributions"][1]["quantity"]["amount"], "2");
    }

    #[test]
    fn hiding_replaces_the_unit_of_a_quantity_in_the_view_too() {
        use crate::exchanges::dto::{ContributionDto, ContributionType, DueDto, QuantityDto};
        let mut terms = RevisionTerms {
            party_a_name: "Ana".into(),
            party_b_name: "Ben".into(),
            terms: MARK.into(),
            contributions: vec![ContributionDto {
                id: Uuid::nil(),
                from: Slot::A,
                r#type: ContributionType::Item,
                description: MARK.into(),
                quantity: Some(QuantityDto {
                    amount: "3".into(),
                    unit: Some(MARK.into()),
                }),
                due: DueDto::OnAgreement,
                completion_criteria: Some(MARK.into()),
                required: true,
                amount_minor: None,
            }],
        };
        hide_in_terms(&mut terms, "Hidden");
        let text = serde_json::to_string(&terms).unwrap();
        assert!(!text.contains(MARK), "{text}");
        assert!(text.contains("Ana") && text.contains("\"3\""), "{text}");
    }
}
