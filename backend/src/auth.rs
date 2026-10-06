//! One-time codes and sessions (DESIGN.md §8).
//!
//! A person proves control of an email address or phone number by entering a
//! code sent to it. That creates or signs into an account and yields a
//! session. Neither the code nor the session token is ever stored.
//!
//! **Limits** (DESIGN.md §8, §18 item 4). Asking for a code never ends the
//! codes already sent: the most recent few for an identifier and purpose
//! stay live until each expires, a code offered is checked against all of
//! them, and using one uses them all. So someone who knows an address can no
//! longer keep its owner out by asking for codes; the newest code the owner
//! received works. Guessing is bounded per code and per identifier per day,
//! and asking per identifier and per address per hour. Wrong guesses are
//! also counted per requester's address per hour, but only at identifiers
//! that have live codes, and past that count a wrong code is answered
//! `TOO_MANY_REQUESTS` while a right one still works: many people can share
//! one address (an office, a mobile carrier), and what some of them get
//! wrong must not lock out the rest. An IPv6 requester is counted by its
//! /64. Deletion codes are counted apart, against the account
//! (see [`Requester`]). The numbers are in [`AuthRules`]; the limits per
//! address and the identifier's daily cap on failed guesses can be set by a
//! deployment (`crate::config`).
//!
//! The counts live in `sign_in_limit`, one row per thing counted and window,
//! rather than behind advisory locks alone: an advisory lock can make
//! counting and inserting atomic, but something has to hold the count, and
//! failed guesses and requests by address leave no row of their own to count.
//! The row is locked while a request is decided, which makes each limit
//! exact under concurrency. Windows are fixed hours and UTC days, so a burst
//! straddling the turn of a window can reach twice a limit.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorCode};
use crate::languages;

/// The numbers behind code and session handling. Placeholders, every one:
/// none is a recorded design decision yet (DESIGN.md §8, §18 item 4). Every
/// limit is a count of 1 or more; there is no setting for "no limit".
#[derive(Clone, Debug)]
pub struct AuthRules {
    /// How long a code works. A placeholder.
    pub code_ttl: Duration,
    /// Wrong guesses allowed against one code before it is dead. A guess is
    /// checked against every live code, so it counts against each of them.
    /// A placeholder.
    pub code_max_failed_attempts: i16,
    /// Sign-in codes one identifier may be sent per hour. A placeholder.
    pub codes_per_hour: i64,
    /// How many of the most recent codes for one identifier and purpose stay
    /// live, each until it expires. A placeholder.
    pub live_codes: i64,
    /// Failed sign-in guesses one identifier may take per UTC day. Past
    /// that, codes are refused, right or wrong, until the day ends. A
    /// placeholder; a deployment may set it
    /// (`SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY`, `crate::config`).
    pub failed_guesses_per_identifier_per_day: i64,
    /// Failed guesses one account may make at its deletion codes per UTC
    /// day, refused in the same way past that. A placeholder.
    pub failed_deletion_guesses_per_day: i64,
    /// Sign-in codes one requester's network address may ask for per hour,
    /// whatever the identifiers. A placeholder; a deployment may set it
    /// (`SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR`).
    pub code_requests_per_address_per_hour: i64,
    /// Deletion codes one account may ask for per hour. A placeholder.
    pub deletion_codes_per_hour: i64,
    /// Codes the whole service may send by text message per hour, whoever
    /// asks: each costs money, and the limits above are per requester and
    /// per number, which many requesters and many numbers get round. Past
    /// it, a code for a phone number is refused with `TOO_MANY_REQUESTS`
    /// until the hour turns. A placeholder; a deployment may set it
    /// (`SMS_MAX_PER_HOUR`). Only a message the provider takes is counted:
    /// one it refuses gives its place back, so numbers the provider will
    /// not send to (its geographic permissions, a number that does not
    /// exist) cannot use the cap up.
    pub sms_codes_per_hour: i64,
    /// Codes the whole service may send by text message per hour to numbers
    /// that begin alike: the country code and the three digits after it, the
    /// area code for `+1` (see [`AuthRules::sms_prefix`]). Without it, a few
    /// requesters with fresh numbers could use the whole service's cap and
    /// turn off phone sign-in for everyone; with it they can turn it off for
    /// one area code at a time, and need many to reach the cap. A
    /// placeholder; a deployment may set it (`SMS_MAX_PER_PREFIX_PER_HOUR`).
    pub sms_codes_per_prefix_per_hour: i64,
    /// The country calling codes, digits only (`"1"`), whose phone numbers
    /// the service takes. A code for any other number is refused with
    /// `PHONE_COUNTRY_NOT_SERVED` before anything is counted or sent, and so
    /// is attaching one to an account. `+1` is the North American Numbering
    /// Plan: the US, Canada and some twenty Caribbean and Pacific countries
    /// and territories, which share it; the SMS provider's geographic
    /// permissions are what narrow it further (docs/operations.md). The
    /// launch market is the US; a deployment may set it
    /// (`SMS_ALLOWED_COUNTRY_CODES`).
    pub phone_country_codes: Vec<String>,
    /// How long a session lasts. A placeholder.
    pub session_ttl: Duration,
}

impl Default for AuthRules {
    fn default() -> Self {
        Self {
            code_ttl: Duration::minutes(10),
            code_max_failed_attempts: 5,
            codes_per_hour: 5,
            live_codes: 3,
            failed_guesses_per_identifier_per_day: 20,
            failed_deletion_guesses_per_day: 20,
            code_requests_per_address_per_hour: 10,
            deletion_codes_per_hour: 5,
            sms_codes_per_hour: 50,
            sms_codes_per_prefix_per_hour: 10,
            phone_country_codes: vec!["1".to_owned()],
            session_ttl: Duration::days(30),
        }
    }
}

impl AuthRules {
    /// The allowed country code a phone number begins with, if any. Country
    /// calling codes are a prefix code (no code begins another), so at most
    /// one matches. Always `None` for an email address.
    fn phone_country<'a>(&'a self, identifier: &Identifier) -> Option<&'a str> {
        let Identifier::Phone(phone) = identifier else {
            return None;
        };
        let digits = &phone[1..];
        self.phone_country_codes
            .iter()
            .map(String::as_str)
            .find(|code| digits.starts_with(code))
    }

    /// Whether the service takes this identifier: any email address, and a
    /// phone number of an allowed country.
    pub fn takes(&self, identifier: &Identifier) -> bool {
        match identifier {
            Identifier::Email(_) => true,
            Identifier::Phone(_) => self.phone_country(identifier).is_some(),
        }
    }

    /// Refuses a phone number of a country the service does not take
    /// ([`AuthRules::phone_country_codes`]).
    pub fn check_taken(&self, identifier: &Identifier) -> Result<(), ApiError> {
        if self.takes(identifier) {
            Ok(())
        } else {
            Err(ErrorCode::PhoneCountryNotServed.into())
        }
    }

    /// What a phone number is counted under for
    /// [`AuthRules::sms_codes_per_prefix_per_hour`]: its country code and
    /// the three digits after it, `+1202` for `+12025550142`. For `+1` that
    /// is the area code; elsewhere it is roughly a region or a mobile
    /// network, which is close enough for a limit.
    pub fn sms_prefix(&self, identifier: &Identifier) -> String {
        let phone = identifier.as_str();
        let country = self.phone_country(identifier).map_or(0, str::len);
        let end = (1 + country + 3).min(phone.len());
        phone[..end].to_owned()
    }
}

/// What a code was asked for. A code does only that: one sent to sign in
/// cannot delete the account, and one sent to delete it cannot sign in. The
/// message that carries a code says which it is, so that nobody is talked
/// into reading out a "sign-in code" that would delete their account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Proving control of an identifier: signing in, or attaching it to an
    /// account.
    SignIn,
    /// Confirming that the account is to be deleted (`crate::deletion`).
    DeleteAccount,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::SignIn => "sign-in",
            Purpose::DeleteAccount => "delete-account",
        }
    }
}

/// Who is asking for a code, or offering one back. That decides what the
/// code is for and which limits count the request.
///
/// Signing in is open to anyone, so it is limited by the identifier and by
/// the requester's network address. Deleting is open only to the account's
/// own session, so it is limited by the account, and by nothing someone
/// without that session can use up: not the identifier's sign-in limits, and
/// not an address, which the owner may share with whoever is flooding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requester {
    /// Someone proving control of an identifier, to sign in or to attach it
    /// to their account, known by the address the request came from
    /// (`crate::http::ClientAddress`). `None` only for a request with no
    /// connection behind it; all such requests share one count.
    SignIn { address: Option<IpAddr> },
    /// A signed-in account confirming that it is to be deleted.
    DeleteAccount { account: Uuid },
}

impl Requester {
    pub fn purpose(self) -> Purpose {
        match self {
            Requester::SignIn { .. } => Purpose::SignIn,
            Requester::DeleteAccount { .. } => Purpose::DeleteAccount,
        }
    }
}

// ---- Delivery ---------------------------------------------------------------

pub type SendFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// A one-time code on its way to someone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeMessage<'a> {
    pub to: &'a Identifier,
    pub code: &'a str,
    /// What the code is for. The message must say so.
    pub purpose: Purpose,
    /// The language to write the message in: a supported language tag, or
    /// whatever the client asked for, to be resolved by the wording.
    pub language: &'a str,
}

/// Delivers a one-time code by email or SMS.
pub trait CodeSender: Send + Sync {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a>;

    /// Whether a code to `to` costs a message's price, as a text message
    /// does: such codes are also counted against the service's hourly cap
    /// ([`AuthRules::sms_codes_per_hour`]).
    fn charged_per_message(&self, _to: &Identifier) -> bool {
        false
    }

    /// Whether this sender can deliver a code to an identifier of this kind
    /// at all, which `GET /v1/meta` tells the clients (`sign_in_channels`)
    /// so that they ask only for what can be used. A sender refusing a kind
    /// here refuses it in [`CodeSender::send`] too.
    fn delivers(&self, _channel: SignInChannel) -> bool {
        true
    }

    /// The email address codes sent by email come from, so that the clients
    /// can tell someone waiting for one what to look for (`GET /v1/meta`,
    /// `code_sender`): the address part of the From mailbox only, never its
    /// display name. `None` where there is no such address, as for the
    /// development log.
    fn email_sender(&self) -> Option<String> {
        None
    }
}

/// A kind of identifier a code can be sent to: what someone can sign in with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SignInChannel {
    Email,
    Phone,
}

impl SignInChannel {
    pub const ALL: [SignInChannel; 2] = [SignInChannel::Email, SignInChannel::Phone];
}

/// The kinds of identifier `sender` can deliver codes to, email first.
pub fn sign_in_channels(sender: &dyn CodeSender) -> Vec<SignInChannel> {
    SignInChannel::ALL
        .into_iter()
        .filter(|channel| sender.delivers(*channel))
        .collect()
}

/// Development delivery: writes the code to the service log. Never configured
/// in production, where anyone who can read logs could sign in as anyone.
/// A phone number is written masked, as everywhere else; the code, and an
/// email address, in full, since reading them there is how a developer signs
/// in (the end-to-end tests find codes by address).
pub struct LogSender;

impl CodeSender for LogSender {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            let to = match message.to {
                Identifier::Email(email) => email.clone(),
                phone @ Identifier::Phone(_) => phone.masked(),
            };
            tracing::info!(
                to,
                code = message.code,
                purpose = message.purpose.as_str(),
                language = message.language,
                "one-time code (development delivery)"
            );
            Ok(())
        })
    }
}

/// The language to write to someone in: their account's preference if an
/// account has this identifier, otherwise the first supported language among
/// those the client asked for in `Accept-Language`, otherwise the default.
/// Looked up as a side query, so a failure here costs the language, not the
/// code.
pub async fn language_for(
    db: &PgPool,
    identifier: &Identifier,
    accept_language: Option<&str>,
) -> String {
    let column = match identifier {
        Identifier::Email(_) => "email",
        Identifier::Phone(_) => "phone",
    };
    let preference: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT language FROM account WHERE {column} = $1 AND status = 'ACTIVE'"
    )))
    .bind(identifier.as_str())
    .fetch_optional(db)
    .await
    .unwrap_or_default();
    preference
        .or_else(|| {
            accept_language?
                .split(',')
                .map(|entry| entry.split(';').next().unwrap_or("").trim())
                .find_map(|tag| languages::resolve(tag).map(str::to_owned))
        })
        .unwrap_or_else(|| languages::default().to_owned())
}

// ---- Codes and tokens -------------------------------------------------------

/// Six decimal digits, uniformly distributed.
pub fn generate_code() -> String {
    // Reject the top sliver of the range so the remainder is unbiased.
    const LIMIT: u32 = u32::MAX - u32::MAX % 1_000_000;
    loop {
        let n = getrandom::u32().expect("the operating system provides randomness");
        if n < LIMIT {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

fn code_mac(secret: &[u8], purpose: Purpose, identifier: &Identifier, code: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    // The purpose is part of what is hashed, which is what ties a stored code
    // to it. A sign-in code is hashed as it always was. No identifier
    // contains a NUL, so the two forms cannot be confused.
    if purpose != Purpose::SignIn {
        mac.update(purpose.as_str().as_bytes());
        mac.update(b"\0");
    }
    mac.update(identifier.as_str().as_bytes());
    mac.update(b"\0");
    mac.update(code.as_bytes());
    mac
}

pub fn code_hash(secret: &[u8], purpose: Purpose, identifier: &Identifier, code: &str) -> [u8; 32] {
    code_mac(secret, purpose, identifier, code)
        .finalize()
        .into_bytes()
        .into()
}

/// A session or invitation token: 256 random bits as 64 hex characters.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

// ---- Limits -----------------------------------------------------------------

/// The `sign_in_limit` scopes that count text messages for the whole
/// service, per hour. The metrics read them back (`crate::metrics`).
pub const SMS_SENT: &str = "sms-sent";
pub const SMS_REFUSED: &str = "sms-refused";
pub const SMS_FAILED: &str = "sms-failed";
pub const SMS_SENT_BY_PREFIX: &str = "sms-sent-by-prefix";
pub const SMS_REFUSED_PREFIX: &str = "sms-refused-prefix";
pub const SMS_REFUSED_COUNTRY: &str = "sms-refused-country";

/// What every SMS count is counted under: one subject, the whole service.
const EVERYONE: &str = "the whole service";

/// What `sign_in_limit` counts. Each is counted in fixed windows: the hour,
/// or the UTC day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Counted {
    CodeRequestsByAddress,
    FailedGuessesByIdentifier,
    CodeRequestsByAccount,
    FailedGuessesByAccount,
    /// Codes the SMS provider took, or is being handed, by the whole
    /// service.
    SmsSent,
    /// The same, per number prefix ([`AuthRules::sms_prefix`]).
    SmsSentByPrefix,
    /// Codes not sent by SMS because the hourly cap was reached.
    SmsRefused,
    /// Codes not sent by SMS because the prefix's hourly cap was reached.
    SmsRefusedPrefix,
    /// Codes not sent because the number's country is not served.
    SmsRefusedCountry,
    /// Codes the SMS provider did not take.
    SmsFailed,
}

impl Counted {
    fn scope(self) -> &'static str {
        match self {
            Counted::CodeRequestsByAddress => "code-requests-by-address",
            Counted::FailedGuessesByIdentifier => "failed-guesses-by-identifier",
            Counted::CodeRequestsByAccount => "code-requests-by-account",
            Counted::FailedGuessesByAccount => "failed-guesses-by-account",
            Counted::SmsSent => SMS_SENT,
            Counted::SmsSentByPrefix => SMS_SENT_BY_PREFIX,
            Counted::SmsRefused => SMS_REFUSED,
            Counted::SmsRefusedPrefix => SMS_REFUSED_PREFIX,
            Counted::SmsRefusedCountry => SMS_REFUSED_COUNTRY,
            Counted::SmsFailed => SMS_FAILED,
        }
    }

    /// The window, as `date_trunc` names it.
    fn window(self) -> &'static str {
        match self {
            Counted::FailedGuessesByIdentifier | Counted::FailedGuessesByAccount => "day",
            _ => "hour",
        }
    }
}

/// One count in `sign_in_limit`: what is counted, and of whom. Whom is kept
/// only as a keyed hash, so the table holds no address or identifier.
struct Counter {
    counted: Counted,
    subject: [u8; 32],
}

impl Counter {
    fn new(secret: &[u8], counted: Counted, subject: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(b"sign-in-limit\0");
        mac.update(counted.scope().as_bytes());
        mac.update(b"\0");
        mac.update(subject.as_bytes());
        Self {
            counted,
            subject: mac.finalize().into_bytes().into(),
        }
    }

    fn address(secret: &[u8], counted: Counted, address: Option<IpAddr>) -> Self {
        // Not an address, so it cannot be mistaken for one.
        let subject = address.map_or_else(|| "unknown".to_owned(), requester_network);
        Self::new(secret, counted, &subject)
    }

    /// The count so far in the current window. The row stays locked until
    /// the transaction ends, so nobody else decides on it meanwhile.
    async fn hold(&self, conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
        self.add(conn, 0).await
    }

    /// Adds to the count in the current window and returns the new count.
    async fn add(&self, conn: &mut PgConnection, by: i32) -> Result<i64, sqlx::Error> {
        let count: i32 = sqlx::query_scalar(
            "INSERT INTO sign_in_limit (scope, subject, window_start, count)
             VALUES ($1, $2, date_trunc($3, now(), 'UTC'), $4)
             ON CONFLICT (scope, subject, window_start)
             DO UPDATE SET count = sign_in_limit.count + EXCLUDED.count
             RETURNING count",
        )
        .bind(self.counted.scope())
        .bind(self.subject.as_slice())
        .bind(self.counted.window())
        .bind(by)
        .fetch_one(conn)
        .await?;
        Ok(i64::from(count))
    }

    /// Takes one place in the current window and says which window it was,
    /// so that [`Counter::release`] can give it back there.
    async fn take(&self, conn: &mut PgConnection) -> Result<OffsetDateTime, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO sign_in_limit (scope, subject, window_start, count)
             VALUES ($1, $2, date_trunc($3, now(), 'UTC'), 1)
             ON CONFLICT (scope, subject, window_start)
             DO UPDATE SET count = sign_in_limit.count + 1
             RETURNING window_start",
        )
        .bind(self.counted.scope())
        .bind(self.subject.as_slice())
        .bind(self.counted.window())
        .fetch_one(conn)
        .await
    }

    /// Gives back a place taken in `window`, even if that window has since
    /// ended. Never below zero.
    async fn release(
        &self,
        conn: &mut PgConnection,
        window: OffsetDateTime,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE sign_in_limit SET count = count - 1
             WHERE scope = $1 AND subject = $2 AND window_start = $3 AND count > 0",
        )
        .bind(self.counted.scope())
        .bind(self.subject.as_slice())
        .bind(window)
        .execute(conn)
        .await?;
        Ok(())
    }
}

/// The network a requester's address is counted as. An IPv4 address is
/// itself, and so is one written as IPv6 (`::ffff:192.0.2.50`), or a
/// requester could double its allowance by switching notation. An IPv6
/// address is counted by its /64: that is what one subscriber is usually
/// given, and any address within it is theirs to use, so counting single
/// addresses would let one requester take a fresh allowance per request.
fn requester_network(address: IpAddr) -> String {
    match address {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let [a, b, c, d, ..] = v6.segments();
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
            }
        },
    }
}

/// One request at a time per identifier and purpose, so that counting,
/// inserting and checking codes cannot be raced. Sign-in and deletion take
/// different locks, so neither waits on the other.
async fn lock_codes(
    conn: &mut PgConnection,
    identifier: &Identifier,
    purpose: Purpose,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "one-time-code:{}:{}",
            purpose.as_str(),
            identifier.as_str()
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Forgets counts whose window ended more than a day ago; none of them can
/// matter any more. Returns how many were removed. Called by the worker.
pub async fn purge_sign_in_limits(db: &PgPool) -> Result<u64, sqlx::Error> {
    let removed =
        sqlx::query("DELETE FROM sign_in_limit WHERE window_start < now() - interval '2 days'")
            .execute(db)
            .await?
            .rows_affected();
    Ok(removed)
}

// ---- Requesting and verifying a code ----------------------------------------

/// Issues a new code for an identifier and sends it, in `language` (see
/// [`language_for`]). Codes already sent for the same identifier and purpose
/// keep working until they expire, except that only the newest
/// [`AuthRules::live_codes`] are kept: older ones stop working.
///
/// Refused with `TOO_MANY_REQUESTS` when the requester's address has asked
/// for too many sign-in codes this hour, when the identifier has been sent
/// too many, or, for deletion, when the account has asked for too many, or,
/// for a code that would go by text message, when the service has sent its
/// hourly cap of them ([`AuthRules::sms_codes_per_hour`]) or of them to
/// numbers beginning alike ([`AuthRules::sms_codes_per_prefix_per_hour`]);
/// with `PHONE_COUNTRY_NOT_SERVED`, before anything is counted, for a phone
/// number of a country the service does not take; and with
/// `TOO_MANY_GUESSES` while the identifier has used up its wrong sign-in
/// guesses for the day, or, for deletion, the account its wrong deletion
/// guesses, since no code sent then could work.
pub async fn request_code(
    db: &PgPool,
    secret: &[u8],
    rules: &AuthRules,
    sender: &dyn CodeSender,
    identifier: &Identifier,
    requester: Requester,
    language: &str,
) -> Result<(), ApiError> {
    let purpose = requester.purpose();
    let charged = sender.charged_per_message(identifier);

    // Before anything is counted: a number the service would never send to
    // must not use up anybody's allowance, the requester's included.
    if !rules.takes(identifier) {
        // For the metrics, only where a message would have been paid for;
        // a failure to count is not the person's problem.
        if charged && let Ok(mut conn) = db.acquire().await {
            let _ = Counter::new(secret, Counted::SmsRefusedCountry, EVERYONE)
                .add(&mut conn, 1)
                .await;
        }
        return Err(ErrorCode::PhoneCountryNotServed.into());
    }

    // A number that replied STOP gets no text, a code included, until it
    // replies START (`crate::notifications::sms_updates`). Refused before
    // anything is counted, with an answer that points to email instead.
    if charged && is_opted_out(db, identifier).await? {
        return Err(ErrorCode::PhoneOptedOut.into());
    }

    let mut tx = db.begin().await?;

    // The requester first, and then the identifier, always in that order.
    let (requests, limit) = match requester {
        Requester::SignIn { address } => (
            Counter::address(secret, Counted::CodeRequestsByAddress, address),
            rules.code_requests_per_address_per_hour,
        ),
        Requester::DeleteAccount { account } => (
            Counter::new(secret, Counted::CodeRequestsByAccount, &account.to_string()),
            rules.deletion_codes_per_hour,
        ),
    };
    if requests.hold(&mut tx).await? >= limit {
        return Err(ErrorCode::TooManyRequests.into());
    }
    // Counted whether or not the identifier's own limit then refuses it.
    requests.add(&mut tx, 1).await?;

    lock_codes(&mut tx, identifier, purpose).await?;

    // Whoever has used up their wrong guesses for the day would be refused
    // any code sent now, right or wrong, so none is sent: for signing in the
    // identifier, for deleting the account.
    let (guessed, guess_limit) = match requester {
        Requester::SignIn { .. } => (
            Counter::new(
                secret,
                Counted::FailedGuessesByIdentifier,
                identifier.as_str(),
            ),
            rules.failed_guesses_per_identifier_per_day,
        ),
        Requester::DeleteAccount { account } => (
            Counter::new(
                secret,
                Counted::FailedGuessesByAccount,
                &account.to_string(),
            ),
            rules.failed_deletion_guesses_per_day,
        ),
    };
    if guessed.hold(&mut tx).await? >= guess_limit {
        tx.commit().await?;
        return Err(ErrorCode::TooManyGuesses.into());
    }

    // A deletion code goes only to the account's own identifier, at the
    // account's own request, so the account's count above is what limits it.
    // Counting it against the identifier as well would let anyone asking
    // for sign-in codes use up the owner's way to delete.
    if purpose == Purpose::SignIn {
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM one_time_code
             WHERE identifier = $1 AND purpose = $2 AND created_at > now() - interval '1 hour'",
        )
        .bind(identifier.as_str())
        .bind(purpose.as_str())
        .fetch_one(&mut *tx)
        .await?;
        if recent >= rules.codes_per_hour {
            tx.commit().await?;
            return Err(ErrorCode::TooManyRequests.into());
        }
    }

    // A text message costs money whoever asks for it, so the service as a
    // whole sends only so many an hour, and only so many of them to numbers
    // beginning alike. Last of the limits, so that only a code that would
    // otherwise go is counted against them. A place is taken in each before
    // the message is handed over and given back if the provider does not
    // take it, so a message refused costs no place while the cap still
    // holds exactly: the rows stay locked until the code is stored, the
    // prefix's always before the whole service's, which makes both exact
    // across every copy of the API.
    let mut taken = None;
    if charged {
        let prefix = Counter::new(
            secret,
            Counted::SmsSentByPrefix,
            &rules.sms_prefix(identifier),
        );
        let everyone = Counter::new(secret, Counted::SmsSent, EVERYONE);
        let refusal = if prefix.hold(&mut tx).await? >= rules.sms_codes_per_prefix_per_hour {
            Some((Counted::SmsRefusedPrefix, "numbers beginning alike"))
        } else if everyone.hold(&mut tx).await? >= rules.sms_codes_per_hour {
            Some((Counted::SmsRefused, "the whole service"))
        } else {
            None
        };
        if let Some((counted, whose)) = refusal {
            Counter::new(secret, counted, EVERYONE)
                .add(&mut tx, 1)
                .await?;
            tx.commit().await?;
            tracing::warn!(
                cap = rules.sms_codes_per_hour,
                prefix_cap = rules.sms_codes_per_prefix_per_hour,
                whose,
                "a code was not sent by SMS: an hourly cap is reached"
            );
            return Err(ErrorCode::TooManyRequests.into());
        }
        let prefix_window = prefix.take(&mut tx).await?;
        let everyone_window = everyone.take(&mut tx).await?;
        taken = Some([(prefix, prefix_window), (everyone, everyone_window)]);
    }

    let code = generate_code();
    // The clock, not the transaction's start: requests for one identifier
    // are ordered by the lock above, and so are their codes.
    sqlx::query(
        "INSERT INTO one_time_code (identifier, purpose, code_hash, expires_at, created_at)
         VALUES ($1, $2, $3, clock_timestamp() + $4 * interval '1 second', clock_timestamp())",
    )
    .bind(identifier.as_str())
    .bind(purpose.as_str())
    .bind(code_hash(secret, purpose, identifier, &code).as_slice())
    .bind(rules.code_ttl.whole_seconds() as f64)
    .execute(&mut *tx)
    .await?;

    // Only the newest few stay live.
    sqlx::query(
        "UPDATE one_time_code SET consumed_at = now()
         WHERE identifier = $1 AND purpose = $2 AND consumed_at IS NULL
           AND id NOT IN (
               SELECT id FROM one_time_code
               WHERE identifier = $1 AND purpose = $2
               ORDER BY created_at DESC
               LIMIT $3)",
    )
    .bind(identifier.as_str())
    .bind(purpose.as_str())
    .bind(rules.live_codes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let sent = sender
        .send(CodeMessage {
            to: identifier,
            code: &code,
            purpose,
            language,
        })
        .await;
    if let Err(error) = sent {
        // The error names no address or number and quotes no provider's
        // text (each sender sees to it).
        tracing::error!(%error, "one-time code could not be delivered");
        // The places it took are given back, so that a message the
        // provider refused costs none, and it is counted for the metrics.
        // A failure here is not the person's problem: a place not given
        // back stays taken until the hour turns, erring on the side of
        // fewer messages. One statement at a time, so no lock is held
        // across them.
        if let Some(taken) = taken
            && let Ok(mut conn) = db.acquire().await
        {
            for (counter, window) in &taken {
                let _ = counter.release(&mut conn, *window).await;
            }
            let _ = Counter::new(secret, Counted::SmsFailed, EVERYONE)
                .add(&mut conn, 1)
                .await;
        }
        return Err(ErrorCode::ServiceUnavailable.into());
    }
    Ok(())
}

/// Whether `identifier` is a phone number that replied STOP to our texts
/// and has not replied START since.
pub async fn is_opted_out(db: &PgPool, identifier: &Identifier) -> Result<bool, sqlx::Error> {
    let Identifier::Phone(phone) = identifier else {
        return Ok(false);
    };
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sms_opt_out WHERE phone = $1)")
        .bind(phone)
        .fetch_one(db)
        .await
}

/// A place taken under the hourly caps on text messages for one text that
/// is not a code: an agreement update (`crate::notifications::sms_updates`).
/// Update texts and codes share the caps, the whole service's and the one
/// per number prefix, since they cost the same.
pub struct SmsPlace {
    taken: [(Counter, OffsetDateTime); 2],
}

impl SmsPlace {
    /// Takes a place for a text to `identifier` under both hourly caps, or
    /// `None` if either is reached, which is counted for the metrics as a
    /// code refused at the cap is. Run it in the transaction that sends the
    /// text: the counts stay locked until it ends.
    pub async fn take(
        conn: &mut PgConnection,
        secret: &[u8],
        rules: &AuthRules,
        identifier: &Identifier,
    ) -> Result<Option<Self>, sqlx::Error> {
        let prefix = Counter::new(
            secret,
            Counted::SmsSentByPrefix,
            &rules.sms_prefix(identifier),
        );
        let everyone = Counter::new(secret, Counted::SmsSent, EVERYONE);
        let refusal = if prefix.hold(conn).await? >= rules.sms_codes_per_prefix_per_hour {
            Some(Counted::SmsRefusedPrefix)
        } else if everyone.hold(conn).await? >= rules.sms_codes_per_hour {
            Some(Counted::SmsRefused)
        } else {
            None
        };
        if let Some(counted) = refusal {
            Counter::new(secret, counted, EVERYONE).add(conn, 1).await?;
            return Ok(None);
        }
        let prefix_window = prefix.take(conn).await?;
        let everyone_window = everyone.take(conn).await?;
        Ok(Some(Self {
            taken: [(prefix, prefix_window), (everyone, everyone_window)],
        }))
    }

    /// Gives the place back because the provider did not take the text,
    /// and counts that for the metrics.
    pub async fn give_back(
        self,
        conn: &mut PgConnection,
        secret: &[u8],
    ) -> Result<(), sqlx::Error> {
        for (counter, window) in &self.taken {
            counter.release(conn, *window).await?;
        }
        Counter::new(secret, Counted::SmsFailed, EVERYONE)
            .add(conn, 1)
            .await?;
        Ok(())
    }
}

/// Checks a code against every live code for the identifier and purpose,
/// and if it matches one, uses them all up. A wrong guess is counted against
/// each live code and against the identifier's day (for deletion, the
/// account's day) before the refusal is returned, so guessing cannot be
/// retried for free. A code asked for one purpose and offered for another
/// is a wrong guess.
///
/// Refused with `TOO_MANY_GUESSES`, right or wrong, once the identifier (for
/// deletion, the account) has used up its failed guesses for the day.
/// Otherwise a wrong code is refused with `INVALID_CODE`, the same answer
/// whether or not the identifier had a live code, so the answer never says
/// whether someone is signing in.
///
/// Wrong guesses are not limited by the requester's address. A limit there
/// could only refuse a wrong code differently from a code at an identifier
/// with nothing live, which would say that a code had just been sent, or
/// refuse right codes too, which would let one requester lock out everyone
/// sharing its address. What bounds guessing is the attempts per code, the
/// identifier's daily cap, and the limit on code requests by address, since
/// every code guessed at must first be asked for.
pub async fn verify_code(
    db: &PgPool,
    secret: &[u8],
    rules: &AuthRules,
    identifier: &Identifier,
    code: &str,
    requester: Requester,
) -> Result<(), ApiError> {
    let offered = OfferedCode {
        secret,
        rules,
        identifier,
        code,
        requester,
    };
    let mut tx = db.begin().await?;
    let checked = offered.check(&mut tx).await?;
    tx.commit().await?;
    match checked {
        CodeCheck::Matched => Ok(()),
        CodeCheck::Refused(error) => Err(error),
    }
}

/// A code someone offered back, with what is needed to check it. For a
/// caller that must use the code up in the same transaction as what it
/// confirms, so that if that cannot be done the code is not spent either
/// (`crate::deletion`). [`verify_code`] is the same check on its own.
#[derive(Clone, Copy)]
pub struct OfferedCode<'a> {
    pub secret: &'a [u8],
    pub rules: &'a AuthRules,
    pub identifier: &'a Identifier,
    pub code: &'a str,
    pub requester: Requester,
}

/// What [`OfferedCode::check`] found. Either way the transaction holds what
/// it wrote, and only committing it makes that stand.
#[must_use]
pub enum CodeCheck {
    /// The code matched, and every live code for the identifier and purpose
    /// is marked used. Rolling back leaves them live, as they were.
    Matched,
    /// The code was refused. A wrong guess is counted in the transaction,
    /// which must be committed before the refusal is returned, or the guess
    /// would cost nothing.
    Refused(ApiError),
}

impl OfferedCode<'_> {
    /// Checks the code within `conn`'s transaction, as [`verify_code`]
    /// describes. The limit rows and the codes it reads stay locked until
    /// the transaction ends, so nobody else checks a code for the same
    /// identifier and purpose meanwhile.
    pub async fn check(&self, conn: &mut PgConnection) -> Result<CodeCheck, ApiError> {
        let Self {
            secret,
            rules,
            identifier,
            code,
            requester,
        } = *self;
        let purpose = requester.purpose();

        // A sign-in guess is counted against the identifier; a deletion is
        // guessed at only through the account's own session, so it is counted
        // against the account.
        let (by_owner, owner_limit) = match requester {
            Requester::SignIn { .. } => (
                Counter::new(
                    secret,
                    Counted::FailedGuessesByIdentifier,
                    identifier.as_str(),
                ),
                rules.failed_guesses_per_identifier_per_day,
            ),
            Requester::DeleteAccount { account } => (
                Counter::new(
                    secret,
                    Counted::FailedGuessesByAccount,
                    &account.to_string(),
                ),
                rules.failed_deletion_guesses_per_day,
            ),
        };
        lock_codes(conn, identifier, purpose).await?;
        if by_owner.hold(conn).await? >= owner_limit {
            return Ok(CodeCheck::Refused(ErrorCode::TooManyGuesses.into()));
        }

        let live: Vec<(Uuid, Vec<u8>)> = sqlx::query_as(
            "SELECT id, code_hash FROM one_time_code
             WHERE identifier = $1 AND purpose = $2
               AND consumed_at IS NULL AND expires_at > now() AND failed_attempts < $3
             FOR UPDATE",
        )
        .bind(identifier.as_str())
        .bind(purpose.as_str())
        .bind(rules.code_max_failed_attempts)
        .fetch_all(&mut *conn)
        .await?;

        // Constant-time comparison against each, and no stopping at the first
        // match, so the time taken says nothing about which one it was.
        let offered = code.trim();
        let matches = live
            .iter()
            .map(|(_, expected)| {
                code_mac(secret, purpose, identifier, offered)
                    .verify_slice(expected)
                    .is_ok()
            })
            .fold(false, |any, this| any | this);

        if matches {
            sqlx::query(
                "UPDATE one_time_code SET consumed_at = now()
                 WHERE identifier = $1 AND purpose = $2 AND consumed_at IS NULL",
            )
            .bind(identifier.as_str())
            .bind(purpose.as_str())
            .execute(&mut *conn)
            .await?;
            return Ok(CodeCheck::Matched);
        }

        // With no live code there was nothing to guess at, so nothing is
        // charged: otherwise anyone could use up an identifier's day without
        // a code ever being sent. The answer is the same either way.
        if live.is_empty() {
            return Ok(CodeCheck::Refused(ErrorCode::InvalidCode.into()));
        }
        let ids: Vec<Uuid> = live.iter().map(|(id, _)| *id).collect();
        sqlx::query(
            "UPDATE one_time_code SET failed_attempts = failed_attempts + 1 WHERE id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        by_owner.add(conn, 1).await?;
        Ok(CodeCheck::Refused(ErrorCode::InvalidCode.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requesters_are_counted_by_ipv4_address_and_by_ipv6_slash_64() {
        let network = |text: &str| requester_network(text.parse().unwrap());
        assert_eq!(network("192.0.2.50"), "192.0.2.50");
        // The same IPv4 address written as IPv6 is the same requester.
        assert_eq!(network("::ffff:192.0.2.50"), "192.0.2.50");
        // Every address in one /64 is one requester; the next /64 is another.
        assert_eq!(network("2001:db8:2::1"), "2001:db8:2:0::/64");
        assert_eq!(network("2001:db8:2::12"), network("2001:db8:2::1"));
        assert_eq!(
            network("2001:db8:2:0:ffff:ffff:ffff:ffff"),
            network("2001:db8:2::1")
        );
        assert_ne!(network("2001:db8:2:1::1"), network("2001:db8:2::1"));

        let secret = b"secret";
        let counter = |text: &str| {
            Counter::address(
                secret,
                Counted::CodeRequestsByAddress,
                Some(text.parse().unwrap()),
            )
            .subject
        };
        assert_eq!(counter("2001:db8:2::1"), counter("2001:db8:2::12"));
        assert_eq!(counter("::ffff:192.0.2.50"), counter("192.0.2.50"));
        assert_ne!(counter("192.0.2.50"), counter("192.0.2.51"));
    }

    #[test]
    fn phone_numbers_are_taken_by_country_code_and_counted_by_prefix() {
        let rules = AuthRules::default();
        let parse = |text: &str| Identifier::parse(text).unwrap();
        assert!(rules.takes(&parse("+12025550142")));
        assert!(rules.takes(&parse("ana@example.com")));
        for other in ["+447700900123", "+525512345678", "+79991234567"] {
            assert!(!rules.takes(&parse(other)), "{other}");
            assert!(rules.check_taken(&parse(other)).is_err(), "{other}");
        }
        assert_eq!(rules.sms_prefix(&parse("+12025550142")), "+1202");
        assert_eq!(rules.sms_prefix(&parse("+1 (888) 555-0100")), "+1888");

        let more = AuthRules {
            phone_country_codes: vec!["1".to_owned(), "52".to_owned(), "353".to_owned()],
            ..AuthRules::default()
        };
        assert!(more.takes(&parse("+525512345678")));
        assert_eq!(more.sms_prefix(&parse("+525512345678")), "+52551");
        assert_eq!(more.sms_prefix(&parse("+353861234567")), "+353861");
        assert!(!more.takes(&parse("+5491112345678")));
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let code = generate_code();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
        }
    }

    #[test]
    fn tokens_are_long_and_distinct() {
        let (a, b) = (generate_token(), generate_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn a_code_hash_depends_on_the_secret_the_identifier_the_code_and_the_purpose() {
        let ana = Identifier::parse("ana@example.com").unwrap();
        let ben = Identifier::parse("ben@example.com").unwrap();
        let sign_in = Purpose::SignIn;
        let base = code_hash(b"secret", sign_in, &ana, "123456");

        assert_eq!(code_hash(b"secret", sign_in, &ana, "123456"), base);
        assert_ne!(code_hash(b"other", sign_in, &ana, "123456"), base);
        assert_ne!(code_hash(b"secret", sign_in, &ben, "123456"), base);
        assert_ne!(code_hash(b"secret", sign_in, &ana, "123457"), base);
        assert_ne!(
            code_hash(b"secret", Purpose::DeleteAccount, &ana, "123456"),
            base
        );
    }
}
