//! "Yuppers.app agreement updates": the second SMS program (DESIGN.md §12).
//!
//! **Opting in.** A party to an agreement may turn on text updates for it,
//! by ticking a box that shows the consent wording the terms quote word for
//! word (`smsUpdates.consent` in the wording files). The account must have
//! a verified phone number of a country the service texts, which is added
//! the usual way (`POST /v1/auth/codes`, then `POST /v1/me/identifiers`).
//! Ticking it stores the subscription (`sms_update`) and a record of the
//! consent (`sms_consent`: the account, the agreement, the number, the time,
//! the version and language of the wording shown, and the client), with the
//! request's address and user agent kept apart for as long as a signature's
//! (`sms_consent_network`). A confirmation text is queued at once. Unticking
//! removes the subscription and records that too.
//!
//! **Sending.** Each status change the terms name
//! ([`Notice::texted_as_update`]) queues one `SMS` row in the outbox per
//! party told about it who has updates on for that agreement, in the same
//! transaction as the event, as email and push are queued. Only the kind of
//! text is stored: no number, no wording. The worker writes the text when it
//! sends it ([`deliver_sms_due_until`]), and sends it only if the account
//! still has the number the updates were turned on with, the number has not
//! replied STOP, and its country is served. Update texts share the hourly
//! caps of one-time codes (`crate::auth::SmsPlace`), and each person is
//! queued at most [`texts_per_person_per_day`] a day.
//!
//! **STOP and START.** Twilio posts every text a number sends us to
//! `POST /v1/sms/inbound` (`crate::http::sms`). A stop keyword puts the
//! number on `sms_opt_out`, turns off every agreement's updates for it and
//! records both; nothing more is texted to it, codes included, until it
//! sends a start keyword, which takes it off the list but turns no
//! agreement's updates back on. HELP is answered by Twilio's Advanced
//! Opt-Out, so the service does not reply to anything.
//!
//! **Off.** With `SMS_DELIVERY=off` no update text is queued, the API
//! refuses to turn updates on and says so in `GET /v1/meta`
//! (`sms_updates`), and the worker closes anything still queued unsent.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::outbox::{Delivered, DeliveryRules};
use super::sms::{Sms, SmsSender};
use super::wording::Wording;
use crate::auth::{AuthRules, SmsPlace};
use crate::domain::Rules;
use crate::domain::identity::Identifier;
use crate::domain::notification::Notice;

/// The version of the consent wording, `smsUpdates.consent`, that the
/// clients show beside the box. A client must name it when it turns
/// updates on, and it is stored with the consent; a new wording gets a new
/// version here and in `packages/shared/src/sms-updates.ts`.
pub const CONSENT_VERSION: &str = "2026-10-05";

/// Update texts one person may be queued per UTC day, confirmations
/// included, unless the deployment says otherwise
/// (`SMS_UPDATES_PER_PERSON_PER_DAY`). A placeholder that bounds the cost of
/// an agreement changed over and over; past it, that day's further changes
/// are not texted (email and push still go).
pub const DEFAULT_TEXTS_PER_PERSON_PER_DAY: i64 = 20;

/// The payload of a queued update text, `{"sms": UPDATE, "notice": …}`.
pub const UPDATE: &str = "UPDATE";
/// The payload of a queued confirmation, `{"sms": OPT_IN_CONFIRMATION}`.
pub const OPT_IN_CONFIRMATION: &str = "OPT_IN_CONFIRMATION";

// Whether this process texts updates, and the daily cap. Set once at start
// by each binary that changes exchanges (`configure`), from `SMS_DELIVERY`.
// A process-wide setting rather than an argument, because an update is
// queued wherever an exchange changes (a request, a timer, a deletion, a
// review), and every one of those paths would otherwise have to carry it.
static TEXTING: AtomicBool = AtomicBool::new(false);
static PER_DAY: AtomicI64 = AtomicI64::new(DEFAULT_TEXTS_PER_PERSON_PER_DAY);

/// Says whether update texts are sent by this deployment (`SMS_DELIVERY` is
/// not `off`), and how many one person may be queued a day. Until it is
/// called, none are queued.
pub fn configure(texting: bool, per_person_per_day: i64) {
    TEXTING.store(texting, Ordering::Relaxed);
    PER_DAY.store(per_person_per_day.max(1), Ordering::Relaxed);
}

/// Whether update texts are queued at all.
pub fn texting() -> bool {
    TEXTING.load(Ordering::Relaxed)
}

/// Update texts one person may be queued per UTC day.
pub fn texts_per_person_per_day() -> i64 {
    PER_DAY.load(Ordering::Relaxed)
}

// ---- Queuing ----------------------------------------------------------------

/// Queues the update text about `notice` for `recipient`, if it is a status
/// change the terms name, updates are on for this agreement with the
/// number the account still has, that number has not replied STOP, and the
/// person is under the day's cap. Called from `outbox::enqueue`, in the
/// transaction that records the event.
pub(crate) async fn enqueue_update(
    conn: &mut PgConnection,
    exchange: Uuid,
    event_sequence: i64,
    recipient: Uuid,
    notice: Notice,
) -> Result<(), sqlx::Error> {
    if !texting() || !notice.texted_as_update() {
        return Ok(());
    }
    let payload = json!({ "sms": UPDATE, "notice": notice.as_str() });
    queue(conn, exchange, Some(event_sequence), recipient, payload).await
}

async fn queue(
    conn: &mut PgConnection,
    exchange: Uuid,
    event_sequence: Option<i64>,
    recipient: Uuid,
    payload: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO outbox (kind, recipient_account_id, exchange_id, event_sequence, payload)
         SELECT 'SMS', a.id, $2, $3, $4 FROM account a
         JOIN sms_update u ON u.account_id = a.id AND u.exchange_id = $2 AND u.phone = a.phone
         WHERE a.id = $1 AND a.status = 'ACTIVE'
           AND NOT EXISTS (SELECT 1 FROM sms_opt_out o WHERE o.phone = a.phone)
           AND (SELECT count(*) FROM outbox q
                WHERE q.kind = 'SMS' AND q.recipient_account_id = $1
                  AND q.created_at >= date_trunc('day', now(), 'UTC')) < $5",
    )
    .bind(recipient)
    .bind(exchange)
    .bind(event_sequence)
    .bind(payload)
    .bind(texts_per_person_per_day())
    .execute(conn)
    .await?;
    Ok(())
}

// ---- Turning updates on and off ---------------------------------------------

/// Where a change to someone's updates came from, as `sms_consent.source`
/// stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Web,
    Ios,
    Android,
    /// A client that did not say which it is.
    Unknown,
    /// A reply by text: STOP or START.
    SmsReply,
    AccountDeleted,
    PhoneChanged,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Web => "WEB",
            Source::Ios => "IOS",
            Source::Android => "ANDROID",
            Source::Unknown => "UNKNOWN",
            Source::SmsReply => "SMS_REPLY",
            Source::AccountDeleted => "ACCOUNT_DELETED",
            Source::PhoneChanged => "PHONE_CHANGED",
        }
    }

    /// The client named in an `X-Client-Version` header (`web/1.2.0`).
    pub fn of_client(header: Option<&str>) -> Self {
        match header.and_then(|value| value.split('/').next()) {
            Some("web") => Source::Web,
            Some("ios") => Source::Ios,
            Some("android") => Source::Android,
            _ => Source::Unknown,
        }
    }
}

/// What a person ticked: the version and language of the wording shown.
#[derive(Clone, Debug)]
pub struct Consent<'a> {
    pub version: &'a str,
    /// A supported language tag.
    pub language: &'a str,
    pub source: Source,
    pub address: Option<std::net::IpAddr>,
    pub user_agent: Option<&'a str>,
}

/// Where one account stands on text updates for one agreement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    /// Whether updates are on: there is a subscription, with the number the
    /// account has now.
    pub on: bool,
    /// The account's phone number, if it has one.
    pub phone: Option<String>,
    /// Whether that number replied STOP.
    pub opted_out: bool,
}

/// Where `account` stands on updates for `exchange`.
pub async fn standing(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Uuid,
) -> Result<Standing, sqlx::Error> {
    let (phone, on, opted_out): (Option<String>, bool, bool) = sqlx::query_as(
        "SELECT a.phone,
                EXISTS (SELECT 1 FROM sms_update u
                        WHERE u.account_id = a.id AND u.exchange_id = $2 AND u.phone = a.phone),
                EXISTS (SELECT 1 FROM sms_opt_out o WHERE o.phone = a.phone)
         FROM account a WHERE a.id = $1",
    )
    .bind(account)
    .bind(exchange)
    .fetch_one(conn)
    .await?;
    Ok(Standing {
        on,
        phone,
        opted_out,
    })
}

/// Turns updates on for `exchange` with `phone`, records the consent and
/// queues the confirmation. Returns `false`, changing nothing, if they were
/// on already with that number. The caller has checked that the account is
/// a party, that the agreement may have updates, and that the number may be
/// texted.
pub async fn turn_on(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Uuid,
    phone: &str,
    consent: &Consent<'_>,
) -> Result<bool, sqlx::Error> {
    let already: Option<String> = sqlx::query_scalar(
        "SELECT phone FROM sms_update WHERE account_id = $1 AND exchange_id = $2 FOR UPDATE",
    )
    .bind(account)
    .bind(exchange)
    .fetch_optional(&mut *conn)
    .await?;
    if already.as_deref() == Some(phone) {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO sms_update (account_id, exchange_id, phone) VALUES ($1, $2, $3)
         ON CONFLICT (account_id, exchange_id)
         DO UPDATE SET phone = EXCLUDED.phone, turned_on_at = now()",
    )
    .bind(account)
    .bind(exchange)
    .bind(phone)
    .execute(&mut *conn)
    .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sms_consent
             (action, account_id, exchange_id, phone, source, consent_version, consent_language)
         VALUES ('OPT_IN', $1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(account)
    .bind(exchange)
    .bind(phone)
    .bind(consent.source.as_str())
    .bind(consent.version)
    .bind(consent.language)
    .fetch_one(&mut *conn)
    .await?;
    let user_agent: Option<String> = consent
        .user_agent
        .map(|agent| agent.chars().take(512).collect());
    sqlx::query(
        "INSERT INTO sms_consent_network (consent_id, ip_address, user_agent)
         VALUES ($1, $2::inet, $3)",
    )
    .bind(id)
    .bind(consent.address.map(|address| address.to_string()))
    .bind(user_agent)
    .execute(&mut *conn)
    .await?;
    if texting() {
        queue(
            conn,
            exchange,
            None,
            account,
            json!({ "sms": OPT_IN_CONFIRMATION }),
        )
        .await?;
    }
    Ok(true)
}

/// Turns updates off for one agreement and records it. Returns `false` if
/// they were not on.
pub async fn turn_off(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Uuid,
    source: Source,
) -> Result<bool, sqlx::Error> {
    let removed = sqlx::query(
        "WITH gone AS (
             DELETE FROM sms_update WHERE account_id = $1 AND exchange_id = $2
             RETURNING account_id, exchange_id, phone)
         INSERT INTO sms_consent (action, account_id, exchange_id, phone, source)
         SELECT 'OPT_OUT', account_id, exchange_id, phone, $3 FROM gone",
    )
    .bind(account)
    .bind(exchange)
    .bind(source.as_str())
    .execute(conn)
    .await?
    .rows_affected();
    Ok(removed > 0)
}

/// Turns off every agreement's updates for an account whose number is no
/// longer `keep` (it was replaced, or the account is being deleted, with
/// `keep` empty), recording each. Returns how many were turned off.
pub async fn forget_numbers(
    conn: &mut PgConnection,
    account: Uuid,
    keep: Option<&str>,
    source: Source,
) -> Result<u64, sqlx::Error> {
    let removed = sqlx::query(
        "WITH gone AS (
             DELETE FROM sms_update
             WHERE account_id = $1 AND ($2::text IS NULL OR phone <> $2)
             RETURNING account_id, exchange_id, phone)
         INSERT INTO sms_consent (action, account_id, exchange_id, phone, source)
         SELECT 'OPT_OUT', account_id, exchange_id, phone, $3 FROM gone",
    )
    .bind(account)
    .bind(keep)
    .bind(source.as_str())
    .execute(conn)
    .await?
    .rows_affected();
    // Anything already queued for the old number is dropped when it comes
    // to be sent, since the subscription is gone.
    Ok(removed)
}

// ---- Replies: STOP and START ------------------------------------------------

/// What a text received from a number asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    Stop,
    Start,
    Help,
}

impl Keyword {
    /// The keyword a message is, if it is one: the whole message, ignoring
    /// case and the space around it, as Twilio matches them. Twilio's
    /// standard opt-out words, and OPTOUT and REVOKE, which the carriers
    /// also expect to work; its opt-in words; and its help words.
    pub fn of_message(body: &str) -> Option<Self> {
        let word = body.trim().to_ascii_uppercase();
        match word.as_str() {
            "STOP" | "STOPALL" | "UNSUBSCRIBE" | "CANCEL" | "END" | "QUIT" | "OPTOUT"
            | "REVOKE" => Some(Keyword::Stop),
            "START" | "UNSTOP" | "YES" => Some(Keyword::Start),
            "HELP" | "INFO" => Some(Keyword::Help),
            _ => None,
        }
    }

    /// Twilio's own reading, `OptOutType`, which it sends with Advanced
    /// Opt-Out on and which counts the words configured there.
    pub fn of_opt_out_type(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "STOP" => Some(Keyword::Stop),
            "START" => Some(Keyword::Start),
            "HELP" => Some(Keyword::Help),
            _ => None,
        }
    }
}

/// Records a stop keyword from `phone`: the number goes on the opt-out list,
/// every agreement's updates to it are turned off, and nothing still queued
/// for it is sent. Returns how many agreements' updates were turned off.
pub async fn stop(db: &PgPool, phone: &str, keyword: &str) -> Result<u64, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO sms_opt_out (phone) VALUES ($1)
         ON CONFLICT (phone) DO UPDATE SET opted_out_at = now()",
    )
    .bind(phone)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO sms_consent (action, phone, source, keyword)
         VALUES ('STOP', $1, 'SMS_REPLY', $2)",
    )
    .bind(phone)
    .bind(keyword_text(keyword))
    .execute(&mut *tx)
    .await?;
    let turned_off = sqlx::query(
        "WITH gone AS (
             DELETE FROM sms_update WHERE phone = $1
             RETURNING account_id, exchange_id, phone)
         INSERT INTO sms_consent (action, account_id, exchange_id, phone, source)
         SELECT 'OPT_OUT', account_id, exchange_id, phone, 'SMS_REPLY' FROM gone",
    )
    .bind(phone)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(turned_off)
}

/// Records a start keyword from `phone`: it is taken off the opt-out list.
/// No agreement's updates come back on; each is turned on again by ticking
/// its box.
pub async fn start(db: &PgPool, phone: &str, keyword: &str) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM sms_opt_out WHERE phone = $1")
        .bind(phone)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sms_consent (action, phone, source, keyword)
         VALUES ('START', $1, 'SMS_REPLY', $2)",
    )
    .bind(phone)
    .bind(keyword_text(keyword))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The word as it is recorded: trimmed, upper case, and short.
fn keyword_text(keyword: &str) -> String {
    let word: String = keyword
        .trim()
        .to_ascii_uppercase()
        .chars()
        .take(32)
        .collect();
    if word.is_empty() {
        "?".to_owned()
    } else {
        word
    }
}

// ---- Retention --------------------------------------------------------------

/// Removes consent records past their retention (`Rules::sms_consent_retention`):
/// those older than it, except an opt-in whose updates are still on and a
/// STOP whose number is still opted out, which are what the texting (or not
/// texting) now rests on. And the address and user agent of an opt-in after
/// as long as a signature's. Returns how many rows went. Called by the
/// worker.
pub async fn purge_consent(
    db: &PgPool,
    rules: &Rules,
    at: OffsetDateTime,
) -> Result<u64, sqlx::Error> {
    let network = sqlx::query("DELETE FROM sms_consent_network WHERE recorded_at < $1")
        .bind(at - rules.network_metadata_retention)
        .execute(db)
        .await?
        .rows_affected();
    let records = sqlx::query(
        "DELETE FROM sms_consent c
         WHERE c.created_at < $1
           AND NOT (c.action = 'OPT_IN' AND EXISTS (
                   SELECT 1 FROM sms_update u
                   WHERE u.account_id = c.account_id AND u.exchange_id = c.exchange_id
                     AND u.turned_on_at <= c.created_at))
           AND NOT (c.action = 'STOP' AND EXISTS (
                   SELECT 1 FROM sms_opt_out o
                   WHERE o.phone = c.phone AND o.opted_out_at <= c.created_at))",
    )
    .bind(at - rules.sms_consent_retention)
    .execute(db)
    .await?
    .rows_affected();
    Ok(network + records)
}

// ---- Delivering -------------------------------------------------------------

/// What the worker needs to send update texts.
pub struct SmsDelivery {
    /// The SMS provider, or none while `SMS_DELIVERY=off`: then whatever is
    /// queued is closed unsent.
    pub sender: Option<Arc<dyn SmsSender>>,
    pub wording: Wording,
    /// Where the web app is served from; an update links into it.
    pub web_origin: String,
    pub rules: DeliveryRules,
    /// The countries texted and the hourly caps, shared with codes.
    pub auth: AuthRules,
    /// `APP_SECRET`, which keys the counts of the hourly caps as the API
    /// keys them, so that both count against the same rows.
    pub secret: Vec<u8>,
}

/// Why a queued text was closed when texting is off.
pub const SMS_OFF: &str = "not sent: text messages are off";

/// How long a text held back by the hourly caps may wait before it is
/// dropped: by then the change it reports is old news.
const HELD_BACK_AT_MOST: Duration = Duration::days(1);

/// A row taken for sending: id, recipient, exchange, payload, the tries made
/// before this one, and when it was queued.
type Claimed = (i64, Option<Uuid>, Option<Uuid>, Value, i32, OffsetDateTime);

enum Attempt {
    Sent,
    Failed(String),
    /// Not sent and not a failure: tried again at the given time.
    Later(OffsetDateTime, &'static str),
    Dropped(&'static str),
}

/// Sends every update text due at `at`, one row per transaction, up to the
/// batch size and within its time budget, stopping between two once
/// `stopping` says so. Safe to run from several workers at once.
pub async fn deliver_sms_due_until(
    db: &PgPool,
    delivery: &SmsDelivery,
    at: OffsetDateTime,
    stopping: impl Fn() -> bool,
) -> Result<Delivered, sqlx::Error> {
    let started = std::time::Instant::now();
    let now = || at + started.elapsed();
    let rules = &delivery.rules;
    let mut delivered = Delivered::default();

    let Some(sender) = &delivery.sender else {
        let closed = sqlx::query(
            "UPDATE outbox SET completed_at = $1, last_error = $2
             WHERE id IN (SELECT id FROM outbox
                          WHERE kind = 'SMS' AND completed_at IS NULL
                          LIMIT $3
                          FOR UPDATE SKIP LOCKED)",
        )
        .bind(now())
        .bind(SMS_OFF)
        .bind(rules.batch as i64)
        .execute(db)
        .await?
        .rows_affected();
        delivered.dropped = closed as usize;
        return Ok(delivered);
    };

    for _ in 0..rules.batch {
        if started.elapsed() >= rules.batch_budget || stopping() {
            delivered.cut_short = true;
            break;
        }
        if !deliver_next(db, delivery, sender.as_ref(), now(), &mut delivered).await? {
            break;
        }
    }
    Ok(delivered)
}

async fn deliver_next(
    db: &PgPool,
    delivery: &SmsDelivery,
    sender: &dyn SmsSender,
    at: OffsetDateTime,
    delivered: &mut Delivered,
) -> Result<bool, sqlx::Error> {
    let rules = &delivery.rules;
    let mut tx = db.begin().await?;
    let claimed: Option<Claimed> = sqlx::query_as(
        "SELECT id, recipient_account_id, exchange_id, payload, attempts, created_at FROM outbox
         WHERE kind = 'SMS' AND completed_at IS NULL AND available_at <= $1 AND attempts < $2
         ORDER BY available_at, id
         LIMIT 1
         FOR UPDATE SKIP LOCKED",
    )
    .bind(at)
    .bind(rules.max_attempts)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, recipient, exchange, payload, attempts, queued)) = claimed else {
        return Ok(false);
    };

    let attempt = match prepare(&mut tx, delivery, recipient, exchange, &payload).await? {
        Err(attempt) => attempt,
        Ok((phone, text)) => send(db, delivery, sender, &phone, &text, at, queued).await?,
    };

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
                tracing::error!(outbox = id, error, "update text given up on");
            } else {
                tracing::warn!(outbox = id, error, "update text not sent; will retry");
            }
        }
        Attempt::Later(when, why) => {
            sqlx::query("UPDATE outbox SET last_error = $2, available_at = $3 WHERE id = $1")
                .bind(id)
                .bind(why)
                .bind(when)
                .execute(&mut *tx)
                .await?;
            tracing::info!(outbox = id, "update text held back by the hourly cap");
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

/// The number and the text for a claimed row, or why there is none to send.
async fn prepare(
    conn: &mut PgConnection,
    delivery: &SmsDelivery,
    recipient: Option<Uuid>,
    exchange: Option<Uuid>,
    payload: &Value,
) -> Result<Result<(String, String), Attempt>, sqlx::Error> {
    let kind = payload["sms"].as_str();
    let readable = match kind {
        Some(UPDATE) => payload["notice"]
            .as_str()
            .and_then(Notice::parse)
            .is_some_and(Notice::texted_as_update),
        Some(OPT_IN_CONFIRMATION) => true,
        _ => false,
    };
    let (Some(recipient), Some(exchange), true) = (recipient, exchange, readable) else {
        return Ok(Err(Attempt::Failed(format!(
            "unreadable payload: {payload}"
        ))));
    };

    // Read now, not when it was queued: the person may have turned updates
    // off, changed their number or replied STOP since.
    let found: Option<(String, String, bool)> = sqlx::query_as(
        "SELECT a.phone, a.language,
                EXISTS (SELECT 1 FROM sms_opt_out o WHERE o.phone = a.phone)
         FROM account a
         JOIN sms_update u ON u.account_id = a.id AND u.exchange_id = $2 AND u.phone = a.phone
         WHERE a.id = $1 AND a.status = 'ACTIVE'",
    )
    .bind(recipient)
    .bind(exchange)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((phone, language, opted_out)) = found else {
        return Ok(Err(Attempt::Dropped(
            "not sent: text updates are no longer on for this number",
        )));
    };
    if opted_out {
        return Ok(Err(Attempt::Dropped("not sent: the number replied STOP")));
    }
    let Ok(identifier) = Identifier::parse(&phone) else {
        return Ok(Err(Attempt::Dropped("not sent: not a phone number")));
    };
    if !delivery.auth.takes(&identifier) {
        return Ok(Err(Attempt::Dropped(
            "not sent: the number's country is not texted",
        )));
    }
    let text = match kind {
        Some(UPDATE) => {
            let link = format!("{}/exchanges/{exchange}", delivery.web_origin);
            delivery.wording.update_sms(&language, &link)
        }
        _ => delivery.wording.opt_in_sms(&language),
    };
    Ok(Ok((phone, text)))
}

/// Takes a place under the hourly caps, sends, and gives the place back if
/// the provider does not take the text. The places are taken and given back
/// in transactions of their own, so their rows are not held locked while
/// the provider answers.
async fn send(
    db: &PgPool,
    delivery: &SmsDelivery,
    sender: &dyn SmsSender,
    phone: &str,
    text: &str,
    at: OffsetDateTime,
    queued: OffsetDateTime,
) -> Result<Attempt, sqlx::Error> {
    let identifier = Identifier::Phone(phone.to_owned());
    let mut places = db.begin().await?;
    let place = SmsPlace::take(&mut places, &delivery.secret, &delivery.auth, &identifier).await?;
    places.commit().await?;
    let Some(place) = place else {
        if at - queued >= HELD_BACK_AT_MOST {
            return Ok(Attempt::Dropped(
                "not sent: held back by the hourly cap on text messages for a day",
            ));
        }
        // The next hour, when the caps start again.
        let next_hour = at.replace_minute(0).and_then(|t| t.replace_second(0));
        let next_hour = next_hour
            .and_then(|t| t.replace_nanosecond(0))
            .map_or(at + Duration::hours(1), |t| t + Duration::hours(1));
        return Ok(Attempt::Later(
            next_hour,
            "waiting: the hourly cap on text messages is reached",
        ));
    };
    let sent = tokio::time::timeout(
        delivery.rules.send_timeout,
        sender.send(Sms { to: phone, text }),
    )
    .await;
    let error = match sent {
        Ok(Ok(())) => return Ok(Attempt::Sent),
        // The senders name no number; this makes sure of it.
        Ok(Err(error)) => format!("{error:#}").replace(phone, "[recipient]"),
        Err(_) => format!("no answer within {:?}", delivery.rules.send_timeout),
    };
    let mut back = db.begin().await?;
    place.give_back(&mut back, &delivery.secret).await?;
    back.commit().await?;
    Ok(Attempt::Failed(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages;
    use crate::notifications::sms::{Encoding, encoding};

    #[test]
    fn keywords_are_twilios_and_the_whole_message() {
        for word in [
            "STOP",
            "stop",
            " Stop ",
            "STOPALL",
            "UNSUBSCRIBE",
            "CANCEL",
            "END",
            "QUIT",
            "OPTOUT",
            "REVOKE",
        ] {
            assert_eq!(Keyword::of_message(word), Some(Keyword::Stop), "{word}");
        }
        for word in ["START", "unstop", "Yes"] {
            assert_eq!(Keyword::of_message(word), Some(Keyword::Start), "{word}");
        }
        assert_eq!(Keyword::of_message("help"), Some(Keyword::Help));
        for other in ["please stop", "STOP!", "", "thanks"] {
            assert_eq!(Keyword::of_message(other), None, "{other}");
        }
        assert_eq!(Keyword::of_opt_out_type("STOP"), Some(Keyword::Stop));
        assert_eq!(Keyword::of_opt_out_type("START"), Some(Keyword::Start));
        assert_eq!(Keyword::of_opt_out_type("HELP"), Some(Keyword::Help));
        assert_eq!(Keyword::of_opt_out_type("other"), None);
    }

    #[test]
    fn the_client_is_read_from_its_version_header() {
        assert_eq!(Source::of_client(Some("web/1.0.0")), Source::Web);
        assert_eq!(Source::of_client(Some("ios/1.0.0")), Source::Ios);
        assert_eq!(Source::of_client(Some("android/2.1.0")), Source::Android);
        assert_eq!(Source::of_client(Some("curl/8")), Source::Unknown);
        assert_eq!(Source::of_client(None), Source::Unknown);
    }

    /// An exchange's link as the live service writes it: the longest an
    /// update's link is.
    const LINK: &str = "https://yuppers.app/exchanges/0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn the_texts_read_as_written_name_nothing_agreed_and_are_sent_as_expected() {
        let wording = Wording::embedded().unwrap();
        let cases = [
            (
                wording.update_sms("en", LINK),
                format!(
                    "Yuppers.app: an agreement you turned on updates for has changed. \
                     See it: {LINK}. Reply STOP to opt out."
                ),
                // Three characters over one segment with the live origin:
                // the wording the owner chose, kept as written (README).
                Encoding::Gsm7 { septets: 163 },
            ),
            (
                wording.update_sms("es", LINK),
                format!(
                    "Yuppers.app: hubo un cambio en un acuerdo que sigues. \
                     Velo: {LINK}. Responde STOP para cancelar."
                ),
                Encoding::Gsm7 { septets: 156 },
            ),
            (
                wording.opt_in_sms("en"),
                "Yuppers.app: You're signed up for text updates about this agreement, one text \
                 per status change. Msg frequency varies. Msg & data rates may apply. Reply HELP \
                 for help, STOP to opt out."
                    .to_owned(),
                Encoding::Gsm7 { septets: 184 },
            ),
            (
                wording.opt_in_sms("es"),
                "Yuppers.app: Te suscribiste a las actualizaciones por mensaje de este acuerdo, \
                 un mensaje por cada cambio de estado. Frecuencia variable. Pueden aplicarse \
                 tarifas por mensajes y datos. Responde HELP para ayuda o STOP para cancelar."
                    .to_owned(),
                Encoding::Gsm7 { septets: 231 },
            ),
        ];
        for (written, expected, sent_as) in cases {
            assert_eq!(written, expected);
            assert_eq!(encoding(&written), sent_as, "{written}");
        }
    }

    #[test]
    fn every_language_writes_both_texts_in_the_gsm_alphabet_with_stop() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            let update = wording.update_sms(language, LINK);
            let confirmation = wording.opt_in_sms(language);
            for text in [&update, &confirmation] {
                // GSM, so a segment holds 160 characters (153 when split),
                // not UCS-2's 70.
                assert!(
                    matches!(encoding(text), Encoding::Gsm7 { septets } if septets <= 2 * 153),
                    "{language}: {text:?} is {:?}",
                    encoding(text)
                );
                assert!(text.starts_with("Yuppers.app: "), "{text:?}");
                assert!(text.contains("STOP"), "{text:?}");
                assert!(!text.contains('{') && !text.contains('}'), "{text:?}");
            }
            assert!(update.contains(LINK), "{update:?}");
            assert!(confirmation.contains("HELP"), "{confirmation:?}");
        }
    }
}
