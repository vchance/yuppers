//! The exchange API end to end: HTTP requests in, stored agreement out.

mod common;

use axum::http::{Method, StatusCode};
use common::{App, accept, consent, fence_job};
use serde_json::{Value, json};
use uuid::Uuid;
use yuppers_backend::domain::canonical::content_hash;
use yuppers_backend::exchanges::repo;

const DATABASE: &str = "yuppers_test_api";

async fn app() -> App {
    App::start(DATABASE).await
}

fn statuses(view: &Value) -> Vec<&str> {
    view["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["status"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn two_people_agree_on_a_deal_and_complete_it() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());

    // Ana starts a private draft and keeps a working copy.
    let exchange = app.draft(&ana).await;
    let path = format!("/v1/exchanges/{exchange}");
    let view = app.view(&ana, &exchange).await;
    assert_eq!(
        (&view["state"], &view["you"], &view["version"]),
        (&json!("DRAFT"), &json!("A"), &json!(0))
    );
    assert_eq!(view["display_code"].as_str().unwrap().len(), 9);

    let saved = app
        .call(
            Some(&ana),
            Method::PUT,
            &format!("{path}/draft"),
            Some(json!({ "body": { "terms": "Repair the" } })),
            &[],
        )
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.view(&ana, &exchange).await["draft"]["terms"],
        "Repair the"
    );

    // Sending signs it, opens the negotiation and yields the invitation link.
    let sent = app
        .send(&ana, &exchange, fence_job(repair, payment))
        .await
        .ok();
    let token = sent["invitation_token"].as_str().unwrap().to_owned();
    let view = &sent["exchange"];
    assert_eq!(view["state"], "NEGOTIATING");
    assert_eq!(view["counterparty"], "UNCLAIMED");
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
    assert_eq!(view["open_revision"]["sequence"], 1);
    assert_eq!(
        view["draft"],
        Value::Null,
        "the working copy is gone once sent"
    );
    let revision = view["open_revision"]["id"].as_str().unwrap().to_owned();

    // Signed out, the link says nothing about itself; signed in, Ben reads
    // the proposal from it. Reading claims nothing.
    app.call(
        None,
        Method::POST,
        "/v1/invitations/preview",
        Some(json!({ "token": token })),
        &[],
    )
    .await
    .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    let preview = app
        .post(&ben, "/v1/invitations/preview", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(
        preview["revision"]["terms"]["terms"],
        "Repair the back fence."
    );
    assert_eq!(preview["bound"], false);

    // He cannot see the exchange itself until he claims the link.
    app.get(&ben, &path)
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    let view = app
        .post(&ben, "/v1/invitations/claim", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(
        (&view["you"], &view["counterparty"]),
        (&json!("B"), &json!("CLAIMED"))
    );

    // Ana is shown who claimed it, partly hidden, and Ben is not shown that.
    let claimant = app.view(&ana, &exchange).await["claimant"].clone();
    assert_eq!(claimant["display_name"], "Ben");
    assert!(
        claimant["identifier"]
            .as_str()
            .unwrap()
            .contains("•••@example.test")
    );
    assert_eq!(view["claimant"], Value::Null);

    // Ben signs. It waits for Ana to confirm who he is.
    let view = app.command(&ben, &exchange, accept(&revision)).await.ok();
    assert_eq!(view["state"], "NEGOTIATING");
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A", "B"]));

    let view = app
        .command(&ana, &exchange, json!({ "type": "CONFIRM_COUNTERPARTY" }))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    assert_eq!(view["open_revision"], Value::Null);
    assert_eq!(view["in_force_revision"]["id"], revision.as_str());
    assert_eq!(statuses(&view), ["PENDING", "PENDING"]);

    // Ana does the work; Ben confirms it and pays; Ana confirms the payment.
    let view = app.act(&ana, &exchange, repair, "CLAIM").await.ok();
    assert_eq!(statuses(&view), ["CLAIMED", "PENDING"]);
    app.act(&ben, &exchange, repair, "CONFIRM").await.ok();
    app.act(&ben, &exchange, payment, "CLAIM").await.ok();
    let view = app.act(&ana, &exchange, payment, "CONFIRM").await.ok();
    assert_eq!(
        (&view["state"], &view["closed_outcome"]),
        (&json!("CLOSED"), &json!("COMPLETED"))
    );
    assert_eq!(statuses(&view), ["ACCEPTED", "ACCEPTED"]);

    // Both find it in their lists, each naming the other.
    for (user, you, other) in [(&ana, "A", "Ben Ortiz"), (&ben, "B", "Ana Ruiz")] {
        let list = app.get(user, "/v1/exchanges").await.ok();
        let entry = list
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == exchange.as_str())
            .expect("the exchange is listed");
        assert_eq!(
            (&entry["you"], &entry["other_party_name"]),
            (&json!(you), &json!(other))
        );
        assert_eq!(entry["closed_outcome"], "COMPLETED");
    }

    // The stored history is contiguous and ends with the closure.
    let id: Uuid = exchange.parse().unwrap();
    let events: Vec<(i64, String)> = sqlx::query_as(
        "SELECT sequence, type FROM exchange_event WHERE exchange_id = $1 ORDER BY sequence",
    )
    .bind(id)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    let kinds: Vec<&str> = events.iter().map(|(_, kind)| kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "REVISION_SENT",
            "COUNTERPARTY_CLAIMED",
            "REVISION_ACCEPTED",
            "COUNTERPARTY_CONFIRMED",
            "AGREEMENT_IN_FORCE",
            "CONTRIBUTION_CLAIMED",
            "CONTRIBUTION_CONFIRMED",
            "CONTRIBUTION_CLAIMED",
            "CONTRIBUTION_CONFIRMED",
            "EXCHANGE_CLOSED",
        ]
    );
    let sequences: Vec<i64> = events.iter().map(|(sequence, _)| *sequence).collect();
    assert_eq!(sequences, (1..=10).collect::<Vec<i64>>());

    // Two signatures, each on the revision's own hash.
    let signatures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM acceptance a JOIN revision r ON r.id = a.revision_id
         WHERE a.exchange_id = $1 AND a.content_hash = r.content_hash
           AND a.consent_version = 'test-1'",
    )
    .bind(id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(signatures, 2);
}

#[tokio::test]
async fn what_is_stored_reproduces_the_hash_that_was_signed() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());

    // Every kind of field, including ones a database might store differently
    // from how they were written.
    let mut terms = fence_job(repair, payment);
    terms["party_a_name"] = json!("Ana María Ruiz-Peña");
    terms["terms"] = json!("Reparar la cerca.\nLine two with \"quotes\" and a tab\t.");
    terms["contributions"][0]["quantity"] = json!({ "amount": "1.50", "unit": "días" });
    terms["contributions"][0]["completion_criteria"] = json!("The gate swings freely");
    terms["contributions"][0]["due"] = json!({ "kind": "DATE", "date": "2026-11-01" });
    terms["contributions"][0]["required"] = json!(true);

    let sent = app.send(&ana, &exchange, terms).await.ok();
    let open = &sent["exchange"]["open_revision"];
    let revision: Uuid = open["id"].as_str().unwrap().parse().unwrap();

    let mut conn = app.owner.acquire().await.unwrap();
    let stored = repo::load_revision(&mut conn, revision).await.unwrap();
    let recomputed = content_hash(
        exchange.parse().unwrap(),
        "USD",
        "America/Chicago",
        &stored.revision,
    );

    assert_eq!(stored.content_hash, recomputed);
    let hex: String = recomputed
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(open["content_hash"], hex.as_str());
    assert_eq!(
        open["terms"]["contributions"][0]["quantity"]["amount"],
        "1.50"
    );
}

#[tokio::test]
async fn an_exchange_is_visible_only_to_its_parties() {
    let app = app().await;
    let deal = app.active().await;
    let stranger = app.user("Carla").await;
    let path = format!("/v1/exchanges/{}", deal.exchange);

    app.get(&stranger, &path)
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    app.post(
        &stranger,
        &format!("{path}/commands"),
        json!({ "expected_version": 0, "command": { "type": "PROPOSE_END" } }),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    app.call(None, Method::GET, &path, None, &[])
        .await
        .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    app.get(&deal.ana, "/v1/exchanges/not-an-id")
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    let list = app.get(&stranger, "/v1/exchanges").await.ok();
    assert_eq!(list, json!([]));
}

#[tokio::test]
async fn a_change_based_on_a_stale_view_is_refused() {
    let app = app().await;
    let deal = app.active().await;
    let path = format!("/v1/exchanges/{}/commands", deal.exchange);
    let stale = app.view(&deal.ana, &deal.exchange).await["version"]
        .as_i64()
        .unwrap();

    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    // Ben acts on what he saw before Ana's claim.
    app.post(
        &deal.ben,
        &path,
        json!({ "expected_version": stale, "command": { "type": "PROPOSE_END" } }),
    )
    .await
    .refused(StatusCode::CONFLICT, "VERSION_CONFLICT");
}

#[tokio::test]
async fn two_changes_at_once_come_out_in_order_and_only_one_wins() {
    let app = app().await;
    let deal = app.negotiating().await;
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

    // Both parties send a new revision at the same moment, each having seen
    // the same version.
    let version = app.view(&deal.ana, &deal.exchange).await["version"].clone();
    let path = format!("/v1/exchanges/{}/revisions", deal.exchange);
    let body = |price: i64| {
        let mut terms = fence_job(deal.repair, deal.payment);
        terms["contributions"][1]["amount_minor"] = json!(price);
        json!({ "expected_version": version, "terms": terms, "consent": consent() })
    };
    let (ana, ben) = tokio::join!(
        app.post(&deal.ana, &path, body(45000)),
        app.post(&deal.ben, &path, body(35000)),
    );

    let mut outcomes = [
        (ana.status, ana.code().to_owned()),
        (ben.status, ben.code().to_owned()),
    ];
    outcomes.sort();
    assert_eq!(
        outcomes,
        [
            (StatusCode::OK, String::new()),
            (StatusCode::CONFLICT, "VERSION_CONFLICT".to_owned()),
        ]
    );

    // One new revision was stored, and it is the open one.
    let view = app.view(&deal.ana, &deal.exchange).await;
    assert_eq!(view["open_revision"]["sequence"], 2);
    let revisions: i64 = sqlx::query_scalar("SELECT count(*) FROM revision WHERE exchange_id = $1")
        .bind(deal.exchange.parse::<Uuid>().unwrap())
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(revisions, 2);
}

#[tokio::test]
async fn an_idempotency_key_makes_a_retry_harmless() {
    let app = app().await;
    let deal = app.active().await;
    let path = format!("/v1/exchanges/{}/commands", deal.exchange);
    let version = app.view(&deal.ana, &deal.exchange).await["version"]
        .as_i64()
        .unwrap();
    let body = json!({
        "expected_version": version,
        "command": { "type": "CONTRIBUTION", "contribution": deal.repair, "action": "CLAIM" },
    });
    let key = [("idempotency-key", "attempt-1")];

    let first = app
        .call(
            Some(&deal.ana),
            Method::POST,
            &path,
            Some(body.clone()),
            &key,
        )
        .await
        .ok();
    assert_eq!(first["version"], version + 1);

    // The reply was lost; the client sends the very same request again.
    let retry = app
        .call(Some(&deal.ana), Method::POST, &path, Some(body), &key)
        .await
        .ok();
    assert_eq!(retry["version"], version + 1, "nothing was applied twice");
    assert_eq!(retry["contributions"][0]["status"], "CLAIMED");

    // The same key on a different request is a client bug, and says so.
    let different =
        json!({ "expected_version": version + 1, "command": { "type": "PROPOSE_END" } });
    app.call(Some(&deal.ana), Method::POST, &path, Some(different), &key)
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "IDEMPOTENCY_KEY_REUSED");
}

#[tokio::test]
async fn signing_needs_a_name_an_adult_and_the_current_consent_wording() {
    let app = app().await;
    let terms = || fence_job(Uuid::new_v4(), Uuid::new_v4());

    let nameless = app.user("").await;
    let exchange = app.draft(&nameless).await;
    app.send(&nameless, &exchange, terms())
        .await
        .refused(StatusCode::CONFLICT, "PROFILE_INCOMPLETE");

    let minor = app.user_with("Dani", false).await;
    let exchange = app.draft(&minor).await;
    app.send(&minor, &exchange, terms())
        .await
        .refused(StatusCode::CONFLICT, "PROFILE_INCOMPLETE");

    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    app.post(
        &ana,
        &format!("/v1/exchanges/{exchange}/revisions"),
        json!({
            "expected_version": 0,
            "terms": terms(),
            "consent": { "language": "en", "version": "an-older-wording" },
        }),
    )
    .await
    .refused(StatusCode::CONFLICT, "CONSENT_OUTDATED");

    // The consent must have been shown in a language the product speaks.
    app.post(
        &ana,
        &format!("/v1/exchanges/{exchange}/revisions"),
        json!({
            "expected_version": 0,
            "terms": terms(),
            "consent": { "language": "tlh", "version": common::CONSENT_VERSION },
        }),
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    // Accepting is signing too.
    let deal = app.negotiating().await;
    let reader = app.user_with("Eli", false).await;
    app.post(
        &reader,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    app.command(&reader, &deal.exchange, accept(&deal.revision))
        .await
        .refused(StatusCode::CONFLICT, "PROFILE_INCOMPLETE");
}

#[tokio::test]
async fn a_refused_revision_leaves_nothing_behind() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let id: Uuid = exchange.parse().unwrap();

    // Breaks a rule: nothing in it is required.
    let mut optional = fence_job(Uuid::new_v4(), Uuid::new_v4());
    optional["contributions"][0]["required"] = json!(false);
    optional["contributions"][1]["required"] = json!(false);
    app.send(&ana, &exchange, optional)
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REVISION");

    // Malformed: money without an amount, and a date that is not one.
    let mut no_amount = fence_job(Uuid::new_v4(), Uuid::new_v4());
    no_amount["contributions"][1]["amount_minor"] = Value::Null;
    app.send(&ana, &exchange, no_amount)
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    let mut bad_date = fence_job(Uuid::new_v4(), Uuid::new_v4());
    bad_date["contributions"][0]["due"] = json!({ "kind": "DATE", "date": "2026-02-30" });
    app.send(&ana, &exchange, bad_date)
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    // Borrows a contribution ID from someone else's exchange.
    let other = app.negotiating().await;
    app.send(&ana, &exchange, fence_job(other.repair, Uuid::new_v4()))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REVISION");

    let view = app.view(&ana, &exchange).await;
    assert_eq!(
        (&view["state"], &view["version"]),
        (&json!("DRAFT"), &json!(0))
    );
    let stored: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM revision WHERE exchange_id = $1)
              + (SELECT count(*) FROM exchange_event WHERE exchange_id = $1)
              + (SELECT count(*) FROM invitation WHERE exchange_id = $1)",
    )
    .bind(id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(stored, 0);
}

/// What a client asks when a link turns out spent: is this place already
/// mine? It opens the exchange for the account that took the place, and
/// takes nothing for anyone, even at a link that is still live.
#[tokio::test]
async fn asking_only_for_a_place_already_yours_never_takes_one() {
    let app = app().await;
    let deal = app.negotiating().await;
    let carla = app.user("Carla").await;
    let only_if_yours = json!({ "token": deal.invitation, "only_if_yours": true });
    let place_b = || async {
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT account_id FROM participant WHERE exchange_id = $1 AND slot = 'B'",
        )
        .bind(deal.exchange.parse::<Uuid>().unwrap())
        .fetch_one(&app.owner)
        .await
        .unwrap()
    };

    // The link is live, but asking only for a place already held takes none.
    app.post(&deal.ben, "/v1/invitations/claim", only_if_yours.clone())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    assert_eq!(place_b().await, None);
    // The link still works for whoever then chooses to take it.
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation, "only_if_yours": false }),
    )
    .await
    .ok();
    assert_eq!(place_b().await, Some(deal.ben.id));

    // Now it opens the exchange for Ben, and for nobody else.
    let view = app
        .post(&deal.ben, "/v1/invitations/claim", only_if_yours.clone())
        .await
        .ok();
    assert_eq!(view["id"], deal.exchange.as_str());
    app.post(&carla, "/v1/invitations/claim", only_if_yours)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    assert_eq!(place_b().await, Some(deal.ben.id));
}

#[tokio::test]
async fn an_invitation_naming_someone_is_only_theirs_to_claim() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let carla = app.user("Carla").await;
    let exchange = app.draft(&ana).await;

    let sent = app
        .post(
            &ana,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": consent(),
                "invitation": { "bound_to": ben.email.to_uppercase() },
            }),
        )
        .await
        .ok();
    let token = json!({ "token": sent["invitation_token"] });
    let revision = sent["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    app.post(&carla, "/v1/invitations/claim", token.clone())
        .await
        .refused(StatusCode::FORBIDDEN, "INVITATION_NOT_FOR_YOU");
    app.post(&ana, "/v1/invitations/claim", token.clone())
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // Ben was named, so Ana already knows who he is: no confirmation step.
    let view = app
        .post(&ben, "/v1/invitations/claim", token.clone())
        .await
        .ok();
    assert_eq!(view["counterparty"], "CONFIRMED");
    let view = app.command(&ben, &exchange, accept(&revision)).await.ok();
    assert_eq!(view["state"], "ACTIVE");

    // Claiming again is harmless for Ben and useless for anyone else.
    app.post(&ben, "/v1/invitations/claim", token.clone())
        .await
        .ok();
    app.post(&carla, "/v1/invitations/claim", token.clone())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    app.post(&carla, "/v1/invitations/preview", token)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
}

#[tokio::test]
async fn an_idempotency_key_has_a_length_limit() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let body = json!({
        "expected_version": 0,
        "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
        "consent": consent(),
        "invitation": { "for_anyone": true },
    });
    let path = format!("/v1/exchanges/{exchange}/revisions");
    let long = "k".repeat(201);
    app.call(
        Some(&ana),
        Method::POST,
        &path,
        Some(body.clone()),
        &[("idempotency-key", long.as_str())],
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    let fits = "k".repeat(200);
    app.call(
        Some(&ana),
        Method::POST,
        &path,
        Some(body),
        &[("idempotency-key", fits.as_str())],
    )
    .await
    .ok();
}

#[tokio::test]
async fn a_dead_link_says_nothing_about_why() {
    let app = app().await;
    let deal = app.negotiating().await;
    let path = format!("/v1/exchanges/{}/invitation", deal.exchange);
    let old = json!({ "token": deal.invitation });
    let dana = app.user("Dana").await;

    app.post(
        &dana,
        "/v1/invitations/preview",
        json!({ "token": "made-up" }),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");

    // Only the initiator can replace the link; the old one then stops working.
    app.post(&deal.ben, &path, json!({}))
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    let new = app
        .post(&deal.ana, &path, json!({ "for_anyone": true }))
        .await
        .ok();
    let new = json!({ "token": new["invitation_token"] });

    app.post(&deal.ben, "/v1/invitations/claim", old.clone())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    app.post(&dana, "/v1/invitations/preview", old)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");

    // A block between the two looks exactly like a dead link.
    sqlx::query(
        "INSERT INTO account_block (blocker_account_id, blocked_account_id) VALUES ($1, $2)",
    )
    .bind(deal.ana.id)
    .bind(deal.ben.id)
    .execute(&app.db)
    .await
    .unwrap();
    app.post(&deal.ben, "/v1/invitations/claim", new.clone())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    // To him the preview is dead too, or the two together would tell him
    // of the block. Signed out, nobody is shown anything. To anyone else
    // signed in, it still shows.
    app.post(&deal.ben, "/v1/invitations/preview", new.clone())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    app.call(
        None,
        Method::POST,
        "/v1/invitations/preview",
        Some(new.clone()),
        &[],
    )
    .await
    .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    app.post(&dana, "/v1/invitations/preview", new.clone())
        .await
        .ok();

    // Someone else can still claim it, after which it cannot be replaced.
    let carla = app.user("Carla").await;
    app.post(&carla, "/v1/invitations/claim", new).await.ok();
    app.post(&deal.ana, &path, json!({ "for_anyone": true }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
}

#[tokio::test]
async fn a_counteroffer_replaces_the_offer_and_an_amendment_replaces_the_agreement() {
    let app = app().await;
    let deal = app.negotiating().await;
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

    // Ben counters at a lower price.
    let mut counter = fence_job(deal.repair, deal.payment);
    counter["contributions"][1]["amount_minor"] = json!(35000);
    let sent = app.send(&deal.ben, &deal.exchange, counter).await.ok();
    assert_eq!(sent["invitation_token"], Value::Null);
    let open = &sent["exchange"]["open_revision"];
    assert_eq!(
        (&open["sequence"], &open["author"]),
        (&json!(2), &json!("B"))
    );
    let second = open["id"].as_str().unwrap().to_owned();

    // Ana's screen still shows the first offer; accepting it is refused.
    app.command(&deal.ana, &deal.exchange, accept(&deal.revision))
        .await
        .refused(StatusCode::CONFLICT, "STALE_REVISION");
    let view = app
        .command(&deal.ana, &deal.exchange, accept(&second))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    assert_eq!(
        view["in_force_revision"]["terms"]["contributions"][1]["amount_minor"],
        35000
    );

    // Work starts, then the parties agree to add a clean-up task.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    let cleanup = Uuid::new_v4();
    let mut amended = fence_job(deal.repair, deal.payment);
    amended["contributions"][1]["amount_minor"] = json!(38000);
    amended["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": cleanup,
            "from": "A",
            "type": "TASK",
            "description": "Haul away the old panels",
            "quantity": null,
            "due": { "kind": "AFTER_CONTRIBUTION", "contribution": deal.repair },
            "completion_criteria": null,
            "required": true,
            "amount_minor": null,
        }));
    let view = app.send(&deal.ana, &deal.exchange, amended).await.ok()["exchange"].clone();
    let third = view["open_revision"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        view["in_force_revision"]["id"],
        second.as_str(),
        "nothing changes until accepted"
    );
    assert_eq!(statuses(&view), ["CLAIMED", "PENDING"]);

    let view = app
        .command(&deal.ben, &deal.exchange, accept(&third))
        .await
        .ok();
    assert_eq!(view["in_force_revision"]["id"], third.as_str());
    assert_eq!(view["open_revision"], Value::Null);
    // The repair keeps its claim; the payment changed and the task is new.
    assert_eq!(statuses(&view), ["CLAIMED", "PENDING", "PENDING"]);

    // Each revision records the one it answered.
    let parents: Vec<(i32, Option<i32>)> = sqlx::query_as(
        "SELECT r.sequence, p.sequence FROM revision r
         LEFT JOIN revision p ON p.id = r.parent_revision_id
         WHERE r.exchange_id = $1 ORDER BY r.sequence",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(parents, [(1, None), (2, Some(1)), (3, Some(2))]);
}

#[tokio::test]
async fn a_dispute_must_say_why_and_a_remedy_must_say_what() {
    let app = app().await;
    let deal = app.active().await;
    let dispute = |note: Value| json!({ "type": "CONTRIBUTION", "contribution": deal.repair, "action": "DISPUTE", "note": note });
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    app.command(&deal.ben, &deal.exchange, dispute(json!("  ")))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    let view = app
        .command(
            &deal.ben,
            &deal.exchange,
            dispute(json!("The gate still sticks.")),
        )
        .await
        .ok();
    assert_eq!(statuses(&view), ["DISPUTED", "PENDING"]);

    // Claiming again without saying what changed is refused.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({
            "type": "CONTRIBUTION", "contribution": deal.repair, "action": "CLAIM",
            "note": "Rehung the gate.",
        }),
    )
    .await
    .ok();

    // Roles follow the contribution: Ben cannot take back Ana's claim.
    app.act(&deal.ben, &deal.exchange, deal.repair, "RETRACT_CLAIM")
        .await
        .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");

    let notes: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT note FROM exchange_event
         WHERE exchange_id = $1 AND type LIKE 'CONTRIBUTION_%' ORDER BY sequence",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        notes,
        [
            None,
            Some("The gate still sticks.".to_owned()),
            Some("Rehung the gate.".to_owned())
        ]
    );
}

#[tokio::test]
async fn the_parties_can_agree_to_end_early() {
    let app = app().await;
    let deal = app.active().await;
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();

    let view = app
        .command(&deal.ana, &deal.exchange, json!({ "type": "PROPOSE_END" }))
        .await
        .ok();
    assert_eq!(view["end_proposed_by"], "A");
    app.command(&deal.ana, &deal.exchange, json!({ "type": "ACCEPT_END" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    let view = app
        .command(&deal.ben, &deal.exchange, json!({ "type": "ACCEPT_END" }))
        .await
        .ok();
    assert_eq!(view["closed_outcome"], "ENDED_BY_AGREEMENT");
    assert_eq!(statuses(&view), ["ACCEPTED", "WAIVED"]);

    // Closed is final.
    app.act(&deal.ben, &deal.exchange, deal.payment, "CLAIM")
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
}

#[tokio::test]
async fn a_close_request_is_shown_to_both_and_can_carry_statements() {
    let app = app().await;
    let deal = app.active().await;

    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "ADD_STATEMENT", "note": "Too early" }),
    )
    .await
    .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    let view = app
        .command(
            &deal.ana,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE", "note": "No reply in weeks." }),
        )
        .await
        .ok();
    assert_eq!(view["close_requested_by"], "A");
    assert!(view["close_requested_at"].is_string());

    let view = app
        .command(
            &deal.ben,
            &deal.exchange,
            json!({ "type": "ADD_STATEMENT", "note": "I was away." }),
        )
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");

    app.command(
        &deal.ben,
        &deal.exchange,
        json!({ "type": "RETRACT_CLOSE" }),
    )
    .await
    .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");
    let view = app
        .command(
            &deal.ana,
            &deal.exchange,
            json!({ "type": "RETRACT_CLOSE" }),
        )
        .await
        .ok();
    assert_eq!(view["close_requested_by"], Value::Null);
}

#[tokio::test]
async fn starting_exchanges_is_limited_per_day_and_needs_a_real_timezone() {
    let rules = yuppers_backend::domain::Rules {
        exchanges_per_day: 2,
        ..Default::default()
    };
    let app = App::start_with(DATABASE, rules).await;
    let ana = app.user("Ana").await;

    app.post(&ana, "/v1/exchanges", json!({ "timezone": "Mars/Olympus" }))
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    app.draft(&ana).await;
    app.draft(&ana).await;
    app.post(
        &ana,
        "/v1/exchanges",
        json!({ "timezone": "America/Chicago" }),
    )
    .await
    .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
}

// ---- Limits, and things a hostile party might try ---------------------------

#[tokio::test]
async fn a_revision_is_bounded_in_size_and_in_time() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let refused = |terms: Value| {
        let (app, ana, exchange) = (&app, &ana, &exchange);
        async move {
            app.send(ana, exchange, terms)
                .await
                .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REVISION");
        }
    };

    // More contributions than a revision may hold, each waiting on the next:
    // refused at once, without walking the chain.
    let ids: Vec<Uuid> = (0..2000).map(|_| Uuid::new_v4()).collect();
    let mut many = fence_job(Uuid::new_v4(), Uuid::new_v4());
    many["contributions"] = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            json!({
                "id": id, "from": "A", "type": "TASK", "description": "x", "quantity": null,
                "due": { "kind": "AFTER_CONTRIBUTION", "contribution": ids[(index + 1) % ids.len()] },
                "completion_criteria": null, "required": true, "amount_minor": null,
            })
        })
        .collect();
    let started = std::time::Instant::now();
    refused(many).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));

    let mut long_terms = fence_job(Uuid::new_v4(), Uuid::new_v4());
    long_terms["terms"] = json!("x".repeat(20_001));
    refused(long_terms).await;

    let mut long_name = fence_job(Uuid::new_v4(), Uuid::new_v4());
    long_name["party_b_name"] = json!("x".repeat(201));
    refused(long_name).await;

    let mut long_description = fence_job(Uuid::new_v4(), Uuid::new_v4());
    long_description["contributions"][0]["description"] = json!("x".repeat(2_001));
    refused(long_description).await;

    // A due date in the year 9999 once took the worker down for good.
    let mut far = fence_job(Uuid::new_v4(), Uuid::new_v4());
    far["contributions"][0]["due"] = json!({ "kind": "DATE", "date": "9999-12-20" });
    refused(far).await;

    assert_eq!(app.view(&ana, &exchange).await["state"], "DRAFT");
}

#[tokio::test]
async fn text_the_database_cannot_hold_is_refused_not_crashed_on() {
    let app = app().await;
    let deal = app.active().await;
    let ana = &deal.ana;
    let invalid = |reply: common::Reply| {
        reply.refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    };

    let exchange = app.draft(ana).await;
    let mut terms = fence_job(Uuid::new_v4(), Uuid::new_v4());
    terms["terms"] = json!("before\u{0}after");
    invalid(app.send(ana, &exchange, terms).await);

    invalid(
        app.post(ana, "/v1/exchanges", json!({ "timezone": "UTC\u{0}" }))
            .await,
    );
    invalid(
        app.call(
            Some(ana),
            Method::PUT,
            &format!("/v1/exchanges/{exchange}/draft"),
            Some(json!({ "body": { "note\u{0}": "x" } })),
            &[],
        )
        .await,
    );
    invalid(
        app.command(
            ana,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE", "note": "why\u{0}" }),
        )
        .await,
    );

    // A note longer than the limit, and a working copy too large to keep.
    invalid(
        app.command(
            ana,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE", "note": "x".repeat(1_001) }),
        )
        .await,
    );
    invalid(
        app.call(
            Some(ana),
            Method::PUT,
            &format!("/v1/exchanges/{exchange}/draft"),
            Some(json!({ "body": { "terms": "x".repeat(200_001) } })),
            &[],
        )
        .await,
    );

    // A body that is not declared as JSON is not read as JSON.
    let reply = app
        .call(
            Some(ana),
            Method::POST,
            "/v1/exchanges",
            None,
            &[("content-type", "text/plain")],
        )
        .await;
    invalid(reply);
}

#[tokio::test]
async fn an_idempotency_key_belongs_to_one_exchange() {
    let app = app().await;
    let first = app.active().await;
    // The same two people, so the same account could reuse a key.
    let second = app.draft(&first.ana).await;
    let sent = app
        .send(
            &first.ana,
            &second,
            fence_job(Uuid::new_v4(), Uuid::new_v4()),
        )
        .await
        .ok();
    app.post(
        &first.ben,
        "/v1/invitations/claim",
        json!({ "token": sent["invitation_token"] }),
    )
    .await
    .ok();

    let key = [("idempotency-key", "one-key")];
    let body = |version: &Value| json!({ "expected_version": version, "command": { "type": "PROPOSE_END" } });
    let version = app.view(&first.ana, &first.exchange).await["version"].clone();
    app.call(
        Some(&first.ana),
        Method::POST,
        &format!("/v1/exchanges/{}/commands", first.exchange),
        Some(body(&version)),
        &key,
    )
    .await
    .ok();

    // The same key and the same body, aimed at a different exchange: it used
    // to answer 200 and silently do nothing.
    app.call(
        Some(&first.ana),
        Method::POST,
        &format!("/v1/exchanges/{second}/commands"),
        Some(body(&version)),
        &key,
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "IDEMPOTENCY_KEY_REUSED");
}

#[tokio::test]
async fn one_party_cannot_change_an_exchange_faster_than_the_other_can_follow() {
    let app = app().await;
    let deal = app.active().await;

    // Ben proposes and cancels ending, over and over, to keep the version
    // moving so that nothing Ana does can land.
    let mut stopped = false;
    for round in 0..40 {
        let kind = if round % 2 == 0 {
            "PROPOSE_END"
        } else {
            "CANCEL_END"
        };
        let reply = app
            .command(&deal.ben, &deal.exchange, json!({ "type": kind }))
            .await;
        if reply.status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(reply.code(), "TOO_MANY_REQUESTS");
            stopped = true;
            break;
        }
        reply.ok();
    }
    assert!(stopped, "Ben was never slowed down");

    // Ana is not the one being limited, and can still close the exchange.
    let view = app
        .command(
            &deal.ana,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE" }),
        )
        .await
        .ok();
    assert_eq!(view["close_requested_by"], "A");
}

#[tokio::test]
async fn a_declined_amendment_renames_nobody() {
    let app = app().await;
    let deal = app.active().await;

    let mut amended = fence_job(deal.repair, deal.payment);
    amended["party_a_name"] = json!("Ana the Unreliable");
    amended["contributions"][1]["amount_minor"] = json!(90_000);
    let sent = app.send(&deal.ben, &deal.exchange, amended).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "DECLINE", "revision": revision }),
    )
    .await
    .ok();

    // Ben's list still shows the name in the agreement that is in force.
    let list = app.get(&deal.ben, "/v1/exchanges").await.ok();
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == deal.exchange.as_str())
        .unwrap();
    assert_eq!(entry["other_party_name"], "Ana Ruiz");

    // And the declined amount did not raise the exchange's risk tier.
    let tier: i16 = sqlx::query_scalar("SELECT risk_tier FROM exchange WHERE id = $1")
        .bind(deal.exchange.parse::<Uuid>().unwrap())
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(tier, 0);
}

#[tokio::test]
async fn an_agreement_over_the_threshold_raises_the_tier_once_in_force() {
    let app = app().await;
    let deal = app.active().await;
    let mut amended = fence_job(deal.repair, deal.payment);
    amended["party_b_name"] = json!("Benjamín Ortiz");
    amended["contributions"][1]["amount_minor"] = json!(90_000);
    let sent = app.send(&deal.ben, &deal.exchange, amended).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.command(&deal.ana, &deal.exchange, accept(&revision))
        .await
        .ok();

    let (tier, name): (i16, String) = sqlx::query_as(
        "SELECT e.risk_tier, p.display_name FROM exchange e
         JOIN participant p ON p.exchange_id = e.id AND p.slot = 'B'
         WHERE e.id = $1",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((tier, name.as_str()), (1, "Benjamín Ortiz"));
}

#[tokio::test]
async fn invitation_links_cannot_be_reissued_without_limit() {
    let app = app().await;
    let deal = app.negotiating().await;
    let path = format!("/v1/exchanges/{}/invitation", deal.exchange);

    // The first link was issued with the proposal; four replacements make five.
    for _ in 0..4 {
        app.post(&deal.ana, &path, json!({ "for_anyone": true }))
            .await
            .ok();
    }
    app.post(&deal.ana, &path, json!({ "for_anyone": true }))
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
}

#[tokio::test]
async fn the_claim_is_recorded_with_who_claimed() {
    let app = app().await;
    let deal = app.active().await;

    let claimant: Option<String> = sqlx::query_scalar(
        "SELECT data->>'account' FROM exchange_event
         WHERE exchange_id = $1 AND type = 'COUNTERPARTY_CLAIMED'",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(claimant, Some(deal.ben.id.to_string()));
}

#[tokio::test]
async fn a_close_request_cannot_be_answered_by_waiving_everything() {
    let app = app().await;
    let deal = app.active().await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "REQUEST_CLOSE" }),
    )
    .await
    .ok();

    // Ben owes the payment. "Agreeing to end" here used to release him.
    app.command(&deal.ben, &deal.exchange, json!({ "type": "ACCEPT_END" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let view = app.view(&deal.ana, &deal.exchange).await;
    assert_eq!(statuses(&view), ["CLAIMED", "PENDING"]);
}

// ---- What a walk-through of the use cases asked for -------------------------

#[tokio::test]
async fn a_draft_never_sent_can_be_discarded_and_leaves_the_list() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let exchange = app.draft(&ana).await;
    let path = format!("/v1/exchanges/{exchange}");
    app.call(
        Some(&ana),
        Method::PUT,
        &format!("{path}/draft"),
        Some(json!({ "body": { "terms": "Half-written" } })),
        &[],
    )
    .await;

    // Nobody else has a draft to discard: to Ben it does not exist.
    app.post(
        &ben,
        &format!("{path}/commands"),
        json!({ "expected_version": 0, "command": { "type": "DISCARD" } }),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    let view = app
        .command(&ana, &exchange, json!({ "type": "DISCARD" }))
        .await
        .ok();
    assert_eq!(
        (
            &view["state"],
            &view["closed_outcome"],
            &view["closed_reason"]
        ),
        (&json!("CLOSED"), &json!("NOT_AGREED"), &json!("DISCARDED"))
    );
    assert_eq!(view["draft"], Value::Null, "the working copy goes with it");
    assert_eq!(view["open_revision"], Value::Null);
    assert_eq!(view["in_force_revision"], Value::Null);

    // The record says what happened, truthfully: closed, nothing agreed,
    // because its initiator discarded it.
    let history = app.get(&ana, &format!("{path}/history")).await.ok();
    let events = history["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "EXCHANGE_CLOSED");
    assert_eq!(events[0]["actor"], "A");
    assert_eq!(events[0]["outcome"], "NOT_AGREED");
    assert_eq!(events[0]["reason"], "DISCARDED");
    let record = app.get(&ana, &format!("{path}/record")).await.ok();
    assert_eq!(record["exchange"]["closed_reason"], "DISCARDED");

    // Closed is closed: nothing more can be written or sent.
    app.call(
        Some(&ana),
        Method::PUT,
        &format!("{path}/draft"),
        Some(json!({ "body": { "terms": "Again" } })),
        &[],
    )
    .await
    .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    app.send(&ana, &exchange, fence_job(Uuid::new_v4(), Uuid::new_v4()))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    app.command(&ana, &exchange, json!({ "type": "DISCARD" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // In the list it is a closed exchange like any other, so it can be put
    // out of the way with them.
    let list = app.get(&ana, "/v1/exchanges").await.ok();
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == exchange.as_str())
        .expect("still listed");
    assert_eq!(
        (&entry["state"], &entry["closed_outcome"]),
        (&json!("CLOSED"), &json!("NOT_AGREED"))
    );

    // Once something has been sent, there is no discarding: the offer is
    // withdrawn or declined instead.
    let deal = app.negotiating().await;
    app.command(&deal.ana, &deal.exchange, json!({ "type": "DISCARD" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let deal = app.active().await;
    app.command(&deal.ben, &deal.exchange, json!({ "type": "DISCARD" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
}

fn moment(value: &Value) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(
        value.as_str().expect("an RFC 3339 moment"),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap()
}

#[tokio::test]
async fn a_close_request_says_when_it_lapses_and_a_contribution_says_since_when_it_stands() {
    let rules = yuppers_backend::domain::Rules {
        close_response_window: time::Duration::days(3),
        ..Default::default()
    };
    let app = App::start_with(DATABASE, rules).await;
    let deal = app.active().await;

    let view = app.view(&deal.ana, &deal.exchange).await;
    assert_eq!(view["close_request_lapses_at"], Value::Null);
    // Each contribution came to stand as it does when the agreement did.
    let agreed = moment(&view["contributions"][0]["since"]);
    assert_eq!(moment(&view["contributions"][1]["since"]), agreed);

    let view = app
        .act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    assert!(
        moment(&view["contributions"][0]["since"]) >= agreed,
        "the claim is what the repair now stands on"
    );
    assert_eq!(
        moment(&view["contributions"][1]["since"]),
        agreed,
        "the payment stands as it did"
    );

    let view = app
        .command(
            &deal.ben,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE" }),
        )
        .await
        .ok();
    // The window is the service's to know; the client is told the moment.
    assert_eq!(
        moment(&view["close_request_lapses_at"]),
        moment(&view["close_requested_at"]) + time::Duration::days(3)
    );
    assert_eq!(
        app.view(&deal.ben, &deal.exchange).await["close_request_lapses_at"],
        view["close_request_lapses_at"],
        "both parties are told the same moment"
    );

    let view = app
        .command(
            &deal.ben,
            &deal.exchange,
            json!({ "type": "RETRACT_CLOSE" }),
        )
        .await
        .ok();
    assert_eq!(view["close_request_lapses_at"], Value::Null);
}

#[tokio::test]
async fn an_invitation_preview_names_the_currency_and_the_timezone() {
    let app = app().await;
    let deal = app.negotiating().await;
    let preview = app
        .post(
            &deal.ben,
            "/v1/invitations/preview",
            json!({ "token": deal.invitation }),
        )
        .await
        .ok();
    let view = app.view(&deal.ana, &deal.exchange).await;
    assert_eq!(preview["currency"], view["currency"]);
    assert_eq!(preview["timezone"], "America/Chicago");
    assert_eq!(preview["timezone"], view["timezone"]);
}

/// A link anyone holding it can claim is never what a request gets by
/// leaving something out: the first proposal and every replacement say who
/// the link is for, a person or anyone, and never both.
#[tokio::test]
async fn who_an_invitation_is_for_is_said_outright() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let path = format!("/v1/exchanges/{exchange}/revisions");
    for invitation in [
        None,
        Some(json!({})),
        Some(json!({ "for_anyone": false })),
        Some(json!({ "bound_to": null })),
        Some(json!({ "bound_to": "ben@example.test", "for_anyone": true })),
    ] {
        let mut body = json!({
            "expected_version": 0,
            "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
            "consent": consent(),
        });
        if let Some(invitation) = &invitation {
            body["invitation"] = invitation.clone();
        }
        app.post(&ana, &path, body)
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    }
    // Nothing was sent.
    assert_eq!(app.view(&ana, &exchange).await["version"], 0);

    let sent = app
        .post(
            &ana,
            &path,
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": consent(),
                "invitation": { "bound_to": "ben@example.test" },
            }),
        )
        .await
        .ok();
    assert!(sent["invitation_token"].is_string());

    let link = format!("/v1/exchanges/{exchange}/invitation");
    for options in [
        json!({}),
        json!({ "for_anyone": false }),
        json!({ "bound_to": "ben@example.test", "for_anyone": true }),
    ] {
        app.post(&ana, &link, options)
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    }
    for options in [
        json!({ "for_anyone": true }),
        json!({ "bound_to": "ben@example.test" }),
        json!({ "bound_to": "ben@example.test", "for_anyone": false }),
    ] {
        assert!(app.post(&ana, &link, options).await.ok()["invitation_token"].is_string());
    }
}
