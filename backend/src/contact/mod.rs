//! Email addresses and phone numbers at rest (migration 0025; README,
//! "Contact details at rest"). A value that must be read back is encrypted,
//! and a value that must only be found or compared is kept as a keyed hash,
//! its blind index. The database holds none in the clear.
//!
//! **One secret.** `CONTACT_DATA_KEY` is 32 random bytes, apart from
//! `APP_SECRET`. HKDF-SHA256 derives from it the encryption key and a
//! one-byte key ID. The blind-index key is derived from it too, once, the
//! first time `migrate` runs with a key, and kept in the database encrypted
//! under it (`contact_key`, [`store`]): rotating `CONTACT_DATA_KEY`
//! re-encrypts that row with the rest, so the index key, and with it every
//! index, never changes. An index that changed with each rotation would have
//! to be looked for under two keys until every row was recomputed, and a
//! join between two tables, an opt-out against an account's number above
//! all, could miss while they disagreed: that is a text to someone who said
//! STOP. The cost is that after a leak of the key, the indexes stay under a
//! key the leak revealed; recovering from that is a planned re-index, not a
//! rotation (docs/operations.md, "Contact data key").
//!
//! **Encryption.** XChaCha20-Poly1305 with a random 24-byte nonce. What is
//! stored is one byte naming the key, the nonce, and the ciphertext with its
//! tag. The associated data names the table and column ([`Field`]), so a
//! value copied into another column, or another table, does not decrypt
//! there. A value that does not decrypt, under a key that is not configured,
//! a wrong key, or after tampering, is an error ([`Unreadable`]): never a
//! guess, never empty.
//!
//! **Blind index.** HMAC-SHA256 under the index key of the normalized value
//! (a lower-case email address, an E.164 phone number, as
//! [`Identifier`] makes them), with its kind in what is hashed. The same
//! value has the same index in every table, so lookups, joins and the unique
//! constraints use it.
//!
//! **Where it is decrypted.** Where a value is sent to (an email, a text
//! message, a code), shown to its owner (the account, the number for text
//! updates), re-encrypted under a new key (`contact-data rotate`), and
//! shown masked where someone must tell who an account is: to the
//! initiator of an exchange who must confirm a claimant, and to the owner
//! of the service in `staff list`. The records of consent also bind each
//! number to its row ([`Field::row`]). The privacy policy says the same
//! ("Security"). Nothing decrypted is logged, put in an error, counted in a
//! metric or written to the outbox. (The development deliveries, `log`,
//! write a code's email address to the log, as they always have; a
//! deployment never uses them.)
//!
//! The keys are installed once per process ([`install`]), as
//! `notifications::sms_updates::configure` does for texting: a contact
//! detail is read or written wherever an account, an exchange or a
//! subscription changes, and every one of those paths would otherwise have
//! to carry them.

pub mod rotate;
pub mod store;

use std::fmt;
use std::sync::OnceLock;

use base64::Engine as _;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::aead::Payload;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorCode};

/// How long a nonce is: XChaCha20's 24 bytes, random for every value.
const NONCE_BYTES: usize = 24;
/// How long Poly1305's tag is.
const TAG_BYTES: usize = 16;
/// The key ID, the nonce and the tag around the ciphertext.
pub const OVERHEAD: usize = 1 + NONCE_BYTES + TAG_BYTES;

/// What HKDF extracts with, so that these keys are this service's alone.
const SALT: &[u8] = b"yuppers contact data";

/// One `CONTACT_DATA_KEY` (or `CONTACT_DATA_KEY_PREVIOUS`): 32 random bytes.
/// Never printed: its `Debug` says only which key it is.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key(id {})", self.id())
    }
}

/// Why a setting is not a key. Names the setting, never its value.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BadKey {
    #[error("{0} is not base64 (generate one with `openssl rand -base64 32`)")]
    NotBase64(&'static str),
    #[error("{0} is not 32 bytes once decoded (generate one with `openssl rand -base64 32`)")]
    WrongLength(&'static str),
    /// Every byte a printable character: a phrase someone typed and encoded,
    /// not random bytes.
    #[error("{0} is text, not 32 random bytes (generate one with `openssl rand -base64 32`)")]
    NotRandom(&'static str),
    #[error(
        "{0} is one of the keys published in the repository, for development, tests and CI, \
         and WEB_ORIGIN is not this machine (generate one with `openssl rand -base64 32`)"
    )]
    Published(&'static str),
    #[error(
        "{0} is one of the keys published in the repository, for development, tests and CI, \
         and the database is not on this machine (generate one with `openssl rand -base64 32`)"
    )]
    PublishedDatabase(&'static str),
}

/// The keys written in the repository, for development (`.env.example`),
/// the tests (`tests/common`) and CI: anyone can read them, so a process whose `WEB_ORIGIN` is not this machine refuses them
/// ([`Key::refuse_published`]), and so do `migrate` and `contact-data` when
/// the database is not ([`Key::refuse_published_for_database`]).
pub const PUBLISHED_KEYS: [&str; 3] = [
    // .env.example
    "caRGia2fmQ1tSAu6rulYEdw2aTw9YK6HguX6H6SWMLU=",
    // backend/tests/common
    "ynS/WXUM1m1WI7fr0aperankrkmPmF/RkljW3na68ag=",
    // .github/workflows/ci.yml
    "5p7nTzW0IIJkboTNJ4ePM4NlIYippLdb7EvFQ27RWMk=",
];

impl Key {
    /// Reads a key written in base64, as `openssl rand -base64 32` writes
    /// it, padded or not. `name` is the setting's, for the error. A key
    /// whose bytes are all printable characters is refused: it was typed,
    /// not generated.
    pub fn parse(name: &'static str, text: &str) -> Result<Self, BadKey> {
        let bytes = Self::decode(text).ok_or(BadKey::NotBase64(name))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| BadKey::WrongLength(name))?;
        if bytes.iter().all(|byte| (0x20..0x7f).contains(byte)) {
            return Err(BadKey::NotRandom(name));
        }
        Ok(Self(bytes))
    }

    fn decode(text: &str) -> Option<Vec<u8>> {
        let text = text.trim();
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(text))
            .ok()
    }

    /// Whether this is one of the [`PUBLISHED_KEYS`], compared as bytes.
    pub fn is_published(&self) -> bool {
        PUBLISHED_KEYS
            .iter()
            .any(|published| Self::decode(published).as_deref() == Some(&self.0[..]))
    }

    /// Refuses one of the [`PUBLISHED_KEYS`] unless `web_origin` is this
    /// machine (`localhost`, a loopback address) or not given.
    pub fn refuse_published(
        &self,
        name: &'static str,
        web_origin: Option<&str>,
    ) -> Result<(), BadKey> {
        if self.is_published() && web_origin.is_some_and(|origin| !is_local_origin(origin)) {
            return Err(BadKey::Published(name));
        }
        Ok(())
    }

    /// Refuses one of the [`PUBLISHED_KEYS`] unless the database
    /// `database_url` names is on this machine (`localhost`, a loopback
    /// address, or a Unix socket): for `migrate` and `contact-data`, which
    /// may run without `WEB_ORIGIN`.
    pub fn refuse_published_for_database(
        &self,
        name: &'static str,
        database_url: &str,
    ) -> Result<(), BadKey> {
        let env = |name: &str| std::env::var(name).ok();
        if self.is_published() && !is_local_database(database_url, &env) {
            return Err(BadKey::PublishedDatabase(name));
        }
        Ok(())
    }

    /// A key from its bytes, for tests.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn derive(&self, info: &[u8], out: &mut [u8]) {
        Hkdf::<Sha256>::new(Some(SALT), &self.0)
            .expand(info, out)
            .expect("HKDF-SHA256 gives up to 8160 bytes");
    }

    /// The byte that names this key at the front of what it encrypts.
    pub fn id(&self) -> u8 {
        let mut id = [0u8; 1];
        self.derive(b"key id v1", &mut id);
        id[0]
    }

    fn sealer(&self) -> Sealer {
        let mut key = [0u8; 32];
        self.derive(b"encryption v1", &mut key);
        let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("a 32-byte key");
        key.fill(0);
        Sealer {
            id: self.id(),
            cipher,
        }
    }

    /// The blind-index key this key gives when it is the first one: what
    /// `migrate` stores, encrypted, the first time it runs ([`store`]).
    fn first_index_key(&self) -> IndexKey {
        let mut key = [0u8; 32];
        self.derive(b"blind index v1", &mut key);
        IndexKey(key)
    }
}

/// Whether an origin such as `http://127.0.0.1:8080` is this machine: a host
/// of `localhost` (or under it) or a loopback address.
fn is_local_origin(origin: &str) -> bool {
    let rest = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    is_loopback_host(host_of(authority))
}

/// Whether every host a PostgreSQL connection string names is this machine:
/// `localhost`, a loopback address, or a Unix socket (a path). Anything it
/// cannot read, such as the keyword form, is not.
///
/// A string that names no host at all connects where `PGHOSTADDR` or
/// `PGHOST` says, as libpq and sqlx both read them (`env`), and only
/// without either to the default socket or `localhost`; so those are what
/// is checked then.
fn is_local_database(url: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let authority = path.split('/').next().unwrap_or("");
    let hosts = authority
        .rsplit_once('@')
        .map_or(authority, |(_, hosts)| hosts);
    let mut named: Vec<String> = hosts
        .split(',')
        .map(|host| host_of(host).to_owned())
        .collect();
    for pair in query.split('&') {
        if let Some(value) = pair
            .strip_prefix("host=")
            .or_else(|| pair.strip_prefix("hostaddr="))
        {
            named = value.split(',').map(str::to_owned).collect();
        }
    }
    if named.iter().all(String::is_empty) {
        let from_env = ["PGHOSTADDR", "PGHOST"]
            .into_iter()
            .filter_map(env)
            .find(|value| !value.trim().is_empty());
        if let Some(value) = from_env {
            named = value
                .split(',')
                .map(|host| host.trim().to_owned())
                .collect();
        }
    }
    named.iter().all(|host| {
        let host = host.to_ascii_lowercase();
        host.is_empty()
            || host.starts_with('/')
            || host.starts_with("%2f")
            || is_loopback_host(&host)
    })
}

/// The host of `host[:port]` or `[address]:port`.
fn host_of(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        // [::1]:8080
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// `CONTACT_DATA_KEY`, and `CONTACT_DATA_KEY_PREVIOUS` while what it
/// encrypted is still being re-encrypted (`contact-data rotate`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyConfig {
    pub current: Key,
    pub previous: Option<Key>,
}

/// Why the two keys cannot be used together.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyClash {
    #[error("CONTACT_DATA_KEY_PREVIOUS is the same as CONTACT_DATA_KEY")]
    Same,
    /// One in 256 pairs of keys share an ID byte, and a value would not say
    /// which of them encrypted it.
    #[error(
        "CONTACT_DATA_KEY and CONTACT_DATA_KEY_PREVIOUS have the same key ID; \
         generate another CONTACT_DATA_KEY"
    )]
    SameId,
}

impl KeyConfig {
    pub fn new(current: Key, previous: Option<Key>) -> Result<Self, KeyClash> {
        if let Some(previous) = &previous {
            if *previous == current {
                return Err(KeyClash::Same);
            }
            if previous.id() == current.id() {
                return Err(KeyClash::SameId);
            }
        }
        Ok(Self { current, previous })
    }
}

/// An encryption key and the ID byte it writes.
#[derive(Clone)]
struct Sealer {
    id: u8,
    cipher: XChaCha20Poly1305,
}

/// The blind-index key.
#[derive(Clone)]
struct IndexKey([u8; 32]);

impl Drop for IndexKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// A column that holds encrypted contact details: its table and column are
/// the associated data of every value in it, and for the records of consent
/// the row's ID too ([`Field::row`]), and for payment options the account
/// they belong to ([`Field::owned_by`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub table: &'static str,
    pub column: &'static str,
    /// The row a value belongs to, where it is bound to it.
    pub row: Option<i64>,
    /// The account a value belongs to, where it is bound to it.
    pub owner: Option<uuid::Uuid>,
    /// The row a value belongs to, by its UUID, where it is bound to it.
    pub record: Option<uuid::Uuid>,
}

impl Field {
    pub const ACCOUNT_EMAIL: Field = Field::new("account", "email");
    pub const ACCOUNT_PHONE: Field = Field::new("account", "phone");
    pub const SMS_CONSENT_PHONE: Field = Field::new("sms_consent", "phone");
    pub const SMS_CODE_CONSENT_PHONE: Field = Field::new("sms_code_consent", "phone");
    /// Whom an invitation names (`crate::exchanges::service`), bound to the
    /// invitation ([`Field::record`]).
    pub const INVITATION_EMAIL: Field = Field::new("invitation", "bound_email");
    pub const INVITATION_PHONE: Field = Field::new("invitation", "bound_phone");
    /// Where the notice that two accounts were combined goes
    /// (`crate::combine`), bound to its row ([`Field::row`]).
    pub const COMBINE_NOTICE_EMAIL: Field = Field::new("combine_notice", "email");
    pub const COMBINE_NOTICE_PHONE: Field = Field::new("combine_notice", "phone");
    /// The blind-index key itself, in `contact_key`.
    pub const INDEX_KEY: Field = Field::new("contact_key", "index_key");

    pub const fn new(table: &'static str, column: &'static str) -> Self {
        Self {
            table,
            column,
            row: None,
            owner: None,
            record: None,
        }
    }

    /// The same column, for the value of the row with this ID. The records
    /// of consent bind each number to its row, so a ciphertext copied there
    /// from another record does not decrypt.
    pub const fn row(self, id: i64) -> Self {
        Self {
            row: Some(id),
            ..self
        }
    }

    /// The same column, for the value of this account. Payment options
    /// bind each value to their account, so a ciphertext copied into another
    /// account's row does not decrypt there.
    pub const fn owned_by(self, account: uuid::Uuid) -> Self {
        Self {
            owner: Some(account),
            ..self
        }
    }

    /// The same column, for the value of the row with this UUID. An
    /// invitation binds whom it names to itself, so a ciphertext copied from
    /// another invitation does not decrypt.
    pub const fn record(self, id: uuid::Uuid) -> Self {
        Self {
            record: Some(id),
            ..self
        }
    }

    fn associated_data(self) -> Vec<u8> {
        let mut data = format!("yuppers contact v1\0{}.{}", self.table, self.column);
        if let Some(id) = self.row {
            data.push_str(&format!("\0row {id}"));
        }
        if let Some(account) = self.owner {
            data.push_str(&format!("\0account {account}"));
        }
        if let Some(record) = self.record {
            data.push_str(&format!("\0record {record}"));
        }
        data.into_bytes()
    }

    /// The account's column for an identifier of this kind.
    pub fn account(kind: Kind) -> Self {
        match kind {
            Kind::Email => Field::ACCOUNT_EMAIL,
            Kind::Phone => Field::ACCOUNT_PHONE,
        }
    }
}

/// What kind of value an index is of. Part of what is hashed, so that the
/// kinds never share an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Email,
    Phone,
}

impl Kind {
    pub fn of(identifier: &Identifier) -> Self {
        match identifier {
            Identifier::Email(_) => Kind::Email,
            Identifier::Phone(_) => Kind::Phone,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Kind::Email => "email",
            Kind::Phone => "phone",
        }
    }

    /// The `account` columns for this kind: encrypted, index.
    pub fn account_columns(self) -> (&'static str, &'static str) {
        match self {
            Kind::Email => ("email_encrypted", "email_index"),
            Kind::Phone => ("phone_encrypted", "phone_index"),
        }
    }
}

/// A value that does not decrypt: its key is not configured, the key is
/// wrong, it was moved from another column, or it was changed. Says nothing
/// about the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a stored contact detail does not decrypt under the configured keys")]
pub struct Unreadable;

impl From<Unreadable> for sqlx::Error {
    /// As a column that could not be decoded, which is what it is: the
    /// redacted form of a database error says "decode" and nothing more.
    fn from(unreadable: Unreadable) -> Self {
        sqlx::Error::Decode(Box::new(unreadable))
    }
}

impl From<Unreadable> for ApiError {
    fn from(unreadable: Unreadable) -> Self {
        tracing::error!(%unreadable, "a request needed a contact detail it could not read");
        ErrorCode::Internal.into()
    }
}

/// The keys of a running process: the current key, which encrypts, the
/// previous one if any, which only decrypts, and the blind-index key.
#[derive(Clone)]
pub struct Keys {
    current: Sealer,
    previous: Option<Sealer>,
    index: IndexKey,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keys")
            .field("current", &self.current.id)
            .field("previous", &self.previous.as_ref().map(|sealer| sealer.id))
            .finish_non_exhaustive()
    }
}

impl Keys {
    fn with_index(config: &KeyConfig, index: IndexKey) -> Self {
        Self {
            current: config.current.sealer(),
            previous: config.previous.as_ref().map(Key::sealer),
            index,
        }
    }

    /// The keys a database first given `config.current` holds: the index
    /// key derived from it. For a new database ([`store::bootstrap`]) and
    /// for tests; a running service reads the index key from the database
    /// ([`store::open`]).
    pub fn first(config: &KeyConfig) -> Self {
        Self::with_index(config, config.current.first_index_key())
    }

    /// The ID of the key that encrypts.
    pub fn current_id(&self) -> u8 {
        self.current.id
    }

    /// The IDs of every key that decrypts.
    pub fn known_ids(&self) -> Vec<u8> {
        std::iter::once(self.current.id)
            .chain(self.previous.as_ref().map(|sealer| sealer.id))
            .collect()
    }

    fn seal_bytes(&self, field: Field, value: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).expect("the operating system provides randomness");
        let ciphertext = self
            .current
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: value,
                    aad: &field.associated_data(),
                },
            )
            .expect("XChaCha20-Poly1305 encrypts anything this short");
        let mut sealed = Vec::with_capacity(1 + NONCE_BYTES + ciphertext.len());
        sealed.push(self.current.id);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        sealed
    }

    fn open_bytes(&self, field: Field, sealed: &[u8]) -> Result<Vec<u8>, Unreadable> {
        if sealed.len() < OVERHEAD {
            return Err(Unreadable);
        }
        let (id, rest) = sealed.split_first().ok_or(Unreadable)?;
        let sealer = std::iter::once(&self.current)
            .chain(self.previous.as_ref())
            .find(|sealer| sealer.id == *id)
            .ok_or(Unreadable)?;
        let (nonce, ciphertext) = rest.split_at(NONCE_BYTES);
        let nonce: [u8; NONCE_BYTES] = nonce.try_into().map_err(|_| Unreadable)?;
        sealer
            .cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &field.associated_data(),
                },
            )
            .map_err(|_| Unreadable)
    }

    /// Encrypts `value` for `field` under the current key.
    pub fn seal(&self, field: Field, value: &str) -> Vec<u8> {
        self.seal_bytes(field, value.as_bytes())
    }

    /// Decrypts a value stored in `field`.
    pub fn open(&self, field: Field, sealed: &[u8]) -> Result<String, Unreadable> {
        String::from_utf8(self.open_bytes(field, sealed)?).map_err(|_| Unreadable)
    }

    /// The value a row holds in `field`, if it holds one.
    pub fn reveal(
        &self,
        field: Field,
        encrypted: Option<&[u8]>,
    ) -> Result<Option<String>, Unreadable> {
        encrypted.map(|sealed| self.open(field, sealed)).transpose()
    }

    /// The blind index of a normalized value of `kind`.
    pub fn index(&self, kind: Kind, value: &str) -> [u8; 32] {
        let mut mac =
            <Hmac<Sha256> as KeyInit>::new_from_slice(&self.index.0).expect("HMAC takes any key");
        mac.update(b"yuppers contact index v1\0");
        mac.update(kind.as_str().as_bytes());
        mac.update(b"\0");
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// The blind index of an identifier.
    pub fn index_of(&self, identifier: &Identifier) -> [u8; 32] {
        self.index(Kind::of(identifier), identifier.as_str())
    }

    /// What an identifier is stored as on an account: its encrypted copy
    /// and its index.
    pub fn sealed(&self, identifier: &Identifier) -> Sealed {
        let kind = Kind::of(identifier);
        Sealed {
            encrypted: self.seal(Field::account(kind), identifier.as_str()),
            index: self.index(kind, identifier.as_str()),
        }
    }
}

/// An identifier as an account stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub encrypted: Vec<u8>,
    pub index: [u8; 32],
}

static KEYS: OnceLock<Keys> = OnceLock::new();

/// Installs this process's keys. Every binary that reads or writes contact
/// details calls it once at start, before serving anything; a second call
/// changes nothing (the tests install the same test keys from each test).
/// Returns whether these are the keys now installed.
pub fn install(keys: Keys) -> bool {
    let current = keys.current.id;
    let installed = KEYS.get_or_init(|| keys);
    installed.current.id == current
}

/// This process's keys.
///
/// # Panics
///
/// If none were installed: a binary that handles contact details must
/// install its keys before it serves anything, and one that does not handle
/// them never reaches here.
pub fn keys() -> &'static Keys {
    KEYS.get()
        .expect("contact keys are installed at start (contact::install)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(byte: u8) -> KeyConfig {
        KeyConfig::new(Key::from_bytes([byte; 32]), None).unwrap()
    }

    #[test]
    fn a_value_comes_back_as_it_went_in() {
        let keys = Keys::first(&config(7));
        for value in ["ana@example.com", "+12025550142"] {
            let sealed = keys.seal(Field::ACCOUNT_EMAIL, value);
            assert_eq!(sealed.len(), OVERHEAD + value.len());
            assert_eq!(sealed[0], keys.current_id());
            assert!(
                !sealed.windows(value.len()).any(|w| w == value.as_bytes()),
                "no plaintext in what is stored"
            );
            assert_eq!(keys.open(Field::ACCOUNT_EMAIL, &sealed).unwrap(), value);
        }
    }

    #[test]
    fn every_value_gets_a_nonce_of_its_own() {
        let keys = Keys::first(&config(7));
        let a = keys.seal(Field::ACCOUNT_EMAIL, "ana@example.com");
        let b = keys.seal(Field::ACCOUNT_EMAIL, "ana@example.com");
        assert_ne!(a, b);
    }

    #[test]
    fn a_wrong_key_fails_closed() {
        let right = Keys::first(&config(7));
        let wrong = Keys::first(&config(8));
        let sealed = right.seal(Field::ACCOUNT_PHONE, "+12025550142");
        assert_eq!(wrong.open(Field::ACCOUNT_PHONE, &sealed), Err(Unreadable));
        // The same ID byte with another key: the tag does not check out.
        let mut forged = wrong.seal(Field::ACCOUNT_PHONE, "+12025550142");
        forged[0] = right.current_id();
        assert_eq!(right.open(Field::ACCOUNT_PHONE, &forged), Err(Unreadable));
        // Changed in storage, or cut short.
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert_eq!(right.open(Field::ACCOUNT_PHONE, &changed), Err(Unreadable));
        assert_eq!(
            right.open(Field::ACCOUNT_PHONE, &sealed[..OVERHEAD - 1]),
            Err(Unreadable)
        );
        assert_eq!(right.open(Field::ACCOUNT_PHONE, &[]), Err(Unreadable));
    }

    #[test]
    fn a_value_does_not_decrypt_in_another_column() {
        let keys = Keys::first(&config(7));
        let sealed = keys.seal(Field::ACCOUNT_PHONE, "+12025550142");
        for other in [
            Field::ACCOUNT_EMAIL,
            Field::SMS_CONSENT_PHONE,
            Field::SMS_CODE_CONSENT_PHONE,
        ] {
            assert_eq!(keys.open(other, &sealed), Err(Unreadable), "{other:?}");
        }
        assert!(keys.open(Field::ACCOUNT_PHONE, &sealed).is_ok());
    }

    #[test]
    fn the_previous_key_decrypts_and_only_the_current_one_encrypts() {
        let old = Keys::first(&config(7));
        let sealed = old.seal(Field::ACCOUNT_EMAIL, "ana@example.com");
        let rotated = KeyConfig::new(Key::from_bytes([9; 32]), Some(Key::from_bytes([7; 32])));
        let rotated = Keys::with_index(&rotated.unwrap(), old.index.clone());
        assert_eq!(
            rotated.open(Field::ACCOUNT_EMAIL, &sealed).unwrap(),
            "ana@example.com"
        );
        let resealed = rotated.seal(Field::ACCOUNT_EMAIL, "ana@example.com");
        assert_eq!(resealed[0], rotated.current_id());
        assert_ne!(resealed[0], old.current_id());
        assert_eq!(old.open(Field::ACCOUNT_EMAIL, &resealed), Err(Unreadable));
        // The index key is the database's, so indexes do not move.
        assert_eq!(
            rotated.index(Kind::Email, "ana@example.com"),
            old.index(Kind::Email, "ana@example.com")
        );
    }

    #[test]
    fn an_index_depends_on_the_key_the_kind_and_the_value() {
        let keys = Keys::first(&config(7));
        let base = keys.index(Kind::Email, "ana@example.com");
        assert_eq!(keys.index(Kind::Email, "ana@example.com"), base);
        assert_eq!(
            keys.index_of(&Identifier::parse(" Ana@Example.COM ").unwrap()),
            base,
            "computed on the normalized value"
        );
        assert_ne!(keys.index(Kind::Email, "ben@example.com"), base);
        assert_ne!(keys.index(Kind::Phone, "ana@example.com"), base);
        assert_ne!(
            Keys::first(&config(8)).index(Kind::Email, "ana@example.com"),
            base
        );
    }

    #[test]
    fn a_us_number_has_one_index_however_it_was_typed() {
        let keys = Keys::first(&config(7));
        // What is stored and indexed is E.164, as before numbers could be
        // typed without +1, so rows indexed then still match.
        let stored = keys.index(Kind::Phone, "+18565488780");
        for typed in [
            "8565488780",
            "+18565488780",
            "(856) 548-8780",
            "1-856-548-8780",
        ] {
            assert_eq!(
                keys.index_of(&Identifier::parse(typed).unwrap()),
                stored,
                "{typed}"
            );
        }
        assert_ne!(keys.index(Kind::Phone, "8565488780"), stored);
    }

    #[test]
    fn a_key_is_32_bytes_of_base64_and_is_never_printed() {
        let text = "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA=";
        let key = Key::parse("CONTACT_DATA_KEY", text).unwrap();
        assert_eq!(
            Key::parse("CONTACT_DATA_KEY", text.trim_end_matches('=')).unwrap(),
            key
        );
        assert!(!format!("{key:?}").contains(text));
        assert!(
            !format!("{:?}", Keys::first(&KeyConfig::new(key, None).unwrap())).contains("q83v")
        );
        assert_eq!(
            Key::parse("CONTACT_DATA_KEY", "not base64!"),
            Err(BadKey::NotBase64("CONTACT_DATA_KEY"))
        );
        assert_eq!(
            Key::parse("CONTACT_DATA_KEY", "c2hvcnQ="),
            Err(BadKey::WrongLength("CONTACT_DATA_KEY"))
        );
        let error = Key::parse("CONTACT_DATA_KEY", "c2hvcnQ=")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("c2hvcnQ"), "{error}");
    }

    #[test]
    fn two_keys_must_differ_in_their_id() {
        let a = Key::from_bytes([1; 32]);
        assert_eq!(
            KeyConfig::new(a.clone(), Some(a.clone())),
            Err(KeyClash::Same)
        );
        // Some other key shares a's ID: one in 256 do.
        let twin = (2..=u16::MAX)
            .map(|n| {
                let mut bytes = [0; 32];
                bytes[..2].copy_from_slice(&n.to_le_bytes());
                Key::from_bytes(bytes)
            })
            .find(|key| key.id() == a.id())
            .expect("some key among thousands shares an ID byte");
        assert_eq!(KeyConfig::new(twin, Some(a)), Err(KeyClash::SameId));
    }

    #[test]
    fn a_record_s_number_does_not_decrypt_in_another_record() {
        let keys = Keys::first(&config(7));
        let field = Field::SMS_CONSENT_PHONE;
        let sealed = keys.seal(field.row(41), "+12025550142");
        assert_eq!(keys.open(field.row(41), &sealed).unwrap(), "+12025550142");
        assert_eq!(keys.open(field.row(42), &sealed), Err(Unreadable));
        assert_eq!(keys.open(field, &sealed), Err(Unreadable));
    }

    #[test]
    fn a_typed_phrase_is_not_a_key() {
        // 32 printable characters, base64-encoded: what someone would make
        // by encoding a password.
        let typed =
            base64::engine::general_purpose::STANDARD.encode("a-phrase-of-32-printable-chars!!");
        assert_eq!(
            Key::parse("CONTACT_DATA_KEY", &typed),
            Err(BadKey::NotRandom("CONTACT_DATA_KEY"))
        );
    }

    #[test]
    fn a_published_key_works_only_on_this_machine() {
        for published in PUBLISHED_KEYS {
            let key = Key::parse("CONTACT_DATA_KEY", published).unwrap();
            assert!(key.is_published());
            for local in [
                None,
                Some("http://localhost:5173"),
                Some("http://127.0.0.1:8080"),
                Some("http://127.0.0.1"),
                Some("http://[::1]:8080/"),
                Some("http://app.localhost"),
            ] {
                assert_eq!(
                    key.refuse_published("CONTACT_DATA_KEY", local),
                    Ok(()),
                    "{local:?}"
                );
            }
            for remote in [
                "https://yuppers.app",
                "https://yuppers.app:443",
                "https://localhost.example.com",
                "http://10.0.0.2:8080",
            ] {
                assert_eq!(
                    key.refuse_published("CONTACT_DATA_KEY", Some(remote)),
                    Err(BadKey::Published("CONTACT_DATA_KEY")),
                    "{remote}"
                );
            }
        }
        let own = Key::from_bytes([0xa5; 32]);
        assert!(!own.is_published());
        assert_eq!(
            own.refuse_published("CONTACT_DATA_KEY", Some("https://yuppers.app")),
            Ok(())
        );
    }

    #[test]
    fn a_published_key_is_refused_for_a_database_elsewhere() {
        let key = Key::parse("CONTACT_DATA_KEY", PUBLISHED_KEYS[0]).unwrap();
        let no_env = |_: &str| None;
        for local in [
            "postgres://exchange:exchange@127.0.0.1:5432/yuppers",
            "postgres://exchange:exchange@localhost/yuppers",
            "postgresql://u:p@[::1]:5432/d",
            "postgres:///yuppers",
            "postgres://u@/yuppers?host=/var/run/postgresql",
            "postgres://u:p@127.0.0.1,localhost:5433/d",
        ] {
            assert!(is_local_database(local, &no_env), "{local}");
        }
        for remote in [
            "postgres://exchange:exchange@postgres:5432/yuppers",
            "postgres://u:p@dpg-abc123-a/yuppers",
            "postgres://u:p@db.example.com:5432/d?sslmode=require",
            "postgres://u:p@127.0.0.1:5432/d?host=db.example.com",
            "postgres://u:p@127.0.0.1,db.example.com/d",
            "postgres://u:p@10.0.0.2/d",
            "host=127.0.0.1 dbname=d",
        ] {
            assert!(!is_local_database(remote, &no_env), "{remote}");
        }
        // Wherever this runs, a database elsewhere is refused.
        assert_eq!(
            key.refuse_published_for_database("CONTACT_DATA_KEY", "postgres://u:p@10.0.0.2/d"),
            Err(BadKey::PublishedDatabase("CONTACT_DATA_KEY"))
        );
        let own = Key::from_bytes([0xa5; 32]);
        assert_eq!(
            own.refuse_published_for_database("CONTACT_DATA_KEY", "postgres://u:p@postgres/d"),
            Ok(())
        );
    }

    #[test]
    fn a_string_without_a_host_is_where_pghost_says() {
        fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        }
        for url in [
            "postgres:///yuppers",
            "postgres://u:p@/yuppers",
            "postgres://u@:5432/d",
        ] {
            assert!(
                !is_local_database(url, &env(&[("PGHOST", "db.example.com")])),
                "{url}"
            );
            assert!(
                !is_local_database(url, &env(&[("PGHOSTADDR", "10.0.0.2")])),
                "{url}"
            );
            assert!(
                !is_local_database(
                    url,
                    &env(&[("PGHOST", "localhost"), ("PGHOSTADDR", "10.0.0.2")])
                ),
                "{url}: PGHOSTADDR is used before PGHOST"
            );
            assert!(
                !is_local_database(url, &env(&[("PGHOST", "localhost,db.example.com")])),
                "{url}"
            );
            assert!(
                is_local_database(url, &env(&[("PGHOST", "localhost")])),
                "{url}"
            );
            assert!(
                is_local_database(url, &env(&[("PGHOST", "/var/run/postgresql")])),
                "{url}"
            );
            assert!(is_local_database(url, &env(&[("PGHOST", "")])), "{url}");
        }
        // A host in the string is used whatever PGHOST says, as sqlx does.
        let remote = env(&[("PGHOST", "db.example.com")]);
        assert!(is_local_database("postgres://u:p@127.0.0.1/d", &remote));
        assert!(is_local_database(
            "postgres://u@/d?host=/var/run/postgresql",
            &remote
        ));
    }
}
