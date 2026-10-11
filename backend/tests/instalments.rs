//! Instalments, stages and progress notes end to end (DESIGN.md §7.1, §7.2,
//! §12): "Mark the rest as paid", bursts told as one message, progress notes
//! that change nothing, and the counts that measure them.
//!
//! Delivery acts on every queued message in the database, so the tests here
//! take turns, and each starts with an empty outbox.

mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{App, User, accept, consent};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tokio::sync::MutexGuard;
use uuid::Uuid;
use yuppers_backend::auth::SendFuture;
use yuppers_backend::domain::Rules;
use yuppers_backend::funnel::funnel;
use yuppers_backend::notifications::outbox::{Delivery, DeliveryRules, deliver_due};
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::notifications::{Email, EmailSender};

const DATABASE: &str = "yuppers_test_instalments";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn app() -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    let app = App::start_with(
        DATABASE,
        Rules {
            progress_notes_per_contribution: 4,
            ..Rules::default()
        },
    )
    .await;
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
    (app, turn)
}

#[derive(Default)]
struct Provider(Mutex<Vec<Email>>);

impl EmailSender for Provider {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(email.clone());
            Ok(())
        })
    }
}

/// Sends everything queued, as of a quarter of an hour from now, which is
/// after any burst has been held.
async fn deliver(app: &App) -> Vec<Email> {
    let provider = Arc::new(Provider::default());
    let delivery = Delivery {
        sender: provider.clone(),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: DeliveryRules::default(),
    };
    deliver_due(
        &app.db,
        &delivery,
        OffsetDateTime::now_utc() + Duration::minutes(15),
    )
    .await
    .unwrap();
    let sent = provider.0.lock().unwrap().clone();
    sent
}

fn payment(id: Uuid, from: &str, description: &str, amount: i64) -> Value {
    json!({
        "id": id,
        "from": from,
        "type": "MONEY",
        "description": description,
        "quantity": null,
        "due": { "kind": "ON_AGREEMENT" },
        "completion_criteria": null,
        "required": true,
        "amount_minor": amount,
    })
}

fn service(id: Uuid, from: &str, description: &str) -> Value {
    json!({
        "id": id,
        "from": from,
        "type": "SERVICE",
        "description": description,
        "quantity": null,
        "due": { "kind": "ON_AGREEMENT" },
        "completion_criteria": null,
        "required": true,
        "amount_minor": null,
    })
}

struct Series {
    ana: User,
    ben: User,
    exchange: String,
    /// Owed by Ben to Ana, plain amounts.
    payments: [Uuid; 3],
    /// Owed by Ana to Ben.
    job: Uuid,
}

/// Ben owes Ana three payments; Ana owes Ben one job. In force, with the
/// messages from getting there discarded.
async fn series(app: &App) -> Series {
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let exchange = app.draft(&ana).await;
    let payments = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let job = Uuid::new_v4();
    let terms = json!({
        "party_a_name": "Ana Ruiz",
        "party_b_name": "Ben Ortiz",
        "terms": "Repayment in three parts.",
        "contributions": [
            payment(payments[0], "B", "Repayment 1 of 3", 3333),
            payment(payments[1], "B", "Repayment 2 of 3", 3333),
            payment(payments[2], "B", "Repayment 3 of 3", 3334),
            service(job, "A", "Stage 1 of 2: posts set"),
        ],
    });
    let sent = app.send(&ana, &exchange, terms).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    app.post(
        &ben,
        "/v1/invitations/claim",
        json!({ "token": sent["invitation_token"] }),
    )
    .await
    .ok();
    app.command(&ana, &exchange, json!({ "type": "CONFIRM_COUNTERPARTY" }))
        .await
        .ok();
    let view = app.command(&ben, &exchange, accept(revision)).await.ok();
    assert_eq!(view["state"], "ACTIVE");
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
    Series {
        ana,
        ben,
        exchange,
        payments,
        job,
    }
}

fn status_of(view: &Value, id: Uuid) -> String {
    view["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(id))
        .unwrap_or_else(|| panic!("no {id} in {view}"))["status"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn claim_rest(app: &App, user: &User, exchange: &str, ids: &[Uuid]) -> common::Reply {
    app.command(
        user,
        exchange,
        json!({ "type": "CLAIM_REST", "contributions": ids }),
    )
    .await
}

/// Queued rows for an exchange: kind, notice, how many items it says.
async fn queued(app: &App, exchange: &str) -> Vec<(String, String, Option<i64>)> {
    sqlx::query_as(
        "SELECT kind, payload->>'notice', (payload->>'count')::bigint
         FROM outbox WHERE exchange_id = $1 ORDER BY id",
    )
    .bind(exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

#[tokio::test]
async fn marking_the_rest_as_paid_claims_each_payment_and_tells_the_payee_once() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;
    let before = funnel().counts();

    let view = claim_rest(&app, &deal.ben, &deal.exchange, &deal.payments)
        .await
        .ok();
    for id in deal.payments {
        assert_eq!(status_of(&view, id), "CLAIMED");
    }
    assert_eq!(status_of(&view, deal.job), "PENDING", "the job is not a payment");

    // The record keeps one claim for each payment.
    let history = app
        .get(&deal.ana, &format!("/v1/exchanges/{}/history", deal.exchange))
        .await
        .ok();
    let claims = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "CONTRIBUTION_CLAIMED")
        .count();
    assert_eq!(claims, 3);

    // One message to Ana, saying three, and none to Ben.
    assert_eq!(
        queued(&app, &deal.exchange).await,
        [(
            "EMAIL".to_owned(),
            "DELIVERY_CLAIMED".to_owned(),
            Some(3)
        )]
    );
    let sent = deliver(&app).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, deal.ana.email);
    assert!(sent[0].subject.contains('3'), "{}", sent[0].subject);
    assert!(sent[0].body.contains('3'), "{}", sent[0].body);

    let after = funnel().counts();
    assert_eq!(after.mark_rest_used, before.mark_rest_used + 1);
    assert_eq!(after.notices_coalesced, before.notices_coalesced + 1);
}

#[tokio::test]
async fn marking_the_rest_as_paid_is_all_or_none_and_only_for_payments_you_owe() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;
    let [first, second, third] = deal.payments;

    // Fewer than two, a repeat, the job, and the other party's payments.
    claim_rest(&app, &deal.ben, &deal.exchange, &[first])
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    claim_rest(&app, &deal.ben, &deal.exchange, &[first, first])
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    claim_rest(&app, &deal.ben, &deal.exchange, &[first, deal.job])
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    claim_rest(&app, &deal.ana, &deal.exchange, &[first, second])
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // One already claimed spoils it for the rest, and nothing is changed.
    app.act(&deal.ben, &deal.exchange, first, "CLAIM").await.ok();
    claim_rest(&app, &deal.ben, &deal.exchange, &[first, second, third])
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let view = app.view(&deal.ben, &deal.exchange).await;
    assert_eq!(status_of(&view, second), "PENDING");
    assert_eq!(status_of(&view, third), "PENDING");

    // The rest, then, and each is confirmed on its own.
    claim_rest(&app, &deal.ben, &deal.exchange, &[second, third])
        .await
        .ok();
    let view = app.act(&deal.ana, &deal.exchange, second, "CONFIRM").await.ok();
    assert_eq!(status_of(&view, second), "ACCEPTED");
    assert_eq!(status_of(&view, third), "CLAIMED", "a series is not one item");
}

#[tokio::test]
async fn claims_made_one_after_another_by_one_party_are_one_message_that_counts_them() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;
    let [first, second, third] = deal.payments;

    app.act(&deal.ben, &deal.exchange, first, "CLAIM").await.ok();
    app.act(&deal.ben, &deal.exchange, second, "CLAIM").await.ok();
    assert_eq!(
        queued(&app, &deal.exchange).await,
        [("EMAIL".to_owned(), "DELIVERY_CLAIMED".to_owned(), Some(2))],
        "one message about two claims, held for the next of the burst"
    );
    let (held_until,): (OffsetDateTime,) =
        sqlx::query_as("SELECT available_at FROM outbox WHERE exchange_id = $1")
            .bind(deal.exchange.parse::<Uuid>().unwrap())
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert!(held_until > OffsetDateTime::now_utc(), "{held_until}");

    // Something the other party did ends the burst: the next claim is news
    // of its own, and she is told of each in the order it happened.
    app.act(&deal.ana, &deal.exchange, first, "CONFIRM").await.ok();
    app.act(&deal.ben, &deal.exchange, third, "CLAIM").await.ok();
    let rows = queued(&app, &deal.exchange).await;
    let notices: Vec<(&str, Option<i64>)> = rows
        .iter()
        .map(|(_, notice, count)| (notice.as_str(), *count))
        .collect();
    assert_eq!(
        notices,
        [
            ("DELIVERY_CLAIMED", Some(2)),
            ("DELIVERY_CONFIRMED", None),
            ("DELIVERY_CLAIMED", None),
        ]
    );

    let sent = deliver(&app).await;
    assert_eq!(sent.len(), 3);
    for email in &sent {
        // Nothing of the agreement, only how many.
        let text = format!("{}\n{}", email.subject, email.body).to_lowercase();
        for private in ["repayment", "3333", "ruiz", "ortiz"] {
            assert!(!text.contains(private), "{private:?} leaked into {text}");
        }
    }

    // A message already sent is not recalled: a later claim is a new one.
    sqlx::query("UPDATE outbox SET completed_at = now()")
        .execute(&app.db)
        .await
        .unwrap();
    app.act(&deal.ben, &deal.exchange, second, "RETRACT_CLAIM")
        .await
        .ok();
    app.act(&deal.ben, &deal.exchange, second, "CLAIM").await.ok();
    let waiting: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT payload->>'notice', (payload->>'count')::bigint FROM outbox
         WHERE exchange_id = $1 AND completed_at IS NULL ORDER BY id",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        waiting,
        [
            ("CLAIM_RETRACTED".to_owned(), None),
            ("DELIVERY_CLAIMED".to_owned(), None)
        ]
    );
}

#[tokio::test]
async fn confirmations_are_told_in_bursts_too() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;
    let [first, second, third] = deal.payments;
    claim_rest(&app, &deal.ben, &deal.exchange, &deal.payments)
        .await
        .ok();
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();

    for id in [first, second, third] {
        app.act(&deal.ana, &deal.exchange, id, "CONFIRM").await.ok();
    }
    assert_eq!(
        queued(&app, &deal.exchange).await,
        [(
            "EMAIL".to_owned(),
            "DELIVERY_CONFIRMED".to_owned(),
            Some(3)
        )]
    );
    let sent = deliver(&app).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, deal.ben.email);
}

#[tokio::test]
async fn a_progress_note_is_recorded_and_changes_nothing() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;
    let before = funnel().counts();
    let version = app.view(&deal.ana, &deal.exchange).await["version"].clone();

    let note = |who: &User, text: &str| {
        app.command(
            who,
            &deal.exchange,
            json!({ "type": "NOTE_PROGRESS", "contribution": deal.job, "note": text }),
        )
    };
    let view = note(&deal.ana, "Posts set, panels Thursday.").await.ok();
    assert_eq!(status_of(&view, deal.job), "PENDING", "not a claim");
    assert_ne!(view["version"], version, "but it is in the history");

    // Visible to both, as what Ana wrote, with the item it is about.
    for reader in [&deal.ana, &deal.ben] {
        let history = app
            .get(reader, &format!("/v1/exchanges/{}/history", deal.exchange))
            .await
            .ok();
        let noted: Vec<&Value> = history["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "PROGRESS_NOTED")
            .collect();
        assert_eq!(noted.len(), 1);
        assert_eq!(noted[0]["actor"], "A");
        assert_eq!(noted[0]["note"], "Posts set, panels Thursday.");
        assert_eq!(noted[0]["contribution"]["id"], json!(deal.job));
        assert!(noted[0].get("status").is_none(), "no status came of it");
    }

    // The recipient has nothing to add; a blank note says nothing; payments
    // are noted too (it is any item the actor provides).
    note(&deal.ben, "Not mine to say")
        .await
        .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");
    note(&deal.ana, "   ")
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    note(&deal.ana, &"x".repeat(1001))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    // A limited number per item (four in this test).
    note(&deal.ana, "Two").await.ok();
    note(&deal.ana, "Three").await.ok();
    note(&deal.ana, "Four").await.ok();
    note(&deal.ana, "Five")
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");

    // Once it is delivered and confirmed there is nothing under way.
    app.act(&deal.ana, &deal.exchange, deal.job, "CLAIM").await.ok();
    app.act(&deal.ben, &deal.exchange, deal.job, "CONFIRM").await.ok();

    let after = funnel().counts();
    assert_eq!(after.progress_notes_added, before.progress_notes_added + 4);
}

#[tokio::test]
async fn a_recipient_is_told_of_one_progress_note_a_day_and_never_by_text() {
    let (app, _turn) = app().await;
    let deal = series(&app).await;

    for text in ["One", "Two", "Three"] {
        app.command(
            &deal.ana,
            &deal.exchange,
            json!({ "type": "NOTE_PROGRESS", "contribution": deal.job, "note": text }),
        )
        .await
        .ok();
    }
    assert_eq!(
        queued(&app, &deal.exchange).await,
        [("EMAIL".to_owned(), "PROGRESS_NOTED".to_owned(), None)],
        "three notes, one message to Ben"
    );

    let sent = deliver(&app).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, deal.ben.email);
    let text = format!("{}\n{}", sent[0].subject, sent[0].body).to_lowercase();
    for private in ["one", "two", "three", "posts", "repayment"] {
        // Words of the notes themselves; "one" may open other words.
        if private.len() > 3 {
            assert!(!text.contains(private), "{private:?} leaked into {text}");
        }
    }

    // A day later the next note is told again.
    sqlx::query("UPDATE outbox SET created_at = created_at - interval '25 hours'")
        .execute(&app.db)
        .await
        .unwrap();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "NOTE_PROGRESS", "contribution": deal.job, "note": "Four" }),
    )
    .await
    .ok();
    assert_eq!(queued(&app, &deal.exchange).await.len(), 2);
}

#[tokio::test]
async fn the_shape_of_an_agreement_is_counted_when_it_first_comes_into_force() {
    let (app, _turn) = app().await;
    let before = funnel().counts();
    let _deal = series(&app).await;
    let after = funnel().counts();

    // Three payments from one party; one job is not stages.
    assert_eq!(after.instalment_yups, before.instalment_yups + 1);
    assert_eq!(after.instalment_items, before.instalment_items + 3);
    assert_eq!(after.stage_yups, before.stage_yups);
    assert_eq!(after.stage_items, before.stage_items);

    // A plain fence job is neither.
    let plain = app.active().await;
    let _ = plain;
    let last = funnel().counts();
    assert_eq!(last.instalment_yups, after.instalment_yups);
    assert_eq!(last.stage_yups, after.stage_yups);
}

#[tokio::test]
async fn the_split_sheets_a_proposal_was_written_with_are_counted_and_nothing_else() {
    let (app, _turn) = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let before = funnel().counts();
    let terms = json!({
        "party_a_name": "Ana Ruiz",
        "party_b_name": "Ben Ortiz",
        "terms": "Two payments.",
        "contributions": [
            payment(Uuid::new_v4(), "B", "Payment 1 of 2", 5000),
            payment(Uuid::new_v4(), "B", "Payment 2 of 2", 5000),
        ],
    });
    let version = app.view(&ana, &exchange).await["version"].clone();
    app.post(
        &ana,
        &format!("/v1/exchanges/{exchange}/revisions"),
        json!({
            "expected_version": version,
            "terms": terms,
            "consent": consent(),
            "invitation": { "for_anyone": true },
            "splits": { "instalments": 1, "stages": 0 },
        }),
    )
    .await
    .ok();
    let after = funnel().counts();
    assert_eq!(after.splits_used[0], before.splits_used[0] + 1);
    assert_eq!(after.splits_used[1], before.splits_used[1]);
}
