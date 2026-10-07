//! The blind-index key in the database (`contact_key`, migration 0025), and
//! the check every process makes at start that `CONTACT_DATA_KEY` is the
//! database's key.
//!
//! `migrate` derives the index key from the first `CONTACT_DATA_KEY` it is
//! given and stores it encrypted under that key ([`bootstrap`]); the api and
//! the worker read it back ([`open`]) and refuse to start if it does not
//! decrypt, or if anything stored is under a key they do not have. Rotating
//! the key re-encrypts the row ([`rewrap`]), and leaves the index key as it
//! was.

use sqlx::PgPool;

use super::{Field, IndexKey, KeyConfig, Keys};
use crate::error::Redacted;

/// The columns that hold encrypted contact details, as `(table, column)`.
pub const ENCRYPTED_COLUMNS: [(&str, &str); 4] = [
    ("account", "email_encrypted"),
    ("account", "phone_encrypted"),
    ("sms_consent", "phone_encrypted"),
    ("sms_code_consent", "phone_encrypted"),
];

/// Why the keys could not be opened against the database. Names no key and
/// no value.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(
        "the database has no contact data key yet: run migrate first, with CONTACT_DATA_KEY set"
    )]
    NotBootstrapped,
    #[error(
        "CONTACT_DATA_KEY (and CONTACT_DATA_KEY_PREVIOUS, if set) does not decrypt this \
         database's contact data: it is not the key this database was given"
    )]
    WrongKey,
    #[error(
        "{table}.{column} holds values encrypted under a key that is neither \
         CONTACT_DATA_KEY nor CONTACT_DATA_KEY_PREVIOUS; set the missing key as \
         CONTACT_DATA_KEY_PREVIOUS and run `contact-data rotate` (docs/operations.md)"
    )]
    UnknownKey {
        table: &'static str,
        column: &'static str,
    },
    #[error("{}", Redacted(.0))]
    Database(#[from] sqlx::Error),
}

impl OpenError {
    /// Whether the database could not be asked, as opposed to answering
    /// that the key is wrong: worth waiting for, where the rest is not.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, OpenError::Database(error) if !matches!(error, sqlx::Error::Database(_)))
    }
}

async fn wrapped(db: &PgPool) -> Result<Option<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar("SELECT index_key FROM contact_key")
        .fetch_optional(db)
        .await
}

fn unwrap(config: &KeyConfig, wrapped: &[u8]) -> Result<Keys, OpenError> {
    // Opened with the configured keys and any index key: only the sealer
    // matters for this.
    let opener = Keys::first(config);
    let bytes = opener
        .open_bytes(Field::INDEX_KEY, wrapped)
        .map_err(|_| OpenError::WrongKey)?;
    let index: [u8; 32] = bytes.try_into().map_err(|_| OpenError::WrongKey)?;
    Ok(Keys::with_index(config, IndexKey(index)))
}

/// For `migrate`, as the schema owner: stores the index key the first time,
/// derived from the current key, and returns the keys. Run any number of
/// times, at once or not: the first to store wins, and every run returns
/// what was stored.
pub async fn bootstrap(owner: &PgPool, config: &KeyConfig) -> Result<Keys, OpenError> {
    if wrapped(owner).await?.is_none() {
        let first = Keys::first(config);
        let index_key = first.seal_bytes(Field::INDEX_KEY, &first.index.0);
        sqlx::query("INSERT INTO contact_key (index_key) VALUES ($1) ON CONFLICT (id) DO NOTHING")
            .bind(index_key)
            .execute(owner)
            .await?;
    }
    let stored = wrapped(owner).await?.ok_or(OpenError::NotBootstrapped)?;
    unwrap(config, &stored)
}

/// For the api, the worker and the commands: the keys, once the database
/// shows they are its own. Refused when the database was never given a
/// key, when the index key does not decrypt under the configured keys, and
/// when any value stored is under a key that is not configured, which
/// could then never be read.
pub async fn open(db: &PgPool, config: &KeyConfig) -> Result<Keys, OpenError> {
    let keys = open_index_only(db, config).await?;
    let known: Vec<i16> = keys.known_ids().into_iter().map(i16::from).collect();
    for (table, column) in ENCRYPTED_COLUMNS {
        let unknown: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS (SELECT 1 FROM {table}
                            WHERE {column} IS NOT NULL AND get_byte({column}, 0) <> ALL($1))"
        )))
        .bind(&known)
        .fetch_one(db)
        .await?;
        if unknown {
            return Err(OpenError::UnknownKey { table, column });
        }
    }
    Ok(keys)
}

/// The keys, once the index key decrypts under them, without looking at the
/// values stored: for `contact-data`, which counts and re-encrypts what is
/// under another key rather than refusing it.
pub async fn open_index_only(db: &PgPool, config: &KeyConfig) -> Result<Keys, OpenError> {
    let stored = wrapped(db).await?.ok_or(OpenError::NotBootstrapped)?;
    unwrap(config, &stored)
}

/// [`open`], waiting while the database cannot be reached: the api and the
/// worker may start before it is up, and nothing they do can go on without
/// the keys. A database that answers that the key is wrong is not waited
/// for.
pub async fn open_when_reachable(db: &PgPool, config: &KeyConfig) -> Result<Keys, OpenError> {
    let mut tries: u32 = 0;
    loop {
        match open(db, config).await {
            Err(error) if error.is_unreachable() => {
                // Said at once, then about once a minute.
                if tries.is_multiple_of(30) {
                    tracing::warn!(
                        %error,
                        "cannot reach the database to check CONTACT_DATA_KEY; nothing starts until it can"
                    );
                }
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            done => return done,
        }
    }
}

/// Encrypts the index key again under the current key, if it is under
/// another. As the schema owner. Returns whether it was.
pub async fn rewrap(owner: &PgPool, keys: &Keys) -> Result<bool, OpenError> {
    let stored = wrapped(owner).await?.ok_or(OpenError::NotBootstrapped)?;
    if stored.first() == Some(&keys.current_id()) {
        return Ok(false);
    }
    let index_key = keys.seal_bytes(Field::INDEX_KEY, &keys.index.0);
    sqlx::query("UPDATE contact_key SET index_key = $1, rewrapped_at = now()")
        .bind(index_key)
        .execute(owner)
        .await?;
    Ok(true)
}
