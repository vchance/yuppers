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
use crate::domain::identity::Identifier;
use crate::http::web::DEFAULT_ANDROID_PACKAGE;
use crate::http::{AppLinks, TrustedProxies};
use crate::notifications::expo::{EXPO_ORIGIN, ExpoPushSender};
use crate::notifications::push::{LogPushSender, PushSender};
use crate::notifications::sms::{
    CodeRouter, LogSmsSender, SmsSender, TWILIO_ORIGIN, TwilioCredential, TwilioSmsSender,
};
use crate::notifications::smtp::{Secret, SmtpSender, SmtpSettings, TlsMode};
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

/// The SMTP server, from `SMTP_HOST`, `SMTP_PORT`, `SMTP_TLS`,
/// `SMTP_USERNAME`, `SMTP_PASSWORD` and `SMTP_FROM`. The password is read
/// into a type that cannot be printed.
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
        from: required(get, "SMTP_FROM")?,
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

/// The sender named by `NOTIFICATION_DELIVERY`.
fn email_sender(get: Lookup<'_>) -> anyhow::Result<Arc<dyn EmailSender>> {
    match required(get, "NOTIFICATION_DELIVERY")?.as_str() {
        "log" => Ok(Arc::new(LogEmailSender)),
        "smtp" => Ok(smtp_sender(get)?),
        other => bail!("NOTIFICATION_DELIVERY={other} is not supported; use `smtp` or `log`"),
    }
}

/// The sender of one-time codes: `CODE_DELIVERY` for email addresses, and
/// for phone numbers too while `SMS_DELIVERY` is off (as before SMS was
/// built: `log` writes them to the log, `smtp` refuses them); with
/// `SMS_DELIVERY` on, phone numbers get their codes by text message.
fn code_sender(get: Lookup<'_>) -> anyhow::Result<Arc<dyn CodeSender>> {
    let email: Arc<dyn CodeSender> = match required(get, "CODE_DELIVERY")?.as_str() {
        "log" => Arc::new(LogSender),
        "smtp" => smtp_sender(get)?,
        other => bail!("CODE_DELIVERY={other} is not supported; use `smtp` or `log`"),
    };
    Ok(match sms_sender(get)? {
        None => email,
        Some(sms) => Arc::new(CodeRouter::new(email, sms, Wording::embedded()?)),
    })
}

/// How long the SMS provider has to take a message. A person is waiting on
/// the request that sends it.
const SMS_TIMEOUT: Duration = Duration::from_secs(10);

/// The SMS provider named by `SMS_DELIVERY`, if any. Default `off`.
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
            "SMS_DELIVERY=twilio needs a credential: SMS_API_KEY_SID and SMS_API_KEY_SECRET \
             (recommended), or SMS_AUTH_TOKEN"
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
/// `SIGN_IN_FAILED_GUESSES_PER_IDENTIFIER_PER_DAY`). Each defaults to the
/// placeholder in [`AuthRules::default`].
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
        ..defaults
    })
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

/// The application role's connection, for the api, the worker and
/// `replay-deletions`: `DATABASE_URL`, or else `DATABASE_SERVER_URL` with its
/// user and password replaced by `exchange_app` and `APP_DB_PASSWORD`. The
/// second way is for a platform whose only connection string for the
/// database is the owner's (docs/deploy-render.md): it says where the
/// database is, and the credentials in it are never used. The password is
/// the one `migrate` created the role with (`crate::app_role`).
fn database_url(get: Lookup<'_>) -> anyhow::Result<String> {
    match (
        optional(get, "DATABASE_URL"),
        optional(get, "DATABASE_SERVER_URL"),
    ) {
        (Some(_), Some(_)) => {
            bail!("set DATABASE_URL or DATABASE_SERVER_URL (with APP_DB_PASSWORD), not both")
        }
        (Some(url), None) => Ok(url),
        (None, None) => {
            bail!("DATABASE_URL is not set (nor DATABASE_SERVER_URL and APP_DB_PASSWORD)")
        }
        (None, Some(server)) => {
            let password = AppRolePassword::new(
                required(get, "APP_DB_PASSWORD")
                    .context("DATABASE_SERVER_URL needs the application role's password")?,
            )?;
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
        })
    }
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
    /// Whether codes for phone numbers go by text message (`SMS_DELIVERY`).
    pub sms: bool,
    /// Wallet passes, for the platforms configured (`crate::wallet::config`).
    pub wallet: crate::wallet::WalletConfig,
}

impl ApiConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        load_env();
        let get: Lookup<'_> = &environment;

        let bind_addr: SocketAddr = get("BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8080".to_owned())
            .parse()
            .context("BIND_ADDR is not a valid socket address")?;

        let app_secret = required(get, "APP_SECRET")?.into_bytes();
        if app_secret.len() < 32 {
            bail!("APP_SECRET must be at least 32 bytes");
        }

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
            wallet: crate::wallet::WalletConfig::from_lookup(get)?,
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
        let owner = table(&[("MIGRATION_DATABASE_URL", "postgres://o:p@h/d")]);
        let config = MigrateConfig::from_lookup(&lookup(&owner)).unwrap();
        assert_eq!(config.database_url, "postgres://o:p@h/d");
        assert!(config.create_app_role.is_none(), "off by default");

        let development = table(&[("DATABASE_URL", "postgres://a:b@h/d")]);
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

    #[test]
    fn codes_for_phone_numbers_go_by_sms_only_when_it_is_on() {
        let phone = Identifier::parse("+15551234567").unwrap();
        let email = Identifier::parse("ana@example.test").unwrap();
        let sender = |pairs: &[(&str, &str)]| code_sender(&lookup(&table(pairs))).unwrap();
        // Off: codes go as before, and none is charged as a text message.
        let off = sender(&[("CODE_DELIVERY", "log")]);
        assert!(!off.charged_per_message(&phone));
        let on = sender(&[("CODE_DELIVERY", "log"), ("SMS_DELIVERY", "log")]);
        assert!(on.charged_per_message(&phone));
        assert!(!on.charged_per_message(&email));
    }

    #[test]
    fn the_channels_offered_are_the_ones_the_deployment_can_deliver() {
        use crate::auth::{SignInChannel::*, sign_in_channels};
        const TWILIO: &[(&str, &str)] = &[
            ("SMS_DELIVERY", "twilio"),
            ("SMS_ACCOUNT_SID", ACCOUNT_SID),
            ("SMS_API_KEY_SID", KEY_SID),
            ("SMS_API_KEY_SECRET", "hunter2-sms"),
            ("SMS_FROM", "+15550000000"),
        ];
        let channels = |codes: &str, sms: &[(&str, &str)]| {
            let pairs = [SMTP, &[("CODE_DELIVERY", codes)], sms].concat();
            sign_in_channels(code_sender(&lookup(&table(&pairs))).unwrap().as_ref())
        };
        for sms in [&[][..], &[("SMS_DELIVERY", "off")]] {
            // Codes by email, and nothing to send a text message with: what
            // a deployment has before it buys SMS.
            assert_eq!(channels("smtp", sms), [Email]);
            // The development log takes phone codes too.
            assert_eq!(channels("log", sms), [Email, Phone]);
        }
        for sms in [&[("SMS_DELIVERY", "log")][..], TWILIO] {
            assert_eq!(channels("smtp", sms), [Email, Phone]);
            assert_eq!(channels("log", sms), [Email, Phone]);
        }
    }

    #[test]
    fn phone_numbers_are_taken_from_the_us_and_the_rest_of_nanp_unless_set() {
        let read = |pairs: &[(&str, &str)]| auth_rules(&lookup(&table(pairs)));
        assert_eq!(read(&[]).unwrap().phone_country_codes, ["1"]);
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
