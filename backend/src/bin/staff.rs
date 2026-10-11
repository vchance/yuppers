//! Names who reviews abuse reports (DESIGN.md §9). The owner runs it, never
//! the service: it connects with `MIGRATION_DATABASE_URL`, the schema
//! owner's connection, because the application role can read the list of
//! reviewers and not change it.
//!
//! ```sh
//! cargo run --bin staff -- grant ana@example.com   # or a phone number, or an account ID
//! cargo run --bin staff -- revoke ana@example.com
//! cargo run --bin staff -- list
//! cargo run --bin staff -- backfill-chain   # chain history from before migration 0033
//! cargo run --bin staff -- verify-chain     # recompute every hash; exit 1 on a mismatch
//! ```
//!
//! The account must exist (sign in once first) and be active. Each grant and
//! revoke is written to the review history.

use std::process::ExitCode;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use yuppers_backend::chain;
use yuppers_backend::contact::{self, store};
use yuppers_backend::review;

const USAGE: &str = "usage: staff grant <email|phone|account-id>
       staff revoke <email|phone|account-id>
       staff list
       staff backfill-chain
       staff verify-chain";

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("staff: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Vec<String>) -> anyhow::Result<ExitCode> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if !matches!(
        args.as_slice(),
        ["grant", _] | ["revoke", _] | ["list"] | ["backfill-chain"] | ["verify-chain"]
    ) {
        anyhow::bail!("{USAGE}");
    }

    dotenvy::dotenv().ok();
    // Never DATABASE_URL: the service's role cannot write the list, and a
    // command that fell back to it would only fail later and less clearly.
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .context("set MIGRATION_DATABASE_URL to the schema owner's connection string")?;
    let db = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .context("cannot connect with MIGRATION_DATABASE_URL")?;

    // The history chain (`yuppers_backend::chain`) holds nothing personal, so
    // these need no contact key.
    match args.as_slice() {
        ["backfill-chain"] => {
            let done = chain::backfill(&db).await?;
            println!(
                "{} exchanges: {} entries chained, {} already were",
                done.exchanges, done.chained, done.already
            );
            return Ok(ExitCode::SUCCESS);
        }
        ["verify-chain"] => {
            let found = chain::verify(&db).await?;
            println!(
                "{} exchanges, {} entries, {} not chained yet",
                found.exchanges, found.events, found.unchained
            );
            for (exchange, sequence) in &found.mismatched {
                println!("MISMATCH exchange {exchange} entry {sequence}");
            }
            for (exchange, sequence) in &found.gaps {
                println!("GAP exchange {exchange} before entry {sequence}");
            }
            if found.unchained > 0 {
                println!("run `staff backfill-chain` to chain the rest");
            }
            return Ok(if found.intact() {
                println!("every chained entry matches");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        _ => {}
    }
    // Accounts are found by the blind index of their address, and the list
    // shows each reviewer's address masked (yuppers_backend::contact).
    let keys = yuppers_backend::config::contact_keys_from_env()?;
    contact::install(store::open(&db, &keys).await.context("CONTACT_DATA_KEY")?);

    match args.as_slice() {
        ["grant", who] => {
            if review::grant(&db, who).await? {
                println!("{who} now reviews reports");
            } else {
                println!("{who} already reviews reports");
            }
        }
        ["revoke", who] => {
            if review::revoke(&db, who).await? {
                println!("{who} no longer reviews reports");
            } else {
                println!("{who} did not review reports");
            }
        }
        _ => {
            let reviewers = review::reviewers(&db).await?;
            if reviewers.is_empty() {
                println!("nobody reviews reports yet");
            }
            for reviewer in reviewers {
                let contact = reviewer
                    .email
                    .or(reviewer.phone)
                    .unwrap_or_else(|| "(no address)".to_owned());
                println!(
                    "{}\t{}\t{}\t{}\tsince {}",
                    reviewer.account_id,
                    contact,
                    reviewer.name,
                    reviewer.status.to_lowercase(),
                    reviewer.granted_at.date(),
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
