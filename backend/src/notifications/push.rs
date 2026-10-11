//! Push notifications to the app (DESIGN.md §12, §13.1).
//!
//! **Which channel.** Email goes to everyone with an address, as before.
//! Push goes, as well, to a person who has the app on a device and has
//! turned notifications on: one PUSH row in the outbox beside the EMAIL row,
//! under the outbox's key of one message per event, person and channel. A
//! person with two devices gets the notice on both, which is still one
//! notice per channel. Neither channel waits for the other or depends on it:
//! the push says only that a yup has an update, and the email, which says
//! what kind of update and links to the record, stays the full account of
//! it (§12: push for those who allowed it, email for everyone). A
//! phone-only account has no email, so push is how it hears anything.
//!
//! **What it says.** The same generic text for every notice
//! (`Wording::push`), in the recipient's language: no exchange code, no
//! name, no terms and no amount, because a lock screen is read by whoever
//! holds the phone. The payload carries the exchange's path, which the app
//! opens after signing in, and nothing else.
//!
//! **Delivery.** The worker takes PUSH rows in batches, sends them in as
//! few requests as the push service allows, and records each row as the
//! email outbox does: sent, retried with the same back-off, given up on, or
//! closed unsent. A device the service says is no longer registered is
//! removed at once; the service's later receipts ([`check_receipts`]) can
//! say the same, and remove it then.
//!
//! What this file never does: write a token in full, or anything about the
//! exchange beyond its ID, to a log.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Connection, PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::outbox::{Delivered, DeliveryRules};
use super::wording::Wording;
use crate::domain::notification::Notice;
use crate::domain::reminder;
use crate::error::Redacted;
use crate::exchanges::reminders;

/// One notification for one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushMessage {
    /// The device's push token.
    pub to: String,
    /// The text, generic by design ([`Wording::push`]).
    pub body: String,
    /// Who it is with and what step, for someone who chose to have detail
    /// ([`Wording::push_title`]). Absent otherwise.
    pub title: Option<String>,
    /// What the app does when it is tapped.
    pub data: PushData,
}

/// What a notification carries for the app: the screen to open. The app
/// checks it is an exchange's own address before it goes there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PushData {
    /// `/exchanges/{id}`.
    pub url: String,
}

/// What the push service said about one message when it was handed over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ticket {
    /// Taken. Its receipt, which says whether it reached the device, can be
    /// asked for later under this ID, if there is one.
    Accepted { receipt: Option<String> },
    /// Not taken.
    Refused(Refusal),
}

/// Why the push service would not take a message, or could not deliver it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The token belongs to no installed app any more: the app was removed,
    /// or the person turned notifications off. The device is removed.
    DeviceNotRegistered,
    /// Anything else, by the service's own name for it, such as
    /// `MessageRateExceeded`. Never the service's message, which can quote
    /// the token.
    Other(String),
}

/// What became of a message once the push service tried to deliver it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receipt {
    Delivered,
    Refused(Refusal),
}

pub type PushFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// A push service.
pub trait PushSender: Send + Sync {
    /// The most messages one [`send`](Self::send) takes.
    fn batch_limit(&self) -> usize {
        100
    }

    /// The most IDs one [`receipts`](Self::receipts) takes.
    fn receipt_limit(&self) -> usize {
        1000
    }

    /// Hands over messages, and answers one ticket per message, in order.
    /// An error means none of them can be counted as taken.
    fn send<'a>(&'a self, messages: &'a [PushMessage]) -> PushFuture<'a, Vec<Ticket>>;

    /// The receipts for earlier tickets, by ticket ID. One not yet ready, or
    /// no longer kept, is absent.
    fn receipts<'a>(&'a self, ids: &'a [String]) -> PushFuture<'a, HashMap<String, Receipt>>;
}

/// A push token as a log may show it: what kind of token, and its first
/// characters, enough to tell two apart while looking at a log and too few
/// to send anything to.
pub fn masked_token(token: &str) -> String {
    let shown: String = match token.split_once('[') {
        Some((kind, rest)) => format!("{kind}[{}", rest.chars().take(4).collect::<String>()),
        None => token.chars().take(4).collect(),
    };
    format!("{shown}…")
}

/// Development delivery: writes each notification to the worker's log, with
/// the token masked, instead of sending it. Every message is taken, and
/// none has a receipt to ask for.
pub struct LogPushSender;

impl PushSender for LogPushSender {
    fn send<'a>(&'a self, messages: &'a [PushMessage]) -> PushFuture<'a, Vec<Ticket>> {
        Box::pin(async move {
            for message in messages {
                tracing::info!(
                    to = masked_token(&message.to),
                    body = message.body,
                    url = message.data.url,
                    "push notification (development delivery)"
                );
            }
            Ok(messages
                .iter()
                .map(|_| Ticket::Accepted { receipt: None })
                .collect())
        })
    }

    fn receipts<'a>(&'a self, _ids: &'a [String]) -> PushFuture<'a, HashMap<String, Receipt>> {
        Box::pin(async { Ok(HashMap::new()) })
    }
}

/// What the worker needs to deliver push notifications.
pub struct PushDelivery {
    /// The push service, or none when push is off (`PUSH_DELIVERY=off`):
    /// then whatever is queued for push is closed unsent.
    pub sender: Option<Arc<dyn PushSender>>,
    pub wording: Wording,
    /// The same numbers as email: tries, back-off, batch and time budget.
    pub rules: DeliveryRules,
}

/// Why a queued notification was closed when push is off.
pub const PUSH_OFF: &str = "not sent: push delivery is off";

/// A row taken for sending: id, recipient, exchange, payload, and the tries
/// made before this one.
type Claimed = (i64, Option<Uuid>, Option<Uuid>, Value, i32);

/// What became of one row.
enum Outcome {
    Sent,
    Failed(String),
    Dropped(&'static str),
}

/// What one pass over the queued push notifications did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushDelivered {
    /// The rows, counted as the email outbox counts its own.
    pub rows: Delivered,
    /// Devices removed because the push service said their token is no
    /// longer registered.
    pub devices_removed: usize,
}

/// Sends every push notification due at `at`, up to the batch size and
/// within the batch's time budget, stopping between two requests once
/// `stopping` says so. Safe to run from several workers at once: each row
/// is locked while it is sent, and another worker skips it.
pub async fn deliver_push_due_until(
    db: &PgPool,
    delivery: &PushDelivery,
    at: OffsetDateTime,
    stopping: impl Fn() -> bool,
) -> Result<PushDelivered, sqlx::Error> {
    let started = std::time::Instant::now();
    let now = || at + started.elapsed();
    let rules = &delivery.rules;
    let mut pass = PushDelivered::default();
    let delivered = &mut pass.rows;

    let Some(sender) = &delivery.sender else {
        // Off. Devices may still be registered, by an app that was told push
        // is on, or before it was turned off; what was queued for them is
        // closed rather than left to look like a backlog.
        let closed = sqlx::query(
            "UPDATE outbox SET completed_at = $1, last_error = $2
             WHERE id IN (SELECT id FROM outbox
                          WHERE kind = 'PUSH' AND completed_at IS NULL
                          LIMIT $3
                          FOR UPDATE SKIP LOCKED)",
        )
        .bind(now())
        .bind(PUSH_OFF)
        .bind(rules.batch as i64)
        .execute(db)
        .await?
        .rows_affected();
        delivered.dropped = closed as usize;
        return Ok(pass);
    };

    let mut left = rules.batch;
    let mut removed = 0;
    while left > 0 {
        if started.elapsed() >= rules.batch_budget || stopping() {
            delivered.cut_short = true;
            break;
        }
        let take = left.min(sender.batch_limit().max(1));
        let batch = Batch {
            sender: sender.as_ref(),
            at: now(),
            take,
        };
        let taken = deliver_batch(db, delivery, batch, delivered, &mut removed).await?;
        left -= taken;
        if taken < take {
            break;
        }
    }
    pass.devices_removed = removed;
    Ok(pass)
}

/// One batch: who sends it, as of when, and how many rows to take at most.
struct Batch<'a> {
    sender: &'a dyn PushSender,
    at: OffsetDateTime,
    take: usize,
}

/// Takes up to `take` due rows, sends what they call for, and records how
/// it went. Returns how many rows it took.
async fn deliver_batch(
    db: &PgPool,
    delivery: &PushDelivery,
    Batch { sender, at, take }: Batch<'_>,
    delivered: &mut Delivered,
    removed: &mut usize,
) -> Result<usize, sqlx::Error> {
    let rules = &delivery.rules;
    let mut tx = db.begin().await?;
    let claimed: Vec<Claimed> = sqlx::query_as(
        "SELECT id, recipient_account_id, exchange_id, payload, attempts FROM outbox
         WHERE kind = 'PUSH' AND completed_at IS NULL AND available_at <= $1 AND attempts < $2
         ORDER BY available_at, id
         LIMIT $3
         FOR UPDATE SKIP LOCKED",
    )
    .bind(at)
    .bind(rules.max_attempts)
    .bind(take as i64)
    .fetch_all(&mut *tx)
    .await?;
    if claimed.is_empty() {
        return Ok(0);
    }

    // Every message of the batch, each with the row and device it is for.
    let mut messages: Vec<PushMessage> = Vec::new();
    let mut owners: Vec<(usize, Uuid)> = Vec::new();
    let mut outcomes: Vec<Option<Outcome>> = Vec::with_capacity(claimed.len());
    for (index, (_, recipient, exchange, payload, _)) in claimed.iter().enumerate() {
        match prepare(&mut tx, delivery, *recipient, *exchange, payload, at).await? {
            Ok(for_devices) => {
                for (device, message) in for_devices {
                    owners.push((index, device));
                    messages.push(message);
                }
                outcomes.push(None);
            }
            Err(outcome) => outcomes.push(Some(outcome)),
        }
    }

    // What the service said, per row.
    #[derive(Default)]
    struct Heard {
        accepted: bool,
        unreachable: Option<String>,
        refusals: Vec<String>,
    }
    let mut heard: Vec<Heard> = claimed.iter().map(|_| Heard::default()).collect();
    let limit = sender.batch_limit().max(1);
    for (chunk, chunk_owners) in messages.chunks(limit).zip(owners.chunks(limit)) {
        let answer = tokio::time::timeout(rules.send_timeout, sender.send(chunk)).await;
        let tickets = match answer {
            Ok(Ok(tickets)) if tickets.len() == chunk.len() => tickets,
            Ok(Ok(tickets)) => {
                let error = format!(
                    "the push service answered {} tickets for {} messages",
                    tickets.len(),
                    chunk.len()
                );
                for (row, _) in chunk_owners {
                    heard[*row].unreachable = Some(error.clone());
                }
                continue;
            }
            Ok(Err(error)) => {
                let error = format!("{error:#}");
                for (row, _) in chunk_owners {
                    heard[*row].unreachable = Some(error.clone());
                }
                continue;
            }
            Err(_) => {
                let error = format!("no answer within {:?}", rules.send_timeout);
                for (row, _) in chunk_owners {
                    heard[*row].unreachable = Some(error.clone());
                }
                continue;
            }
        };
        for (ticket, (row, device)) in tickets.into_iter().zip(chunk_owners) {
            match ticket {
                Ticket::Accepted { receipt } => {
                    heard[*row].accepted = true;
                    if let Some(id) = receipt {
                        sqlx::query(
                            "INSERT INTO push_ticket (id, device_id) VALUES ($1, $2)
                             ON CONFLICT (id) DO NOTHING",
                        )
                        .bind(id.chars().take(128).collect::<String>())
                        .bind(device)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
                Ticket::Refused(Refusal::DeviceNotRegistered) => {
                    forget_device(&mut tx, *device).await?;
                    *removed += 1;
                }
                Ticket::Refused(Refusal::Other(name)) => heard[*row].refusals.push(name),
            }
        }
    }

    for (index, (id, _, _, _, attempts)) in claimed.iter().enumerate() {
        let outcome = outcomes[index].take().unwrap_or_else(|| {
            let heard = &heard[index];
            // Taken for one device is sent: trying again would repeat it on
            // that device, and the notice is one per person, not per device.
            if heard.accepted {
                Outcome::Sent
            } else if let Some(error) = &heard.unreachable {
                Outcome::Failed(error.clone())
            } else if !heard.refusals.is_empty() {
                Outcome::Failed(format!(
                    "the push service refused it: {}",
                    heard.refusals.join(", ")
                ))
            } else {
                Outcome::Dropped("not sent: no device the recipient had is registered any more")
            }
        });
        record(&mut tx, rules, *id, *attempts, outcome, at, delivered).await?;
    }
    tx.commit().await?;
    Ok(claimed.len())
}

/// Removes a device the push service no longer knows, with any tickets it
/// was waiting on.
async fn forget_device(conn: &mut PgConnection, device: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM device WHERE id = $1")
        .bind(device)
        .execute(conn)
        .await?;
    Ok(())
}

/// Writes what became of a row, as the email outbox does.
async fn record(
    conn: &mut PgConnection,
    rules: &DeliveryRules,
    id: i64,
    attempts: i32,
    outcome: Outcome,
    at: OffsetDateTime,
    delivered: &mut Delivered,
) -> Result<(), sqlx::Error> {
    match outcome {
        Outcome::Sent => {
            sqlx::query(
                "UPDATE outbox SET completed_at = $2, attempts = attempts + 1, last_error = NULL
                 WHERE id = $1",
            )
            .bind(id)
            .bind(at)
            .execute(conn)
            .await?;
            delivered.sent += 1;
        }
        Outcome::Failed(error) => {
            let error: String = error.chars().take(500).collect();
            sqlx::query(
                "UPDATE outbox SET attempts = attempts + 1, last_error = $2, available_at = $3
                 WHERE id = $1",
            )
            .bind(id)
            .bind(&error)
            .bind(at + rules.backoff(attempts))
            .execute(conn)
            .await?;
            delivered.failed += 1;
            if attempts + 1 >= rules.max_attempts {
                delivered.given_up += 1;
                tracing::error!(outbox = id, error, "push notification given up on");
            } else {
                tracing::warn!(outbox = id, error, "push notification not sent; will retry");
            }
        }
        Outcome::Dropped(reason) => {
            sqlx::query("UPDATE outbox SET completed_at = $2, last_error = $3 WHERE id = $1")
                .bind(id)
                .bind(at)
                .bind(reason)
                .execute(conn)
                .await?;
            delivered.dropped += 1;
        }
    }
    Ok(())
}

/// The messages a claimed row calls for, one per device, or what became of
/// it instead.
async fn prepare(
    conn: &mut PgConnection,
    delivery: &PushDelivery,
    recipient: Option<Uuid>,
    exchange: Option<Uuid>,
    payload: &Value,
    at: OffsetDateTime,
) -> Result<Result<Vec<(Uuid, PushMessage)>, Outcome>, sqlx::Error> {
    // A row this build cannot read may have been written by a newer one, so
    // it is retried like any failure and, at worst, left for inspection.
    let unreadable = || Outcome::Failed(format!("unreadable payload: {payload}"));
    let Some(notice) = payload["notice"].as_str().and_then(Notice::parse) else {
        return Ok(Err(unreadable()));
    };
    let known: Option<Uuid> = sqlx::query_scalar("SELECT id FROM exchange WHERE id = $1")
        .bind(exchange)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(exchange) = known else {
        return Ok(Err(Outcome::Failed("no such exchange".to_owned())));
    };

    // Read now, not when it was queued: the person may have changed their
    // language since, signed out of a device, or left.
    let account: Option<(String, bool)> = sqlx::query_as(
        "SELECT language, notification_detail FROM account WHERE id = $1 AND status = 'ACTIVE'",
    )
    .bind(recipient)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((language, detailed)) = account else {
        return Ok(Err(Outcome::Dropped(
            "not sent: the recipient's account is closed",
        )));
    };

    // A reminder that has stopped being true is not sent, as for email.
    if let Some(kind) = reminder::Kind::of(notice) {
        let Ok(contributions) = Vec::<Uuid>::deserialize(&payload["contributions"]) else {
            return Ok(Err(unreadable()));
        };
        let mut check = conn.begin().await?;
        match reminders::still_true(&mut check, exchange, kind, &contributions, at).await {
            Ok(true) => check.commit().await?,
            Ok(false) => {
                check.commit().await?;
                return Ok(Err(Outcome::Dropped(
                    "not sent: what the reminder said is no longer true",
                )));
            }
            Err(error) => {
                check.rollback().await?;
                return Ok(Err(Outcome::Failed(format!(
                    "the reminder could not be checked: {}",
                    Redacted(&error)
                ))));
            }
        }
    }

    let devices: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT d.id, d.token FROM device d
         JOIN account_session s ON s.id = d.session_id
         WHERE d.account_id = $1 AND s.revoked_at IS NULL AND s.expires_at > now()
         ORDER BY d.created_at, d.id",
    )
    .bind(recipient)
    .fetch_all(&mut *conn)
    .await?;
    if devices.is_empty() {
        return Ok(Err(Outcome::Dropped(
            "not sent: the recipient has no device to send to",
        )));
    }

    let body = delivery.wording.push(&language);
    // Only for someone who chose it, and only who, what step and which code.
    let title = if detailed {
        match super::yup_of(conn, exchange, recipient).await? {
            Some((code, other_party)) => {
                delivery
                    .wording
                    .push_title(&language, notice, &other_party, &code)
            }
            None => None,
        }
    } else {
        None
    };
    let data = PushData {
        url: format!("/exchanges/{exchange}"),
    };
    Ok(Ok(devices
        .into_iter()
        .map(|(device, token)| {
            let message = PushMessage {
                to: token,
                body: body.clone(),
                title: title.clone(),
                data: data.clone(),
            };
            (device, message)
        })
        .collect()))
}

// ---- Receipts and clean-up --------------------------------------------------

/// The numbers behind reading receipts.
#[derive(Clone, Debug)]
pub struct ReceiptRules {
    /// How long after sending a receipt is asked for. Expo's guidance is to
    /// wait about fifteen minutes.
    pub wait: Duration,
    /// How long a ticket is kept waiting for its receipt. Expo keeps
    /// receipts for a day.
    pub keep: Duration,
    /// How long one request for receipts may take.
    pub timeout: std::time::Duration,
    /// How often the worker asks, at most. A receipt not ready yet is asked
    /// for again, and every pass would be every five seconds.
    pub every: std::time::Duration,
}

impl Default for ReceiptRules {
    fn default() -> Self {
        Self {
            wait: Duration::minutes(15),
            keep: Duration::hours(24),
            timeout: std::time::Duration::from_secs(30),
            every: std::time::Duration::from_secs(60),
        }
    }
}

/// What one look at receipts did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiptsRead {
    /// Whether the push service was asked: there were tickets old enough.
    pub asked: bool,
    /// Tickets whose receipt was read, or that were too old to wait for.
    pub settled: usize,
    /// Devices removed because a receipt said their token is not valid.
    pub devices_removed: usize,
}

/// Why receipts could not be read. Either way the tickets stay, to be asked
/// about again on the next pass.
#[derive(Debug, thiserror::Error)]
pub enum ReceiptsError {
    #[error("{}", Redacted(.0))]
    Database(#[from] sqlx::Error),
    /// The push service did not answer, or not usably. Never its own words.
    #[error("{0:#}")]
    Service(anyhow::Error),
}

/// Asks for the receipts of tickets old enough to have one, removes the
/// devices they say are no longer registered, and forgets the tickets it has
/// an answer for or can no longer get one for. One request per call.
pub async fn check_receipts(
    db: &PgPool,
    sender: &dyn PushSender,
    rules: &ReceiptRules,
    at: OffsetDateTime,
) -> Result<ReceiptsRead, ReceiptsError> {
    let due: Vec<(String, Uuid, OffsetDateTime)> = sqlx::query_as(
        "SELECT id, device_id, created_at FROM push_ticket
         WHERE created_at <= $1
         ORDER BY created_at, id
         LIMIT $2",
    )
    .bind(at - rules.wait)
    .bind(sender.receipt_limit().max(1) as i64)
    .fetch_all(db)
    .await?;
    if due.is_empty() {
        return Ok(ReceiptsRead::default());
    }
    let ids: Vec<String> = due.iter().map(|(id, _, _)| id.clone()).collect();
    let receipts = tokio::time::timeout(rules.timeout, sender.receipts(&ids))
        .await
        .map_err(|_| {
            ReceiptsError::Service(anyhow::anyhow!("no answer within {:?}", rules.timeout))
        })?
        .map_err(ReceiptsError::Service)?;

    let mut read = ReceiptsRead {
        asked: true,
        ..ReceiptsRead::default()
    };
    let mut removed = std::collections::HashSet::new();
    let mut tx = db.begin().await?;
    for (id, device, created) in &due {
        match receipts.get(id) {
            Some(Receipt::Refused(Refusal::DeviceNotRegistered)) => {
                // Its tickets go with it.
                if removed.insert(*device) {
                    forget_device(&mut tx, *device).await?;
                    read.devices_removed += 1;
                }
                read.settled += 1;
            }
            Some(receipt) => {
                if let Receipt::Refused(Refusal::Other(name)) = receipt {
                    tracing::warn!(
                        error = name,
                        "the push service could not deliver a notification"
                    );
                }
                forget_ticket(&mut tx, id).await?;
                read.settled += 1;
            }
            None if *created <= at - rules.keep => {
                forget_ticket(&mut tx, id).await?;
                read.settled += 1;
            }
            None => {}
        }
    }
    tx.commit().await?;
    Ok(read)
}

async fn forget_ticket(conn: &mut PgConnection, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM push_ticket WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Forgets tickets older than [`ReceiptRules::keep`], whose receipts the
/// push service no longer has. [`check_receipts`] forgets them too, but only
/// on a pass where the service answered and only the oldest batch; this runs
/// on its own, so the table stays bounded while receipts cannot be read or
/// push has been turned off. Returns how many were removed. Called by the
/// worker.
pub async fn purge_tickets(
    db: &PgPool,
    rules: &ReceiptRules,
    at: OffsetDateTime,
) -> Result<u64, sqlx::Error> {
    let removed = sqlx::query("DELETE FROM push_ticket WHERE created_at <= $1")
        .bind(at - rules.keep)
        .execute(db)
        .await?
        .rows_affected();
    Ok(removed)
}

/// Removes devices whose session has ended: signed out somewhere the app
/// could not say so, revoked, or expired. Nothing is sent to them anyway.
/// Returns how many were removed. Called by the worker.
pub async fn purge_devices(db: &PgPool) -> Result<u64, sqlx::Error> {
    let removed = sqlx::query(
        "DELETE FROM device d USING account_session s
         WHERE s.id = d.session_id AND (s.revoked_at IS NOT NULL OR s.expires_at <= now())",
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_logged_by_its_kind_and_first_characters_only() {
        assert_eq!(
            masked_token("ExponentPushToken[xxxxxxxxxxxxxxxxxxxxxx]"),
            "ExponentPushToken[xxxx…"
        );
        assert_eq!(masked_token("abcdefgh"), "abcd…");
        assert_eq!(masked_token(""), "…");
    }
}
