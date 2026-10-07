//! Applies database migrations. Connects with `MIGRATION_DATABASE_URL` (the
//! schema owner), falling back to `DATABASE_URL` in development.
//!
//! With `MIGRATE_CREATE_APP_ROLE=true` it first creates the application role
//! `exchange_app` with the password in `APP_DB_PASSWORD`, if there is no such
//! role, and checks that the role can do nothing but log in
//! (`yuppers_backend::app_role`). For a managed database where nobody runs
//! `psql` before the first deploy (docs/deploy-render.md); off otherwise.
//!
//! After the migrations it stores the blind-index key, encrypted under
//! `CONTACT_DATA_KEY`, the first time (`yuppers_backend::contact::store`),
//! and checks the key against it every time after.

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use yuppers_backend::app_role::{self, APP_ROLE, Outcome};
use yuppers_backend::build_info::BuildInfo;
use yuppers_backend::config::MigrateConfig;
use yuppers_backend::contact::store;
use yuppers_backend::{db, telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;
    BuildInfo::current().log_start("migrate");
    let config = MigrateConfig::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await?;
    if let Some(password) = &config.create_app_role {
        match app_role::ensure(&pool, APP_ROLE, password).await? {
            Outcome::Created => tracing::info!(role = APP_ROLE, "application role created"),
            Outcome::AlreadyExisted => tracing::info!(
                role = APP_ROLE,
                "application role already exists; its password is left as it is"
            ),
        }
    }
    db::MIGRATOR.run(&pool).await?;
    tracing::info!("migrations applied");
    // Contact details at rest (yuppers_backend::contact): the blind-index
    // key, stored under CONTACT_DATA_KEY the first time; refused if the
    // key is not the one the database was given.
    store::bootstrap(&pool, &config.contact)
        .await
        .context("CONTACT_DATA_KEY")?;
    // A restored copy is migrated before its deletion log is replayed (the
    // replay runs the current code), so this does not refuse; it says what
    // is still to do, and the api and the worker refuse to start until then.
    if db::replay_pending(&pool).await? {
        tracing::warn!("{}", db::REPLAY_PENDING);
    }
    Ok(())
}
