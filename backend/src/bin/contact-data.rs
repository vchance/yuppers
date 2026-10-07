//! Email addresses and phone numbers at rest (`yuppers_backend::contact`;
//! docs/operations.md, "Contact data key"). The owner runs it, never the
//! service: it connects with `MIGRATION_DATABASE_URL`, the schema owner's
//! connection, because only the owner may change the records of consent.
//!
//! ```sh
//! cargo run --bin contact-data -- status   # how many values are under which key
//! cargo run --bin contact-data -- rotate   # re-encrypt under CONTACT_DATA_KEY
//! ```
//!
//! `rotate` re-encrypts every value not under `CONTACT_DATA_KEY`, which
//! `CONTACT_DATA_KEY_PREVIOUS` must decrypt, and the blind-index key with
//! them, and prints the counts. It exits with status 1 if any value could
//! not be decrypted, which is then left as it was:
//! `CONTACT_DATA_KEY_PREVIOUS` must stay until a run reports none. Running
//! it again changes nothing. The blind indexes do not change, so nothing has
//! to be recomputed and nothing waits on it.
//!
//! The published development keys are refused here unless the database is
//! on this machine.

use std::process::ExitCode;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use yuppers_backend::config::contact_keys_for_database;
use yuppers_backend::contact::{rotate, store};

const USAGE: &str = "usage: contact-data status
       contact-data rotate";

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("contact-data: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Vec<String>) -> anyhow::Result<ExitCode> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if !matches!(args.as_slice(), ["status"] | ["rotate"]) {
        anyhow::bail!("{USAGE}");
    }

    dotenvy::dotenv().ok();
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .context("set MIGRATION_DATABASE_URL to the schema owner's connection string")?;
    let db = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .context("cannot connect with MIGRATION_DATABASE_URL")?;
    let config = contact_keys_for_database(&url)?;
    // Only the index key has to decrypt here; a value under a key that is
    // not configured is counted, not refused.
    let keys = store::open_index_only(&db, &config)
        .await
        .context("CONTACT_DATA_KEY")?;

    if args.as_slice() == ["rotate"] {
        let rotation = rotate::rotate(&db, &keys).await?;
        print!("{rotation}");
        if rotation.unreadable() > 0 {
            eprintln!(
                "contact-data: {} values decrypt under neither CONTACT_DATA_KEY nor \
                 CONTACT_DATA_KEY_PREVIOUS and were left as they were; keep the key that \
                 encrypted them as CONTACT_DATA_KEY_PREVIOUS and run this again",
                rotation.unreadable()
            );
            return Ok(ExitCode::FAILURE);
        }
        println!("every value is under CONTACT_DATA_KEY; CONTACT_DATA_KEY_PREVIOUS can be removed");
        return Ok(ExitCode::SUCCESS);
    }

    println!("{:<30} {:>12} {:>10}", "column", "current key", "other key");
    let mut other = 0;
    for (column, standing) in rotate::status(&db, &keys).await? {
        println!(
            "{:<30} {:>12} {:>10}",
            column.name(),
            standing.current_key,
            standing.other_key
        );
        other += standing.other_key;
    }
    println!(
        "CONTACT_DATA_KEY_PREVIOUS: {}",
        match (config.previous.is_some(), other) {
            (_, 0) if config.previous.is_some() => "set, and nothing is left under it: remove it",
            (_, 0) => "not set, and not needed",
            (true, _) => "set; run `contact-data rotate`",
            (false, _) => "not set, but values are under another key: set it and rotate",
        }
    );
    Ok(ExitCode::SUCCESS)
}
