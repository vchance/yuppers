//! The hash chain over each exchange's history (migration 0033;
//! `yuppers_backend::chain`; LEGAL_MEMO.md §2.4): computed as events are
//! written, backfilled for history from before it, verified, and the daily
//! anchor. One test, in a database of its own, because it damages the
//! history on purpose.

mod common;

use common::App;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use yuppers_backend::chain;

const DATABASE: &str = "yuppers_test_history_chain";

/// Runs `staff` as the owner would.
fn staff(app: &App, command: &str) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_staff"))
        .arg(command)
        .env("MIGRATION_DATABASE_URL", &app.owner_url)
        .env("CONTACT_DATA_KEY", common::CONTACT_DATA_KEY)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// Runs statements on history with its guards lifted, in a transaction, as
/// the payments test does for its one move.
async fn with_guards_lifted(app: &App, statement: &str, exchange: Uuid, sequence: Option<i64>) {
    let mut tx = app.owner.begin().await.unwrap();
    for lift in [
        "ALTER TABLE exchange_event DISABLE TRIGGER exchange_event_append_only",
        "ALTER TABLE exchange_event DISABLE TRIGGER exchange_event_chain_only",
    ] {
        sqlx::query(lift).execute(&mut *tx).await.unwrap();
    }
    sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
        .bind(exchange)
        .bind(sequence)
        .execute(&mut *tx)
        .await
        .unwrap();
    for restore in [
        "ALTER TABLE exchange_event ENABLE TRIGGER exchange_event_append_only",
        "ALTER TABLE exchange_event ENABLE TRIGGER exchange_event_chain_only",
    ] {
        sqlx::query(restore).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
}

async fn hashes(app: &App, exchange: Uuid) -> Vec<Option<Vec<u8>>> {
    sqlx::query_scalar(
        "SELECT chain_hash FROM exchange_event WHERE exchange_id = $1 ORDER BY sequence",
    )
    .bind(exchange)
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_history_is_chained_backfilled_verified_and_anchored() {
    let app = App::start(DATABASE).await;
    let deal = app.active().await;
    let exchange: Uuid = deal.exchange.parse().unwrap();
    let other = app.active().await;
    let other_exchange: Uuid = other.exchange.parse().unwrap();

    // Chained as it was written: each row from the one before it.
    let written = hashes(&app, exchange).await;
    assert!(written.len() >= 4);
    assert!(written.iter().all(Option::is_some));
    let mut distinct = written.clone();
    distinct.dedup();
    assert_eq!(distinct.len(), written.len());
    let report = chain::verify(&app.owner).await.unwrap();
    assert!(report.intact(), "{report:?}");
    assert_eq!(report.unchained, 0);

    // The application role cannot touch the chain, and the owner can only
    // fill an empty one, in a backfill.
    let refused = sqlx::query("UPDATE exchange_event SET chain_hash = $1 WHERE exchange_id = $2")
        .bind([0u8; 32].as_slice())
        .bind(exchange)
        .execute(&app.db)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("permission denied"),
        "{refused}"
    );
    let refused = sqlx::query("UPDATE exchange_event SET chain_hash = $1 WHERE exchange_id = $2")
        .bind([0u8; 32].as_slice())
        .bind(exchange)
        .execute(&app.owner)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("append-only"), "{refused}");

    // History from before the chain: the hashes are empty. A new event on it
    // goes on without one, and the check says what is left to do.
    with_guards_lifted(
        &app,
        "UPDATE exchange_event SET chain_hash = NULL WHERE exchange_id = $1 AND $2::bigint IS NULL",
        exchange,
        None,
    )
    .await;
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONTRIBUTION", "contribution": deal.repair, "action": "CLAIM",
                "note": "Rehung the gate." }),
    )
    .await
    .ok();
    let legacy = hashes(&app, exchange).await;
    assert!(legacy.iter().all(Option::is_none));
    let report = chain::verify(&app.owner).await.unwrap();
    assert!(report.intact(), "{report:?}");
    assert_eq!(report.unchained as usize, legacy.len());

    // The backfill, as the owner runs it: every row chained in order, the
    // rows that were chained already left alone, and the same hashes the
    // service computed as it wrote them.
    let (ok, output) = staff(&app, "backfill-chain");
    assert!(ok, "{output}");
    let backfilled = hashes(&app, exchange).await;
    assert!(backfilled.iter().all(Option::is_some));
    assert_eq!(backfilled[..written.len()], written[..]);
    let (ok, output) = staff(&app, "verify-chain");
    assert!(ok, "{output}");
    assert!(output.contains("0 not chained yet"), "{output}");
    let again = chain::backfill(&app.owner).await.unwrap();
    assert_eq!(again.chained, 0);
    assert_eq!(hashes(&app, exchange).await, backfilled);

    // Nothing but the hash changed, and a second exchange never mixes in.
    assert_ne!(hashes(&app, other_exchange).await[0], backfilled[0]);

    // The anchor: how many exchanges, and the digest of their last hashes in
    // order.
    let last: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT DISTINCT ON (exchange_id) chain_hash FROM exchange_event
         ORDER BY exchange_id, sequence DESC",
    )
    .fetch_all(&app.owner)
    .await
    .unwrap();
    let anchor = chain::anchor(&app.owner).await.unwrap();
    assert_eq!(anchor.exchanges as usize, last.len());
    let mut hasher = Sha256::new();
    for hash in &last {
        hasher.update(hash);
    }
    assert_eq!(anchor.digest.as_slice(), hasher.finalize().as_slice());

    // Tampering: one row's note is rewritten, with the guards lifted.
    with_guards_lifted(
        &app,
        "UPDATE exchange_event SET note = 'edited later' WHERE exchange_id = $1 AND sequence = $2",
        exchange,
        Some(3),
    )
    .await;
    let report = chain::verify(&app.owner).await.unwrap();
    assert_eq!(report.mismatched, [(exchange, 3)], "{report:?}");
    assert!(report.gaps.is_empty());
    let (ok, output) = staff(&app, "verify-chain");
    assert!(!ok, "{output}");
    assert!(
        output.contains(&format!("MISMATCH exchange {exchange} entry 3")),
        "{output}"
    );
    // The other exchange is unharmed.
    assert!(report.mismatched.iter().all(|(id, _)| *id == exchange));

    // The backfill takes the exchange row's lock, as a live insert does: with
    // it held, the backfill waits, and finishes once it is released.
    let mut hold = app.owner.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM exchange WHERE id = $1 FOR UPDATE")
        .bind(other_exchange)
        .execute(&mut *hold)
        .await
        .unwrap();
    let owner = app.owner.clone();
    let mut running = tokio::spawn(async move { chain::backfill(&owner).await });
    let waited = tokio::time::timeout(std::time::Duration::from_millis(500), &mut running).await;
    assert!(waited.is_err(), "the backfill did not wait for the lock");
    hold.commit().await.unwrap();
    running.await.unwrap().unwrap();

    // Deletions: the last event of an exchange removed, and all the events
    // of another. The walk starts from the exchanges, so both show as a gap.
    let (_, last): (Uuid, i64) = sqlx::query_as(
        "SELECT id, last_event_seq FROM exchange WHERE id = $1",
    )
    .bind(other_exchange)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    with_guards_lifted(
        &app,
        "DELETE FROM exchange_event WHERE exchange_id = $1 AND sequence = $2",
        other_exchange,
        Some(last),
    )
    .await;
    let report = chain::verify(&app.owner).await.unwrap();
    assert!(!report.intact());
    assert_eq!(report.gaps, [(other_exchange, last)], "{report:?}");
    let (ok, output) = staff(&app, "verify-chain");
    assert!(!ok, "{output}");

    let emptied = app.active().await;
    let emptied_exchange: Uuid = emptied.exchange.parse().unwrap();
    with_guards_lifted(
        &app,
        "DELETE FROM exchange_event WHERE exchange_id = $1 AND $2::bigint IS NULL",
        emptied_exchange,
        None,
    )
    .await;
    let report = chain::verify(&app.owner).await.unwrap();
    assert!(
        report.gaps.iter().any(|(id, _)| *id == emptied_exchange),
        "{report:?}"
    );
}
