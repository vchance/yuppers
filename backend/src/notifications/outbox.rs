//! The transactional outbox for notifications (DESIGN.md §13, §13.2).
//!
//! [`enqueue`] runs inside the transaction that records an event, and
//! [`enqueue_reminder`] inside the one that records a reminder.
//! [`deliver_due`] is the worker's side: claim a message, send it, record how
//! that went.
//!
//! Each notice is queued once per channel (DESIGN.md §12, no duplicates): an
//! `EMAIL` row for a recipient with an email address, and a `PUSH` row for
//! one with a device registered under a live session (`super::push` says why
//! both, and delivers the second). The outbox's key, one row per event,
//! person and channel, holds that even if an event is processed twice.
//!
//! What the columns say about a message:
//!
//! * `completed_at` empty and `attempts` below the limit: waiting, not before
//!   `available_at`;
//! * `completed_at` empty and `attempts` at the limit: given up on, and left
//!   as it is for someone to look at, with the last failure in `last_error`.
//!   A refusal that trying again cannot change ([`super::Undeliverable`],
//!   such as an address the provider will not take, or
//!   [`super::KeyConflict`]) is given up on at once: `attempts` goes
//!   straight to the limit. A refusal of everything this service sends
//!   ([`super::Outage`]: credentials, an unverified sender, a spent quota)
//!   does not count as a try; the message waits longer each time, up to the
//!   hourly ceiling, and is given up on, `attempts` to the limit, once it is
//!   older than [`DeliveryRules::outage_max_age`];
//! * `completed_at` set and `last_error` empty: sent;
//! * `completed_at` set and `last_error` set: closed without sending, because
//!   there was no longer anyone to send it to or, for a reminder, because
//!   what it said had stopped being true.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::wording::{Links, Wording};
use super::{Email, EmailSender, KeyConflict, Outage, Undeliverable};
use crate::contact::{self, Field};
use crate::domain::notification::Notice;
use crate::domain::reminder;
use crate::domain::revision::ContributionId;
use crate::error::Redacted;
use crate::exchanges::reminders;
use crate::review;

/// The numbers behind delivery. Placeholders: none of these is a recorded
/// design decision yet.
#[derive(Clone, Debug)]
pub struct DeliveryRules {
    /// Sends tried before a message is given up on.
    pub max_attempts: i32,
    /// The wait after a first failed send. Each further failure doubles it.
    pub retry_after: Duration,
    /// The longest wait between two tries.
    pub retry_ceiling: Duration,
    /// How old a message may grow while the provider refuses everything
    /// ([`Outage`]) before it is given up on. Such tries are not counted
    /// against [`DeliveryRules::max_attempts`], which an outage of an hour
    /// would otherwise spend.
    pub outage_max_age: Duration,
    /// How long one send may take before it counts as failed.
    pub send_timeout: std::time::Duration,
    /// Messages one pass delivers at most, so a backlog cannot keep the
    /// worker from its other jobs.
    pub batch: usize,
    /// How long one pass may go on taking new messages. A slow or silent
    /// server can make each send last up to `send_timeout`, and a whole
    /// batch of those would hold up the timers and a request to stop for
    /// most of an hour; once this is spent the pass takes no more and the
    /// worker goes round again.
    pub batch_budget: std::time::Duration,
}

impl Default for DeliveryRules {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            retry_after: Duration::minutes(1),
            retry_ceiling: Duration::hours(1),
            outage_max_age: Duration::hours(24),
            send_timeout: std::time::Duration::from_secs(30),
            batch: 100,
            batch_budget: std::time::Duration::from_secs(20),
        }
    }
}

impl DeliveryRules {
    /// How long to wait after a failure, given the failures before it.
    pub(crate) fn backoff(&self, earlier_failures: i32) -> Duration {
        // Past the ceiling long before the shift could overflow.
        let doublings = earlier_failures.clamp(0, 20);
        let wait: Duration = self.retry_after * (1_i32 << doublings);
        wait.min(self.retry_ceiling)
    }

    /// How long to wait after an [`Outage`], for a message queued `age`
    /// ago: as long again as it has waited so far, so the waits double
    /// without a count of tries, between [`DeliveryRules::retry_after`] and
    /// [`DeliveryRules::retry_ceiling`].
    pub(crate) fn outage_backoff(&self, age: Duration) -> Duration {
        age.clamp(self.retry_after, self.retry_ceiling.max(self.retry_after))
    }
}

/// What the worker needs to deliver notifications.
pub struct Delivery {
    pub sender: Arc<dyn EmailSender>,
    pub wording: Wording,
    /// Where the web app is served from; messages link into it.
    pub web_origin: String,
    pub rules: DeliveryRules,
}

/// Why a message was closed unsent when its recipient was removed from the
/// exchange it is about, or left it.
pub const NO_LONGER_A_PARTY: &str = "not sent: the recipient is no longer a party to the exchange";

// ---- Writing ----------------------------------------------------------------

/// Queues an email telling `recipient` about an event. Call it in the
/// transaction that records the event, so the two are stored or lost together.
///
/// Only the kind of message is stored. The address, the language and the
/// text are read when it is sent, so a queued message holds nothing personal
/// and nothing from the agreement.
///
/// One row per channel the recipient can be reached on (DESIGN.md §12): an
/// email for an account with an email address, a push notification for one
/// with a device registered under a live session, and a text for one who
/// turned on text updates for this agreement, if the notice is a status
/// change those are sent for (`super::sms_updates`). One that is suspended
/// or deleted gets nothing.
pub async fn enqueue(
    conn: &mut PgConnection,
    exchange: Uuid,
    event_sequence: i64,
    recipient: Uuid,
    notice: Notice,
) -> Result<(), sqlx::Error> {
    let payload = json!({ "notice": notice.as_str() });
    insert(conn, exchange, Some(event_sequence), recipient, payload).await?;
    super::sms_updates::enqueue_update(conn, exchange, event_sequence, recipient, notice).await
}

/// Queues an email reminding `recipient` of contributions that are due soon
/// or overdue. Call it in the transaction that records the reminder, which
/// is what keeps it from being queued twice: no event caused it, so the
/// outbox's own key (one message per event per person) does not apply.
///
/// The contributions are stored by ID, with nothing they say. They are there
/// so that the message can be checked again when it is sent, which may be
/// after one of them was delivered.
///
/// As with [`enqueue`], one row per channel the recipient can be reached on.
pub async fn enqueue_reminder(
    conn: &mut PgConnection,
    exchange: Uuid,
    recipient: Uuid,
    notice: Notice,
    contributions: &[ContributionId],
) -> Result<(), sqlx::Error> {
    let contributions: Vec<Uuid> = contributions.iter().map(|id| id.0).collect();
    let payload = json!({ "notice": notice.as_str(), "contributions": contributions });
    insert(conn, exchange, None, recipient, payload).await
}

async fn insert(
    conn: &mut PgConnection,
    exchange: Uuid,
    event_sequence: Option<i64>,
    recipient: Uuid,
    payload: Value,
) -> Result<(), sqlx::Error> {
    // Whether there is a device is decided now and again when it is sent:
    // one registered later does not get what was queued before it, and one
    // signed out of meanwhile is not sent to.
    sqlx::query(
        "INSERT INTO outbox (kind, recipient_account_id, exchange_id, event_sequence, payload)
         SELECT 'EMAIL', id, $2, $3, $4 FROM account
         WHERE id = $1 AND status = 'ACTIVE' AND email_index IS NOT NULL
         UNION ALL
         SELECT 'PUSH', a.id, $2, $3, $4 FROM account a
         WHERE a.id = $1 AND a.status = 'ACTIVE'
           AND EXISTS (SELECT 1 FROM device d
                       JOIN account_session s ON s.id = d.session_id
                       WHERE d.account_id = a.id
                         AND s.revoked_at IS NULL AND s.expires_at > now())",
    )
    .bind(recipient)
    .bind(exchange)
    .bind(event_sequence)
    .bind(payload)
    .execute(conn)
    .await?;
    Ok(())
}

// ---- Delivering -------------------------------------------------------------

/// What one pass over the outbox did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delivered {
    pub sent: usize,
    /// Sends that failed, including those counted in `given_up`.
    pub failed: usize,
    /// Failures that were the last try.
    pub given_up: usize,
    /// Closed unsent: the recipient could no longer be emailed, or a
    /// reminder was no longer true.
    pub dropped: usize,
    /// The pass stopped before its batch was done, because its time budget
    /// was spent or the worker is stopping. More may be due.
    pub cut_short: bool,
}

impl Delivered {
    /// Nothing was taken.
    pub fn is_empty(&self) -> bool {
        self.handled() == 0
    }

    /// Messages the pass took: sent, failed or closed unsent.
    pub fn handled(&self) -> usize {
        self.sent + self.failed + self.dropped
    }
}

enum Attempt {
    Sent,
    Failed(String),
    /// Refused for good: not tried again.
    Undeliverable(String),
    /// The provider holds another message under this one's key.
    Conflict(String),
    /// The provider refuses everything for now: not counted as a try.
    Outage(String),
    Dropped(&'static str),
}

/// A row taken for sending: id, recipient, exchange, payload, the tries
/// made before this one, its own key, and when it was queued.
type Claimed = (
    i64,
    Option<Uuid>,
    Option<Uuid>,
    Value,
    i32,
    Uuid,
    OffsetDateTime,
);

/// Sends every email that is due at `at`, up to the batch size and within
/// the batch's time budget. Safe to run from several workers at once.
pub async fn deliver_due(
    db: &PgPool,
    delivery: &Delivery,
    at: OffsetDateTime,
) -> Result<Delivered, sqlx::Error> {
    deliver_due_until(db, delivery, at, || false).await
}

/// [`deliver_due`], stopping between two messages once `stopping` says so:
/// the worker's way of heeding a request to stop without waiting for the
/// rest of a batch.
///
/// The clock runs on from `at` while the pass does: each message is taken,
/// and a failed one put off, as of `at` plus the time the pass has taken so
/// far, so a long pass does not shorten the wait before a retry.
pub async fn deliver_due_until(
    db: &PgPool,
    delivery: &Delivery,
    at: OffsetDateTime,
    stopping: impl Fn() -> bool,
) -> Result<Delivered, sqlx::Error> {
    let started = std::time::Instant::now();
    let clock = Clock { at, started };
    let mut delivered = Delivered::default();
    for _ in 0..delivery.rules.batch {
        if started.elapsed() >= delivery.rules.batch_budget || stopping() {
            delivered.cut_short = true;
            break;
        }
        if !deliver_next(db, delivery, &clock, &mut delivered).await? {
            break;
        }
    }
    Ok(delivered)
}

/// The time as a pass sees it: where it started, plus how long it has run.
struct Clock {
    at: OffsetDateTime,
    started: std::time::Instant,
}

impl Clock {
    fn now(&self) -> OffsetDateTime {
        self.at + self.started.elapsed()
    }
}

/// `text` with every occurrence of `address` replaced, ignoring case. An
/// error from a sender may quote the recipient's address, and neither the
/// log nor `last_error` may hold it; the outbox ID says who it was for.
fn without_address(text: &str, address: &str) -> String {
    if address.is_empty() {
        return text.to_owned();
    }
    // ASCII lowercasing keeps every byte where it was, so the positions
    // found in the lowered copy hold in the original.
    let (lowered, needle) = (text.to_ascii_lowercase(), address.to_ascii_lowercase());
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in lowered.match_indices(&needle) {
        out.push_str(&text[last..at]);
        out.push_str("[recipient]");
        last = at + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Delivers one message, if any is due. Returns whether there was one.
async fn deliver_next(
    db: &PgPool,
    delivery: &Delivery,
    clock: &Clock,
    delivered: &mut Delivered,
) -> Result<bool, sqlx::Error> {
    let rules = &delivery.rules;
    // One message per transaction, and the row stays locked while it is
    // sent. Another worker skips a locked row, so no two send the same one,
    // and a worker that dies mid-send lets go of it for the next to pick up.
    // The cost is one connection held for the length of a send, which the
    // timeout bounds.
    let mut tx = db.begin().await?;
    let claimed: Option<Claimed> = sqlx::query_as(
        "SELECT id, recipient_account_id, exchange_id, payload, attempts, delivery_key, created_at
         FROM outbox
         WHERE kind = 'EMAIL' AND completed_at IS NULL AND available_at <= $1 AND attempts < $2
         ORDER BY available_at, id
         LIMIT 1
         FOR UPDATE SKIP LOCKED",
    )
    .bind(clock.now())
    .bind(rules.max_attempts)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, recipient, exchange, payload, attempts, key, created_at)) = claimed else {
        return Ok(false);
    };

    let row = Row {
        id,
        key,
        recipient,
        exchange,
        payload: &payload,
    };
    let attempt = match prepare(&mut tx, delivery, row, clock.now()).await? {
        Ok(email) => {
            match tokio::time::timeout(rules.send_timeout, delivery.sender.send(&email)).await {
                Ok(Ok(())) => Attempt::Sent,
                Ok(Err(error)) => {
                    let text = without_address(&format!("{error:#}"), &email.to);
                    if error.is::<Undeliverable>() {
                        Attempt::Undeliverable(text)
                    } else if error.is::<KeyConflict>() {
                        Attempt::Conflict(text)
                    } else if error.is::<Outage>() {
                        Attempt::Outage(text)
                    } else {
                        Attempt::Failed(text)
                    }
                }
                Err(_) => Attempt::Failed(format!("no answer within {:?}", rules.send_timeout)),
            }
        }
        Err(attempt) => attempt,
    };

    // After the send, which may have taken many seconds.
    let at = clock.now();
    match attempt {
        Attempt::Sent => {
            sqlx::query(
                "UPDATE outbox SET completed_at = $2, attempts = attempts + 1, last_error = NULL
                 WHERE id = $1",
            )
            .bind(id)
            .bind(at)
            .execute(&mut *tx)
            .await?;
            delivered.sent += 1;
        }
        Attempt::Failed(error) => {
            // Kept short: a provider's error can be a whole page.
            let error: String = error.chars().take(500).collect();
            sqlx::query(
                "UPDATE outbox SET attempts = attempts + 1, last_error = $2, available_at = $3
                 WHERE id = $1",
            )
            .bind(id)
            .bind(&error)
            .bind(at + rules.backoff(attempts))
            .execute(&mut *tx)
            .await?;
            delivered.failed += 1;
            if attempts + 1 >= rules.max_attempts {
                delivered.given_up += 1;
                tracing::error!(
                    outbox = id,
                    error,
                    reason = "out of tries",
                    "notification given up on"
                );
            } else {
                tracing::warn!(outbox = id, error, "notification not sent; will retry");
            }
        }
        Attempt::Outage(error) => {
            let error: String = error.chars().take(500).collect();
            let age = at - created_at;
            if age >= rules.outage_max_age {
                give_up(&mut tx, id, &error, rules.max_attempts).await?;
                delivered.failed += 1;
                delivered.given_up += 1;
                tracing::error!(
                    outbox = id,
                    error,
                    reason = "the provider refused everything for longer than a message waits",
                    max_age_hours = rules.outage_max_age.whole_hours(),
                    "notification given up on"
                );
            } else {
                // The try is not counted: nothing about this message failed.
                sqlx::query("UPDATE outbox SET last_error = $2, available_at = $3 WHERE id = $1")
                    .bind(id)
                    .bind(&error)
                    .bind(at + rules.outage_backoff(age))
                    .execute(&mut *tx)
                    .await?;
                delivered.failed += 1;
                tracing::warn!(
                    outbox = id,
                    error,
                    "notification not sent: the provider refuses everything for now; will retry, \
                     not counting the try"
                );
            }
        }
        Attempt::Conflict(error) => {
            let error: String = error.chars().take(500).collect();
            give_up(&mut tx, id, &error, rules.max_attempts).await?;
            delivered.failed += 1;
            delivered.given_up += 1;
            tracing::error!(
                outbox = id,
                error,
                idempotency_conflict = true,
                reason = "the provider already holds another message under this one's key; one \
                          copy went out, so it is not sent again",
                "notification given up on"
            );
        }
        Attempt::Undeliverable(error) => {
            let error: String = error.chars().take(500).collect();
            give_up(&mut tx, id, &error, rules.max_attempts).await?;
            delivered.failed += 1;
            delivered.given_up += 1;
            tracing::error!(
                outbox = id,
                error,
                refused_for_good = true,
                reason = "the provider refused this message for good",
                "notification given up on"
            );
        }
        Attempt::Dropped(reason) => {
            sqlx::query("UPDATE outbox SET completed_at = $2, last_error = $3 WHERE id = $1")
                .bind(id)
                .bind(at)
                .bind(reason)
                .execute(&mut *tx)
                .await?;
            delivered.dropped += 1;
        }
    }
    tx.commit().await?;
    Ok(true)
}

/// Gives a message up: its tries go to the limit, with why.
async fn give_up(
    conn: &mut PgConnection,
    id: i64,
    error: &str,
    max_attempts: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE outbox SET attempts = GREATEST(attempts + 1, $3), last_error = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .bind(max_attempts)
    .execute(conn)
    .await?;
    Ok(())
}

/// What a claimed row holds.
struct Row<'a> {
    id: i64,
    key: Uuid,
    recipient: Option<Uuid>,
    exchange: Option<Uuid>,
    payload: &'a Value,
}

/// Builds the email for a claimed row, or says why there is none to send.
async fn prepare(
    conn: &mut PgConnection,
    delivery: &Delivery,
    row: Row<'_>,
    at: OffsetDateTime,
) -> Result<Result<Email, Attempt>, sqlx::Error> {
    let payload = row.payload;
    // A row this build cannot read may have been written by a newer one, so
    // it is retried like any failure and, at worst, left for inspection.
    let unreadable = || Attempt::Failed(format!("unreadable payload: {payload}"));
    if payload["staff"].as_str() == Some(review::ALERT_PAYLOAD) {
        return staff_alert(conn, delivery, row).await;
    }
    let Some(notice) = payload["notice"].as_str().and_then(Notice::parse) else {
        return Ok(Err(unreadable()));
    };
    let code: Option<String> =
        sqlx::query_scalar("SELECT display_code FROM exchange WHERE id = $1")
            .bind(row.exchange)
            .fetch_optional(&mut *conn)
            .await?;
    let (Some(exchange), Some(code)) = (row.exchange, code) else {
        return Ok(Err(Attempt::Failed("no such exchange".to_owned())));
    };

    // Read now, not when the message was queued: the person may have changed
    // their address or language since, or left.
    let account: Option<(Option<Vec<u8>>, String)> = sqlx::query_as(
        "SELECT email_encrypted, language FROM account WHERE id = $1 AND status = 'ACTIVE'",
    )
    .bind(row.recipient)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((encrypted, language)) = account else {
        return Ok(Err(Attempt::Dropped(
            "not sent: the recipient can no longer be emailed",
        )));
    };
    let to = match recipient(encrypted) {
        Ok(Some(to)) => to,
        Ok(None) => {
            return Ok(Err(Attempt::Dropped(
                "not sent: the recipient can no longer be emailed",
            )));
        }
        Err(attempt) => return Ok(Err(attempt)),
    };

    // A message about an event stays true: the event happened. A reminder
    // says how things stand, and they may have changed while it waited, most
    // of all when earlier tries failed. One that is no longer true is not
    // sent.
    if let Some(kind) = reminder::Kind::of(notice) {
        let Ok(contributions) = Vec::<Uuid>::deserialize(&payload["contributions"]) else {
            return Ok(Err(unreadable()));
        };
        // In a savepoint, so that a failure here (an exchange whose timezone
        // the database cannot read, say) is this one message's failure, to be
        // retried and in the end given up on, and not the end of the pass.
        let mut check = conn.begin().await?;
        let stands = reminders::still_true(&mut check, exchange, kind, &contributions, at).await;
        match stands {
            Ok(stands) => {
                check.commit().await?;
                if !stands {
                    return Ok(Err(Attempt::Dropped(
                        "not sent: what the reminder said is no longer true",
                    )));
                }
            }
            Err(error) => {
                check.rollback().await?;
                return Ok(Err(Attempt::Failed(format!(
                    "the reminder could not be checked: {}",
                    Redacted(&error)
                ))));
            }
        }
    }

    let link = format!("{}/exchanges/{exchange}", delivery.web_origin);
    let links = Links {
        exchange: &link,
        record: &format!("{link}/record"),
    };
    let rendered = delivery.wording.email(&language, notice, &code, links);
    Ok(Ok(Email {
        to,
        subject: rendered.subject,
        body: rendered.body,
        html: Some(rendered.html),
        reference: row.id,
        key: row.key,
    }))
}

/// The address to send to, decrypted here and nowhere else on the way
/// (`crate::contact`). A value that does not decrypt is a failure, tried
/// again like any other, and its error names no address.
fn recipient(encrypted: Option<Vec<u8>>) -> Result<Option<String>, Attempt> {
    contact::keys()
        .reveal(Field::ACCOUNT_EMAIL, encrypted.as_deref())
        .map_err(|unreadable| Attempt::Failed(unreadable.to_string()))
}

/// The email telling a reviewer that a report is waiting (`crate::review`).
/// Sent only to someone who is still a reviewer and can still be emailed.
async fn staff_alert(
    conn: &mut PgConnection,
    delivery: &Delivery,
    row: Row<'_>,
) -> Result<Result<Email, Attempt>, sqlx::Error> {
    let account: Option<(Option<Vec<u8>>, String)> = sqlx::query_as(
        "SELECT a.email_encrypted, a.language FROM account a
         JOIN staff_member s ON s.account_id = a.id
         WHERE a.id = $1 AND a.status = 'ACTIVE'",
    )
    .bind(row.recipient)
    .fetch_optional(&mut *conn)
    .await?;
    let gone = "not sent: the recipient is no longer a reviewer who can be emailed";
    let Some((encrypted, language)) = account else {
        return Ok(Err(Attempt::Dropped(gone)));
    };
    let to = match recipient(encrypted) {
        Ok(Some(to)) => to,
        Ok(None) => return Ok(Err(Attempt::Dropped(gone))),
        Err(attempt) => return Ok(Err(attempt)),
    };
    let link = format!("{}/staff", delivery.web_origin);
    let rendered = delivery.wording.staff_alert(&language, &link);
    Ok(Ok(Email {
        to,
        subject: rendered.subject,
        body: rendered.body,
        html: Some(rendered.html),
        reference: row.id,
        key: row.key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipient_address_is_taken_out_of_an_error_whatever_its_case() {
        assert_eq!(
            without_address(
                "550 5.1.1 <Ana.Lopez@Example.test> rejected: ana.lopez@example.test unknown",
                "ana.lopez@example.test"
            ),
            "550 5.1.1 <[recipient]> rejected: [recipient] unknown"
        );
        assert_eq!(
            without_address("no answer", "ana@example.test"),
            "no answer"
        );
        assert_eq!(without_address("anything", ""), "anything");
    }

    #[test]
    fn the_wait_doubles_with_each_failure_up_to_the_ceiling() {
        let rules = DeliveryRules {
            retry_after: Duration::minutes(1),
            retry_ceiling: Duration::minutes(10),
            ..DeliveryRules::default()
        };
        let waits: Vec<i64> = (0..6)
            .map(|failures| rules.backoff(failures).whole_minutes())
            .collect();
        assert_eq!(waits, [1, 2, 4, 8, 10, 10]);
        assert_eq!(rules.backoff(i32::MAX), Duration::minutes(10));
    }

    #[test]
    fn an_outage_waits_as_long_again_as_the_message_has_up_to_the_ceiling() {
        let rules = DeliveryRules::default();
        let waits: Vec<i64> = [0, 1, 3, 20, 59, 61, 600, 1440]
            .into_iter()
            .map(|age| rules.outage_backoff(Duration::minutes(age)).whole_minutes())
            .collect();
        assert_eq!(waits, [1, 1, 3, 20, 59, 60, 60, 60]);
        // Over a day, about thirty tries, not eight within the first hour.
        let (mut age, mut tries) = (Duration::ZERO, 0);
        while age < rules.outage_max_age {
            age += rules.outage_backoff(age);
            tries += 1;
        }
        assert!((25..=35).contains(&tries), "{tries}");
    }
}
