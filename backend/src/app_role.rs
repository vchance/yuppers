//! Creating the application role before migrating, where nobody can run
//! `psql` first (docs/deploy-render.md).
//!
//! The migrations grant to `exchange_app` by name and fail if it does not
//! exist (backend/migrations/README.md). On a development machine
//! `docker/postgres-init.sql` creates it; on a managed database that hands
//! out one owner role, `migrate` can do it instead, when
//! `MIGRATE_CREATE_APP_ROLE=true` asks it to, with the password in
//! `APP_DB_PASSWORD`. Nothing changes unless that is set.
//!
//! - The role is created once, if it does not exist, able to log in and
//!   nothing more. A role that already exists is left as it is, its password
//!   included: changing it is a deliberate step (docs/deploy-render.md,
//!   "Rotating the application role's password").
//! - Either way, the role must be restricted: not a superuser, unable to
//!   create roles or databases, bypass row security or replicate, and a
//!   member of no other role: not of the role migrating, and not of the
//!   built-in ones that read or write every table or the server's files
//!   (`pg_read_all_data`, `pg_write_server_files`, ...). If it is any of
//!   those, `migrate` stops before applying anything, rather than grant to
//!   it. What it may do comes only from the migrations' grants.
//! - The password never reaches the server in clear text, not even in the
//!   statement a server might log: the role is created with a SCRAM-SHA-256
//!   verifier computed here, which PostgreSQL stores as it is.

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

/// The application role the migrations grant to.
pub const APP_ROLE: &str = "exchange_app";

/// The shortest password taken.
const MIN_PASSWORD: usize = 16;

/// PostgreSQL's own default for `scram_iterations`.
const SCRAM_ITERATIONS: u32 = 4096;

/// The application role's password, which cannot be printed.
pub struct AppRolePassword(String);

impl std::fmt::Debug for AppRolePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppRolePassword(..)")
    }
}

impl AppRolePassword {
    /// At least 16 characters, each printable ASCII other than a space.
    /// Printable ASCII is what PostgreSQL's SASLprep leaves unchanged, so
    /// the verifier computed here is the one the server would compute; and
    /// what every way of writing a connection string can carry.
    pub fn new(password: String) -> anyhow::Result<Self> {
        if password.len() < MIN_PASSWORD {
            bail!("APP_DB_PASSWORD must be at least {MIN_PASSWORD} characters");
        }
        if !password.bytes().all(|byte| byte.is_ascii_graphic()) {
            bail!("APP_DB_PASSWORD must be printable ASCII, without spaces");
        }
        Ok(Self(password))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// What [`ensure`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    AlreadyExisted,
}

/// Creates `role` with `password` if there is no such role, and checks that
/// the role, new or not, may log in and do nothing more.
pub async fn ensure(
    pool: &PgPool,
    role: &str,
    password: &AppRolePassword,
) -> anyhow::Result<Outcome> {
    let outcome = if exists(pool, role).await? {
        Outcome::AlreadyExisted
    } else {
        let statement: String = sqlx::query_scalar(
            "SELECT format('CREATE ROLE %I LOGIN PASSWORD %L', $1::text, $2::text)",
        )
        .bind(role)
        .bind(scram_verifier(password.expose(), &salt()))
        .fetch_one(pool)
        .await?;
        match sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
        {
            Ok(_) => Outcome::Created,
            // Created by another `migrate` at the same moment.
            Err(error) if sqlstate(&error) == Some("42710") => Outcome::AlreadyExisted,
            Err(error) if sqlstate(&error) == Some("42501") => {
                return Err(error).with_context(|| {
                    format!(
                        "the role migrating may not create roles, so {role} cannot be created \
                         here; create it once with psql (docs/deploy-render.md)"
                    )
                });
            }
            Err(error) => return Err(error).with_context(|| format!("creating {role}")),
        }
    };
    check_restricted(pool, role).await?;
    Ok(outcome)
}

async fn exists(pool: &PgPool, role: &str) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_roles WHERE rolname = $1)")
            .bind(role)
            .fetch_one(pool)
            .await?,
    )
}

/// Refuses a role that can do more than log in.
async fn check_restricted(pool: &PgPool, role: &str) -> anyhow::Result<()> {
    #[allow(clippy::type_complexity)]
    let (login, superuser, create_role, create_db, bypass_rls, replication, member, member_of): (
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        Vec<String>,
    ) = sqlx::query_as(
        "SELECT r.rolcanlogin, r.rolsuper, r.rolcreaterole, r.rolcreatedb, r.rolbypassrls,
                r.rolreplication, pg_has_role(r.rolname, current_user, 'MEMBER'),
                ARRAY(SELECT g.rolname::text FROM pg_auth_members m
                      JOIN pg_roles g ON g.oid = m.roleid
                      WHERE m.member = r.oid ORDER BY 1)
         FROM pg_roles r WHERE r.rolname = $1",
    )
    .bind(role)
    .fetch_one(pool)
    .await?;
    let mut extra = Vec::new();
    for (has, what) in [
        (superuser, "SUPERUSER"),
        (create_role, "CREATEROLE"),
        (create_db, "CREATEDB"),
        (bypass_rls, "BYPASSRLS"),
        (replication, "REPLICATION"),
    ] {
        if has {
            extra.push(what.to_owned());
        }
    }
    if member {
        extra.push("membership of the role migrating".to_owned());
    }
    // Any other role's privileges would come with a membership, the
    // built-in ones (pg_read_all_data, pg_write_server_files,
    // pg_execute_server_program, ...) above all.
    if !member_of.is_empty() {
        extra.push(format!("membership of {}", member_of.join(", ")));
    }
    if !extra.is_empty() {
        bail!(
            "{role} must be able to log in and nothing more, but has {}; nothing was migrated",
            extra.join(", ")
        );
    }
    if !login {
        bail!("{role} exists but cannot log in (LOGIN); nothing was migrated");
    }
    Ok(())
}

fn sqlstate(error: &sqlx::Error) -> Option<&str> {
    error
        .as_database_error()
        .and_then(|error| match error.code() {
            Some(std::borrow::Cow::Borrowed(code)) => Some(code),
            _ => None,
        })
}

fn salt() -> [u8; 16] {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).expect("the operating system provides randomness");
    salt
}

/// The SCRAM-SHA-256 verifier PostgreSQL stores for `password` (RFC 5802,
/// RFC 7677): `SCRAM-SHA-256$<iterations>:<salt>$<StoredKey>:<ServerKey>`.
fn scram_verifier(password: &str, salt: &[u8]) -> String {
    let salted = pbkdf2_sha256(password.as_bytes(), salt, SCRAM_ITERATIONS);
    let client_key = hmac_sha256(&salted, b"Client Key");
    let stored_key = Sha256::digest(client_key);
    let server_key = hmac_sha256(&salted, b"Server Key");
    format!(
        "SCRAM-SHA-256${SCRAM_ITERATIONS}:{}${}:{}",
        STANDARD.encode(salt),
        STANDARD.encode(stored_key),
        STANDARD.encode(server_key)
    )
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// PBKDF2 with HMAC-SHA-256, one block: SCRAM's `Hi()`.
fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut message = salt.to_vec();
    message.extend_from_slice(&1u32.to_be_bytes());
    let mut block = hmac_sha256(password, &message);
    let mut result = block;
    for _ in 1..iterations {
        block = hmac_sha256(password, &block);
        for (out, byte) in result.iter_mut().zip(block) {
            *out ^= byte;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_must_be_long_and_printable() {
        assert!(AppRolePassword::new("short".to_owned()).is_err());
        assert!(
            AppRolePassword::new("sixteen chars ok".to_owned()).is_err(),
            "a space"
        );
        assert!(AppRolePassword::new("sixteen-chärs-ok".to_owned()).is_err());
        let password = AppRolePassword::new("sixteen-chars-ok".to_owned()).unwrap();
        assert_eq!(format!("{password:?}"), "AppRolePassword(..)");
    }

    /// The inputs of RFC 7677's example (password "pencil", salt
    /// "W22ZaJ0SNY7soEsUEjb6gQ==", 4096 iterations), with the keys computed
    /// independently (Python's hashlib and hmac). `tests/app_role.rs` signs
    /// in with a role created this way, which is the proof that counts.
    #[test]
    fn the_verifier_matches_postgresql() {
        let salt = STANDARD.decode("W22ZaJ0SNY7soEsUEjb6gQ==").unwrap();
        assert_eq!(
            scram_verifier("pencil", &salt),
            "SCRAM-SHA-256$4096:W22ZaJ0SNY7soEsUEjb6gQ==$\
             WG5d8oPm3OtcPnkdi4Uo7BkeZkBFzpcXkuLmtbsT4qY=:\
             wfPLwcE6nTWhTAmQ7tl2KeoiWGPlZqQxSrmfPwDl2dU="
        );
    }

    #[test]
    fn pbkdf2_matches_its_test_vector() {
        // RFC 7914 §11: PBKDF2-HMAC-SHA256("passwd", "salt", 1).
        let derived = pbkdf2_sha256(b"passwd", b"salt", 1);
        assert_eq!(
            derived[..16],
            [
                0x55, 0xac, 0x04, 0x6e, 0x56, 0xe3, 0x08, 0x9f, 0xec, 0x16, 0x91, 0xc2, 0x25, 0x44,
                0xb6, 0x05
            ]
        );
    }
}
