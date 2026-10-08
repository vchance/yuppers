//! Working rows that are no use once they are old: ended sign-ins, the
//! idempotency keys of old requests, and finished notifications. The worker
//! removes them about once an hour ([`SWEEP_EVERY`]); the privacy policy
//! states each period (`packages/shared/src/legal-text.ts`, "How long we
//! keep it").
//!
//! Nothing else reads these rows once they are this old:
//!
//! * **Sessions** (`account_session`): a request is matched only to a live
//!   one (`http::extract`), and push devices only under a live one; the
//!   devices of an ended one are removed with it already
//!   (`notifications::push::purge_devices`), and any left go with it by
//!   cascade. A signature records how its signer signed in, in the
//!   acceptance itself, not by pointing at the session. Sign-in limits are
//!   counted in `sign_in_limit` and `one_time_code`, not here.
//! * **Idempotency keys** (`idempotency_key`): read only when the same
//!   account sends the same key again, which a client does only to retry a
//!   request that went unanswered, while it is still open
//!   (`packages/shared/src/idempotency.ts`); a month is far beyond that.
//! * **Notifications** (`outbox`): delivery reads only rows not yet
//!   finished; the daily cap on update texts counts today's; a staff alert
//!   looks for one still waiting; the metrics count the unfinished. A row
//!   finished, sent, closed unsent or given up on, three months ago is read
//!   by none of them. Nothing in the record of an agreement points at one.
//!
//! None of it is in the deletion log a restore replays (`crate::deletion_log`),
//! which names accounts, not rows.

use std::time::Duration as StdDuration;

use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

/// How often the worker sweeps. The periods are days and months, so once an
/// hour is plenty, and keeps the scans off every tick.
pub const SWEEP_EVERY: StdDuration = StdDuration::from_secs(60 * 60);

/// How long a session is kept once it ended, by signing out, by being
/// revoked, or by expiring.
pub const ENDED_SESSION_RETENTION: Duration = Duration::days(30);

/// How long an idempotency key is kept after the request that brought it.
pub const IDEMPOTENCY_KEY_RETENTION: Duration = Duration::days(30);

/// How long a notification is kept after it was queued, once it is
/// finished: sent, closed unsent, or given up on. One still waiting is
/// never removed here.
pub const FINISHED_NOTIFICATION_RETENTION: Duration = Duration::days(90);

/// What one sweep removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub sessions: u64,
    pub idempotency_keys: u64,
    pub notifications: u64,
}

impl Swept {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Removes every row past its period, as of `at`. `max_attempts` is the
/// outbox's limit, at which a message is given up on.
pub async fn sweep(
    db: &PgPool,
    at: OffsetDateTime,
    max_attempts: i32,
) -> Result<Swept, sqlx::Error> {
    let sessions = sqlx::query(
        "DELETE FROM account_session
         WHERE LEAST(expires_at, COALESCE(revoked_at, expires_at)) < $1",
    )
    .bind(at - ENDED_SESSION_RETENTION)
    .execute(db)
    .await?
    .rows_affected();
    let idempotency_keys = sqlx::query("DELETE FROM idempotency_key WHERE created_at < $1")
        .bind(at - IDEMPOTENCY_KEY_RETENTION)
        .execute(db)
        .await?
        .rows_affected();
    let notifications = sqlx::query(
        "DELETE FROM outbox
         WHERE created_at < $1 AND (completed_at IS NOT NULL OR attempts >= $2)",
    )
    .bind(at - FINISHED_NOTIFICATION_RETENTION)
    .bind(max_attempts)
    .execute(db)
    .await?
    .rows_affected();
    Ok(Swept {
        sessions,
        idempotency_keys,
        notifications,
    })
}
