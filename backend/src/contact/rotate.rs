//! Re-encrypting every stored contact detail under the current key
//! (`contact-data rotate`), and saying how many values each key holds
//! (`contact-data status`). The blind indexes never change
//! (`crate::contact`).

use std::fmt;

use sqlx::PgPool;

use super::{Field, Keys, Unreadable, store};

/// Rows re-encrypted per transaction.
const BATCH: i64 = 200;

/// A column that holds encrypted contact details, and the field its values
/// are bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub table: &'static str,
    pub column: &'static str,
    pub field: Field,
    /// Each value is also bound to its row's `id` ([`Field::row`]).
    pub by_row: bool,
}

impl Column {
    /// `table.column`, as reports name it.
    pub fn name(&self) -> String {
        format!("{}.{}", self.table, self.column)
    }
}

/// Every column that holds encrypted contact details.
pub const COLUMNS: [Column; 4] = [
    Column {
        table: "account",
        column: "email_encrypted",
        field: Field::ACCOUNT_EMAIL,
        by_row: false,
    },
    Column {
        table: "account",
        column: "phone_encrypted",
        field: Field::ACCOUNT_PHONE,
        by_row: false,
    },
    Column {
        table: "sms_consent",
        column: "phone_encrypted",
        field: Field::SMS_CONSENT_PHONE,
        by_row: true,
    },
    Column {
        table: "sms_code_consent",
        column: "phone_encrypted",
        field: Field::SMS_CODE_CONSENT_PHONE,
        by_row: true,
    },
];

/// What rotating did to one column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rotated {
    /// Values re-encrypted under the current key.
    pub reencrypted: u64,
    /// Values already under it.
    pub current: u64,
    /// Values no configured key decrypts: left as they were.
    pub unreadable: u64,
}

/// What `contact-data rotate` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rotation {
    pub columns: Vec<(Column, Rotated)>,
    /// Whether the index key was re-encrypted.
    pub index_key_rewrapped: bool,
}

impl Rotation {
    pub fn unreadable(&self) -> u64 {
        self.columns.iter().map(|(_, done)| done.unreadable).sum()
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (column, done) in &self.columns {
            writeln!(
                f,
                "{:<30} re-encrypted {:>6}  already current {:>6}  unreadable {:>4}",
                column.name(),
                done.reencrypted,
                done.current,
                done.unreadable
            )?;
        }
        writeln!(
            f,
            "index key: {}",
            if self.index_key_rewrapped {
                "re-encrypted"
            } else {
                "already under the current key"
            }
        )
    }
}

/// Re-encrypts every stored value, and the index key, under the current
/// key, as the schema owner, which may change the records of consent.
/// Values no configured key decrypts are counted and left as they were.
pub async fn rotate(owner: &PgPool, keys: &Keys) -> Result<Rotation, store::OpenError> {
    let mut rotation = Rotation::default();
    for target in COLUMNS {
        let Column { table, column, .. } = target;
        let id = if target.by_row { "id" } else { "NULL::bigint" };
        let current = i16::from(keys.current_id());
        let mut done = Rotated::default();
        let mut skipped: Vec<String> = Vec::new();
        loop {
            let mut tx = owner.begin().await?;
            let rows: Vec<(String, Option<i64>, Vec<u8>)> =
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT ctid::text, {id}, {column} FROM {table}
                     WHERE {column} IS NOT NULL AND get_byte({column}, 0) <> $1
                       AND NOT (ctid::text = ANY($3))
                     LIMIT $2
                     FOR UPDATE"
                )))
                .bind(current)
                .bind(BATCH)
                .bind(&skipped)
                .fetch_all(&mut *tx)
                .await?;
            let taken = rows.len();
            for (tid, row, sealed) in rows {
                let field = row.map_or(target.field, |row| target.field.row(row));
                match keys.open(field, &sealed) {
                    Ok(value) => {
                        sqlx::query(sqlx::AssertSqlSafe(format!(
                            "UPDATE {table} SET {column} = $2 WHERE ctid = $1::tid"
                        )))
                        .bind(&tid)
                        .bind(keys.seal(field, &value))
                        .execute(&mut *tx)
                        .await?;
                        done.reencrypted += 1;
                    }
                    Err(Unreadable) => {
                        done.unreadable += 1;
                        skipped.push(tid);
                    }
                }
            }
            tx.commit().await?;
            if taken < BATCH as usize {
                break;
            }
        }
        let under_current = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE {column} IS NOT NULL AND get_byte({column}, 0) = $1"
        )))
        .bind(current)
        .fetch_one(owner)
        .await?;
        done.current = u64::try_from(under_current)
            .unwrap_or(0)
            .saturating_sub(done.reencrypted);
        rotation.columns.push((target, done));
    }
    rotation.index_key_rewrapped = store::rewrap(owner, keys).await?;
    Ok(rotation)
}

/// How many values one column holds, under the current key and under any
/// other, which `CONTACT_DATA_KEY_PREVIOUS` must still decrypt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Standing {
    pub current_key: i64,
    pub other_key: i64,
}

/// How every column stands, in the order of [`COLUMNS`].
pub async fn status(db: &PgPool, keys: &Keys) -> Result<Vec<(Column, Standing)>, sqlx::Error> {
    let mut standing = Vec::new();
    for target in COLUMNS {
        let Column { table, column, .. } = target;
        let (current_key, other_key): (i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FILTER (WHERE get_byte({column}, 0) = $1),
                    count(*) FILTER (WHERE get_byte({column}, 0) <> $1)
             FROM {table} WHERE {column} IS NOT NULL"
        )))
        .bind(i16::from(keys.current_id()))
        .fetch_one(db)
        .await?;
        standing.push((
            target,
            Standing {
                current_key,
                other_key,
            },
        ));
    }
    Ok(standing)
}
