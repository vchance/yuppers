//! Payment options (`payments`; migration 0026): saving them, showing them
//! on one agreement, and who sees them when. In the fence job Ben owes Ana
//! the payment, so Ana is the payee.

mod common;

use axum::http::{Method, StatusCode};
use common::{App, Deal, User};
use serde_json::{Value, json};
use yuppers_backend::contact::Field;
use yuppers_backend::payments::WRITES_PER_HOUR;

const DATABASE: &str = "yuppers_test_payments";

fn handles() -> Value {
    json!({
        "venmo": "@Ana-Fixes",
        "cash_app": "$AnaFixes",
        "paypal": "paypal.me/AnaFixes",
        "zelle": "(202) 555-0142",
    })
}

/// As stored: without `@` or `$`, the link's name alone, the number in
/// international form.
fn stored() -> Value {
    json!({
        "venmo": "Ana-Fixes",
        "cash_app": "AnaFixes",
        "paypal": "AnaFixes",
        "zelle": "+12025550142",
    })
}

/// Ben takes the link, Ana confirms him and Ben signs, as `App::active`.
async fn activate(app: &App, deal: &Deal) {
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    app.command(&deal.ben, &deal.exchange, common::accept(&deal.revision))
        .await
        .ok();
}

async fn put(app: &App, user: &User, path: &str, body: Value) -> common::Reply {
    app.call(Some(user), Method::PUT, path, Some(body), &[])
        .await
}

async fn save(app: &App, user: &User, body: Value) -> common::Reply {
    put(app, user, "/v1/me/payment-handles", body).await
}

async fn show(app: &App, user: &User, exchange: &str, on: bool) -> common::Reply {
    put(
        app,
        user,
        &format!("/v1/exchanges/{exchange}/payment-options"),
        json!({ "on": on }),
    )
    .await
}

fn options(view: &Value) -> (Value, Value) {
    (
        view["payment_options"]["shown"].clone(),
        view["payment_options"]["theirs"].clone(),
    )
}

#[tokio::test]
async fn options_are_saved_normalized_encrypted_and_can_be_removed() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;

    // None until saved.
    let none = app.get(&ana, "/v1/me/payment-handles").await.ok();
    assert_eq!(
        none,
        json!({ "venmo": null, "cash_app": null, "paypal": null, "zelle": null })
    );

    assert_eq!(save(&app, &ana, handles()).await.ok(), stored());
    assert_eq!(app.get(&ana, "/v1/me/payment-handles").await.ok(), stored());

    // Encrypted at rest, each bound to its column.
    let row: (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT venmo_encrypted, cash_app_encrypted, paypal_encrypted, zelle_encrypted
         FROM payment_handle WHERE account_id = $1",
    )
    .bind(ana.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    for (sealed, plain) in [
        (&row.0, "Ana-Fixes"),
        (&row.1, "AnaFixes"),
        (&row.2, "AnaFixes"),
        (&row.3, "+12025550142"),
    ] {
        assert!(
            !sealed
                .windows(plain.len())
                .any(|window| window == plain.as_bytes()),
            "{plain} is stored in the clear"
        );
    }
    assert_eq!(
        common::open(Field::PAYMENT_VENMO.owned_by(ana.id), &row.0),
        "Ana-Fixes"
    );
    assert_eq!(
        common::open(Field::PAYMENT_ZELLE.owned_by(ana.id), &row.3),
        "+12025550142"
    );

    // Saving again replaces: one left out is removed.
    let some = save(&app, &ana, json!({ "zelle": "Ana@Example.com" }))
        .await
        .ok();
    assert_eq!(
        some,
        json!({ "venmo": null, "cash_app": null, "paypal": null, "zelle": "ana@example.com" })
    );

    // Removed altogether.
    let reply = app
        .call(
            Some(&ana),
            Method::DELETE,
            "/v1/me/payment-handles",
            None,
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    let left: i32 = sqlx::query_scalar(
        "SELECT num_nonnulls(venmo_encrypted, cash_app_encrypted, paypal_encrypted,
                             zelle_encrypted)
         FROM payment_handle WHERE account_id = $1",
    )
    .bind(ana.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn an_option_that_is_not_one_is_refused_and_nothing_is_saved() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    save(&app, &ana, json!({ "venmo": "ana-fixes" })).await.ok();
    for bad in [
        json!({ "venmo": "ana" }),
        json!({ "venmo": "ana fixes" }),
        json!({ "cash_app": "$12345" }),
        json!({ "cash_app": "a".repeat(21) }),
        json!({ "paypal": "ana_fixes" }),
        json!({ "zelle": "+44 20 7946 0958" }),
        json!({ "zelle": "not an address" }),
        json!({ "venmo": "ana-fixes", "zelle": "+1 416 555 0142" }),
    ] {
        save(&app, &ana, bad.clone())
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    }
    assert_eq!(
        app.get(&ana, "/v1/me/payment-handles").await.ok()["venmo"],
        "ana-fixes",
        "what was saved stays"
    );
}

#[tokio::test]
async fn the_payer_sees_them_only_once_the_payee_shows_them_and_while_money_is_owed() {
    let app = App::start(DATABASE).await;
    let deal = app.negotiating().await;
    let Deal {
        ana,
        ben,
        exchange,
        repair,
        payment,
        ..
    } = &deal;

    // Off by default, and needing something saved to turn on.
    assert_eq!(
        options(&app.view(ana, exchange).await),
        (json!(false), Value::Null)
    );
    show(&app, ana, exchange, true)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    save(&app, ana, handles()).await.ok();
    // While negotiating, Ana can show them already; nothing is owed yet.
    show(&app, ana, exchange, true).await.ok();
    show(&app, ana, exchange, false).await.ok();
    activate(&app, &deal).await;
    let (ana, ben, exchange, repair, payment) = (ana, ben, exchange.as_str(), *repair, *payment);

    // Saving them shows them nowhere.
    assert_eq!(
        options(&app.view(ben, exchange).await),
        (json!(false), Value::Null)
    );

    // Ana shows them on this agreement: Ben, who owes her the payment, sees
    // them; Ana sees only that she shows them.
    assert_eq!(
        show(&app, ana, exchange, true).await.ok(),
        json!({ "on": true })
    );
    assert_eq!(
        options(&app.view(ana, exchange).await),
        (json!(true), Value::Null)
    );
    assert_eq!(
        options(&app.view(ben, exchange).await),
        (json!(false), stored())
    );
    // Nothing of them is in the history.
    let history = app
        .get(ben, &format!("/v1/exchanges/{exchange}/history"))
        .await
        .ok()
        .to_string();
    assert!(!history.contains("AnaFixes"), "{history}");

    // Ben showing his own does not show them to Ana, who owes him no money.
    save(&app, ben, json!({ "venmo": "ben-pays" })).await.ok();
    show(&app, ben, exchange, true).await.ok();
    assert_eq!(
        options(&app.view(ana, exchange).await),
        (json!(true), Value::Null)
    );

    // Once Ben says he paid, they are no longer shown; a dispute brings
    // them back.
    app.act(ben, exchange, repair, "CONFIRM").await.ok();
    app.act(ben, exchange, payment, "CLAIM").await.ok();
    assert_eq!(options(&app.view(ben, exchange).await).1, Value::Null);
    app.command(
        ana,
        exchange,
        json!({ "type": "CONTRIBUTION", "contribution": payment, "action": "DISPUTE",
                "note": "Nothing arrived" }),
    )
    .await
    .ok();
    assert_eq!(options(&app.view(ben, exchange).await).1, stored());

    // Turned off: gone from Ben's view at once. Neither way is a change
    // to the agreement: its version stays as it was.
    let version = app.view(ben, exchange).await["version"].clone();
    assert_eq!(
        show(&app, ana, exchange, false).await.ok(),
        json!({ "on": false })
    );
    assert_eq!(options(&app.view(ben, exchange).await).1, Value::Null);
    show(&app, ana, exchange, true).await.ok();
    assert_eq!(options(&app.view(ben, exchange).await).1, stored());
    assert_eq!(app.view(ben, exchange).await["version"], version);

    // Changed in the account: the new ones are what Ben sees.
    save(&app, ana, json!({ "zelle": "ana@example.com" }))
        .await
        .ok();
    assert_eq!(
        options(&app.view(ben, exchange).await).1,
        json!({ "venmo": null, "cash_app": null, "paypal": null, "zelle": "ana@example.com" })
    );
    // All removed: shown nowhere, and saving again later does not bring
    // them back without Ana's say-so.
    app.call(
        Some(ana),
        Method::DELETE,
        "/v1/me/payment-handles",
        None,
        &[],
    )
    .await;
    save(&app, ana, handles()).await.ok();
    assert_eq!(
        options(&app.view(ben, exchange).await),
        (json!(true), Value::Null)
    );
    assert_eq!(options(&app.view(ana, exchange).await).0, false);

    // Nobody else can turn them on or off for an agreement that is not theirs.
    let cleo = app.user("Cleo").await;
    save(&app, &cleo, json!({ "venmo": "cleo-pays" }))
        .await
        .ok();
    show(&app, &cleo, exchange, true)
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    show(&app, &cleo, "not-an-id", false)
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
}

#[tokio::test]
async fn a_draft_or_a_closed_agreement_cannot_show_them_but_can_stop() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    save(&app, &ana, handles()).await.ok();
    let draft = app.draft(&ana).await;
    show(&app, &ana, &draft, true)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    show(&app, &ana, &draft, false).await.ok();
}

#[tokio::test]
async fn nothing_of_them_reaches_a_notification() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    save(&app, &ana, handles()).await.ok();
    let deal = app.active_between(ana, app.user("Ben").await).await;
    show(&app, &deal.ana, &deal.exchange, true).await.ok();
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    app.act(&deal.ben, &deal.exchange, deal.payment, "CLAIM")
        .await
        .ok();
    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload::text FROM outbox
         WHERE exchange_id = $1::uuid",
    )
    .bind(&deal.exchange)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert!(!payloads.is_empty());
    for payload in payloads {
        for handle in ["Ana-Fixes", "AnaFixes", "2025550142"] {
            assert!(!payload.contains(handle), "{payload}");
        }
    }
}

#[tokio::test]
async fn deleting_the_account_removes_them_and_stops_showing_them() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    save(&app, &ana, handles()).await.ok();
    let deal = app.active_between(ana, app.user("Ben").await).await;
    show(&app, &deal.ana, &deal.exchange, true).await.ok();
    assert_eq!(
        options(&app.view(&deal.ben, &deal.exchange).await).1,
        stored()
    );

    yuppers_backend::deletion::delete_account(&app.db, &app.rules, deal.ana.id)
        .await
        .unwrap();

    let (handles_left, offers_left): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM payment_handle WHERE account_id = $1),
                (SELECT count(*) FROM payment_offer WHERE account_id = $1)",
    )
    .bind(deal.ana.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((handles_left, offers_left), (0, 0));
    assert_eq!(
        options(&app.view(&deal.ben, &deal.exchange).await).1,
        Value::Null
    );
}

#[tokio::test]
async fn a_suspended_payee_s_options_are_not_shown() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    save(&app, &ana, handles()).await.ok();
    let deal = app.active_between(ana, app.user("Ben").await).await;
    show(&app, &deal.ana, &deal.exchange, true).await.ok();
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1")
        .bind(deal.ana.id)
        .execute(&app.owner)
        .await
        .unwrap();
    assert_eq!(
        options(&app.view(&deal.ben, &deal.exchange).await).1,
        Value::Null
    );
}

#[tokio::test]
async fn changes_are_limited_per_hour() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    let deal = app.active_between(ana, app.user("Ben").await).await;
    save(&app, &deal.ana, handles()).await.ok();
    for n in 1..WRITES_PER_HOUR {
        show(&app, &deal.ana, &deal.exchange, n % 2 == 1).await.ok();
    }
    // Saving, removing and showing all count, refused or not.
    save(&app, &deal.ana, handles())
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    show(&app, &deal.ana, &deal.exchange, false)
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    let reply = app
        .call(
            Some(&deal.ana),
            Method::DELETE,
            "/v1/me/payment-handles",
            None,
            &[],
        )
        .await;
    reply.refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    // Reading is not limited.
    app.get(&deal.ana, "/v1/me/payment-handles").await.ok();

    // An hour on, the count starts again.
    sqlx::query(
        "UPDATE payment_handle SET write_window_started_at = now() - interval '61 minutes'
         WHERE account_id = $1",
    )
    .bind(deal.ana.id)
    .execute(&app.owner)
    .await
    .unwrap();
    save(&app, &deal.ana, handles()).await.ok();
}

#[tokio::test]
async fn reading_and_writing_needs_a_session() {
    let app = App::start(DATABASE).await;
    for (method, path) in [
        (Method::GET, "/v1/me/payment-handles"),
        (Method::PUT, "/v1/me/payment-handles"),
        (Method::DELETE, "/v1/me/payment-handles"),
    ] {
        app.call(None, method, path, Some(json!({})), &[])
            .await
            .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    }
}

fn changed(view: &Value) -> Value {
    view["payment_options"]["theirs_changed"].clone()
}

#[tokio::test]
async fn a_payer_is_warned_beside_an_option_changed_since_the_agreement_came_into_force() {
    let app = App::start(DATABASE).await;
    let ana = app.user("Ana").await;
    // Saved before the agreement: nothing to warn about.
    save(&app, &ana, handles()).await.ok();
    let deal = app.active_between(ana, app.user("Ben").await).await;
    let (ana, ben, exchange) = (&deal.ana, &deal.ben, deal.exchange.as_str());
    show(&app, ana, exchange, true).await.ok();
    let nothing = json!({ "venmo": null, "cash_app": null, "paypal": null, "zelle": null });
    assert_eq!(changed(&app.view(ben, exchange).await), nothing);

    // Saving the same again changes nothing.
    save(&app, ana, handles()).await.ok();
    assert_eq!(changed(&app.view(ben, exchange).await), nothing);

    // The Venmo username changes while Ben owes the payment: said beside
    // it, with when, and never what it was.
    let mut new = handles();
    new["venmo"] = json!("someone-else");
    save(&app, ana, new.clone()).await.ok();
    let view = app.view(ben, exchange).await;
    let said = changed(&view);
    assert!(
        said["venmo"]
            .as_str()
            .is_some_and(|at| at.starts_with("20")),
        "{said}"
    );
    assert_eq!(
        (&said["cash_app"], &said["paypal"], &said["zelle"]),
        (&Value::Null, &Value::Null, &Value::Null)
    );
    assert_eq!(view["payment_options"]["theirs"]["venmo"], "someone-else");
    assert!(!view.to_string().contains("Ana-Fixes"), "{view}");
    // Ana's own view says nothing of it.
    assert_eq!(changed(&app.view(ana, exchange).await), nothing);

    // An old agreement: it came into force 60 days ago (history is
    // append-only, so its guard is lifted for this one move, in a database
    // of the tests' own).
    let mut tx = app.owner.begin().await.unwrap();
    for statement in [
        "ALTER TABLE exchange_event DISABLE TRIGGER exchange_event_append_only",
        "UPDATE exchange_event SET occurred_at = occurred_at - interval '60 days'
         WHERE exchange_id = $1::uuid",
        "ALTER TABLE exchange_event ENABLE TRIGGER exchange_event_append_only",
    ] {
        let query = sqlx::query(statement);
        let query = if statement.contains("$1") {
            query.bind(exchange)
        } else {
            query
        };
        query.execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let backdate = |days: i32| {
        sqlx::query(
            // The others as saved before the agreement, now 61 days ago.
            "UPDATE payment_handle
             SET venmo_changed_at = now() - make_interval(days => $2),
                 cash_app_changed_at = now() - interval '61 days',
                 paypal_changed_at = now() - interval '61 days',
                 zelle_changed_at = now() - interval '61 days'
             WHERE account_id = $1",
        )
        .bind(ana.id)
        .bind(days)
    };
    // Changed 30 days into it, 30 days ago: still said, however long ago.
    backdate(30).execute(&app.owner).await.unwrap();
    let said = changed(&app.view(ben, exchange).await);
    assert!(said["venmo"].is_string(), "{said}");
    assert_eq!(
        (&said["cash_app"], &said["paypal"], &said["zelle"]),
        (&Value::Null, &Value::Null, &Value::Null)
    );
    // Changed before it came into force: nothing to say.
    backdate(61).execute(&app.owner).await.unwrap();
    assert_eq!(changed(&app.view(ben, exchange).await), nothing);

    // Removed, then added again: a change too.
    save(&app, ana, json!({ "cash_app": "AnaFixes" }))
        .await
        .ok();
    save(&app, ana, new).await.ok();
    let said = changed(&app.view(ben, exchange).await);
    assert!(said["venmo"].is_string(), "{said}");
    assert!(said["paypal"].is_string(), "{said}");
    assert!(said["cash_app"].is_null(), "{said}");
}
