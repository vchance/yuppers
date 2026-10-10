//! The hash chain over each exchange's history (DESIGN.md §13.2, §14.1;
//! LEGAL_MEMO.md §2.2, §2.4; migration 0033).
//!
//! Every row of `exchange_event` carries a `chain_hash`: the SHA-256 of the
//! previous row's `chain_hash` in the same exchange (the [`genesis`] value for
//! the first) and this row's own stable fields. Changing, removing or
//! inserting a row anywhere in an exchange's history changes every hash from
//! there on, so a copy of the last hash, printed on a record or logged by the
//! worker's daily anchor, shows later whether the history it covered was
//! altered.
//!
//! **What is hashed**, in this order: the domain tag `yuppers history chain
//! v1`, the previous hash (32 bytes), then the fields below. Every field is
//! written with a one-byte presence marker (0 absent, 1 present) and, if
//! present, an 8-byte big-endian length and the bytes, so no two different
//! rows share an encoding:
//!
//! 1. `exchange_id` (16 bytes)
//! 2. `sequence` (8 bytes, big-endian)
//! 3. `type` (UTF-8)
//! 4. `actor_kind` (UTF-8)
//! 5. `actor_slot` (UTF-8, absent for the system)
//! 6. `contribution_id` (16 bytes, may be absent)
//! 7. `revision_id` (16 bytes, may be absent)
//! 8. `note` (UTF-8, may be absent)
//! 9. `evidence_attachment_id` (16 bytes, may be absent)
//! 10. `data` (canonical JSON: object keys sorted, no spaces)
//! 11. `occurred_at` (Unix microseconds, 8 bytes, big-endian)
//!
//! This is the only place the hash is computed: the service when it inserts
//! an event ([`link`], from `exchanges::repo`), the owner's backfill and
//! check (`staff backfill-chain`, `staff verify-chain`), and the daily anchor
//! all use it.

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

/// Keeps this service's hashes apart from any other use of SHA-256.
const DOMAIN: &[u8] = b"yuppers history chain v1";

/// The value the first row of every exchange chains from.
pub fn genesis() -> [u8; 32] {
    Sha256::digest(b"yuppers history chain genesis v1").into()
}

/// One row of `exchange_event`, as far as the chain covers it.
#[derive(Clone, Debug, PartialEq)]
pub struct EventFields {
    pub exchange_id: Uuid,
    pub sequence: i64,
    pub kind: String,
    pub actor_kind: String,
    pub actor_slot: Option<String>,
    pub contribution_id: Option<Uuid>,
    pub revision_id: Option<Uuid>,
    pub note: Option<String>,
    pub evidence_attachment_id: Option<Uuid>,
    pub data: Value,
    pub occurred_at: OffsetDateTime,
}

/// Postgres keeps microseconds; a time is cut to them before it is hashed or
/// stored, so that what is read back hashes as what was written.
pub fn to_micros(time: OffsetDateTime) -> OffsetDateTime {
    let micros = (time.unix_timestamp_nanos() / 1_000) as i64;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .expect("a time that was valid stays valid cut to microseconds")
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn field(hasher: &mut Sha256, bytes: Option<&[u8]>) {
    match bytes {
        None => hasher.update([0u8]),
        Some(bytes) => {
            hasher.update([1u8]);
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
    }
}

fn id(uuid: &Option<Uuid>) -> Option<&[u8]> {
    uuid.as_ref().map(|uuid| uuid.as_bytes().as_slice())
}

/// The hash of a row that follows `previous`.
pub fn link(previous: &[u8; 32], event: &EventFields) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(previous);
    field(&mut hasher, Some(event.exchange_id.as_bytes()));
    field(&mut hasher, Some(&event.sequence.to_be_bytes()));
    field(&mut hasher, Some(event.kind.as_bytes()));
    field(&mut hasher, Some(event.actor_kind.as_bytes()));
    field(&mut hasher, event.actor_slot.as_deref().map(str::as_bytes));
    field(&mut hasher, id(&event.contribution_id));
    field(&mut hasher, id(&event.revision_id));
    field(&mut hasher, event.note.as_deref().map(str::as_bytes));
    field(&mut hasher, id(&event.evidence_attachment_id));
    let mut data = String::new();
    canonical(&event.data, &mut data);
    field(&mut hasher, Some(data.as_bytes()));
    let micros = (event.occurred_at.unix_timestamp_nanos() / 1_000) as i64;
    field(&mut hasher, Some(&micros.to_be_bytes()));
    hasher.finalize().into()
}

fn array(bytes: Vec<u8>) -> Option<[u8; 32]> {
    bytes.try_into().ok()
}

/// What the next event of an exchange chains from, given the sequence of its
/// last event: the genesis value for an exchange with no history, and `None`
/// when the last row has no hash (history from before the chain, until
/// `staff backfill-chain` has run), in which case the new row is left without
/// one as well and the backfill chains the whole exchange.
pub async fn head(
    conn: &mut PgConnection,
    exchange: Uuid,
    last_sequence: i64,
) -> Result<Option<[u8; 32]>, sqlx::Error> {
    if last_sequence == 0 {
        return Ok(Some(genesis()));
    }
    let stored: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT chain_hash FROM exchange_event WHERE exchange_id = $1 AND sequence = $2",
    )
    .bind(exchange)
    .bind(last_sequence)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(stored.flatten().and_then(array))
}

const SELECT: &str = "SELECT exchange_id, sequence, type, actor_kind, actor_slot, contribution_id,
        revision_id, note, evidence_attachment_id, data::text AS data, occurred_at, chain_hash
     FROM exchange_event";

struct Stored {
    fields: EventFields,
    chain_hash: Option<Vec<u8>>,
}

fn stored(row: &sqlx::postgres::PgRow) -> Result<Stored, sqlx::Error> {
    let data: String = row.try_get("data")?;
    Ok(Stored {
        fields: EventFields {
            exchange_id: row.try_get("exchange_id")?,
            sequence: row.try_get("sequence")?,
            kind: row.try_get("type")?,
            actor_kind: row.try_get("actor_kind")?,
            actor_slot: row.try_get("actor_slot")?,
            contribution_id: row.try_get("contribution_id")?,
            revision_id: row.try_get("revision_id")?,
            note: row.try_get("note")?,
            evidence_attachment_id: row.try_get("evidence_attachment_id")?,
            data: serde_json::from_str(&data).unwrap_or(Value::Null),
            occurred_at: row.try_get("occurred_at")?,
        },
        chain_hash: row.try_get("chain_hash")?,
    })
}

async fn exchanges(db: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT exchange_id FROM exchange_event ORDER BY exchange_id")
        .fetch_all(db)
        .await
}

/// What `verify` found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verification {
    pub exchanges: u64,
    pub events: u64,
    /// Rows without a hash yet (history from before the chain).
    pub unchained: u64,
    /// Rows whose stored hash is not what their fields and the row before
    /// give: (exchange, sequence).
    pub mismatched: Vec<(Uuid, i64)>,
    /// Rows after a gap in the sequence, so a row is missing before them.
    pub gaps: Vec<(Uuid, i64)>,
}

impl Verification {
    pub fn intact(&self) -> bool {
        self.mismatched.is_empty() && self.gaps.is_empty()
    }
}

/// Recomputes every hash and reports the rows that do not match. Read-only.
pub async fn verify(db: &PgPool) -> Result<Verification, sqlx::Error> {
    let mut report = Verification::default();
    for exchange in exchanges(db).await? {
        report.exchanges += 1;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE exchange_id = $1 ORDER BY sequence"
        )))
        .bind(exchange)
        .fetch_all(db)
        .await?;
        // None after a row without a hash: nothing to recompute from.
        let mut previous = Some(genesis());
        let mut expected_sequence = 1;
        for row in &rows {
            let event = stored(row)?;
            let sequence = event.fields.sequence;
            report.events += 1;
            if sequence != expected_sequence {
                report.gaps.push((exchange, sequence));
            }
            expected_sequence = sequence + 1;
            let Some(bytes) = event.chain_hash else {
                report.unchained += 1;
                previous = None;
                continue;
            };
            let hash = array(bytes);
            let matches = match (previous, hash) {
                (_, None) => false,
                (None, Some(_)) => true,
                (Some(previous), Some(hash)) => link(&previous, &event.fields) == hash,
            };
            if !matches {
                report.mismatched.push((exchange, sequence));
            }
            previous = hash;
        }
    }
    Ok(report)
}

/// What `backfill` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Backfill {
    pub exchanges: u64,
    pub chained: u64,
    pub already: u64,
}

/// Gives every row without a hash its hash, exchange by exchange in order. A
/// row that has one is never changed; running it again changes nothing. Needs
/// the schema owner's connection: the application role cannot update the
/// history, and the one update the trigger allows (`chain_hash`, from null)
/// is enabled for each transaction alone.
pub async fn backfill(db: &PgPool) -> Result<Backfill, sqlx::Error> {
    let mut report = Backfill::default();
    for exchange in exchanges(db).await? {
        report.exchanges += 1;
        let mut tx = db.begin().await?;
        sqlx::query("SELECT set_config('yuppers.chain_backfill', 'on', true)")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE exchange_id = $1 ORDER BY sequence FOR UPDATE"
        )))
        .bind(exchange)
        .fetch_all(&mut *tx)
        .await?;
        let mut previous = genesis();
        for row in &rows {
            let event = stored(row)?;
            match event.chain_hash.and_then(array) {
                Some(hash) => {
                    report.already += 1;
                    previous = hash;
                }
                None => {
                    let hash = link(&previous, &event.fields);
                    sqlx::query(
                        "UPDATE exchange_event SET chain_hash = $3
                         WHERE exchange_id = $1 AND sequence = $2 AND chain_hash IS NULL",
                    )
                    .bind(exchange)
                    .bind(event.fields.sequence)
                    .bind(hash.as_slice())
                    .execute(&mut *tx)
                    .await?;
                    report.chained += 1;
                    previous = hash;
                }
            }
        }
        tx.commit().await?;
    }
    Ok(report)
}

/// The daily anchor: how many exchanges have a chained history, and the
/// SHA-256 of their last hashes, in exchange ID order, one after the other.
/// Nothing in it is personal.
#[derive(Debug, PartialEq, Eq)]
pub struct Anchor {
    pub exchanges: u64,
    pub digest: [u8; 32],
}

pub async fn anchor(db: &PgPool) -> Result<Anchor, sqlx::Error> {
    let rows: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT DISTINCT ON (exchange_id) chain_hash
         FROM exchange_event
         WHERE chain_hash IS NOT NULL
         ORDER BY exchange_id, sequence DESC",
    )
    .fetch_all(db)
    .await?;
    let mut hasher = Sha256::new();
    for hash in &rows {
        hasher.update(hash);
    }
    Ok(Anchor {
        exchanges: rows.len() as u64,
        digest: hasher.finalize().into(),
    })
}

/// The last hash of one exchange, if its history is chained to the end.
pub async fn last_hash(
    conn: &mut PgConnection,
    exchange: Uuid,
) -> Result<Option<(i64, [u8; 32])>, sqlx::Error> {
    let row: Option<(i64, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT sequence, chain_hash FROM exchange_event
         WHERE exchange_id = $1 ORDER BY sequence DESC LIMIT 1",
    )
    .bind(exchange)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|(sequence, hash)| Some((sequence, array(hash?)?))))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> EventFields {
        EventFields {
            exchange_id: Uuid::from_u128(1),
            sequence: 1,
            kind: "CREATED".into(),
            actor_kind: "SYSTEM".into(),
            actor_slot: None,
            contribution_id: None,
            revision_id: None,
            note: None,
            evidence_attachment_id: None,
            data: serde_json::json!({ "b": 1, "a": [true, null] }),
            occurred_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn every_field_and_the_row_before_change_the_hash() {
        let base = link(&genesis(), &event());
        assert_eq!(base, link(&genesis(), &event()));
        assert_ne!(base, link(&[1; 32], &event()));
        let mut changed = event();
        changed.note = Some(String::new());
        assert_ne!(base, link(&genesis(), &changed));
        let mut changed = event();
        changed.sequence = 2;
        assert_ne!(base, link(&genesis(), &changed));
        let mut changed = event();
        changed.data = serde_json::json!({ "a": [true, null], "b": 2 });
        assert_ne!(base, link(&genesis(), &changed));
    }

    #[test]
    fn the_order_of_keys_in_the_data_does_not_matter() {
        let mut other = event();
        other.data = serde_json::json!({ "a": [true, null], "b": 1 });
        assert_eq!(link(&genesis(), &event()), link(&genesis(), &other));
    }
}
