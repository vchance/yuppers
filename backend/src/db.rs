use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::otel::{SpanExt, SpanKind};

/// A span for one database operation, named by what it does
/// (`exchange.load`, `outbox.claim`), never by the SQL, let alone its
/// parameters (`crate::otel`). Wrap the operation with
/// `tracing::Instrument::instrument`. The span has no fields, so a line
/// logged inside it carries no more than it did.
pub fn span(operation: &'static str) -> tracing::Span {
    let span = tracing::info_span!("db");
    span.otel_name(operation);
    span.otel_kind(SpanKind::Client);
    span.otel_attr("db.system.name", "postgresql");
    span.otel_attr("db.operation.name", operation);
    span
}

/// Migrations embedded at build time. Applied by the `migrate` binary, which
/// connects as the schema owner; the `api` and `worker` processes connect as
/// the restricted application role and never run them (DESIGN.md §13.2).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// What the api and the worker say when they refuse to start on a restored
/// database whose deletion log has not been replayed (migration 0018).
pub const REPLAY_PENDING: &str = "this database was restored from a backup, and the deletion log \
    has not been replayed since: accounts deleted after the backup are live again. Replay the \
    newest deletion log first, with scripts/replay-deletions.sh (in the image: DATABASE_URL=... \
    /usr/local/bin/replay-deletions FILE), then start again (docs/operations.md, \"Replaying \
    deletions\")";

/// Whether the database is a restored copy whose deletion log is still to be
/// replayed: `scripts/restore.sh` marks it so, and `replay-deletions` clears
/// the mark once every account in the log is deleted (migration 0018). The
/// api and the worker do not start while it stands.
pub async fn replay_pending(db: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT restore_replay_pending()")
        .fetch_one(db)
        .await
}

/// Records that the deletion log has been replayed, if the database was
/// waiting for that, and says whether it was. Refused by the database while
/// any account its own deletion log names is not deleted.
pub async fn mark_replayed(db: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT restore_replay_done()")
        .fetch_one(db)
        .await
}

/// Creates the connection pool without connecting. The first query opens a
/// connection, so the process can start and report readiness on its own terms.
pub fn pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect_lazy(database_url)
}
