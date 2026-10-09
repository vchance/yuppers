//! Applies a deletion log to a restored database (docs/operations.md,
//! "Restoring"; `scripts/replay-deletions.sh` runs it).
//!
//!     replay-deletions FILE
//!
//! Connects with `DATABASE_URL` (or `DATABASE_HOST`, `DATABASE_NAME` and
//! `APP_DB_PASSWORD`; `.env.example`), as the application role, so that it can do
//! nothing a person deleting their own account could not, and needs
//! `CONTACT_DATA_KEY`, the key the backup's contact details are under. Each account in
//! the log is deleted again through the service's own deletion
//! (`yuppers_backend::deletion::replay`); one already deleted, or one the
//! database never held, is reported and skipped, so replaying a file twice
//! changes nothing the second time. One suspended in the copy has its
//! suspension lifted, recorded in the review history as the owner's, and is
//! deleted in the same transaction. A line whose time the database
//! contradicts (before the account was created or last suspended there) is
//! reported and left alone. An account combined into one the copy does not
//! hold is followed through the later lines to where that one went; where
//! the log does not say, it is reported as UNRESOLVED and left as it is. It
//! prints a line for each account and a count at the end, and exits with
//! status 1 if any account is left undeleted (a failure, a line left alone,
//! or one unresolved), 2 if the file cannot be read or is
//! damaged, in which case nothing is changed.
//!
//! When every account in the log is deleted, it clears the mark that
//! `scripts/restore.sh` left in the database (migration 0018), so that the
//! api and the worker can start on it.

use std::process::ExitCode;

use anyhow::Context;
use yuppers_backend::contact::{self, store};
use yuppers_backend::domain::Rules;
use yuppers_backend::{db, deletion_log, telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    telemetry::init()?;
    dotenvy::dotenv().ok();
    // A deletion asks to close the account's agreements, which the other
    // party may have turned text updates on for.
    yuppers_backend::config::configure_sms_updates_from_env()?;

    let mut args = std::env::args().skip(1);
    let (Some(file), None) = (args.next(), args.next()) else {
        eprintln!("usage: replay-deletions FILE");
        return Ok(ExitCode::from(2));
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("replay-deletions: cannot read {file}: {error}");
            return Ok(ExitCode::from(2));
        }
    };
    let entries = match deletion_log::parse(&text) {
        Ok(entries) => entries,
        Err(bad) => {
            eprintln!("replay-deletions: {file}, {bad}; nothing was changed");
            return Ok(ExitCode::from(2));
        }
    };

    // DATABASE_URL, or the parts a platform gives (`config::database_url_from_env`).
    let url = yuppers_backend::config::database_url_from_env()?;
    let pool = db::pool(&url)?;
    // A deletion finds what named the account's address by its blind index
    // (yuppers_backend::contact), so the copy's CONTACT_DATA_KEY is needed:
    // the one the backup was made under.
    let keys = yuppers_backend::config::contact_keys_from_env()?;
    contact::install(
        store::open(&pool, &keys)
            .await
            .context("CONTACT_DATA_KEY")?,
    );
    println!("replaying {} deletions from {file}", entries.len());
    let summary = deletion_log::replay(&pool, &Rules::default(), &entries, |line| {
        println!("{line}")
    })
    .await;
    println!("{summary}");
    if summary.complete() {
        match db::mark_replayed(&pool).await {
            Ok(true) => println!(
                "the database is no longer marked as waiting for this replay; the api and the \
                 worker can start on it"
            ),
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "replay-deletions: every account in the log is deleted, but the database \
                     would not clear its mark ({}); the api and the worker will not start on it",
                    yuppers_backend::error::Redacted(&error)
                );
                return Ok(ExitCode::from(1));
            }
        }
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!(
            "replay-deletions: not every account in the log is deleted here; see above \
             (docs/operations.md, \"Restoring\")"
        );
        Ok(ExitCode::from(1))
    }
}
