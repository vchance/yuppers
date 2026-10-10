//! How a signature is attributed to a person (migration 0033; LEGAL_MEMO.md
//! §2.2): the kind of identifier and the session it was made in, kept with
//! the acceptance, and what the record says of it. Never the identifier.

mod common;

use common::{App, Deal};
use serde_json::Value;
use uuid::Uuid;
use yuppers_backend::chain;

const DATABASE: &str = "yuppers_test_attribution";

async fn record_of(app: &App, user: &common::User, exchange: &str) -> Value {
    app.get(user, &format!("/v1/exchanges/{exchange}/record"))
        .await
        .ok()
}

fn signatures(record: &Value) -> Vec<&Value> {
    record["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|revision| revision["signatures"].as_array().unwrap())
        .collect()
}

#[tokio::test]
async fn a_signature_keeps_the_kind_of_identifier_and_the_session_it_was_made_in() {
    let app = App::start(DATABASE).await;
    let deal = app.active().await;
    let exchange: Uuid = deal.exchange.parse().unwrap();

    let rows: Vec<(String, Option<Vec<u8>>, Option<String>, Option<time::OffsetDateTime>, Option<Uuid>, Uuid)> =
        sqlx::query_as(
            "SELECT slot, signer_identifier_hash, signer_identifier_kind, session_verified_at,
                    session_id, account_id
             FROM acceptance WHERE exchange_id = $1 ORDER BY slot",
        )
        .bind(exchange)
        .fetch_all(&app.owner)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for ((_, hash, kind, verified, session, account), user) in
        rows.iter().zip([&deal.ana, &deal.ben])
    {
        assert_eq!(*account, user.id);
        // The blind index of the address the session signed in with, not the
        // address.
        assert_eq!(hash.as_deref(), Some(common::index(&user.email).as_slice()));
        assert_ne!(hash.as_deref(), Some(user.email.as_bytes()));
        assert_eq!(kind.as_deref(), Some("email"));
        assert!(verified.is_some());
        let own: Uuid = sqlx::query_scalar("SELECT id FROM account_session WHERE account_id = $1")
            .bind(user.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
        assert_eq!(*session, Some(own));
    }
}

#[tokio::test]
async fn the_record_says_what_kind_of_identifier_and_how_recently_and_nothing_more() {
    let app = App::start(DATABASE).await;
    let deal: Deal = app.active().await;
    let record = record_of(&app, &deal.ana, &deal.exchange).await;

    let lines: Vec<&str> = signatures(&record)
        .iter()
        .map(|signature| signature["verification"]["attribution"].as_str().unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        assert!(line.contains("email address"), "{line}");
        assert!(line.contains("no more than 1 minute before signing"), "{line}");
    }
    // Nothing about who: neither the address nor its index is in the document.
    let text = record.to_string();
    assert!(!text.contains(&deal.ana.email), "{text}");
    assert!(!text.contains(&deal.ben.email), "{text}");
    assert!(!text.contains("signer_identifier"), "{text}");

    // In the reader's language.
    sqlx::query("UPDATE account SET language = 'es' WHERE id = $1")
        .bind(deal.ana.id)
        .execute(&app.owner)
        .await
        .unwrap();
    let spanish = record_of(&app, &deal.ana, &deal.exchange).await;
    let line = signatures(&spanish)[0]["verification"]["attribution"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(line.contains("correo electrónico"), "{line}");
    assert!(line.contains("1 minuto"), "{line}");
}

#[tokio::test]
async fn a_session_that_did_not_keep_its_identifier_signs_and_the_record_leaves_the_line_out() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    // Sessions from before migration 0033 have neither.
    sqlx::query("UPDATE account_session SET identifier_hash = NULL, identifier_kind = NULL WHERE account_id = $1")
        .bind(ana.id)
        .execute(&app.owner)
        .await
        .unwrap();
    let deal = app.active_between(ana, ben).await;
    let exchange: Uuid = deal.exchange.parse().unwrap();

    let ana_row: (Option<Vec<u8>>, Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT signer_identifier_hash, signer_identifier_kind, session_id
         FROM acceptance WHERE exchange_id = $1 AND account_id = $2",
    )
    .bind(exchange)
    .bind(deal.ana.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((ana_row.0, ana_row.1), (None, None));
    assert!(ana_row.2.is_some());

    let record = record_of(&app, &deal.ana, &deal.exchange).await;
    let with_line = signatures(&record)
        .iter()
        .filter(|signature| signature["verification"].get("attribution").is_some())
        .count();
    assert_eq!(with_line, 1, "only Ben's signature has a line");
}

#[tokio::test]
async fn the_record_ends_with_the_last_fingerprint_of_its_chained_history() {
    let app = App::start(DATABASE).await;
    let deal = app.active().await;
    let exchange: Uuid = deal.exchange.parse().unwrap();
    let record = record_of(&app, &deal.ben, &deal.exchange).await;

    let (sequence, last): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT sequence, chain_hash FROM exchange_event
         WHERE exchange_id = $1 ORDER BY sequence DESC LIMIT 1",
    )
    .bind(exchange)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    let footer = record["history_chain"].as_str().unwrap();
    assert!(footer.contains(&chain::hex(&last)), "{footer}");
    assert!(footer.contains(&format!("entry {sequence},")), "{footer}");
}
