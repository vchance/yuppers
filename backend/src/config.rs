//! What a deployment tells the processes, read from the environment (and
//! `.env` in development). `.env.example` documents every setting.
//!
//! Two settings have no default on purpose: `CODE_DELIVERY` and
//! `NOTIFICATION_DELIVERY`. A deployment must say how messages reach people,
//! so the development delivery, which writes them to a log, is never used by
//! accident.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use axum::http::HeaderName;

use crate::app_role::{APP_ROLE, AppRolePassword};
use crate::auth::{AuthRules, CodeSender, LogSender};
use crate::client_version::{MinimumClientVersions, parse_version};
use crate::contact::{Key, KeyConfig};
use crate::domain::identity::Identifier;
use crate::http::web::DEFAULT_ANDROID_PACKAGE;
use crate::http::{AppLinks, TrustedProxies};
use crate::nanp;
use crate::notifications::expo::{EXPO_ORIGIN, ExpoPushSender};
use crate::notifications::push::{LogPushSender, PushSender};
use crate::notifications::resend::{RESEND_ORIGIN, ResendSender, ResendSettings};
use crate::notifications::sms::{
    CodeRouter, LogSmsSender, PhoneCodes, SmsSender, TWILIO_ORIGIN, TwilioCredential,
    TwilioSmsSender,
};
use crate::notifications::smtp::{Secret, SmtpSender, SmtpSettings, TlsMode};
use crate::notifications::verify::{TwilioVerify, VERIFY_ORIGIN, VerifyServices};
use crate::notifications::wording::Wording;
use crate::notifications::{EmailSender, LogEmailSender};

/// Reads a setting. The environment in production, a table in the tests.
type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn load_env() {
    dotenvy::dotenv().ok();
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A setting that may be left out. One set to nothing counts as left out, so
/// a line in `.env` can show a setting without choosing a value for it.
fn optional(get: Lookup<'_>, name: &str) -> Option<String> {
    get(name).filter(|value| !value.trim().is_empty())
}

fn required(get: Lookup<'_>, name: &str) -> anyhow::Result<String> {
    optional(get, name).with_context(|| format!("{name} is not set"))
}

/// Where the web app is served from, without a trailing slash.
fn web_origin(get: Lookup<'_>) -> anyhow::Result<String> {
    Ok(required(get, "WEB_ORIGIN")?
        .trim_end_matches('/')
        .to_owned())
}

/// How long one SMTP conversation may be silent. Within the outbox's own
/// limit on a send, so that it is this timeout, with its clearer error, that
/// a notification usually fails by.
const SMTP_TIMEOUT: Duration = Duration::from_secs(20);

/// The address email comes from, whichever way it is sent: `EMAIL_FROM`,
/// or else `SMTP_FROM`, the name it had when SMTP was the only way, which
/// still works.
fn email_from(get: Lookup<'_>) -> anyhow::Result<String> {
    optional(get, "EMAIL_FROM")
        .or_else(|| optional(get, "SMTP_FROM"))
        .context("EMAIL_FROM is not set (nor SMTP_FROM, which it replaces)")
}

/// The SMTP server, from `SMTP_HOST`, `SMTP_PORT`, `SMTP_TLS`,
/// `SMTP_USERNAME` and `SMTP_PASSWORD`, sending from [`email_from`]. The
/// password is read into a type that cannot be printed.
fn smtp_settings(get: Lookup<'_>) -> anyhow::Result<SmtpSettings> {
    let tls = match optional(get, "SMTP_TLS") {
        None => TlsMode::Tls,
        Some(value) => TlsMode::parse(&value).with_context(|| {
            format!("SMTP_TLS={value} is not one of `tls`, `starttls` or `none`")
        })?,
    };
    let port = match optional(get, "SMTP_PORT") {
        None => tls.default_port(),
        Some(value) => value
            .parse()
            .with_context(|| format!("SMTP_PORT={value} is not a port number"))?,
    };
    let credentials = match (
        optional(get, "SMTP_USERNAME"),
        optional(get, "SMTP_PASSWORD"),
    ) {
        (Some(username), Some(password)) => Some((username, Secret::new(password))),
        (None, None) => None,
        _ => bail!("SMTP_USERNAME and SMTP_PASSWORD must be set together, or neither"),
    };
    let host = required(get, "SMTP_HOST")?;
    if tls == TlsMode::None {
        refuse_plaintext_remote(get, &host, credentials.is_some())?;
    }
    Ok(SmtpSettings {
        host,
        port,
        tls,
        credentials,
        from: email_from(get)?,
        timeout: SMTP_TIMEOUT,
    })
}

/// `SMTP_TLS=none` is for a relay on this host: anything said to a server
/// across a network, a password first of all, could be read on the way.
/// Refused for any other host, or with credentials, unless
/// `SMTP_ALLOW_PLAINTEXT_REMOTE=true` says it is a test server.
fn refuse_plaintext_remote(get: Lookup<'_>, host: &str, credentials: bool) -> anyhow::Result<()> {
    match optional(get, "SMTP_ALLOW_PLAINTEXT_REMOTE").as_deref() {
        Some("true") => return Ok(()),
        None | Some("false") => {}
        Some(other) => bail!("SMTP_ALLOW_PLAINTEXT_REMOTE={other} is not `true` or `false`"),
    }
    let host = host.trim().trim_end_matches('.');
    let unbracketed = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || unbracketed
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !loopback {
        bail!(
            "SMTP_TLS=none sends everything in plain text, so SMTP_HOST must be this host \
             (localhost, 127.0.0.1 or ::1); set SMTP_ALLOW_PLAINTEXT_REMOTE=true for a test server"
        );
    }
    if credentials {
        bail!(
            "SMTP_TLS=none would send SMTP_USERNAME and SMTP_PASSWORD in plain text; \
             set SMTP_ALLOW_PLAINTEXT_REMOTE=true for a test server"
        );
    }
    Ok(())
}

fn smtp_sender(get: Lookup<'_>) -> anyhow::Result<Arc<SmtpSender>> {
    // The wording is checked now, so a message that cannot be written stops
    // the process from starting instead of failing one send at a time.
    let sender = SmtpSender::new(smtp_settings(get)?, Wording::embedded()?)?;
    Ok(Arc::new(sender))
}

/// How long one request to Resend may take, like SMTP's: within the
/// outbox's own limit on a send.
const RESEND_TIMEOUT: Duration = Duration::from_secs(20);

/// Resend's API, with `RESEND_API_KEY`, sending from [`email_from`]. The
/// key is read into a type that cannot be printed, and no error quotes it.
fn resend_sender(get: Lookup<'_>, origin: &str) -> anyhow::Result<Arc<ResendSender>> {
    let api_key = optional(get, "RESEND_API_KEY")
        .context("RESEND_API_KEY is not set, and delivery by Resend needs it")?;
    let settings = ResendSettings {
        api_key: Secret::new(api_key.trim().to_owned()),
        from: email_from(get)?,
        timeout: RESEND_TIMEOUT,
    };
    Ok(Arc::new(ResendSender::new(
        origin,
        settings,
        Wording::embedded()?,
    )?))
}

/// The sender named by `NOTIFICATION_DELIVERY`.
fn email_sender(get: Lookup<'_>) -> anyhow::Result<Arc<dyn EmailSender>> {
    match required(get, "NOTIFICATION_DELIVERY")?.as_str() {
        "log" => Ok(Arc::new(LogEmailSender)),
        "smtp" => Ok(smtp_sender(get)?),
        "resend" => Ok(resend_sender(get, RESEND_ORIGIN)?),
        other => {
            bail!("NOTIFICATION_DELIVERY={other} is not supported; use `smtp`, `resend` or `log`")
        }
    }
}

/// The sender of one-time codes: `CODE_DELIVERY` for email addresses, and
/// for phone numbers too while `SMS_CODE_DELIVERY` is off (as before SMS
/// was built: `log` writes them to the log, `smtp` and `resend` refuse
/// them); with
/// `SMS_CODE_DELIVERY` on, phone numbers get their codes as it says
/// ([`phone_codes`]). `SMS_DELIVERY` has no part in it: it sends agreement
/// updates only.
fn code_sender(get: Lookup<'_>) -> anyhow::Result<Arc<dyn CodeSender>> {
    let email: Arc<dyn CodeSender> = match required(get, "CODE_DELIVERY")?.as_str() {
        "log" => Arc::new(LogSender),
        "smtp" => smtp_sender(get)?,
        "resend" => resend_sender(get, RESEND_ORIGIN)?,
        other => bail!("CODE_DELIVERY={other} is not supported; use `smtp`, `resend` or `log`"),
    };
    Ok(match phone_codes(get)? {
        None => email,
        Some(phone) => Arc::new(CodeRouter::new(email, phone)),
    })
}

/// How one-time codes reach phone numbers, from `SMS_CODE_DELIVERY`.
/// Default `off`.
///
/// - `off`: as `CODE_DELIVERY` says, with no rule of text messages.
/// - `log`: written to the log, the number masked, and counted, capped and
///   refused after STOP as a text would be. Development only.
/// - `verify`: through Twilio Verify, with the account of the Messages API
///   (`SMS_ACCOUNT_SID` and its credential) and two Verify services:
///   `TWILIO_VERIFY_SERVICE_SID` for signing in and confirming a number,
///   `TWILIO_VERIFY_DELETION_SERVICE_SID` for deleting an account. They must
///   differ: a Verify service sends one number the same code for every
///   request within ten minutes, so one service for both would let a code
///   sent for one purpose work for the other (`notifications::verify`).
fn phone_codes(get: Lookup<'_>) -> anyhow::Result<Option<PhoneCodes>> {
    match optional(get, "SMS_CODE_DELIVERY")
        .as_deref()
        .unwrap_or("off")
    {
        "off" => Ok(None),
        "log" => Ok(Some(PhoneCodes::Log)),
        "verify" => {
            let (account_sid, credential) = twilio_account(get)?;
            let service = |name: &str| -> anyhow::Result<String> {
                let sid = required(get, name)?.trim().to_owned();
                if !is_twilio_sid(&sid, "VA") {
                    bail!(
                        "{name} is not a Verify service SID: VA followed by 32 hexadecimal \
                         digits, as Twilio's console shows it"
                    );
                }
                Ok(sid)
            };
            let services = VerifyServices {
                sign_in: service("TWILIO_VERIFY_SERVICE_SID")?,
                delete_account: service("TWILIO_VERIFY_DELETION_SERVICE_SID")?,
            };
            if services
                .sign_in
                .eq_ignore_ascii_case(&services.delete_account)
            {
                bail!(
                    "TWILIO_VERIFY_SERVICE_SID and TWILIO_VERIFY_DELETION_SERVICE_SID must be two \
                     different Verify services, so that a sign-in code can never confirm a deletion"
                );
            }
            Ok(Some(PhoneCodes::Verify(Arc::new(TwilioVerify::new(
                VERIFY_ORIGIN,
                account_sid,
                credential,
                services,
                SMS_TIMEOUT,
            )))))
        }
        other => {
            bail!("SMS_CODE_DELIVERY={other} is not supported; use `off`, `log` or `verify`")
        }
    }
}

/// How long the SMS provider has to take a message. A person is waiting on
/// the request that sends it.
const SMS_TIMEOUT: Duration = Duration::from_secs(10);

/// The sender of agreement updates by text, named by `SMS_DELIVERY`, if
/// any. Default `off`.
fn sms_sender(get: Lookup<'_>) -> anyhow::Result<Option<Arc<dyn SmsSender>>> {
    match optional(get, "SMS_DELIVERY").as_deref().unwrap_or("off") {
        "off" => Ok(None),
        "log" => Ok(Some(Arc::new(LogSmsSender))),
        "twilio" => {
            let (account_sid, credential) = twilio_account(get)?;
            let from = required(get, "SMS_FROM")?.trim().to_owned();
            let number = matches!(Identifier::parse(&from), Ok(Identifier::Phone(_)));
            if !number && !from.starts_with("MG") {
                bail!(
                    "SMS_FROM={from} is neither a phone number in international form \
                     (+15551234567) nor a Messaging Service SID (MG...)"
                );
            }
            Ok(Some(Arc::new(TwilioSmsSender::new(
                TWILIO_ORIGIN,
                account_sid,
                credential,
                from,
                SMS_TIMEOUT,
            ))))
        }
        other => bail!("SMS_DELIVERY={other} is not supported; use `off`, `log` or `twilio`"),
    }
}

/// The auth token that checks the signature on Twilio's requests to the
/// inbound-message webhook (`POST /v1/sms/inbound`): `SMS_WEBHOOK_AUTH_TOKEN`
/// or, when the service sends with the auth token itself, `SMS_AUTH_TOKEN`.
/// Twilio signs those requests with the account's auth token whatever the
/// service sends with, so with an API key the token must be given here.
/// `None` when there is none, and the webhook then refuses every request.
fn sms_webhook_token(get: Lookup<'_>) -> anyhow::Result<Option<Secret>> {
    if let Some(token) = optional(get, "SMS_WEBHOOK_AUTH_TOKEN") {
        return Ok(Some(Secret::new(token.trim().to_owned())));
    }
    if optional(get, "SMS_DELIVERY").as_deref() != Some("twilio") {
        return Ok(None);
    }
    Ok(match twilio_account(get)?.1 {
        TwilioCredential::AuthToken(token) => Some(token),
        TwilioCredential::ApiKey { .. } => None,
    })
}

/// Whether `value` is a Twilio SID of the kind `prefix` names: the prefix
/// and 32 hexadecimal digits.
fn is_twilio_sid(value: &str, prefix: &str) -> bool {
    value.len() == 34
        && value.starts_with(prefix)
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The Twilio account (`SMS_ACCOUNT_SID`) and exactly one credential for
/// it: an API key (`SMS_API_KEY_SID` and `SMS_API_KEY_SECRET`, recommended)
/// or the account's auth token (`SMS_AUTH_TOKEN`). The secrets are read into
/// a type that cannot be printed, and no message here quotes a value, so a
/// secret pasted into the wrong setting is not written to the log either.
fn twilio_account(get: Lookup<'_>) -> anyhow::Result<(String, TwilioCredential)> {
    let account_sid = required(get, "SMS_ACCOUNT_SID")?.trim().to_owned();
    if !is_twilio_sid(&account_sid, "AC") {
        bail!(
            "SMS_ACCOUNT_SID is not an account SID: AC followed by 32 hexadecimal digits, \
             as Twilio's console shows it"
        );
    }
    let token = optional(get, "SMS_AUTH_TOKEN");
    let key_sid = optional(get, "SMS_API_KEY_SID");
    let key_secret = optional(get, "SMS_API_KEY_SECRET");
    let credential = match (token, key_sid, key_secret) {
        (Some(token), None, None) => {
            TwilioCredential::AuthToken(Secret::new(token.trim().to_owned()))
        }
        (None, Some(sid), Some(secret)) => {
            let sid = sid.trim().to_owned();
            if !is_twilio_sid(&sid, "SK") {
                bail!(
                    "SMS_API_KEY_SID is not an API key SID: SK followed by 32 hexadecimal \
                     digits, as Twilio shows it when the key is created"
                );
            }
            TwilioCredential::ApiKey {
                sid,
                secret: Secret::new(secret.trim().to_owned()),
            }
        }
        (None, None, None) => bail!(
            "Twilio (SMS_DELIVERY=twilio, SMS_CODE_DELIVERY=verify) needs a credential: SMS_API_KEY_SID \
             and SMS_API_KEY_SECRET (recommended), or SMS_AUTH_TOKEN"
        ),
        (Some(_), _, _) => {
            bail!("set either SMS_AUTH_TOKEN or SMS_API_KEY_SID and SMS_API_KEY_SECRET, not both")
        }
        (None, _, _) => bail!("SMS_API_KEY_SID and SMS_API_KEY_SECRET must be set together"),
    };
    Ok((account_sid, credential))
}

/// How push notifications are sent, from `PUSH_DELIVERY`. Default off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushMode {
    Off,
    /// The worker writes each one to its log. Development only.
    Log,
    /// Through Expo's push service.
    Expo,
}

fn push_mode(get: Lookup<'_>) -> anyhow::Result<PushMode> {
    match optional(get, "PUSH_DELIVERY").as_deref().unwrap_or("off") {
        "off" => Ok(PushMode::Off),
        "log" => Ok(PushMode::Log),
        "expo" => Ok(PushMode::Expo),
        other => bail!("PUSH_DELIVERY={other} is not supported; use `off`, `log` or `expo`"),
    }
}

/// How long the push service has to answer, within the outbox's own limit
/// on a send.
const PUSH_TIMEOUT: Duration = Duration::from_secs(20);

/// The push service named by `PUSH_DELIVERY`, with `EXPO_ACCESS_TOKEN` if
/// the Expo project has push security turned on. None when off.
fn push_sender(get: Lookup<'_>) -> anyhow::Result<Option<Arc<dyn PushSender>>> {
    Ok(match push_mode(get)? {
        PushMode::Off => None,
        PushMode::Log => Some(Arc::new(LogPushSender)),
        PushMode::Expo => {
            let token = optional(get, "EXPO_ACCESS_TOKEN")
                .map(|token| Secret::new(token.trim().to_owned()));
            Some(Arc::new(ExpoPushSender::new(
                EXPO_ORIGIN,
                token,
                PUSH_TIMEOUT,
            )))
        }
    })
}

/// Which proxy header to believe, from `TRUSTED_PROXY_HEADER` and
/// `TRUSTED_PROXIES`. The default is none: the connection's peer is the
/// client.
fn trusted_proxies(get: Lookup<'_>) -> anyhow::Result<TrustedProxies> {
    let header = optional(get, "TRUSTED_PROXY_HEADER");
    let count = optional(get, "TRUSTED_PROXIES");
    match (header, count) {
        (None, None) => Ok(TrustedProxies::none()),
        (None, Some(_)) => bail!("TRUSTED_PROXIES is set but TRUSTED_PROXY_HEADER names no header"),
        (Some(header), count) => {
            let name = HeaderName::try_from(header.trim().to_ascii_lowercase())
                .with_context(|| format!("TRUSTED_PROXY_HEADER={header} is not a header name"))?;
            // `Forwarded` (RFC 7239) is written `for=192.0.2.7;proto=https`,
            // which is not read here: every request would fall back to the
            // proxy's address and share one set of limits.
            if name == "forwarded" {
                bail!(
                    "TRUSTED_PROXY_HEADER={header}: the RFC 7239 Forwarded header is not \
                     supported; name a header that holds plain addresses, such as \
                     X-Forwarded-For or the proxy's own (CF-Connecting-IP, Fly-Client-IP)"
                );
            }
            let count: usize = match count {
                None => 1,
                Some(value) => value
                    .trim()
                    .parse()
                    .ok()
                    .filter(|count| *count >= 1)
                    .with_context(|| {
                        format!("TRUSTED_PROXIES={value} is not a count of 1 or more")
                    })?,
            };
            Ok(TrustedProxies::behind(name, count))
        }
    }
}

/// Where to serve metrics, from `METRICS_ADDR`. Default none: no metrics
/// listener at all. Never the public port: a listener of its own, which a
/// deployment exposes only to whatever collects the metrics.
fn metrics_addr(get: Lookup<'_>) -> anyhow::Result<Option<SocketAddr>> {
    optional(get, "METRICS_ADDR")
        .map(|value| {
            value.trim().parse().with_context(|| {
                format!("METRICS_ADDR={value} is not an address and port such as 0.0.0.0:9100")
            })
        })
        .transpose()
}

/// An optional limit: unset or empty means `default`, and anything else must
/// be a whole number of 1 or more. Zero is refused rather than read as "no
/// limit", which would turn a limit off by a slip; a deployment that wants
/// one out of the way sets it high.
fn limit(get: Lookup<'_>, name: &str, default: i64) -> anyhow::Result<i64> {
    match optional(get, name) {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse::<i32>()
            .ok()
            .filter(|limit| *limit >= 1)
            .map(i64::from)
            .with_context(|| {
                format!(
                    "{name}={value} is not a whole number from 1 to {}",
                    i32::MAX
                )
            }),
    }
}

/// The rules for one-time codes, with the limits on sign-in that a
/// deployment may set (`SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR`,
/// `SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY`), and how long a session
/// lasts (`SESSION_IDLE_DAYS`, `SESSION_MAX_DAYS`). Each defaults to the
/// value in [`AuthRules::default`].
///
/// `SIGN_IN_FAILED_GUESSES_PER_ADDRESS_PER_HOUR` was once a setting here.
/// Wrong guesses are no longer limited by address (`crate::auth::verify_code`
/// says why), so a deployment still setting it is refused at start rather
/// than left believing it bounds something.
fn auth_rules(get: Lookup<'_>) -> anyhow::Result<AuthRules> {
    const RETIRED: &str = "SIGN_IN_FAILED_GUESSES_PER_ADDRESS_PER_HOUR";
    if optional(get, RETIRED).is_some() {
        anyhow::bail!(
            "{RETIRED} is no longer a setting: wrong codes are not limited by address. \
             Remove it; see \"Signing in\" in README.md for what bounds guessing"
        );
    }
    let defaults = AuthRules::default();
    let (session_idle, session_max) = session_days(get, &defaults)?;
    Ok(AuthRules {
        code_requests_per_address_per_hour: limit(
            get,
            "SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR",
            defaults.code_requests_per_address_per_hour,
        )?,
        failed_guesses_per_identifier_per_day: limit(
            get,
            "SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY",
            defaults.failed_guesses_per_identifier_per_day,
        )?,
        sms_codes_per_hour: limit(get, "SMS_MAX_PER_HOUR", defaults.sms_codes_per_hour)?,
        sms_codes_per_prefix_per_hour: limit(
            get,
            "SMS_MAX_PER_PREFIX_PER_HOUR",
            defaults.sms_codes_per_prefix_per_hour,
        )?,
        phone_country_codes: country_codes(get)?.unwrap_or(defaults.phone_country_codes),
        phone_regions: regions(get)?.unwrap_or(defaults.phone_regions),
        session_idle,
        session_max,
        ..defaults
    })
}

/// How long a session lasts unused, and at most (`SESSION_IDLE_DAYS`,
/// `SESSION_MAX_DAYS`), in whole days of 1 or more. The most may not be less
/// than the idle time: a deployment that wrote them the wrong way round
/// would otherwise get sessions that never slide.
fn session_days(
    get: Lookup<'_>,
    defaults: &AuthRules,
) -> anyhow::Result<(time::Duration, time::Duration)> {
    let idle = limit(get, "SESSION_IDLE_DAYS", defaults.session_idle.whole_days())?;
    let max = limit(get, "SESSION_MAX_DAYS", defaults.session_max.whole_days())?;
    if max < idle {
        anyhow::bail!("SESSION_MAX_DAYS={max} is less than SESSION_IDLE_DAYS={idle}");
    }
    Ok((time::Duration::days(idle), time::Duration::days(max)))
}

/// `SMS_UPDATES_PER_PERSON_PER_DAY`: how many update texts one person may
/// be queued a day (`notifications::sms_updates`).
fn sms_updates_per_day(get: Lookup<'_>) -> anyhow::Result<i64> {
    limit(
        get,
        "SMS_UPDATES_PER_PERSON_PER_DAY",
        crate::notifications::sms_updates::DEFAULT_TEXTS_PER_PERSON_PER_DAY,
    )
}

/// Whether text messages are sent at all (`SMS_DELIVERY` is not `off`), and
/// so whether agreement updates are, for a process that changes exchanges
/// but sends nothing itself, such as `staff` and `replay-deletions`.
pub fn configure_sms_updates_from_env() -> anyhow::Result<()> {
    load_env();
    let get: Lookup<'_> = &environment;
    let on = !matches!(
        optional(get, "SMS_DELIVERY").as_deref().unwrap_or("off"),
        "off"
    );
    crate::notifications::sms_updates::configure(on, sms_updates_per_day(get)?);
    Ok(())
}

/// `SMS_ALLOWED_COUNTRY_CODES`: country calling codes, `+1,+52`, as digits.
/// `None` when unset, for the default.
fn country_codes(get: Lookup<'_>) -> anyhow::Result<Option<Vec<String>>> {
    const NAME: &str = "SMS_ALLOWED_COUNTRY_CODES";
    let Some(value) = optional(get, NAME) else {
        return Ok(None);
    };
    let codes = value
        .split(',')
        .map(|code| {
            let digits = code.trim().strip_prefix('+')?;
            let valid = (1..=3).contains(&digits.len())
                && digits.bytes().all(|b| b.is_ascii_digit())
                && !digits.starts_with('0');
            valid.then(|| digits.to_owned())
        })
        .collect::<Option<Vec<_>>>()
        .with_context(|| {
            format!(
                "{NAME}={value} is not a comma-separated list of country calling codes \
                 such as +1 or +1,+52"
            )
        })?;
    Ok(Some(codes))
}

/// `SMS_ALLOWED_REGIONS`: the parts of the North American Numbering Plan
/// whose `+1` numbers are taken, `US` or `US,CA` (`crate::nanp`). `None`
/// when unset, for the default, the US alone.
fn regions(get: Lookup<'_>) -> anyhow::Result<Option<Vec<nanp::Region>>> {
    const NAME: &str = "SMS_ALLOWED_REGIONS";
    let Some(value) = optional(get, NAME) else {
        return Ok(None);
    };
    let regions = value
        .split(',')
        .map(nanp::Region::parse)
        .collect::<Option<Vec<_>>>()
        .with_context(|| format!("{NAME}={value} is not a comma-separated list of US and CA"))?;
    Ok(Some(regions))
}

/// The application role's connection, for the api, the worker and
/// `replay-deletions`: `DATABASE_URL`; or else one made of `DATABASE_HOST`,
/// `DATABASE_PORT` (optional) and `DATABASE_NAME`, with `exchange_app` and
/// `APP_DB_PASSWORD`; or else `DATABASE_SERVER_URL` with its user and
/// password replaced by those. The second and third ways are for a platform
/// that gives the database's address but no connection string for any role
/// but the owner (docs/deploy-render.md). The second is preferred, since it
/// puts no owner's password in the process's environment; the third, the
/// older way, is ignored when the second is set, so that a deployment can
/// move from one to the other before removing it. The password is the one
/// `migrate` created the role with (`crate::app_role`).
fn database_url(get: Lookup<'_>) -> anyhow::Result<String> {
    let parts = optional(get, "DATABASE_HOST");
    let server = optional(get, "DATABASE_SERVER_URL");
    match (optional(get, "DATABASE_URL"), parts, server) {
        (Some(_), None, None) | (None, ..) => {}
        (Some(_), ..) => bail!(
            "set DATABASE_URL, or DATABASE_HOST and DATABASE_NAME (with APP_DB_PASSWORD), not both"
        ),
    }
    if let Some(url) = optional(get, "DATABASE_URL") {
        return Ok(url);
    }
    let password = || -> anyhow::Result<AppRolePassword> {
        AppRolePassword::new(
            required(get, "APP_DB_PASSWORD")
                .context("the application role's connection needs APP_DB_PASSWORD")?,
        )
    };
    if let Some(host) = optional(get, "DATABASE_HOST") {
        let host = host.trim();
        let name = required(get, "DATABASE_NAME")
            .context("DATABASE_HOST needs DATABASE_NAME, the database's name")?;
        let port = optional(get, "DATABASE_PORT");
        // A host name or address, a port of digits, a name: nothing that
        // could reshape the URL.
        let plain = |text: &str| {
            !text.is_empty()
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
        };
        if !plain(host) {
            bail!("DATABASE_HOST must be a host name or an IPv4 address");
        }
        if port
            .as_deref()
            .is_some_and(|port| port.parse::<u16>().map_or(true, |port| port == 0))
        {
            bail!("DATABASE_PORT must be a port number");
        }
        if !plain(name.trim()) {
            bail!("DATABASE_NAME must be a database's name");
        }
        let port = port.map(|port| format!(":{port}")).unwrap_or_default();
        return Ok(format!(
            "postgres://{APP_ROLE}:{}@{host}{port}/{}",
            percent_encode(password()?.expose()),
            name.trim()
        ));
    }
    match optional(get, "DATABASE_SERVER_URL") {
        None => {
            bail!("DATABASE_URL is not set (nor DATABASE_HOST, DATABASE_NAME and APP_DB_PASSWORD)")
        }
        Some(server) => {
            let password = password()?;
            // Never quote the URL in an error: it holds the owner's password.
            let (scheme, rest) = server
                .trim()
                .split_once("://")
                .filter(|(scheme, _)| matches!(*scheme, "postgres" | "postgresql"))
                .context("DATABASE_SERVER_URL is not a postgres:// connection string")?;
            // The credentials end at the last `@` before the path.
            let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
            let (authority, path) = rest.split_at(authority_end);
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            if host.is_empty() || !path.starts_with('/') || path.len() < 2 {
                bail!("DATABASE_SERVER_URL must name a host and a database");
            }
            Ok(format!(
                "{scheme}://{APP_ROLE}:{}@{host}{path}",
                percent_encode(password.expose())
            ))
        }
    }
}

/// The application role's connection string from the environment: see
/// [`database_url`]. For `replay-deletions`.
pub fn database_url_from_env() -> anyhow::Result<String> {
    load_env();
    database_url(&environment)
}

/// Every byte but the URL's unreserved characters as `%XX`, so a password
/// or name can sit in a connection string whatever it holds.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// Configuration for the `migrate` binary.
pub struct MigrateConfig {
    /// The schema owner's connection string: `MIGRATION_DATABASE_URL`, or
    /// `DATABASE_URL` in development.
    pub database_url: String,
    /// When `MIGRATE_CREATE_APP_ROLE=true`: create the application role with
    /// this password if it does not exist, before migrating
    /// (`crate::app_role`).
    pub create_app_role: Option<AppRolePassword>,
    /// `CONTACT_DATA_KEY`: `migrate` stores the blind-index key under it the
    /// first time, and checks it against that every time after
    /// (`crate::contact`).
    pub contact: KeyConfig,
}

impl MigrateConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        load_env();
        Self::from_lookup(&environment)
    }

    fn from_lookup(get: Lookup<'_>) -> anyhow::Result<Self> {
        let database_url = optional(get, "MIGRATION_DATABASE_URL")
            .or_else(|| optional(get, "DATABASE_URL"))
            .context("set MIGRATION_DATABASE_URL or DATABASE_URL")?;
        let create_app_role = match optional(get, "MIGRATE_CREATE_APP_ROLE").as_deref() {
            None | Some("false") => None,
            Some("true") => Some(AppRolePassword::new(
                required(get, "APP_DB_PASSWORD")
                    .context("MIGRATE_CREATE_APP_ROLE=true needs the role's password")?,
            )?),
            Some(other) => bail!("MIGRATE_CREATE_APP_ROLE={other} is not `true` or `false`"),
        };
        Ok(Self {
            contact: contact_keys_for(get, Some(&database_url))?,
            database_url,
            create_app_role,
        })
    }
}

/// Configuration for the `worker` process.
pub struct WorkerConfig {
    /// The connection string for the restricted application role.
    pub database_url: String,
    /// Notifications link into the web app.
    pub web_origin: String,
    pub email_sender: Arc<dyn EmailSender>,
    /// The push service, or none while push is off (`PUSH_DELIVERY`).
    pub push_sender: Option<Arc<dyn PushSender>>,
    /// Where to serve the worker's metrics, if anywhere.
    pub metrics_addr: Option<SocketAddr>,
    /// Wallet passes, for the platforms configured (`crate::wallet::config`).
    pub wallet: crate::wallet::WalletConfig,
    /// Agreement updates by text, while `SMS_DELIVERY` is on.
    pub sms: Option<WorkerSms>,
    /// Update texts one person may be queued a day.
    pub sms_updates_per_day: i64,
    /// `CONTACT_DATA_KEY`, which decrypts the addresses and numbers sent to.
    pub contact: KeyConfig,
}

/// What the worker needs to send agreement updates by text.
pub struct WorkerSms {
    pub sender: Arc<dyn SmsSender>,
    /// `APP_SECRET`, which keys the hourly caps' counts as the API keys
    /// them: codes and update texts count against the same caps.
    pub app_secret: Vec<u8>,
    /// The countries texted and the hourly caps.
    pub auth: AuthRules,
}

impl WorkerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        load_env();
        let get: Lookup<'_> = &environment;
        Ok(Self {
            database_url: database_url(get)?,
            web_origin: web_origin(get)?,
            email_sender: email_sender(get)?,
            push_sender: push_sender(get)?,
            metrics_addr: metrics_addr(get)?,
            wallet: crate::wallet::WalletConfig::from_lookup(get)?,
            sms: match sms_sender(get)? {
                Some(sender) => Some(WorkerSms {
                    sender,
                    app_secret: app_secret(get).context(
                        "the worker sends agreement updates by text while SMS_DELIVERY is on, \
                         and counts them under the API's hourly caps with APP_SECRET",
                    )?,
                    auth: auth_rules(get)?,
                }),
                None => None,
            },
            sms_updates_per_day: sms_updates_per_day(get)?,
            contact: contact_keys(get)?,
        })
    }
}

/// `APP_SECRET`: at least 32 bytes.
fn app_secret(get: Lookup<'_>) -> anyhow::Result<Vec<u8>> {
    let secret = required(get, "APP_SECRET")?.into_bytes();
    if secret.len() < 32 {
        bail!("APP_SECRET must be at least 32 bytes");
    }
    Ok(secret)
}

/// `CONTACT_DATA_KEY`, which encrypts every email address and phone number
/// stored (`crate::contact`), and `CONTACT_DATA_KEY_PREVIOUS`, which only
/// decrypts, while what it encrypted is re-encrypted. Each is 32 bytes in
/// base64. Required by every process that reads or writes contact details:
/// there is no state of the database in which they can do without it.
/// Neither is ever written to a log or an error.
fn contact_keys(get: Lookup<'_>) -> anyhow::Result<KeyConfig> {
    contact_keys_for(get, None)
}

/// [`contact_keys`], for a process that also says which database it works
/// on (`migrate`, `contact-data`): the published keys are refused too when
/// that database is not on this machine, whatever `WEB_ORIGIN` says.
fn contact_keys_for(get: Lookup<'_>, database_url: Option<&str>) -> anyhow::Result<KeyConfig> {
    let current = required(get, "CONTACT_DATA_KEY").context(
        "every email address and phone number is stored encrypted under it; generate one \
         with `openssl rand -base64 32` and keep a copy outside the platform \
         (docs/operations.md, \"Contact data key\")",
    )?;
    // The keys published in the repository, for development, the tests and
    // CI, are refused wherever the service is reached from elsewhere, by
    // every process, migrate included.
    let web_origin = optional(get, "WEB_ORIGIN");
    let current = Key::parse("CONTACT_DATA_KEY", &current)?;
    current.refuse_published("CONTACT_DATA_KEY", web_origin.as_deref())?;
    if let Some(url) = database_url {
        current.refuse_published_for_database("CONTACT_DATA_KEY", url)?;
    }
    let previous = optional(get, "CONTACT_DATA_KEY_PREVIOUS")
        .map(|text| Key::parse("CONTACT_DATA_KEY_PREVIOUS", &text))
        .transpose()?;
    if let Some(previous) = &previous {
        previous.refuse_published("CONTACT_DATA_KEY_PREVIOUS", web_origin.as_deref())?;
        if let Some(url) = database_url {
            previous.refuse_published_for_database("CONTACT_DATA_KEY_PREVIOUS", url)?;
        }
    }
    Ok(KeyConfig::new(current, previous)?)
}

/// The contact data keys from the environment: see [`contact_keys`]. For
/// the commands (`staff`, `contact-data`, `replay-deletions`).
pub fn contact_keys_from_env() -> anyhow::Result<KeyConfig> {
    load_env();
    contact_keys(&environment)
}

/// [`contact_keys_from_env`] for a command that works on the database
/// `database_url` names (`contact-data`): the published keys are refused
/// unless it is on this machine.
pub fn contact_keys_for_database(database_url: &str) -> anyhow::Result<KeyConfig> {
    load_env();
    contact_keys_for(&environment, Some(database_url))
}

/// An optional `MIN_CLIENT_VERSION_*` value: unset or empty means no minimum,
/// and anything else must read as a version.
fn min_client_version(get: Lookup<'_>, name: &str) -> anyhow::Result<Option<String>> {
    match optional(get, name) {
        None => Ok(None),
        Some(value) => {
            let value = value.trim();
            if parse_version(value).is_none() {
                bail!("{name}={value} is not a version such as 1.4.0");
            }
            Ok(Some(value.to_owned()))
        }
    }
}

/// The oldest build of each client that may still change anything
/// (`crate::client_version`). Nothing is required unless a deployment says so.
fn min_client_versions(get: Lookup<'_>) -> anyhow::Result<MinimumClientVersions> {
    Ok(MinimumClientVersions {
        web: min_client_version(get, "MIN_CLIENT_VERSION_WEB")?,
        ios: min_client_version(get, "MIN_CLIENT_VERSION_IOS")?,
        android: min_client_version(get, "MIN_CLIENT_VERSION_ANDROID")?,
    })
}

/// A comma-separated setting as its non-empty items, trimmed.
fn list(get: Lookup<'_>, name: &str) -> Vec<String> {
    optional(get, name)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Which apps may open the web origin's invitation links, from
/// `APPLE_APP_ID`, `ANDROID_SHA256_CERT_FINGERPRINTS` and `ANDROID_PACKAGE`
/// (`crate::http::web::AppLinks`). Nothing unless they are set.
fn app_links(get: Lookup<'_>) -> anyhow::Result<AppLinks> {
    let package =
        optional(get, "ANDROID_PACKAGE").unwrap_or_else(|| DEFAULT_ANDROID_PACKAGE.to_owned());
    AppLinks::new(
        &list(get, "APPLE_APP_ID"),
        package.trim(),
        &list(get, "ANDROID_SHA256_CERT_FINGERPRINTS"),
    )
    .map_err(|error| anyhow::anyhow!(error))
    .context("APPLE_APP_ID, ANDROID_SHA256_CERT_FINGERPRINTS or ANDROID_PACKAGE")
}

/// Configuration for the `api` process.
pub struct ApiConfig {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    /// Keys the hashes of one-time codes. At least 32 bytes.
    pub app_secret: Vec<u8>,
    /// Where the web app is served from, such as `https://app.example.com`.
    /// Cookie sessions are only honored for requests from this origin.
    pub web_origin: String,
    pub code_sender: Arc<dyn CodeSender>,
    /// The built web app to serve alongside the API, if any (`WEB_DIR`).
    pub web_dir: Option<PathBuf>,
    pub proxies: TrustedProxies,
    pub min_client_versions: MinimumClientVersions,
    /// Which apps may open the web origin's invitation links, if any.
    pub app_links: AppLinks,
    /// Where to serve the API's metrics, if anywhere. Never `bind_addr`.
    pub metrics_addr: Option<SocketAddr>,
    /// The rules for one-time codes, some of them set by the deployment.
    pub auth: AuthRules,
    /// Whether the worker sends push notifications (`PUSH_DELIVERY`), which
    /// the API tells the apps so they offer them only then.
    pub push_notifications: bool,
    /// Whether agreement updates are sent by text (`SMS_DELIVERY`), and so
    /// whether they can be turned on.
    pub sms: bool,
    /// Whether codes for phone numbers are counted as text messages
    /// (`SMS_CODE_DELIVERY` is not `off`).
    pub sms_codes: bool,
    /// Update texts one person may be queued a day
    /// (`SMS_UPDATES_PER_PERSON_PER_DAY`).
    pub sms_updates_per_day: i64,
    /// The auth token that checks Twilio's signature on the inbound-message
    /// webhook, if there is one ([`sms_webhook_token`]).
    pub sms_webhook_token: Option<Secret>,
    /// Wallet passes, for the platforms configured (`crate::wallet::config`).
    pub wallet: crate::wallet::WalletConfig,
    /// `CONTACT_DATA_KEY`, and `CONTACT_DATA_KEY_PREVIOUS` if set.
    pub contact: KeyConfig,
}

impl ApiConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        load_env();
        let get: Lookup<'_> = &environment;

        let bind_addr: SocketAddr = get("BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8080".to_owned())
            .parse()
            .context("BIND_ADDR is not a valid socket address")?;

        let app_secret = app_secret(get)?;

        let metrics_addr = metrics_addr(get)?;
        if metrics_addr.is_some_and(|metrics| metrics.port() == bind_addr.port()) {
            bail!("METRICS_ADDR must use a port of its own, not BIND_ADDR's");
        }

        Ok(Self {
            metrics_addr,
            database_url: database_url(get)?,
            bind_addr,
            app_secret,
            web_origin: web_origin(get)?,
            code_sender: code_sender(get)?,
            web_dir: optional(get, "WEB_DIR").map(PathBuf::from),
            proxies: trusted_proxies(get)?,
            min_client_versions: min_client_versions(get)?,
            app_links: app_links(get)?,
            auth: auth_rules(get)?,
            push_notifications: push_mode(get)? != PushMode::Off,
            sms: sms_sender(get)?.is_some(),
            sms_codes: phone_codes(get)?.is_some(),
            sms_updates_per_day: sms_updates_per_day(get)?,
            sms_webhook_token: sms_webhook_token(get)?,
            wallet: crate::wallet::WalletConfig::from_lookup(get)?,
            contact: contact_keys(get)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn table(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn lookup(table: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |name| table.get(name).cloned()
    }

    const SMTP: &[(&str, &str)] = &[
        ("SMTP_HOST", "smtp.example.test"),
        ("SMTP_FROM", "Yuppers <no-reply@example.test>"),
    ];

    #[test]
    fn the_application_connection_is_given_whole_or_from_the_server() {
        let whole = table(&[("DATABASE_URL", "postgres://a:b@h:5432/d")]);
        assert_eq!(
            database_url(&lookup(&whole)).unwrap(),
            "postgres://a:b@h:5432/d"
        );

        let server = table(&[
            (
                "DATABASE_SERVER_URL",
                "postgresql://owner:s3cr@t@dpg-abc123-a/yuppers",
            ),
            ("APP_DB_PASSWORD", "p@ss/word+with=odd:chars"),
        ]);
        assert_eq!(
            database_url(&lookup(&server)).unwrap(),
            "postgresql://exchange_app:p%40ss%2Fword%2Bwith%3Dodd%3Achars@dpg-abc123-a/yuppers"
        );
        let mut with_port_and_options = server.clone();
        with_port_and_options.insert(
            "DATABASE_SERVER_URL".to_owned(),
            "postgres://owner:pw@db.example.test:6543/yuppers?sslmode=require".to_owned(),
        );
        assert!(
            database_url(&lookup(&with_port_and_options))
                .unwrap()
                .ends_with("%3Achars@db.example.test:6543/yuppers?sslmode=require")
        );
        let mut no_credentials = server.clone();
        no_credentials.insert(
            "DATABASE_SERVER_URL".to_owned(),
            "postgres://h/d".to_owned(),
        );
        assert!(
            database_url(&lookup(&no_credentials))
                .unwrap()
                .ends_with("%3Achars@h/d")
        );

        let mut both = server.clone();
        both.insert("DATABASE_URL".to_owned(), "postgres://a:b@h/d".to_owned());
        assert!(database_url(&lookup(&both)).is_err(), "never both");

        // From the address's parts, with no owner's credentials anywhere.
        let parts = table(&[
            ("DATABASE_HOST", "dpg-abc123-a"),
            ("DATABASE_PORT", "5432"),
            ("DATABASE_NAME", "yuppers"),
            ("APP_DB_PASSWORD", "p@ss/word+with=odd:chars"),
        ]);
        assert_eq!(
            database_url(&lookup(&parts)).unwrap(),
            "postgres://exchange_app:p%40ss%2Fword%2Bwith%3Dodd%3Achars@dpg-abc123-a:5432/yuppers"
        );
        let mut no_port = parts.clone();
        no_port.remove("DATABASE_PORT");
        assert!(
            database_url(&lookup(&no_port))
                .unwrap()
                .ends_with("@dpg-abc123-a/yuppers")
        );
        // Preferred to DATABASE_SERVER_URL while both are set, on the way
        // from one to the other.
        let mut moving = parts.clone();
        moving.insert(
            "DATABASE_SERVER_URL".to_owned(),
            "postgresql://owner:s3cr@t@elsewhere/other".to_owned(),
        );
        assert_eq!(
            database_url(&lookup(&moving)).unwrap(),
            database_url(&lookup(&parts)).unwrap()
        );
        let mut with_url = parts.clone();
        with_url.insert("DATABASE_URL".to_owned(), "postgres://a:b@h/d".to_owned());
        assert!(database_url(&lookup(&with_url)).is_err(), "never both");
        for (name, bad) in [
            ("DATABASE_HOST", "h/d"),
            ("DATABASE_HOST", "u:p@h"),
            ("DATABASE_HOST", ""),
            ("DATABASE_PORT", "0"),
            ("DATABASE_PORT", "54321x"),
            ("DATABASE_NAME", "d?sslmode=disable"),
        ] {
            let mut wrong = parts.clone();
            wrong.insert(name.to_owned(), bad.to_owned());
            assert!(database_url(&lookup(&wrong)).is_err(), "{name}={bad}");
        }
        let mut no_name = parts.clone();
        no_name.remove("DATABASE_NAME");
        assert!(database_url(&lookup(&no_name)).is_err());
        let mut no_password = parts.clone();
        no_password.remove("APP_DB_PASSWORD");
        assert!(database_url(&lookup(&no_password)).is_err());
        assert!(database_url(&lookup(&table(&[]))).is_err(), "required");
        let mut no_password = server.clone();
        no_password.remove("APP_DB_PASSWORD");
        assert!(database_url(&lookup(&no_password)).is_err());
        let mut short = server.clone();
        short.insert("APP_DB_PASSWORD".to_owned(), "short".to_owned());
        assert!(database_url(&lookup(&short)).is_err(), "a short password");
        for bad in [
            "mysql://u:p@h/d",
            "postgres://u:p@h",
            "postgres://u:p@/d",
            "h/d",
        ] {
            let mut bad_url = server.clone();
            bad_url.insert("DATABASE_SERVER_URL".to_owned(), bad.to_owned());
            let error = database_url(&lookup(&bad_url)).unwrap_err().to_string();
            assert!(!error.contains("u:p"), "never quoted: {error}");
        }
    }

    #[test]
    fn migrate_creates_the_application_role_only_when_asked() {
        let owner = table(&[
            ("MIGRATION_DATABASE_URL", "postgres://o:p@h/d"),
            ("CONTACT_DATA_KEY", CONTACT_KEY),
        ]);
        let config = MigrateConfig::from_lookup(&lookup(&owner)).unwrap();
        assert_eq!(config.database_url, "postgres://o:p@h/d");
        assert!(config.create_app_role.is_none(), "off by default");

        let development = table(&[
            ("DATABASE_URL", "postgres://a:b@h/d"),
            ("CONTACT_DATA_KEY", CONTACT_KEY),
        ]);
        let config = MigrateConfig::from_lookup(&lookup(&development)).unwrap();
        assert_eq!(config.database_url, "postgres://a:b@h/d", "the fallback");
        assert!(MigrateConfig::from_lookup(&lookup(&table(&[]))).is_err());

        let mut asked = owner.clone();
        asked.insert("MIGRATE_CREATE_APP_ROLE".to_owned(), "true".to_owned());
        assert!(
            MigrateConfig::from_lookup(&lookup(&asked)).is_err(),
            "needs APP_DB_PASSWORD"
        );
        asked.insert(
            "APP_DB_PASSWORD".to_owned(),
            "a-long-enough-password".to_owned(),
        );
        let config = MigrateConfig::from_lookup(&lookup(&asked)).unwrap();
        assert_eq!(
            config.create_app_role.unwrap().expose(),
            "a-long-enough-password"
        );

        for value in ["false", ""] {
            let mut off = asked.clone();
            off.insert("MIGRATE_CREATE_APP_ROLE".to_owned(), value.to_owned());
            assert!(
                MigrateConfig::from_lookup(&lookup(&off))
                    .unwrap()
                    .create_app_role
                    .is_none()
            );
        }
        let mut wrong = asked;
        wrong.insert("MIGRATE_CREATE_APP_ROLE".to_owned(), "yes".to_owned());
        assert!(MigrateConfig::from_lookup(&lookup(&wrong)).is_err());
    }

    /// A key for these tests alone.
    const CONTACT_KEY: &str = "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA=";

    #[test]
    fn nothing_starts_without_a_contact_data_key_of_32_bytes_and_none_is_ever_quoted() {
        let base = [("MIGRATION_DATABASE_URL", "postgres://o:p@h/d")];
        let missing = MigrateConfig::from_lookup(&lookup(&table(&base)));
        let error = format!("{:#}", missing.err().expect("refused without a key"));
        assert!(error.contains("CONTACT_DATA_KEY is not set"), "{error}");
        assert!(error.contains("openssl rand -base64 32"), "{error}");

        for bad in [
            "not base64 at all!",
            // 16 bytes, and 33.
            "AAECAwQFBgcICQoLDA0ODw==",
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g",
            // Hex, as `openssl rand -hex 32` writes it: 48 bytes once read as base64.
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        ] {
            let malformed = table(&[base[0], ("CONTACT_DATA_KEY", bad)]);
            let error = match MigrateConfig::from_lookup(&lookup(&malformed)) {
                Ok(_) => panic!("{bad} was taken as a key"),
                Err(error) => format!("{error:#}"),
            };
            assert!(error.starts_with("CONTACT_DATA_KEY is not"), "{error}");
            assert!(!error.contains(bad), "never quoted: {error}");
        }

        let given = table(&[base[0], ("CONTACT_DATA_KEY", CONTACT_KEY)]);
        let config = MigrateConfig::from_lookup(&lookup(&given)).unwrap();
        assert!(config.contact.previous.is_none());

        let mut rotating = given.clone();
        rotating.insert(
            "CONTACT_DATA_KEY_PREVIOUS".to_owned(),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=".to_owned(),
        );
        let config = MigrateConfig::from_lookup(&lookup(&rotating)).unwrap();
        assert!(config.contact.previous.is_some());
        rotating.insert("CONTACT_DATA_KEY_PREVIOUS".to_owned(), "short".to_owned());
        assert!(MigrateConfig::from_lookup(&lookup(&rotating)).is_err());
        rotating.insert(
            "CONTACT_DATA_KEY_PREVIOUS".to_owned(),
            CONTACT_KEY.to_owned(),
        );
        let error = MigrateConfig::from_lookup(&lookup(&rotating))
            .err()
            .expect("the same key twice is refused")
            .to_string();
        assert!(error.contains("the same as CONTACT_DATA_KEY"), "{error}");
    }

    #[test]
    fn a_published_contact_data_key_is_refused_unless_the_origin_is_this_machine() {
        use crate::contact::PUBLISHED_KEYS;
        for published in PUBLISHED_KEYS {
            let mut settings = table(&[
                ("MIGRATION_DATABASE_URL", "postgres://o:p@h/d"),
                ("CONTACT_DATA_KEY", published),
            ]);
            // Migrate refuses it for a database elsewhere, without WEB_ORIGIN.
            let error = format!(
                "{:#}",
                MigrateConfig::from_lookup(&lookup(&settings))
                    .err()
                    .expect("refused")
            );
            assert!(
                error.contains("the database is not on this machine"),
                "{error}"
            );
            assert!(!error.contains(published), "never quoted: {error}");
            settings.insert(
                "MIGRATION_DATABASE_URL".to_owned(),
                "postgres://o:p@127.0.0.1:5432/d".to_owned(),
            );
            // Without WEB_ORIGIN, as a developer runs migrate, and on this
            // machine.
            assert!(MigrateConfig::from_lookup(&lookup(&settings)).is_ok());
            settings.insert("WEB_ORIGIN".to_owned(), "http://localhost:8080".to_owned());
            assert!(MigrateConfig::from_lookup(&lookup(&settings)).is_ok());
            // Anywhere else, every process refuses it, migrate included.
            settings.insert("WEB_ORIGIN".to_owned(), "https://yuppers.app".to_owned());
            let error = format!(
                "{:#}",
                MigrateConfig::from_lookup(&lookup(&settings))
                    .err()
                    .expect("refused")
            );
            assert!(error.contains("published in the repository"), "{error}");
            assert!(!error.contains(published), "never quoted: {error}");
            // As the previous key too.
            settings.insert("CONTACT_DATA_KEY".to_owned(), CONTACT_KEY.to_owned());
            settings.insert("CONTACT_DATA_KEY_PREVIOUS".to_owned(), published.to_owned());
            assert!(MigrateConfig::from_lookup(&lookup(&settings)).is_err());
        }
    }

    #[test]
    fn a_delivery_must_be_named_and_must_be_one_that_exists() {
        let none = table(&[]);
        assert!(email_sender(&lookup(&none)).is_err(), "no default");
        assert!(code_sender(&lookup(&none)).is_err(), "no default");
        for other in ["", "LOG", "ses", "sms"] {
            let table = table(&[("NOTIFICATION_DELIVERY", other), ("CODE_DELIVERY", other)]);
            assert!(email_sender(&lookup(&table)).is_err(), "{other:?}");
            assert!(code_sender(&lookup(&table)).is_err(), "{other:?}");
        }
        let log = table(&[("NOTIFICATION_DELIVERY", "log"), ("CODE_DELIVERY", "log")]);
        assert!(email_sender(&lookup(&log)).is_ok());
        assert!(code_sender(&lookup(&log)).is_ok());
    }

    #[test]
    fn push_is_off_unless_named_and_must_be_a_delivery_that_exists() {
        let read = |pairs: &[(&str, &str)]| push_mode(&lookup(&table(pairs)));
        assert_eq!(read(&[]).unwrap(), PushMode::Off);
        assert_eq!(read(&[("PUSH_DELIVERY", " ")]).unwrap(), PushMode::Off);
        assert_eq!(read(&[("PUSH_DELIVERY", "off")]).unwrap(), PushMode::Off);
        assert_eq!(read(&[("PUSH_DELIVERY", "log")]).unwrap(), PushMode::Log);
        assert_eq!(read(&[("PUSH_DELIVERY", "expo")]).unwrap(), PushMode::Expo);
        for wrong in ["on", "apns", "fcm", "EXPO"] {
            assert!(read(&[("PUSH_DELIVERY", wrong)]).is_err(), "{wrong}");
        }
        assert!(push_sender(&lookup(&table(&[]))).unwrap().is_none());
        // The access token is optional: only an Expo project with push
        // security turned on asks for it.
        for pairs in [
            vec![("PUSH_DELIVERY", "expo")],
            vec![("PUSH_DELIVERY", "expo"), ("EXPO_ACCESS_TOKEN", "token")],
        ] {
            assert!(push_sender(&lookup(&table(&pairs))).unwrap().is_some());
        }
    }

    #[test]
    fn sms_is_off_unless_named_and_twilio_needs_its_three_settings() {
        let read = |pairs: &[(&str, &str)]| sms_sender(&lookup(&table(pairs)));
        assert!(read(&[]).unwrap().is_none());
        assert!(read(&[("SMS_DELIVERY", "off")]).unwrap().is_none());
        assert!(read(&[("SMS_DELIVERY", "log")]).unwrap().is_some());
        for wrong in ["on", "sns", "TWILIO", "smtp"] {
            assert!(read(&[("SMS_DELIVERY", wrong)]).is_err(), "{wrong}");
        }

        let twilio = [
            ("SMS_DELIVERY", "twilio"),
            ("SMS_ACCOUNT_SID", ACCOUNT_SID),
            ("SMS_AUTH_TOKEN", "hunter2-sms"),
            ("SMS_FROM", "+15550000000"),
        ];
        assert!(read(&twilio).unwrap().is_some());
        for missing in 1..twilio.len() {
            let mut pairs = twilio.to_vec();
            pairs.remove(missing);
            let error = read(&pairs).err().expect("refused");
            assert!(error.to_string().contains(twilio[missing].0), "{error}");
        }
        let service = [&twilio[..3], &[("SMS_FROM", "MG0123")]].concat();
        assert!(read(&service).unwrap().is_some());
        let wrong = [&twilio[..3], &[("SMS_FROM", "Yuppers")]].concat();
        let error = format!("{:#}", read(&wrong).err().expect("refused"));
        assert!(
            error.contains("SMS_FROM") && !error.contains("hunter2-sms"),
            "{error}"
        );
    }

    /// Obviously fake SIDs of the right shape.
    const ACCOUNT_SID: &str = "AC00000000000000000000000000000000";
    const KEY_SID: &str = "SK00000000000000000000000000000000";

    #[test]
    fn twilio_takes_exactly_one_credential_an_api_key_or_the_auth_token() {
        let read = |pairs: &[(&str, &str)]| {
            let pairs = [&[("SMS_ACCOUNT_SID", ACCOUNT_SID)][..], pairs].concat();
            twilio_account(&lookup(&table(&pairs)))
        };
        let refused = |pairs: &[(&str, &str)], says: &str| {
            let error = format!("{:#}", read(pairs).expect_err("refused"));
            assert!(error.contains(says), "{says} in {error}");
            for secret in ["hunter2-token", "hunter2-key"] {
                assert!(!error.contains(secret), "{error}");
            }
        };
        let token = ("SMS_AUTH_TOKEN", "hunter2-token");
        let key_sid = ("SMS_API_KEY_SID", KEY_SID);
        let key_secret = ("SMS_API_KEY_SECRET", "hunter2-key");

        // The API key.
        let (account, credential) = read(&[key_sid, key_secret]).unwrap();
        assert_eq!(account, ACCOUNT_SID);
        assert_eq!(
            credential,
            TwilioCredential::ApiKey {
                sid: KEY_SID.to_owned(),
                secret: Secret::new("hunter2-key".to_owned()),
            }
        );
        assert!(!format!("{credential:?}").contains("hunter2-key"));
        // The auth token.
        let (_, credential) = read(&[token]).unwrap();
        assert_eq!(
            credential,
            TwilioCredential::AuthToken(Secret::new("hunter2-token".to_owned()))
        );
        assert!(!format!("{credential:?}").contains("hunter2-token"));
        // A setting left empty in .env counts as unset.
        assert!(read(&[("SMS_AUTH_TOKEN", ""), key_sid, key_secret]).is_ok());

        // Neither, both, or half a key.
        refused(&[], "needs a credential");
        refused(&[token, key_sid, key_secret], "not both");
        refused(&[token, key_sid], "not both");
        refused(&[token, key_secret], "not both");
        refused(&[key_sid], "must be set together");
        refused(&[key_secret], "must be set together");

        // SIDs of the wrong shape, including each in the other's place.
        let short = "SK0000000000000000000000000000000";
        let not_hex = "SK0000000000000000000000000000000g";
        for wrong in [ACCOUNT_SID, short, not_hex, "hunter2-key"] {
            refused(
                &[("SMS_API_KEY_SID", wrong), key_secret],
                "SMS_API_KEY_SID is not an API key SID",
            );
        }
        for wrong in [
            KEY_SID,
            "AC0123",
            "AC0000000000000000000000000000000x",
            "hunter2-token",
        ] {
            let pairs = table(&[("SMS_ACCOUNT_SID", wrong), token]);
            let error = format!("{:#}", twilio_account(&lookup(&pairs)).unwrap_err());
            assert!(
                error.contains("SMS_ACCOUNT_SID is not an account SID"),
                "{error}"
            );
            assert!(!error.contains("hunter2-token"), "{error}");
        }
        assert!(
            twilio_account(&lookup(&table(&[token])))
                .unwrap_err()
                .to_string()
                .contains("SMS_ACCOUNT_SID is not set")
        );
        // Upper-case hexadecimal digits are as good.
        let upper = "ACABCDEF0123456789ABCDEF0123456789";
        let pairs = table(&[("SMS_ACCOUNT_SID", upper), token]);
        assert_eq!(twilio_account(&lookup(&pairs)).unwrap().0, upper);
    }

    #[test]
    fn the_webhook_is_checked_with_the_auth_token_given_for_it_or_sent_with() {
        let read = |pairs: &[(&str, &str)]| {
            sms_webhook_token(&lookup(&table(pairs)))
                .unwrap()
                .map(|token| token.expose().to_owned())
        };
        let twilio = [
            ("SMS_DELIVERY", "twilio"),
            ("SMS_ACCOUNT_SID", ACCOUNT_SID),
            ("SMS_FROM", "+15550000000"),
        ];
        let with_token = [&twilio[..], &[("SMS_AUTH_TOKEN", "hunter2-token")]].concat();
        assert_eq!(read(&with_token).as_deref(), Some("hunter2-token"));
        let with_key = [
            &twilio[..],
            &[
                ("SMS_API_KEY_SID", KEY_SID),
                ("SMS_API_KEY_SECRET", "hunter2-key"),
            ],
        ]
        .concat();
        // An API key's secret cannot check Twilio's signature.
        assert_eq!(read(&with_key), None);
        let named = [
            &with_key[..],
            &[("SMS_WEBHOOK_AUTH_TOKEN", " hunter2-webhook ")],
        ]
        .concat();
        assert_eq!(read(&named).as_deref(), Some("hunter2-webhook"));
        assert_eq!(read(&[("SMS_DELIVERY", "log")]), None);
        assert!(!format!("{:?}", sms_webhook_token(&lookup(&table(&named)))).contains("hunter2"));
    }

    #[test]
    fn sms_delivery_by_twilio_starts_with_an_api_key() {
        let pairs = table(&[
            ("SMS_DELIVERY", "twilio"),
            ("SMS_ACCOUNT_SID", ACCOUNT_SID),
            ("SMS_API_KEY_SID", KEY_SID),
            ("SMS_API_KEY_SECRET", "hunter2-key"),
            ("SMS_FROM", "+15550000000"),
        ]);
        assert!(sms_sender(&lookup(&pairs)).unwrap().is_some());
    }

    const SIGN_IN_SERVICE: &str = "VA00000000000000000000000000000001";
    const DELETION_SERVICE: &str = "VA00000000000000000000000000000002";

    /// `SMS_CODE_DELIVERY=verify` with everything it needs.
    const VERIFY: &[(&str, &str)] = &[
        ("SMS_CODE_DELIVERY", "verify"),
        ("SMS_ACCOUNT_SID", ACCOUNT_SID),
        ("SMS_API_KEY_SID", KEY_SID),
        ("SMS_API_KEY_SECRET", "hunter2-verify"),
        ("TWILIO_VERIFY_SERVICE_SID", SIGN_IN_SERVICE),
        ("TWILIO_VERIFY_DELETION_SERVICE_SID", DELETION_SERVICE),
    ];

    /// `SMS_DELIVERY=twilio` with everything it needs.
    const TWILIO: &[(&str, &str)] = &[
        ("SMS_DELIVERY", "twilio"),
        ("SMS_ACCOUNT_SID", ACCOUNT_SID),
        ("SMS_API_KEY_SID", KEY_SID),
        ("SMS_API_KEY_SECRET", "hunter2-sms"),
        ("SMS_FROM", "+15550000000"),
    ];

    #[test]
    fn codes_for_phone_numbers_go_as_sms_code_delivery_says_and_never_by_sms_delivery() {
        let phone = Identifier::parse("+15551234567").unwrap();
        let email = Identifier::parse("ana@example.test").unwrap();
        let sender = |pairs: &[(&str, &str)]| code_sender(&lookup(&table(pairs))).unwrap();
        // Off: codes go as before, and none is charged as a text message,
        // whatever SMS_DELIVERY says: it sends agreement updates only.
        for updates in [&[][..], &[("SMS_DELIVERY", "log")][..], TWILIO] {
            let off = sender(&[&[("CODE_DELIVERY", "log")][..], updates].concat());
            assert!(!off.charged_per_message(&phone));
            assert!(off.verifier(&phone).is_none());
        }
        // The development log: counted as a text, made by the service.
        let log = sender(&[("CODE_DELIVERY", "log"), ("SMS_CODE_DELIVERY", "log")]);
        assert!(log.charged_per_message(&phone));
        assert!(!log.charged_per_message(&email));
        assert!(log.verifier(&phone).is_none());
        // Verify: counted as a text, made and checked by Twilio, for phone
        // numbers only; and so with agreement updates on as well.
        for updates in [&[][..], TWILIO] {
            let verify = sender(&[&[("CODE_DELIVERY", "log")][..], VERIFY, updates].concat());
            assert!(verify.charged_per_message(&phone));
            assert!(verify.verifier(&phone).is_some());
            assert!(verify.verifier(&email).is_none());
            assert!(!verify.charged_per_message(&email));
        }
    }

    #[test]
    fn verify_needs_the_account_a_credential_and_two_different_services() {
        let read = |pairs: &[(&str, &str)]| phone_codes(&lookup(&table(pairs)));
        assert!(read(&[]).unwrap().is_none());
        assert!(read(&[("SMS_CODE_DELIVERY", "off")]).unwrap().is_none());
        assert!(matches!(
            read(&[("SMS_CODE_DELIVERY", "log")]).unwrap(),
            Some(PhoneCodes::Log)
        ));
        assert!(matches!(read(VERIFY).unwrap(), Some(PhoneCodes::Verify(_))));
        for wrong in ["on", "twilio", "VERIFY", "sms"] {
            let error = format!("{:#}", read(&[("SMS_CODE_DELIVERY", wrong)]).err().unwrap());
            assert!(error.contains("SMS_CODE_DELIVERY"), "{error}");
        }
        let refused = |pairs: Vec<(&str, &str)>, says: &str| {
            let error = format!("{:#}", read(&pairs).err().expect("refused"));
            assert!(error.contains(says), "{says} in {error}");
            assert!(!error.contains("hunter2"), "{error}");
        };
        // Each setting is needed.
        for missing in 1..VERIFY.len() {
            let mut pairs = VERIFY.to_vec();
            let (name, _) = pairs.remove(missing);
            let says = if name.starts_with("SMS_API_KEY") {
                "SMS_API_KEY_SID and SMS_API_KEY_SECRET"
            } else {
                name
            };
            refused(pairs, says);
        }
        // The auth token will do in place of the key.
        let with_token: Vec<(&str, &str)> = VERIFY
            .iter()
            .copied()
            .filter(|(name, _)| !name.starts_with("SMS_API_KEY"))
            .chain([("SMS_AUTH_TOKEN", "hunter2-token")])
            .collect();
        assert!(read(&with_token).unwrap().is_some());
        // A service SID is VA and 32 hexadecimal digits.
        for wrong in ["VA123", "MG00000000000000000000000000000001", "Yuppers.app"] {
            let mut pairs = VERIFY.to_vec();
            pairs[4].1 = wrong;
            refused(
                pairs,
                "TWILIO_VERIFY_SERVICE_SID is not a Verify service SID",
            );
        }
        // One service for both purposes would let a sign-in code confirm a
        // deletion.
        let mut pairs = VERIFY.to_vec();
        pairs[5].1 = SIGN_IN_SERVICE;
        refused(pairs, "must be two different Verify services");
    }

    #[test]
    fn the_channels_offered_are_the_ones_the_deployment_can_deliver() {
        use crate::auth::{SignInChannel::*, sign_in_channels};
        let channels = |codes: &str, sms: &[(&str, &str)]| {
            let pairs = [SMTP, &[("CODE_DELIVERY", codes)], sms].concat();
            sign_in_channels(code_sender(&lookup(&table(&pairs))).unwrap().as_ref())
        };
        for sms in [
            &[][..],
            &[("SMS_CODE_DELIVERY", "off")],
            // Agreement updates send no codes.
            &[("SMS_DELIVERY", "log")],
            TWILIO,
        ] {
            // Codes by email, and nothing to send a code by text with: what
            // a deployment has before it buys Verify.
            assert_eq!(channels("smtp", sms), [Email]);
            // The development log takes phone codes too.
            assert_eq!(channels("log", sms), [Email, Phone]);
        }
        for sms in [&[("SMS_CODE_DELIVERY", "log")][..], VERIFY] {
            assert_eq!(channels("smtp", sms), [Email, Phone]);
            assert_eq!(channels("log", sms), [Email, Phone]);
        }
    }

    #[test]
    fn phone_numbers_are_taken_from_the_us_and_the_rest_of_nanp_unless_set() {
        let read = |pairs: &[(&str, &str)]| auth_rules(&lookup(&table(pairs)));
        assert_eq!(read(&[]).unwrap().phone_country_codes, ["1"]);
        // Of +1, the US alone, unless Canada is added.
        assert_eq!(read(&[]).unwrap().phone_regions, [nanp::Region::Us]);
        assert_eq!(
            read(&[("SMS_ALLOWED_REGIONS", "US, CA")])
                .unwrap()
                .phone_regions,
            [nanp::Region::Us, nanp::Region::Canada]
        );
        for wrong in ["MX", "US,", "US;CA", "+1"] {
            assert!(read(&[("SMS_ALLOWED_REGIONS", wrong)]).is_err(), "{wrong}");
        }
        assert_eq!(
            read(&[("SMS_ALLOWED_COUNTRY_CODES", " +1, +52 ")])
                .unwrap()
                .phone_country_codes,
            ["1", "52"]
        );
        for wrong in ["1", "+", "+0", "+1234", "+1,", "+1;+52", "+1,52", "+a"] {
            assert!(
                read(&[("SMS_ALLOWED_COUNTRY_CODES", wrong)]).is_err(),
                "{wrong}"
            );
        }
        assert_eq!(read(&[]).unwrap().sms_codes_per_prefix_per_hour, 10);
        assert_eq!(
            read(&[("SMS_MAX_PER_PREFIX_PER_HOUR", "3")])
                .unwrap()
                .sms_codes_per_prefix_per_hour,
            3
        );
        assert!(read(&[("SMS_MAX_PER_PREFIX_PER_HOUR", "0")]).is_err());
    }

    #[test]
    fn sessions_last_30_days_unused_and_180_at_most_unless_set() {
        let read = |pairs: &[(&str, &str)]| auth_rules(&lookup(&table(pairs)));
        let rules = read(&[]).unwrap();
        assert_eq!(rules.session_idle, time::Duration::days(30));
        assert_eq!(rules.session_max, time::Duration::days(180));
        let rules = read(&[("SESSION_IDLE_DAYS", "7"), ("SESSION_MAX_DAYS", "7")]).unwrap();
        assert_eq!(rules.session_idle, time::Duration::days(7));
        assert_eq!(rules.session_max, time::Duration::days(7));
        for wrong in ["0", "-1", "a month"] {
            assert!(read(&[("SESSION_IDLE_DAYS", wrong)]).is_err(), "{wrong}");
            assert!(read(&[("SESSION_MAX_DAYS", wrong)]).is_err(), "{wrong}");
        }
        // The most a session lasts can't be less than how long it lasts unused.
        assert!(read(&[("SESSION_IDLE_DAYS", "60"), ("SESSION_MAX_DAYS", "30")]).is_err());
        assert!(read(&[("SESSION_MAX_DAYS", "29")]).is_err());
    }

    #[test]
    fn the_sms_cap_defaults_to_the_placeholder_and_is_a_count_of_one_or_more() {
        let read = |pairs: &[(&str, &str)]| auth_rules(&lookup(&table(pairs)));
        assert_eq!(
            read(&[]).unwrap().sms_codes_per_hour,
            AuthRules::default().sms_codes_per_hour
        );
        assert_eq!(
            read(&[("SMS_MAX_PER_HOUR", "200")])
                .unwrap()
                .sms_codes_per_hour,
            200
        );
        for wrong in ["0", "-1", "lots"] {
            assert!(read(&[("SMS_MAX_PER_HOUR", wrong)]).is_err(), "{wrong}");
        }
    }

    #[test]
    fn smtp_delivery_needs_a_host_and_a_from_address() {
        let mut pairs = vec![("NOTIFICATION_DELIVERY", "smtp"), ("CODE_DELIVERY", "smtp")];
        let bare = table(&pairs);
        assert!(email_sender(&lookup(&bare)).is_err());
        assert!(code_sender(&lookup(&bare)).is_err());
        pairs.extend_from_slice(SMTP);
        let whole = table(&pairs);
        assert!(email_sender(&lookup(&whole)).is_ok());
        assert!(code_sender(&lookup(&whole)).is_ok());
    }

    #[test]
    fn resend_delivery_needs_its_key_and_a_from_address_and_never_quotes_the_key() {
        const KEY: &str = "re_not_a_real_key_for_tests";
        let read = |pairs: &[(&str, &str)]| {
            let both = [
                &[
                    ("NOTIFICATION_DELIVERY", "resend"),
                    ("CODE_DELIVERY", "resend"),
                ][..],
                pairs,
            ]
            .concat();
            let table = table(&both);
            let email = email_sender(&lookup(&table)).map(|_| ());
            let code = code_sender(&lookup(&table)).map(|sender| sender.email_sender());
            (email, code)
        };

        // No key: refused, whatever else is there, and both processes say so.
        for pairs in [
            &[("EMAIL_FROM", "Yuppers <no-reply@example.test>")][..],
            &[
                ("EMAIL_FROM", "Yuppers <no-reply@example.test>"),
                ("RESEND_API_KEY", "  "),
            ],
        ] {
            let (email, code) = read(pairs);
            for error in [email.err().unwrap(), code.err().unwrap()] {
                assert!(format!("{error:#}").contains("RESEND_API_KEY"), "{error:#}");
            }
        }
        // No sender address.
        let (email, code) = read(&[("RESEND_API_KEY", KEY)]);
        for error in [email.err().unwrap(), code.err().unwrap()] {
            let error = format!("{error:#}");
            assert!(error.contains("EMAIL_FROM"), "{error}");
            assert!(!error.contains(KEY), "{error}");
        }
        // A sender address that is not one: the error quotes it, not the key.
        let (email, _) = read(&[("RESEND_API_KEY", KEY), ("EMAIL_FROM", "nobody")]);
        let error = format!("{:#}", email.err().unwrap());
        assert!(error.contains("nobody") && !error.contains(KEY), "{error}");

        // EMAIL_FROM, or SMTP_FROM where it is all there is, and EMAIL_FROM
        // where both are.
        for (pairs, want) in [
            (
                &[("EMAIL_FROM", "Yuppers <no-reply@yuppers.example>")][..],
                "no-reply@yuppers.example",
            ),
            (
                &[("SMTP_FROM", "Yuppers <old@yuppers.example>")],
                "old@yuppers.example",
            ),
            (
                &[
                    ("EMAIL_FROM", "new@yuppers.example"),
                    ("SMTP_FROM", "old@yuppers.example"),
                ],
                "new@yuppers.example",
            ),
        ] {
            let (email, code) = read(&[pairs, &[("RESEND_API_KEY", KEY)]].concat());
            assert!(email.is_ok());
            assert_eq!(code.unwrap().as_deref(), Some(want));
        }
    }

    #[test]
    fn smtp_sends_from_email_from_or_else_smtp_from() {
        let from = |pairs: &[(&str, &str)]| {
            let table = table(&[&[("SMTP_HOST", "smtp.example.test")][..], pairs].concat());
            smtp_settings(&lookup(&table)).map(|settings| settings.from)
        };
        assert_eq!(
            from(&[("SMTP_FROM", "a@example.test")]).unwrap(),
            "a@example.test"
        );
        assert_eq!(
            from(&[("EMAIL_FROM", "b@example.test")]).unwrap(),
            "b@example.test"
        );
        assert_eq!(
            from(&[
                ("EMAIL_FROM", "b@example.test"),
                ("SMTP_FROM", "a@example.test")
            ])
            .unwrap(),
            "b@example.test"
        );
        assert!(from(&[]).is_err());
    }

    #[test]
    fn the_tls_mode_picks_the_port_unless_one_is_given() {
        let settings = |extra: &[(&str, &str)]| {
            let table = table(&[SMTP, extra].concat());
            smtp_settings(&lookup(&table))
        };
        let default = settings(&[]).unwrap();
        assert_eq!((default.tls, default.port), (TlsMode::Tls, 465));
        let starttls = settings(&[("SMTP_TLS", "starttls")]).unwrap();
        assert_eq!((starttls.tls, starttls.port), (TlsMode::StartTls, 587));
        let local = settings(&[
            ("SMTP_TLS", "none"),
            ("SMTP_PORT", "2525"),
            ("SMTP_HOST", "localhost"),
        ])
        .unwrap();
        assert_eq!((local.tls, local.port), (TlsMode::None, 2525));
        assert!(settings(&[("SMTP_TLS", "ssl")]).is_err());
        assert!(settings(&[("SMTP_PORT", "smtp")]).is_err());
    }

    #[test]
    fn plain_text_smtp_is_for_this_host_only_without_a_password() {
        let settings = |extra: &[(&str, &str)]| {
            let table = table(&[SMTP, &[("SMTP_TLS", "none")], extra].concat());
            smtp_settings(&lookup(&table))
        };
        for host in [
            "localhost",
            "LOCALHOST.",
            "127.0.0.1",
            "127.8.9.10",
            "::1",
            "[::1]",
        ] {
            assert!(settings(&[("SMTP_HOST", host)]).is_ok(), "{host}");
        }
        for host in [
            "smtp.example.test",
            "10.0.0.5",
            "::2",
            "localhost.example.test",
        ] {
            assert!(settings(&[("SMTP_HOST", host)]).is_err(), "{host}");
        }
        let password = [
            ("SMTP_HOST", "127.0.0.1"),
            ("SMTP_USERNAME", "u"),
            ("SMTP_PASSWORD", "hunter2"),
        ];
        let refused = settings(&password).unwrap_err();
        assert!(!format!("{refused:#}").contains("hunter2"));

        // Said outright, for a test server.
        let allow = ("SMTP_ALLOW_PLAINTEXT_REMOTE", "true");
        assert!(settings(&[allow]).is_ok());
        assert!(settings(&[&password[..], &[allow]].concat()).is_ok());
        assert!(settings(&[("SMTP_ALLOW_PLAINTEXT_REMOTE", "false")]).is_err());
        assert!(settings(&[("SMTP_ALLOW_PLAINTEXT_REMOTE", "yes")]).is_err());

        // Encrypted connections are not affected.
        let table = table(&[SMTP, &password[1..]].concat());
        assert!(smtp_settings(&lookup(&table)).is_ok());
    }

    #[test]
    fn smtp_credentials_come_as_a_pair_and_the_password_is_not_printable() {
        let settings = |extra: &[(&str, &str)]| {
            let table = table(&[SMTP, extra].concat());
            smtp_settings(&lookup(&table))
        };
        assert_eq!(settings(&[]).unwrap().credentials, None);
        assert!(settings(&[("SMTP_USERNAME", "u")]).is_err());
        assert!(settings(&[("SMTP_PASSWORD", "p")]).is_err());
        let both = settings(&[("SMTP_USERNAME", "u"), ("SMTP_PASSWORD", "hunter2")]).unwrap();
        assert_eq!(
            both.credentials,
            Some(("u".to_owned(), Secret::new("hunter2".to_owned())))
        );
        assert!(!format!("{both:?}").contains("hunter2"));
    }

    #[test]
    fn no_proxy_header_is_trusted_unless_one_is_named() {
        let none = table(&[]);
        assert_eq!(
            trusted_proxies(&lookup(&none)).unwrap(),
            TrustedProxies::none()
        );
        let blank = table(&[("TRUSTED_PROXY_HEADER", ""), ("TRUSTED_PROXIES", "")]);
        assert_eq!(
            trusted_proxies(&lookup(&blank)).unwrap(),
            TrustedProxies::none()
        );

        let one = table(&[("TRUSTED_PROXY_HEADER", "X-Forwarded-For")]);
        assert_eq!(
            trusted_proxies(&lookup(&one)).unwrap(),
            TrustedProxies::behind(HeaderName::from_static("x-forwarded-for"), 1)
        );
        let two = table(&[
            ("TRUSTED_PROXY_HEADER", "X-Forwarded-For"),
            ("TRUSTED_PROXIES", "2"),
        ]);
        assert_eq!(
            trusted_proxies(&lookup(&two)).unwrap(),
            TrustedProxies::behind(HeaderName::from_static("x-forwarded-for"), 2)
        );

        for wrong in [
            table(&[("TRUSTED_PROXIES", "2")]),
            table(&[
                ("TRUSTED_PROXY_HEADER", "X-Forwarded-For"),
                ("TRUSTED_PROXIES", "0"),
            ]),
            table(&[
                ("TRUSTED_PROXY_HEADER", "X-Forwarded-For"),
                ("TRUSTED_PROXIES", "two"),
            ]),
            table(&[("TRUSTED_PROXY_HEADER", "not a header")]),
            table(&[("TRUSTED_PROXY_HEADER", "Forwarded")]),
            table(&[("TRUSTED_PROXY_HEADER", " forwarded ")]),
        ] {
            assert!(trusted_proxies(&lookup(&wrong)).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn metrics_are_off_unless_an_address_is_given() {
        let read = |value: &str| metrics_addr(&lookup(&table(&[("METRICS_ADDR", value)])));
        assert_eq!(metrics_addr(&lookup(&table(&[]))).unwrap(), None);
        assert_eq!(read(" ").unwrap(), None);
        assert_eq!(
            read("0.0.0.0:9100").unwrap(),
            Some("0.0.0.0:9100".parse().unwrap())
        );
        assert!(read("9100").is_err());
        assert!(read("localhost:9100").is_err());
    }

    #[test]
    fn the_sign_in_limits_default_to_the_placeholders_and_may_be_raised_or_lowered() {
        let defaults = AuthRules::default();
        for unset in [
            table(&[]),
            table(&[
                ("SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR", ""),
                ("SIGN_IN_FAILED_GUESSES_PER_ADDRESS_PER_HOUR", " "),
                ("SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY", ""),
            ]),
        ] {
            let rules = auth_rules(&lookup(&unset)).unwrap();
            assert_eq!(
                (
                    rules.code_requests_per_address_per_hour,
                    rules.failed_guesses_per_identifier_per_day,
                ),
                (
                    defaults.code_requests_per_address_per_hour,
                    defaults.failed_guesses_per_identifier_per_day,
                ),
            );
        }

        let set = table(&[
            ("SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR", "10000"),
            ("SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY", " 3 "),
        ]);
        let rules = auth_rules(&lookup(&set)).unwrap();
        assert_eq!(rules.code_requests_per_address_per_hour, 10_000);
        assert_eq!(rules.failed_guesses_per_identifier_per_day, 3);
        // Nothing else is a setting.
        assert_eq!(rules.codes_per_hour, defaults.codes_per_hour);
        assert_eq!(
            rules.failed_deletion_guesses_per_day,
            defaults.failed_deletion_guesses_per_day
        );
        assert_eq!(
            rules.deletion_codes_per_hour,
            defaults.deletion_codes_per_hour
        );
    }

    #[test]
    fn a_sign_in_limit_is_a_count_of_one_or_more_and_zero_is_not_no_limit() {
        for name in [
            "SIGN_IN_CODE_REQUESTS_PER_ADDRESS_PER_HOUR",
            "SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY",
        ] {
            for wrong in ["0", "-1", "ten", "1.5", "2147483648", "unlimited"] {
                let settings = table(&[(name, wrong)]);
                assert!(auth_rules(&lookup(&settings)).is_err(), "{name}={wrong}");
            }
            let most = table(&[(name, "2147483647")]);
            assert!(auth_rules(&lookup(&most)).is_ok(), "{name}");
        }
    }

    #[test]
    fn the_retired_limit_on_wrong_codes_by_address_is_refused_if_set() {
        let name = "SIGN_IN_FAILED_GUESSES_PER_ADDRESS_PER_HOUR";
        // Empty is the same as unset, as for every optional setting.
        assert!(auth_rules(&lookup(&table(&[(name, " ")]))).is_ok());
        for value in ["30", "1000000"] {
            let error = auth_rules(&lookup(&table(&[(name, value)])))
                .err()
                .unwrap_or_else(|| panic!("{name}={value} was accepted"));
            assert!(error.to_string().contains(name), "{error}");
        }
    }

    #[test]
    fn a_minimum_client_version_is_optional_but_must_be_a_version() {
        let name = "MIN_CLIENT_VERSION_WEB";
        let read = |value: Option<&str>| {
            let settings = match value {
                Some(value) => table(&[(name, value)]),
                None => table(&[]),
            };
            min_client_version(&lookup(&settings), name)
        };
        assert_eq!(read(None).unwrap(), None);
        assert_eq!(read(Some(" ")).unwrap(), None);
        assert_eq!(read(Some(" 1.4.0 ")).unwrap().as_deref(), Some("1.4.0"));
        assert!(read(Some("v1")).is_err());
    }

    #[test]
    fn app_links_are_off_unless_named_and_a_wrong_value_stops_the_start() {
        let read = |pairs: &[(&str, &str)]| app_links(&lookup(&table(pairs)));
        assert!(read(&[]).unwrap().served().is_empty());
        assert!(
            read(&[
                ("APPLE_APP_ID", " "),
                ("ANDROID_SHA256_CERT_FINGERPRINTS", "")
            ])
            .unwrap()
            .served()
            .is_empty()
        );

        let fingerprint = ["AB"; 32].join(":");
        let both = read(&[
            (
                "APPLE_APP_ID",
                "ABCDE12345.app.yuppers, VWXYZ67890.app.yuppers",
            ),
            ("ANDROID_SHA256_CERT_FINGERPRINTS", &fingerprint),
        ])
        .unwrap();
        assert_eq!(both.served().len(), 2);

        assert!(read(&[("APPLE_APP_ID", "app.yuppers")]).is_err());
        assert!(read(&[("ANDROID_SHA256_CERT_FINGERPRINTS", "AB:CD")]).is_err());
        assert!(
            read(&[
                ("ANDROID_SHA256_CERT_FINGERPRINTS", &fingerprint),
                ("ANDROID_PACKAGE", "yuppers"),
            ])
            .is_err()
        );
    }
}
