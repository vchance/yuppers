//! Someone who opened an invitation that named nobody, before the initiator
//! has confirmed them (DESIGN.md §8): what they can do, how they are removed
//! or leave, and that nothing they did binds anyone afterwards.

mod common;

use axum::http::{Method, StatusCode};
use common::{App, Deal, Reply, User, accept, consent, fence_job};
use serde_json::{Value, json};
use uuid::Uuid;

const DATABASE: &str = "yuppers_test_claimant";

const FOREIGN_KEY: &str = "23503";
const CHECK: &str = "23514";

async fn app() -> App {
    App::start(DATABASE).await
}

fn id(exchange: &str) -> Uuid {
    exchange.parse().unwrap()
}

async fn claim(app: &App, user: &User, token: &str) -> Reply {
    app.post(user, "/v1/invitations/claim", json!({ "token": token }))
        .await
}

async fn leave(app: &App, user: &User, exchange: &str) -> Reply {
    app.call(
        Some(user),
        Method::POST,
        &format!("/v1/exchanges/{exchange}/leave"),
        None,
        &[],
    )
    .await
}

async fn confirm(app: &App, deal: &Deal) -> Reply {
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
}

async fn reject(app: &App, deal: &Deal) -> Reply {
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "REJECT_COUNTERPARTY" }),
    )
    .await
}

/// A new invitation link for the deal, which only its initiator can ask for.
async fn new_link(app: &App, deal: &Deal) -> Reply {
    app.post(
        &deal.ana,
        &format!("/v1/exchanges/{}/invitation", deal.exchange),
        json!({ "for_anyone": true }),
    )
    .await
}

#[track_caller]
fn gone(reply: &Reply) {
    reply.refused(StatusCode::NOT_FOUND, "NOT_FOUND");
}

/// Everything about an exchange that a party can ask for. To someone who is
/// not one, each must answer as if there were no such exchange.
async fn every_way_in(app: &App, user: &User, deal: &Deal) -> Vec<Reply> {
    let path = format!("/v1/exchanges/{}", deal.exchange);
    let version = app.view(&deal.ana, &deal.exchange).await["version"].clone();
    let command = |command: Value| json!({ "expected_version": version, "command": command });
    vec![
        app.get(user, &path).await,
        app.get(user, &format!("{path}/history")).await,
        app.get(user, &format!("{path}/record")).await,
        app.post(
            user,
            &format!("{path}/commands"),
            command(accept(&deal.revision)),
        )
        .await,
        app.post(
            user,
            &format!("{path}/commands"),
            command(json!({ "type": "DECLINE", "revision": deal.revision })),
        )
        .await,
        app.post(
            user,
            &format!("{path}/revisions"),
            json!({
                "expected_version": version,
                "terms": fence_job(deal.repair, deal.payment),
                "consent": consent(),
            }),
        )
        .await,
        app.call(
            Some(user),
            Method::PUT,
            &format!("{path}/draft"),
            Some(json!({ "body": { "terms": "mine" } })),
            &[],
        )
        .await,
        leave(app, user, &deal.exchange).await,
        app.post(
            user,
            &format!("{path}/invitation"),
            json!({ "for_anyone": true }),
        )
        .await,
        app.post(
            user,
            &format!("{path}/reports"),
            json!({ "reason": "SCAM" }),
        )
        .await,
        app.get(user, &format!("{path}/block")).await,
        app.call(Some(user), Method::PUT, &format!("{path}/block"), None, &[])
            .await,
        app.call(
            Some(user),
            Method::DELETE,
            &format!("{path}/block"),
            None,
            &[],
        )
        .await,
    ]
}

/// type, who did it, the revision it is about, and its stored details.
type StoredEvent = (String, Option<String>, Option<Uuid>, Value);

async fn events(app: &App, exchange: &str) -> Vec<StoredEvent> {
    sqlx::query_as(
        "SELECT type, actor_slot, revision_id, data FROM exchange_event
         WHERE exchange_id = $1 ORDER BY sequence",
    )
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

fn kinds(events: &[StoredEvent]) -> Vec<&str> {
    events.iter().map(|event| event.0.as_str()).collect()
}

/// Every signature on the deal's first revision: slot, holding, account.
async fn signatures(app: &App, deal: &Deal) -> Vec<(String, i32, Uuid)> {
    sqlx::query_as(
        "SELECT slot, holding, account_id FROM acceptance
         WHERE revision_id = $1 ORDER BY slot, holding",
    )
    .bind(id(&deal.revision))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

/// What is queued for a person about an exchange: the notice, and why it was
/// closed unsent if it was.
async fn queued(app: &App, user: &User, exchange: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT payload->>'notice', last_error FROM outbox
         WHERE recipient_account_id = $1 AND exchange_id = $2 ORDER BY id",
    )
    .bind(user.id)
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned())
        .unwrap_or_else(|| panic!("not a database error: {error}"))
}

/// Every string anywhere in a JSON value, and every object key.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
        Value::Object(map) => {
            for (key, item) in map {
                out.push(key.clone());
                strings(item, out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn a_stranger_with_a_forwarded_link_can_read_and_sign_and_is_removed() {
    let app = app().await;
    let deal = app.negotiating().await;
    let (ana, ben, exchange) = (&deal.ana, &deal.ben, deal.exchange.as_str());
    let sam = app.user("Sam Stranger").await;
    assert_eq!(app.view(ana, exchange).await["invitation_open"], true);

    // The link meant for Ben reaches Sam, who signs in and opens it.
    let view = claim(&app, &sam, &deal.invitation).await.ok();
    assert_eq!(view["invitation_open"], Value::Null);
    assert_eq!(
        (&view["you"], &view["counterparty"]),
        (&json!("B"), &json!("CLAIMED"))
    );
    let before = app.view(ana, exchange).await;
    assert_eq!(before["invitation_open"], Value::Null);

    // Sam can read. Sam cannot decline, which would close Ana's exchange,
    // or counter, which would replace the offer she signed, or do anything
    // else to it.
    assert_eq!(
        app.view(&sam, exchange).await["open_revision"]["terms"]["terms"],
        "Repair the back fence."
    );
    for command in [
        json!({ "type": "DECLINE", "revision": deal.revision }),
        json!({ "type": "PROPOSE_END" }),
        json!({ "type": "REQUEST_CLOSE", "note": "Not for me." }),
    ] {
        app.command(&sam, exchange, command)
            .await
            .refused(StatusCode::CONFLICT, "AWAITING_CONFIRMATION");
    }
    let mut counter = fence_job(deal.repair, deal.payment);
    counter["contributions"][1]["amount_minor"] = json!(1);
    app.send(&sam, exchange, counter)
        .await
        .refused(StatusCode::CONFLICT, "AWAITING_CONFIRMATION");
    // Removing and confirming are Ana's to do.
    for kind in ["REJECT_COUNTERPARTY", "CONFIRM_COUNTERPARTY"] {
        app.command(&sam, exchange, json!({ "type": kind }))
            .await
            .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");
    }
    assert_eq!(
        app.view(ana, exchange).await,
        before,
        "none of it changed anything"
    );

    // Sam can sign. It waits, as before, for Ana to confirm who signed.
    let view = app
        .command(&sam, exchange, accept(&deal.revision))
        .await
        .ok();
    assert_eq!(view["state"], "NEGOTIATING");
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A", "B"]));

    // Ana sees who it is, and that it is not Ben. With someone in Ben's
    // place she cannot send out another link.
    let view = app.view(ana, exchange).await;
    assert_eq!(view["claimant"]["display_name"], "Sam Stranger");
    new_link(&app, &deal)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // "Not who I invited."
    let view = reject(&app, &deal).await.ok();
    assert_eq!(
        (
            &view["state"],
            &view["counterparty"],
            &view["claimant"],
            &view["invitation_open"]
        ),
        (
            &json!("NEGOTIATING"),
            &json!("UNCLAIMED"),
            &Value::Null,
            &json!(false)
        )
    );
    assert_eq!(
        view["open_revision"]["id"],
        deal.revision.as_str(),
        "Ana's offer stands"
    );
    assert_eq!(
        view["open_revision"]["accepted_by"],
        json!(["A"]),
        "and only she has signed it"
    );
    // There is nobody left to confirm or to remove.
    confirm(&app, &deal)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    reject(&app, &deal)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // For Sam the exchange is gone, exactly as for someone who never had
    // anything to do with it.
    let nobody = app.user("Nobody").await;
    let (for_sam, for_nobody) = (
        every_way_in(&app, &sam, &deal).await,
        every_way_in(&app, &nobody, &deal).await,
    );
    for (sam, nobody) in for_sam.iter().zip(&for_nobody) {
        gone(sam);
        assert_eq!((sam.status, &sam.body), (nobody.status, &nobody.body));
    }
    assert_eq!(app.get(&sam, "/v1/exchanges").await.ok(), json!([]));
    assert_eq!(app.get(&sam, "/v1/blocks").await.ok(), json!([]));
    // The link is spent, and says so like any dead link.
    let dead = claim(&app, &nobody, "made-up").await;
    dead.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    for user in [&sam, &nobody, ben] {
        let reply = claim(&app, user, &deal.invitation).await;
        assert_eq!((reply.status, &reply.body), (dead.status, &dead.body));
    }
    app.post(
        &nobody,
        "/v1/invitations/preview",
        json!({ "token": deal.invitation }),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    // Sam is sent nothing about any of it.
    assert_eq!(queued(&app, &sam, exchange).await, []);

    // Ana makes a new link and sends it to Ben, who opens it.
    let token = new_link(&app, &deal).await.ok()["invitation_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(app.view(ana, exchange).await["invitation_open"], true);
    claim(&app, &sam, &deal.invitation)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    let view = claim(&app, ben, &token).await.ok();
    assert_eq!(view["counterparty"], "CLAIMED");
    assert_eq!(
        view["open_revision"]["accepted_by"],
        json!(["A"]),
        "Sam's signature is not Ben's"
    );

    // Confirming Ben does not bring Sam's signature back to life.
    let view = confirm(&app, &deal).await.ok();
    assert_eq!(
        (&view["state"], &view["counterparty"]),
        (&json!("NEGOTIATING"), &json!("CONFIRMED"))
    );
    assert_eq!(view["in_force_revision"], Value::Null);

    // Ben signs the same revision in his own right, and it is agreed.
    let view = app
        .command(ben, exchange, accept(&deal.revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    assert_eq!(view["in_force_revision"]["id"], deal.revision.as_str());
    assert_eq!(view["in_force_revision"]["accepted_by"], json!(["A", "B"]));

    // Everything that happened is still on record, under the account that
    // did it: Sam's claim, Sam's signature, and Sam's time in Ben's place.
    let history = events(&app, exchange).await;
    assert_eq!(
        kinds(&history),
        [
            "REVISION_SENT",
            "COUNTERPARTY_CLAIMED",
            "REVISION_ACCEPTED",
            "COUNTERPARTY_REJECTED",
            "COUNTERPARTY_CLAIMED",
            "COUNTERPARTY_CONFIRMED",
            "REVISION_ACCEPTED",
            "AGREEMENT_IN_FORCE",
        ]
    );
    assert_eq!(history[1].3["account"], sam.id.to_string());
    let rejected = &history[3];
    assert_eq!(
        (rejected.1.as_deref(), rejected.2),
        (Some("A"), Some(id(&deal.revision)))
    );
    assert_eq!(
        rejected.3,
        json!({ "account": sam.id, "signature_void": true })
    );
    assert_eq!(history[4].3["account"], ben.id.to_string());
    assert_eq!(
        signatures(&app, &deal).await,
        [
            ("A".to_owned(), 1, ana.id),
            ("B".to_owned(), 1, sam.id),
            ("B".to_owned(), 2, ben.id),
        ]
    );
    let holdings: Vec<(i32, Uuid, bool)> = sqlx::query_as(
        "SELECT holding, account_id, ended_at IS NOT NULL FROM slot_holding
         WHERE exchange_id = $1 AND slot = 'B' ORDER BY holding",
    )
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(holdings, [(1, sam.id, true), (2, ben.id, false)]);

    // And still nothing of it is Sam's to see.
    for reply in every_way_in(&app, &sam, &deal).await {
        gone(&reply);
    }
}

#[tokio::test]
async fn nothing_a_removed_claimant_did_or_does_can_bring_an_agreement_into_force() {
    let app = app().await;
    let deal = app.negotiating().await;
    let (ben, exchange) = (&deal.ben, deal.exchange.as_str());
    let sam = app.user("Sam Stranger").await;
    claim(&app, &sam, &deal.invitation).await.ok();

    // Sam signs, with an idempotency key, as a client would.
    let path = format!("/v1/exchanges/{exchange}/commands");
    let version = app.view(&sam, exchange).await["version"].clone();
    let signing = json!({ "expected_version": version, "command": accept(&deal.revision) });
    let key = [("idempotency-key", "sam-signs")];
    app.call(Some(&sam), Method::POST, &path, Some(signing.clone()), &key)
        .await
        .ok();

    reject(&app, &deal).await.ok();

    // The same request again, as a retry of one whose reply was lost, and a
    // fresh one from a session Sam still has: neither finds an exchange.
    let retry = || app.call(Some(&sam), Method::POST, &path, Some(signing.clone()), &key);
    gone(&retry().await);
    let version = app.view(&deal.ana, exchange).await["version"].clone();
    let fresh = json!({ "expected_version": version, "command": accept(&deal.revision) });
    gone(&app.post(&sam, &path, fresh).await);

    // Ben takes his place and is confirmed. Sam's signature is on that very
    // revision, and it stays nobody's.
    let token = new_link(&app, &deal).await.ok()["invitation_token"]
        .as_str()
        .unwrap()
        .to_owned();
    claim(&app, ben, &token).await.ok();
    let view = confirm(&app, &deal).await.ok();
    assert_eq!(view["state"], "NEGOTIATING");
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
    gone(&retry().await);
    assert_eq!(app.view(ben, exchange).await["state"], "NEGOTIATING");
    assert_eq!(signatures(&app, &deal).await.len(), 2);

    // Below the service, the database refuses it too. As the role the
    // service runs as: Sam cannot sign for a place Sam no longer holds,
    let forged = sqlx::query(
        "INSERT INTO acceptance
            (exchange_id, revision_id, slot, holding, account_id, content_hash, auth_method,
             authenticated_at, consent_language, consent_version)
         SELECT exchange_id, id, 'B', 1, $2, content_hash, 'EMAIL_OTP', now(), 'en', 'test-1'
         FROM revision WHERE id = $1",
    )
    .bind(id(&deal.revision))
    .bind(sam.id)
    .execute(&app.db)
    .await
    .expect_err("a signature from an account that does not hold the slot");
    assert_eq!(sqlstate(&forged), FOREIGN_KEY, "{forged}");

    // and the revision cannot be put in force on the signatures it has,
    // which are Ana's and one made by someone since removed.
    let forced = sqlx::query(
        "UPDATE exchange SET state = 'ACTIVE', open_revision_id = NULL, in_force_revision_id = $2
         WHERE id = $1",
    )
    .bind(id(exchange))
    .bind(id(&deal.revision))
    .execute(&app.db)
    .await
    .expect_err("an agreement resting on a removed claimant's signature");
    assert_eq!(sqlstate(&forced), CHECK, "{forced}");

    // Ben's own signature is what it takes.
    let view = app
        .command(ben, exchange, accept(&deal.revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
}

#[tokio::test]
async fn an_unconfirmed_claimant_can_leave() {
    let app = app().await;
    let deal = app.negotiating().await;
    let (ana, exchange) = (&deal.ana, deal.exchange.as_str());
    let sam = app.user("Sam Stranger").await;
    claim(&app, &sam, &deal.invitation).await.ok();
    app.command(&sam, exchange, accept(&deal.revision))
        .await
        .ok();

    // It is not Ana's exchange to leave.
    leave(&app, ana, exchange)
        .await
        .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");

    let left = leave(&app, &sam, exchange).await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    assert_eq!(left.body, Value::Null);

    // Gone for Sam, a second try included, and the link is spent.
    for reply in every_way_in(&app, &sam, &deal).await {
        gone(&reply);
    }
    claim(&app, &sam, &deal.invitation)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");

    // Ana's offer is as she sent it, signed by her alone, and she is told.
    let view = app.view(ana, exchange).await;
    assert_eq!(
        (
            &view["state"],
            &view["counterparty"],
            &view["invitation_open"]
        ),
        (&json!("NEGOTIATING"), &json!("UNCLAIMED"), &json!(false))
    );
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
    let told: Vec<String> = queued(&app, ana, exchange)
        .await
        .into_iter()
        .map(|(notice, _)| notice)
        .collect();
    assert_eq!(
        told,
        [
            "INVITATION_CLAIMED_UNCONFIRMED",
            "ACCEPTANCE_WAITING",
            "CLAIMANT_LEFT"
        ]
    );
    let history = events(&app, exchange).await;
    let released = history.last().unwrap();
    assert_eq!(
        (
            released.0.as_str(),
            released.1.as_deref(),
            released.2,
            &released.3
        ),
        (
            "COUNTERPARTY_RELEASED",
            Some("B"),
            Some(id(&deal.revision)),
            &json!({ "account": sam.id, "signature_void": true })
        )
    );

    // Sam may come back through a new link, as anyone may. What Sam signed
    // before does not come back with them.
    let token = new_link(&app, &deal).await.ok()["invitation_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let view = claim(&app, &sam, &token).await.ok();
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
    let view = confirm(&app, &deal).await.ok();
    assert_eq!(view["state"], "NEGOTIATING");

    // Once confirmed, leaving is no longer the way out: declining is.
    leave(&app, &sam, exchange)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let view = app
        .command(
            &sam,
            exchange,
            json!({ "type": "DECLINE", "revision": deal.revision }),
        )
        .await
        .ok();
    assert_eq!(view["closed_reason"], "DECLINED");
}

#[tokio::test]
async fn someone_the_invitation_names_is_a_party_from_the_start() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let exchange = app.draft(&ana).await;
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());
    let sent = app
        .post(
            &ana,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(repair, payment),
                "consent": consent(),
                "invitation": { "bound_to": ben.email },
            }),
        )
        .await
        .ok();
    let view = claim(&app, &ben, sent["invitation_token"].as_str().unwrap())
        .await
        .ok();
    assert_eq!(view["counterparty"], "CONFIRMED");

    // Ana named him, so she cannot now say he is not who she invited, and
    // he has no claim to give up.
    app.command(&ana, &exchange, json!({ "type": "REJECT_COUNTERPARTY" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    leave(&app, &ben, &exchange)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // He negotiates as a party does: he counters, and Ana accepts.
    let mut counter = fence_job(repair, payment);
    counter["contributions"][1]["amount_minor"] = json!(35000);
    let sent = app.send(&ben, &exchange, counter).await.ok();
    let second = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let view = app.command(&ana, &exchange, accept(second)).await.ok();
    assert_eq!(view["state"], "ACTIVE");

    // And he could have declined.
    let other = app.draft(&ana).await;
    let sent = app
        .post(
            &ana,
            &format!("/v1/exchanges/{other}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": consent(),
                "invitation": { "bound_to": ben.email },
            }),
        )
        .await
        .ok();
    claim(&app, &ben, sent["invitation_token"].as_str().unwrap())
        .await
        .ok();
    let first = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let view = app
        .command(
            &ben,
            &other,
            json!({ "type": "DECLINE", "revision": first }),
        )
        .await
        .ok();
    assert_eq!(view["closed_reason"], "DECLINED");
}

#[tokio::test]
async fn removing_and_signing_at_the_same_moment_leave_no_signature_that_counts() {
    let app = app().await;

    for round in 0..8 {
        let deal = app.negotiating().await;
        let exchange = deal.exchange.as_str();
        let sam = app.user("Sam Stranger").await;
        claim(&app, &sam, &deal.invitation).await.ok();

        // Sam signs at the moment Ana says Sam is not who she invited, each
        // having seen the same version.
        let path = format!("/v1/exchanges/{exchange}/commands");
        let version = app.view(&deal.ana, exchange).await["version"].clone();
        let based_on = |command: Value| json!({ "expected_version": version, "command": command });
        let (signed, rejected) = tokio::join!(
            app.post(&sam, &path, based_on(accept(&deal.revision))),
            app.post(
                &deal.ana,
                &path,
                based_on(json!({ "type": "REJECT_COUNTERPARTY" }))
            ),
        );
        match (signed.status, rejected.status) {
            // Ana was first: there was no exchange for Sam to sign.
            (StatusCode::NOT_FOUND, StatusCode::OK) => gone(&signed),
            // Sam was first: Ana's view was out of date, she looks again and
            // removes Sam, signature and all.
            (StatusCode::OK, StatusCode::CONFLICT) => {
                rejected.refused(StatusCode::CONFLICT, "VERSION_CONFLICT");
                reject(&app, &deal).await.ok();
            }
            other => panic!("round {round}: {other:?} {} {}", signed.body, rejected.body),
        }

        let view = app.view(&deal.ana, exchange).await;
        assert_eq!(
            (&view["state"], &view["counterparty"]),
            (&json!("NEGOTIATING"), &json!("UNCLAIMED")),
            "round {round}"
        );
        assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
        gone(&app.get(&sam, &format!("/v1/exchanges/{exchange}")).await);

        // Whichever way it went, confirming the next person binds nobody.
        let token = new_link(&app, &deal).await.ok()["invitation_token"]
            .as_str()
            .unwrap()
            .to_owned();
        claim(&app, &deal.ben, &token).await.ok();
        assert_eq!(
            confirm(&app, &deal).await.ok()["state"],
            "NEGOTIATING",
            "round {round}"
        );
    }
}

#[tokio::test]
async fn leaving_and_being_confirmed_at_the_same_moment_end_one_way_or_the_other() {
    let app = app().await;

    for round in 0..8 {
        let deal = app.negotiating().await;
        let exchange = deal.exchange.as_str();
        let sam = app.user("Sam Stranger").await;
        claim(&app, &sam, &deal.invitation).await.ok();
        app.command(&sam, exchange, accept(&deal.revision))
            .await
            .ok();

        let path = format!("/v1/exchanges/{exchange}/commands");
        let version = app.view(&deal.ana, exchange).await["version"].clone();
        let confirming = json!({
            "expected_version": version,
            "command": { "type": "CONFIRM_COUNTERPARTY" },
        });
        let (left, confirmed) = tokio::join!(
            leave(&app, &sam, exchange),
            app.post(&deal.ana, &path, confirming),
        );
        let view = app.view(&deal.ana, exchange).await;
        match (left.status, confirmed.status) {
            // Sam was first: there is nobody for Ana to confirm, and the
            // signature went with Sam.
            (StatusCode::NO_CONTENT, StatusCode::CONFLICT) => {
                confirmed.refused(StatusCode::CONFLICT, "VERSION_CONFLICT");
                assert_eq!(
                    (&view["state"], &view["counterparty"]),
                    (&json!("NEGOTIATING"), &json!("UNCLAIMED")),
                    "round {round}"
                );
                gone(&app.get(&sam, &format!("/v1/exchanges/{exchange}")).await);
            }
            // Ana was first: Sam is confirmed, the signature took effect,
            // and a party cannot leave an agreement.
            (StatusCode::CONFLICT, StatusCode::OK) => {
                left.refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
                assert_eq!(view["state"], "ACTIVE", "round {round}");
                assert_eq!(app.view(&sam, exchange).await["state"], "ACTIVE");
            }
            other => panic!("round {round}: {other:?} {} {}", left.body, confirmed.body),
        }
    }
}

#[tokio::test]
async fn what_was_waiting_to_be_sent_to_a_removed_claimant_is_not_sent() {
    let app = app().await;
    let deal = app.negotiating().await;
    let exchange = deal.exchange.as_str();
    let sam = app.user("Sam Stranger").await;
    claim(&app, &sam, &deal.invitation).await.ok();

    // Ana changes her offer while Sam is in Ben's place. Sam is told, in the
    // words for someone not yet confirmed, who can sign or leave and nothing
    // else; the worker has not sent it yet.
    let mut changed = fence_job(deal.repair, deal.payment);
    changed["contributions"][1]["amount_minor"] = json!(45000);
    app.send(&deal.ana, exchange, changed).await.ok();
    assert_eq!(
        queued(&app, &sam, exchange).await,
        [("REVISION_SENT_UNCONFIRMED".to_owned(), None)]
    );
    // Sam had started on an answer, kept for them alone.
    app.call(
        Some(&sam),
        Method::PUT,
        &format!("/v1/exchanges/{exchange}/draft"),
        Some(json!({ "body": { "terms": "My version" } })),
        &[],
    )
    .await;

    reject(&app, &deal).await.ok();

    // The message is closed unsent and no other is queued; the working copy
    // is gone.
    let waiting = queued(&app, &sam, exchange).await;
    assert_eq!(waiting.len(), 1);
    assert!(
        waiting[0]
            .1
            .as_deref()
            .is_some_and(|why| why.contains("no longer a party")),
        "{waiting:?}"
    );
    let unsent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE recipient_account_id = $1 AND completed_at IS NULL",
    )
    .bind(sam.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(unsent, 0);
    let drafts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM exchange_draft WHERE account_id = $1")
            .bind(sam.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(drafts, 0);
}

#[tokio::test]
async fn the_record_keeps_a_removed_claimant_without_saying_who_they_were() {
    let app = app().await;
    let deal = app.negotiating().await;
    let (ana, ben, exchange) = (&deal.ana, &deal.ben, deal.exchange.as_str());
    let sam = app.user("Sam Stranger").await;
    let lee = app.user("Lee Leaver").await;

    // Sam opens the link, signs and is removed. Lee opens the next, and
    // leaves. Ben opens the third, is confirmed and signs.
    claim(&app, &sam, &deal.invitation).await.ok();
    app.command(&sam, exchange, accept(&deal.revision))
        .await
        .ok();
    reject(&app, &deal).await.ok();
    let token = new_link(&app, &deal).await.ok()["invitation_token"].clone();
    claim(&app, &lee, token.as_str().unwrap()).await.ok();
    leave(&app, &lee, exchange).await;
    let token = new_link(&app, &deal).await.ok()["invitation_token"].clone();
    claim(&app, ben, token.as_str().unwrap()).await.ok();
    confirm(&app, &deal).await.ok();
    app.command(ben, exchange, accept(&deal.revision))
        .await
        .ok();

    for reader in [ana, ben] {
        let record = app
            .get(reader, &format!("/v1/exchanges/{exchange}/record"))
            .await
            .ok();
        let events = record["events"].as_array().unwrap();
        let told: Vec<(&str, &str, bool)> = events
            .iter()
            .map(|event| {
                (
                    event["type"].as_str().unwrap(),
                    event["actor"].as_str().unwrap(),
                    event["by_removed_claimant"] == true,
                )
            })
            .collect();
        assert_eq!(
            told,
            [
                ("REVISION_SENT", "A", false),
                ("COUNTERPARTY_CLAIMED", "B", true),
                ("REVISION_ACCEPTED", "B", true),
                ("COUNTERPARTY_REJECTED", "A", false),
                ("COUNTERPARTY_CLAIMED", "B", true),
                ("COUNTERPARTY_RELEASED", "B", true),
                ("COUNTERPARTY_CLAIMED", "B", false),
                ("COUNTERPARTY_CONFIRMED", "A", false),
                ("REVISION_ACCEPTED", "B", false),
                ("AGREEMENT_IN_FORCE", "B", false),
            ]
        );
        // The removal says which signature went with it.
        assert_eq!(events[3]["signature_void"], true);
        assert_eq!(events[3]["revision"]["sequence"], 1);
        assert_eq!(events[5]["signature_void"], false);
        assert!(events[5].get("revision").is_none());

        // The signatures that count are Ana's and Ben's. Sam's is kept
        // apart, void from the moment Sam was removed.
        let revision = &record["revisions"][0];
        let parties: Vec<&str> = revision["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|signature| signature["party"].as_str().unwrap())
            .collect();
        assert_eq!(parties, ["A", "B"]);
        assert_eq!(revision["signatures"][1]["name"], "Ben Ortiz");
        assert_eq!(
            revision["signatures"][1]["signed_at"], events[8]["at"],
            "the second signature is Ben's own"
        );
        let void = revision["void_signatures"].as_array().unwrap();
        assert_eq!(void.len(), 1);
        assert_eq!(void[0]["party"], "B");
        assert_eq!(void[0]["signed_at"], events[2]["at"]);
        assert_eq!(void[0]["content_hash"], revision["content_hash"]);
        assert_eq!(void[0]["verification"]["method"], "EMAIL_OTP");
        assert!(void[0]["void_since"].is_string());
        assert!(void[0].get("name").is_none());
        assert_eq!(revision["standing"]["status"], "IN_FORCE");

        // The history at the foot of the exchange says the same.
        let history = app
            .get(reader, &format!("/v1/exchanges/{exchange}/history"))
            .await
            .ok();
        assert_eq!(history["events"], record["events"]);

        // Neither says who Sam or Lee were.
        let mut said = Vec::new();
        strings(&record, &mut said);
        strings(&history, &mut said);
        for removed in [&sam, &lee] {
            let secrets = [removed.id.to_string(), removed.email.clone()];
            assert!(
                said.iter()
                    .all(|text| secrets.iter().all(|secret| !text.contains(secret.as_str()))),
                "the record names an account that was removed"
            );
        }
        assert!(
            said.iter()
                .all(|text| !text.contains("Stranger") && !text.contains("Leaver"))
        );
    }

    // A record with nobody removed says nothing about any of this.
    let plain = app.active().await;
    let record = app
        .get(
            &plain.ana,
            &format!("/v1/exchanges/{}/record", plain.exchange),
        )
        .await
        .ok();
    assert!(record["revisions"][0].get("void_signatures").is_none());
    assert!(
        record["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event.get("by_removed_claimant").is_none())
    );
}
