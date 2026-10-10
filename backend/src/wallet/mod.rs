//! Wallet passes (DESIGN.md §11): each party's view of one exchange in their
//! phone's wallet, kept up to date as the exchange changes.
//!
//! A pass is a status view and nothing more. It is optional, it never stands
//! in for signing in (invariant 1), and it carries no terms: its face is made
//! by one pure function, [`pass::render`], from the exchange as its party
//! sees it, and both platforms draw that same model, so the two faces cannot
//! drift apart. What it may show follows the lock-screen rule (§11, §12):
//! the product, the exchange's display code, the other party's alias if one
//! is set, how the agreement stands in a few generic words, how many
//! contributions are still open, the next due date, and a link back to the
//! exchange, which asks its reader to sign in. Never what anyone owes, an
//! amount, a description or a name from the agreement.
//!
//! * [`apple`] builds the signed `.pkpass` and speaks Apple's pass web
//!   service protocol (the routes are in `crate::http::wallet`); a change is
//!   pushed to the devices that registered through APNs ([`apple::push`]).
//! * [`google`] creates the generic pass class and object through the Google
//!   Wallet API and makes the signed "Save to Google Wallet" link that names
//!   the object; a change is patched into the object ([`google::objects`]).
//! * [`store`] is the `wallet_pass` table and Apple's device registrations;
//!   [`store::mark_exchange_changed`] runs in the transaction of every change
//!   to an exchange, and [`delivery`] is the worker's side.
//!
//! Nothing here runs until a deployment configures a platform ([`config`]):
//! until then `GET /v1/meta` names no platform, the clients show no button,
//! and the endpoints answer `WALLET_UNAVAILABLE`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use time::OffsetDateTime;
use utoipa::ToSchema;

pub mod apple;
pub mod config;
pub mod delivery;
pub mod google;
pub mod net;
pub mod pass;
mod pem;
pub mod store;
#[cfg(test)]
pub(crate) mod testkit;
pub mod wording;

pub use config::WalletConfig;
use google::objects::WalletObjects;

/// A wallet a pass can be added to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WalletPlatform {
    Apple,
    Google,
}

impl WalletPlatform {
    /// As stored in `wallet_pass.platform`.
    pub fn as_str(self) -> &'static str {
        match self {
            WalletPlatform::Apple => "APPLE",
            WalletPlatform::Google => "GOOGLE",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "APPLE" => Some(WalletPlatform::Apple),
            "GOOGLE" => Some(WalletPlatform::Google),
            _ => None,
        }
    }
}

/// How much the status line of a pass says (`WALLET_STATUS_ON_FACE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusOnFace {
    /// How the agreement stands: *Waiting for you*, *Disputed*, *Overdue*,
    /// *Due soon* or *In force*, and the next due date. A deployment
    /// setting for now; a per-person opt-in is planned (docs/wallet.md).
    Detailed,
    /// *In force* for every agreement in force, and no next due date, so
    /// nothing on the face says that anything is pressing (DESIGN.md §11
    /// makes status on the lock screen opt-in). The default (DESIGN.md §11,
    /// the owner's decision of 3 October 2026).
    #[default]
    Neutral,
}

/// The numbers behind passes. Placeholders: none is a recorded design
/// decision.
#[derive(Clone, Debug)]
pub struct WalletRules {
    /// How often one party may be handed their pass for one exchange on one
    /// platform in an hour. Signing costs little; this keeps a script from
    /// making the service sign without end.
    pub issues_per_hour: i32,
    /// How long a link to download an Apple pass works. It works once.
    pub download_link_ttl: time::Duration,
    /// How many of an Apple pass's authentication tokens stay valid. Each
    /// handing out makes a new one; the copies on a person's other devices
    /// keep updating with theirs until this many newer ones exist.
    pub auth_tokens_kept: i64,
    /// New device registrations one pass takes in an hour.
    pub registrations_per_hour: i32,
    /// How long a device's registration for a voided pass is kept after the
    /// device was told that the pass changed, for it to fetch the void face.
    pub void_fetch_grace: time::Duration,
    /// How long it is kept at most after the voiding, told or not.
    pub void_registration_kept: time::Duration,
    /// Requests to the device log endpoint one address may make a minute.
    pub device_logs_per_minute: u32,
}

impl Default for WalletRules {
    fn default() -> Self {
        Self {
            issues_per_hour: 10,
            download_link_ttl: time::Duration::minutes(10),
            auth_tokens_kept: 3,
            registrations_per_hour: 10,
            void_fetch_grace: time::Duration::days(1),
            void_registration_kept: time::Duration::days(30),
            device_logs_per_minute: 10,
        }
    }
}

/// Everything the service needs to issue passes, for the platforms that are
/// configured. Built once per process from [`WalletConfig`].
pub struct Wallet {
    /// Apple as configured; [`Wallet::apple`] is Apple while its certificate
    /// is valid.
    apple: Option<apple::AppleIssuer>,
    pub google: Option<google::GoogleIssuer>,
    /// Creates and updates Google objects: when a save link is made, so that
    /// the link only names an object that already exists, and in the worker.
    pub google_objects: Option<Arc<dyn WalletObjects>>,
    /// Where the web app is served from. A pass links into it, and Apple's
    /// web service is reached through it.
    pub web_origin: String,
    pub wording: Arc<wording::WalletWording>,
    pub rules: WalletRules,
    pub status_on_face: StatusOnFace,
    /// Set once the expiry of the pass type certificate has been logged.
    apple_expiry_logged: AtomicBool,
    /// The device log endpoint's count per address.
    device_logs: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl Wallet {
    /// A wallet with no platform: the default, and what every deployment has
    /// until it configures one.
    pub fn off(web_origin: &str) -> Self {
        Self::new(&WalletConfig::default(), web_origin)
            .expect("the embedded wording has the default language's wallet section")
    }

    /// The configured platforms. Fails if the wording cannot say what a pass
    /// needs, or the client for Google's API cannot be made.
    pub fn new(config: &WalletConfig, web_origin: &str) -> anyhow::Result<Self> {
        let web_origin = web_origin.trim_end_matches('/').to_owned();
        let google_objects: Option<Arc<dyn WalletObjects>> = match (&config.google, config.delivery)
        {
            (Some(_), Some(config::UpdateDelivery::Log)) => {
                Some(Arc::new(google::objects::LogObjects))
            }
            (Some(google), Some(config::UpdateDelivery::Live)) => Some(Arc::new(
                google::objects::ApiObjects::new(google.account.clone())?,
            )),
            _ => None,
        };
        Ok(Self {
            apple: config
                .apple
                .as_ref()
                .map(|apple| apple::AppleIssuer::new(apple.clone(), &web_origin)),
            google: config.google.clone(),
            google_objects,
            web_origin,
            wording: Arc::new(wording::WalletWording::embedded()?),
            rules: WalletRules::default(),
            status_on_face: config.status_on_face,
            apple_expiry_logged: AtomicBool::new(false),
            device_logs: Mutex::new(HashMap::new()),
        })
    }

    /// The same, creating and updating Google objects through `objects`.
    pub fn with_google_objects(mut self, objects: Arc<dyn WalletObjects>) -> Self {
        if self.google.is_some() {
            self.google_objects = Some(objects);
        }
        self
    }

    /// Apple, while its pass type certificate is valid. An expired one signs
    /// passes Wallet refuses and pushes APNs refuses, so Apple is then
    /// unavailable (`WALLET_UNAVAILABLE`) while everything else goes on. It
    /// is checked on every use, so a certificate that expires while the
    /// process runs takes Apple off then; logged once, as an error.
    pub fn apple(&self) -> Option<&apple::AppleIssuer> {
        self.apple_at(OffsetDateTime::now_utc())
    }

    /// [`Wallet::apple`] at `now`.
    pub fn apple_at(&self, now: OffsetDateTime) -> Option<&apple::AppleIssuer> {
        let issuer = self.apple.as_ref()?;
        let expires = issuer.settings.signer.not_after();
        if expires > now {
            return Some(issuer);
        }
        if !self.apple_expiry_logged.swap(true, Ordering::Relaxed) {
            tracing::error!(
                %expires,
                "APPLE_PASS_CERT has expired: Apple Wallet passes are off until it is renewed (docs/wallet.md)"
            );
        }
        None
    }

    /// When the configured pass type certificate expires, valid or not.
    pub fn apple_certificate_expires(&self) -> Option<OffsetDateTime> {
        self.apple
            .as_ref()
            .map(|issuer| issuer.settings.signer.not_after())
    }

    /// The platforms a pass can be added to now, in a fixed order.
    pub fn platforms(&self) -> Vec<WalletPlatform> {
        let mut platforms = Vec::new();
        if self.apple().is_some() {
            platforms.push(WalletPlatform::Apple);
        }
        if self.google.is_some() {
            platforms.push(WalletPlatform::Google);
        }
        platforms
    }

    /// The link to an exchange that a pass carries. Like a notification's, it
    /// opens the exchange for someone signed in and asks anyone else to sign
    /// in; it lets nobody in by itself.
    pub fn exchange_link(&self, exchange: uuid::Uuid) -> String {
        format!("{}/exchanges/{exchange}", self.web_origin)
    }

    /// Whether `address` may send a device log at `now`: a few a minute per
    /// address, counted in this process. Anyone may call that endpoint.
    pub fn device_log_allowed(&self, address: Option<IpAddr>, now: Instant) -> bool {
        let Some(address) = address else {
            return true;
        };
        let mut counts = self
            .device_logs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let minute = std::time::Duration::from_secs(60);
        // Kept small: windows that have ended are forgotten when it grows.
        if counts.len() >= 10_000 {
            counts.retain(|_, (started, _)| now.duration_since(*started) < minute);
        }
        let entry = counts.entry(address).or_insert((now, 0));
        if now.duration_since(entry.0) >= minute {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
        entry.1 <= self.rules.device_logs_per_minute
    }

    /// `yuppers_wallet_cert_expiry_seconds`, when Apple is configured
    /// (docs/operations.md, "Metrics").
    pub fn render_metrics(&self, text: &mut crate::metrics::Text) {
        if let Some(expires) = self.apple_certificate_expires() {
            text.single(
                crate::metrics::WALLET_CERT_EXPIRY,
                crate::metrics::Kind::Gauge,
                "Seconds until the Apple pass type certificate expires; negative once it has, and Apple Wallet passes are then off.",
                (expires - OffsetDateTime::now_utc()).whole_seconds() as f64,
            );
        }
    }
}

/// 256 random bits, base64url: a token only its holder knows. Only its
/// SHA-256 is stored ([`store::token_hash`]).
pub(crate) fn random_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
