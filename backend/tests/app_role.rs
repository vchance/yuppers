//! `migrate`'s optional first step, creating the application role
//! (`yuppers_backend::app_role`), against a real server.
//!
//! A role belongs to the whole cluster, so the test works on a role of its
//! own, never `exchange_app`: `TEST_APP_ROLE`, by default
//! `yuppers_test_app_role`, dropped before and after. It needs a connection
//! that may create roles; `MIGRATION_DATABASE_URL` is one in CI, where the
//! owner is the container's superuser. Where the owner may not (a
//! development machine set up as the README says), the test says so and
//! passes without checking anything.

use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};
use yuppers_backend::app_role::{self, AppRolePassword, Outcome};

fn env(name: &str) -> Option<String> {
    dotenvy::dotenv().ok();
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(1).connect(url).await
}

/// `url` with another user and password.
fn as_role(url: &str, role: &str, password: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap();
    let host = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    format!("{scheme}://{role}:{password}@{host}")
}

async fn drop_role(owner: &PgPool, role: &str) {
    let statement: String = sqlx::query_scalar("SELECT format('DROP ROLE IF EXISTS %I', $1::text)")
        .bind(role)
        .fetch_one(owner)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(owner)
        .await
        .unwrap();
}

async fn alter_role(owner: &PgPool, role: &str, attributes: &str) {
    let statement: String = sqlx::query_scalar("SELECT format('ALTER ROLE %I ', $1::text)")
        .bind(role)
        .fetch_one(owner)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("{statement}{attributes}")))
        .execute(owner)
        .await
        .unwrap();
}

#[tokio::test]
async fn migrate_creates_a_restricted_role_once_and_never_changes_it() {
    let url = env("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL must be set");
    let role = env("TEST_APP_ROLE").unwrap_or_else(|| "yuppers_test_app_role".to_owned());
    let owner = connect(&url).await.expect("the owner can connect");
    let row =
        sqlx::query("SELECT rolsuper, rolcreaterole FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&owner)
            .await
            .unwrap();
    if !row.get::<bool, _>(0) && !row.get::<bool, _>(1) {
        eprintln!("skipped: the role of MIGRATION_DATABASE_URL may not create roles");
        return;
    }
    drop_role(&owner, &role).await;

    let first = AppRolePassword::new("first-password-0123456789".to_owned()).unwrap();
    assert_eq!(
        app_role::ensure(&owner, &role, &first).await.unwrap(),
        Outcome::Created
    );

    // Signs in with the password it was given, and with no other.
    let signed_in = connect(&as_role(&url, &role, first.expose()))
        .await
        .expect("the new role signs in with its password");
    let (user, superuser): (String, bool) = sqlx::query_as(
        "SELECT current_user::text, rolsuper FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&signed_in)
    .await
    .unwrap();
    assert_eq!(user, role);
    assert!(!superuser);
    signed_in.close().await;
    // A server that trusts local connections (a development machine's
    // pg_hba.conf often does) lets any password in, and so cannot tell a
    // right one from a wrong one. CI's server checks them (scram-sha-256).
    let checks_passwords = connect(&as_role(&url, &role, "not-the-password-0123456789"))
        .await
        .is_err();
    if !checks_passwords {
        eprintln!("this server does not check passwords here; the password checks are skipped");
    }

    // A second run leaves the role and its password as they were.
    let second = AppRolePassword::new("second-password-0123456789".to_owned()).unwrap();
    assert_eq!(
        app_role::ensure(&owner, &role, &second).await.unwrap(),
        Outcome::AlreadyExisted
    );
    if checks_passwords {
        connect(&as_role(&url, &role, first.expose()))
            .await
            .expect("the first password still works")
            .close()
            .await;
        assert!(
            connect(&as_role(&url, &role, second.expose()))
                .await
                .is_err(),
            "the second run did not change the password"
        );
    }

    // A role that may do more than log in is refused, new or not.
    for (grant, revoke) in [
        ("CREATEDB", "NOCREATEDB"),
        ("CREATEROLE", "NOCREATEROLE"),
        ("NOLOGIN", "LOGIN"),
    ] {
        alter_role(&owner, &role, grant).await;
        let error = app_role::ensure(&owner, &role, &first).await.unwrap_err();
        assert!(error.to_string().contains(&role), "{grant}: {error:#}");
        alter_role(&owner, &role, revoke).await;
    }
    app_role::ensure(&owner, &role, &first).await.unwrap();

    // So is one that is a member of any other role, a built-in one that
    // reads every table above all.
    for granted in ["pg_read_all_data", "pg_write_server_files"] {
        grant_role(&owner, granted, &role, "GRANT", "TO").await;
        let error = app_role::ensure(&owner, &role, &first).await.unwrap_err();
        assert!(error.to_string().contains(granted), "{granted}: {error:#}");
        grant_role(&owner, granted, &role, "REVOKE", "FROM").await;
    }
    app_role::ensure(&owner, &role, &first).await.unwrap();

    drop_role(&owner, &role).await;
}

/// `GRANT granted TO role`, or `REVOKE granted FROM role`.
async fn grant_role(owner: &PgPool, granted: &str, role: &str, verb: &str, to: &str) {
    let statement: String =
        sqlx::query_scalar("SELECT format('%s %I %s %I', $1::text, $2::text, $3::text, $4::text)")
            .bind(verb)
            .bind(granted)
            .bind(to)
            .bind(role)
            .fetch_one(owner)
            .await
            .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(owner)
        .await
        .unwrap();
}
