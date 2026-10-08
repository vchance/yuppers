//! Report and block, end to end: HTTP requests in, stored reports and blocks
//! out, and what a block does to the exchanges two people share.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{Method, StatusCode};
use common::{App, Deal, Reply, User, accept, consent, fence_job};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;
use yuppers_backend::exchanges::service::run_timers;
use yuppers_backend::safety::{REPORT_DETAILS_MAX_CHARS, REPORTS_PER_ACCOUNT_PER_DAY};

const DATABASE: &str = "yuppers_test_safety";

async fn app() -> App {
    App::start(DATABASE).await
}

fn id(exchange: &str) -> Uuid {
    exchange.parse().unwrap()
}

/// reporter, the party reported, reason, details, status
type StoredReport = (Option<Uuid>, Option<Uuid>, String, Option<String>, String);

/// The reports stored about an exchange, oldest first.
async fn reports(app: &App, exchange: &str) -> Vec<StoredReport> {
    sqlx::query_as(
        "SELECT reporter_account_id, subject_account_id, reason, details, status
         FROM report WHERE subject_exchange_id = $1 ORDER BY created_at, id",
    )
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

async fn report(app: &App, user: &User, exchange: &str, body: Value) -> Reply {
    app.post(user, &format!("/v1/exchanges/{exchange}/reports"), body)
        .await
}

/// A report through an invitation link, signed in or not.
async fn report_link(app: &App, user: Option<&User>, body: Value) -> Reply {
    app.call(
        user,
        Method::POST,
        "/v1/invitations/report",
        Some(body),
        &[],
    )
    .await
}

async fn block_call(app: &App, user: &User, method: Method, exchange: &str) -> Reply {
    app.call(
        Some(user),
        method,
        &format!("/v1/exchanges/{exchange}/block"),
        None,
        &[],
    )
    .await
}

#[track_caller]
fn done(reply: Reply) {
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    assert_eq!(reply.body, Value::Null, "nothing comes back but the status");
}

async fn block(app: &App, user: &User, exchange: &str) {
    done(block_call(app, user, Method::PUT, exchange).await);
}

async fn has_blocked(app: &App, user: &User, exchange: &str) -> Value {
    block_call(app, user, Method::GET, exchange).await.ok()["blocked"].clone()
}

async fn blocked_people(app: &App, user: &User) -> Vec<Value> {
    app.get(user, "/v1/blocks")
        .await
        .ok()
        .as_array()
        .unwrap()
        .clone()
}

async fn claim(app: &App, user: &User, token: &str) -> Reply {
    app.post(user, "/v1/invitations/claim", json!({ "token": token }))
        .await
}

/// The types of an exchange's events with who caused each, oldest first.
async fn events(app: &App, exchange: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT type, actor_slot FROM exchange_event WHERE exchange_id = $1 ORDER BY sequence",
    )
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

/// The notices queued for a person about an exchange, oldest first.
async fn notices(app: &App, user: &User, exchange: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT payload->>'notice' FROM outbox
         WHERE recipient_account_id = $1 AND exchange_id = $2 ORDER BY id",
    )
    .bind(user.id)
    .bind(id(exchange))
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

/// `from` proposes the fence job to whoever opens the link. Returns the
/// exchange, the open revision and the invitation token.
async fn propose(app: &App, from: &User) -> (String, String, String) {
    let exchange = app.draft(from).await;
    let sent = app
        .send(from, &exchange, fence_job(Uuid::new_v4(), Uuid::new_v4()))
        .await
        .ok();
    (
        exchange,
        sent["exchange"]["open_revision"]["id"]
            .as_str()
            .unwrap()
            .to_owned(),
        sent["invitation_token"].as_str().unwrap().to_owned(),
    )
}

/// `from` proposes to `to`, who opens the link. Nothing is signed by `to`.
async fn negotiation(app: &App, from: &User, to: &User) -> (String, String) {
    let (exchange, revision, token) = propose(app, from).await;
    claim(app, to, &token).await.ok();
    (exchange, revision)
}

/// A second agreement in force between the two people of a deal.
async fn another_agreement(app: &App, deal: &Deal) -> String {
    let (exchange, revision) = negotiation(app, &deal.ana, &deal.ben).await;
    app.command(
        &deal.ana,
        &exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    let view = app
        .command(&deal.ben, &exchange, accept(&revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    exchange
}

async fn block_each_other(app: &App, deal: &Deal) {
    for user in [&deal.ana, &deal.ben] {
        block(app, user, &deal.exchange).await;
        assert_eq!(has_blocked(app, user, &deal.exchange).await, true);
    }
}

// ---- Reports ----------------------------------------------------------------

#[tokio::test]
async fn a_party_reports_the_exchange_and_the_other_party_is_told_nothing() {
    let app = app().await;
    let deal = app.active().await;
    let before = app.view(&deal.ana, &deal.exchange).await;
    let told_before = notices(&app, &deal.ana, &deal.exchange).await;

    done(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "HARASSMENT", "details": "  Threatening messages by phone.  " }),
        )
        .await,
    );

    // Stored with what a reviewer needs: who, about which exchange and which
    // party, why, and when.
    assert_eq!(
        reports(&app, &deal.exchange).await,
        [(
            Some(deal.ben.id),
            Some(deal.ana.id),
            "HARASSMENT".to_owned(),
            Some("Threatening messages by phone.".to_owned()),
            "OPEN".to_owned(),
        )]
    );
    let age: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM now() - created_at)::float8 FROM report
         WHERE subject_exchange_id = $1",
    )
    .bind(id(&deal.exchange))
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert!((0.0..60.0).contains(&age), "{age}");

    // Nothing about the exchange changed, and nothing was sent to anyone.
    assert_eq!(app.view(&deal.ana, &deal.exchange).await, before);
    assert_eq!(notices(&app, &deal.ana, &deal.exchange).await, told_before);
    assert_eq!(events(&app, &deal.exchange).await.len(), 5);

    // Either party can report, at any stage, the initiator included.
    done(
        report(
            &app,
            &deal.ana,
            &deal.exchange,
            json!({ "reason": "SCAM", "details": null }),
        )
        .await,
    );
    let stored = reports(&app, &deal.exchange).await;
    assert_eq!(
        stored[1],
        (
            Some(deal.ana.id),
            Some(deal.ben.id),
            "SCAM".to_owned(),
            None,
            "OPEN".to_owned()
        )
    );
}

#[tokio::test]
async fn reporting_the_same_exchange_again_changes_nothing() {
    let app = app().await;
    let deal = app.active().await;
    let first = json!({ "reason": "UNWANTED", "details": "I never asked for this." });

    done(report(&app, &deal.ben, &deal.exchange, first.clone()).await);
    // The same again, and a different one: both answered like the first.
    done(report(&app, &deal.ben, &deal.exchange, first).await);
    done(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "HARASSMENT", "details": "And again." }),
        )
        .await,
    );

    let stored = reports(&app, &deal.exchange).await;
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].2, "UNWANTED");
    assert_eq!(stored[0].3.as_deref(), Some("I never asked for this."));

    // Once a reviewer has dealt with it, a new report is a new matter.
    sqlx::query(
        "UPDATE report SET status = 'DISMISSED', resolved_at = now()
         WHERE subject_exchange_id = $1",
    )
    .bind(id(&deal.exchange))
    .execute(&app.db)
    .await
    .unwrap();
    done(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "HARASSMENT", "details": "It started again." }),
        )
        .await,
    );
    assert_eq!(reports(&app, &deal.exchange).await.len(), 2);
}

#[tokio::test]
async fn a_report_is_bounded_and_needs_a_reason_from_the_list() {
    let app = app().await;
    let deal = app.active().await;
    let invalid = |reply: Reply| {
        reply.refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    };

    let too_long = "é".repeat(REPORT_DETAILS_MAX_CHARS + 1);
    invalid(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "SCAM", "details": too_long }),
        )
        .await,
    );
    // "Something else" has to say what.
    invalid(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "OTHER" }),
        )
        .await,
    );
    invalid(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "OTHER", "details": "   " }),
        )
        .await,
    );
    // A reason of the caller's own invention, or none.
    invalid(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "BECAUSE", "details": "x" }),
        )
        .await,
    );
    invalid(report(&app, &deal.ben, &deal.exchange, json!({ "details": "x" })).await);
    assert!(reports(&app, &deal.exchange).await.is_empty());

    // Exactly the limit is fine, counted in characters.
    let longest = "é".repeat(REPORT_DETAILS_MAX_CHARS);
    done(
        report(
            &app,
            &deal.ben,
            &deal.exchange,
            json!({ "reason": "OTHER", "details": longest }),
        )
        .await,
    );
    let stored = reports(&app, &deal.exchange).await;
    assert_eq!(stored[0].3.as_deref(), Some(longest.as_str()));

    // With nobody on the other side there is no one to report.
    let ana = app.user("Ana").await;
    let draft = app.draft(&ana).await;
    report(&app, &ana, &draft, json!({ "reason": "SCAM" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    let (unclaimed, _, _) = propose(&app, &ana).await;
    report(&app, &ana, &unclaimed, json!({ "reason": "SCAM" }))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    assert!(reports(&app, &unclaimed).await.is_empty());
}

#[tokio::test]
async fn an_account_can_file_only_so_many_reports_in_a_day() {
    let app = app().await;
    let deal = app.active().await;
    let second = another_agreement(&app, &deal).await;
    let third = another_agreement(&app, &deal).await;
    let body = json!({ "reason": "SCAM", "details": null });

    // Ben has already filed all but one of the day's reports, and one more
    // that is older than a day and no longer counts.
    for age in
        std::iter::repeat_n("1 hour", REPORTS_PER_ACCOUNT_PER_DAY as usize - 1).chain(["25 hours"])
    {
        sqlx::query(
            "INSERT INTO report (reporter_account_id, subject_account_id, reason, created_at)
             VALUES ($1, $2, 'SCAM', now() - $3::interval)",
        )
        .bind(deal.ben.id)
        .bind(deal.ana.id)
        .bind(age)
        .execute(&app.db)
        .await
        .unwrap();
    }

    done(report(&app, &deal.ben, &deal.exchange, body.clone()).await);
    report(&app, &deal.ben, &second, body.clone())
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    assert!(reports(&app, &second).await.is_empty());

    // Repeating the report already filed is still answered as received: it
    // stores nothing, so it is not what the limit is for.
    done(report(&app, &deal.ben, &deal.exchange, body.clone()).await);
    assert_eq!(reports(&app, &deal.exchange).await.len(), 1);

    // The limit is Ben's alone.
    done(report(&app, &deal.ana, &third, body).await);
}

#[tokio::test]
async fn someone_who_is_not_a_party_is_answered_as_if_there_were_no_exchange() {
    let app = app().await;
    let deal = app.active().await;
    let stranger = app.user("Carla").await;
    let nowhere = Uuid::new_v4().to_string();
    let body = json!({ "reason": "HARASSMENT", "details": "x" });

    // Each request, on the real exchange and on one that does not exist:
    // the same status and the same body.
    for exchange in [deal.exchange.as_str(), nowhere.as_str(), "not-an-id"] {
        let replies = [
            report(&app, &stranger, exchange, body.clone()).await,
            // Refused before the report is even looked at.
            report(&app, &stranger, exchange, json!({ "reason": "OTHER" })).await,
            block_call(&app, &stranger, Method::GET, exchange).await,
            block_call(&app, &stranger, Method::PUT, exchange).await,
            block_call(&app, &stranger, Method::DELETE, exchange).await,
        ];
        for reply in replies {
            reply.refused(StatusCode::NOT_FOUND, "NOT_FOUND");
            assert_eq!(reply.body, json!({ "code": "NOT_FOUND" }), "{exchange}");
        }
    }

    // Nothing was stored, and nobody was blocked.
    assert!(reports(&app, &deal.exchange).await.is_empty());
    let blocks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM account_block
         WHERE blocker_account_id = $1 OR blocked_account_id = $1",
    )
    .bind(stranger.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(blocks, 0);
    assert_eq!(blocked_people(&app, &stranger).await, Vec::<Value>::new());

    // Without a session, a party's endpoints say so and nothing more.
    let path = format!("/v1/exchanges/{}", deal.exchange);
    for (method, path, body) in [
        (Method::POST, format!("{path}/reports"), Some(body)),
        (Method::GET, format!("{path}/block"), None),
        (Method::PUT, format!("{path}/block"), None),
        (Method::DELETE, format!("{path}/block"), None),
        (Method::GET, "/v1/blocks".to_owned(), None),
    ] {
        app.call(None, method, &path, body, &[])
            .await
            .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    }
}

#[tokio::test]
async fn the_holder_of_a_live_link_reports_the_proposal_once_signed_in() {
    let app = app().await;
    let deal = app.negotiating().await;
    let token = &deal.invitation;
    let body =
        json!({ "token": token, "reason": "UNWANTED", "details": "I don't know this person." });

    // Signed out, a report is refused before the link is looked at.
    report_link(&app, None, body.clone())
        .await
        .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    assert!(reports(&app, &deal.exchange).await.is_empty());

    // Signed in, Ben reports it: he is the reporter, Ana the person reported.
    done(report_link(&app, Some(&deal.ben), body.clone()).await);
    assert_eq!(
        reports(&app, &deal.exchange).await,
        [(
            Some(deal.ben.id),
            Some(deal.ana.id),
            "UNWANTED".to_owned(),
            Some("I don't know this person.".to_owned()),
            "OPEN".to_owned(),
        )]
    );

    // The same report again stores nothing, and another is still his one.
    done(report_link(&app, Some(&deal.ben), body).await);
    done(
        report_link(
            &app,
            Some(&deal.ben),
            json!({ "token": token, "reason": "SCAM", "details": "Something more." }),
        )
        .await,
    );
    assert_eq!(reports(&app, &deal.exchange).await.len(), 1);

    // The proposal is still there for him to read, and Ana sees nothing of this.
    app.post(
        &deal.ben,
        "/v1/invitations/preview",
        json!({ "token": token }),
    )
    .await
    .ok();
    assert_eq!(notices(&app, &deal.ana, &deal.exchange).await.len(), 0);
    assert_eq!(app.view(&deal.ana, &deal.exchange).await["version"], 1);

    // The same bounds as for a party.
    let carla = app.user("Carla").await;
    report_link(
        &app,
        Some(&carla),
        json!({ "token": token, "reason": "OTHER" }),
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    report_link(
        &app,
        Some(&carla),
        json!({ "token": token, "reason": "SCAM", "details": "x".repeat(REPORT_DETAILS_MAX_CHARS + 1) }),
    )
    .await
    .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");

    // Ana opening her own link has nobody to report.
    report_link(
        &app,
        Some(&deal.ana),
        json!({ "token": token, "reason": "SCAM" }),
    )
    .await
    .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    assert_eq!(reports(&app, &deal.exchange).await.len(), 1);
}

#[tokio::test]
async fn reports_from_links_count_against_the_accounts_daily_limit() {
    let app = app().await;
    let deal = app.negotiating().await;
    let other = app.negotiating().await;
    let carla = app.user("Carla").await;

    // Carla has already filed all but one of the day's reports.
    for _ in 1..REPORTS_PER_ACCOUNT_PER_DAY {
        sqlx::query(
            "INSERT INTO report (reporter_account_id, subject_account_id, reason, created_at)
             VALUES ($1, $2, 'SCAM', now() - interval '1 hour')",
        )
        .bind(carla.id)
        .bind(deal.ana.id)
        .execute(&app.db)
        .await
        .unwrap();
    }

    let scam = |token: &str| json!({ "token": token, "reason": "SCAM" });
    done(report_link(&app, Some(&carla), scam(&deal.invitation)).await);
    report_link(&app, Some(&carla), scam(&other.invitation))
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    assert!(reports(&app, &other.exchange).await.is_empty());

    // The limit is Carla's alone.
    done(report_link(&app, Some(&other.ben), scam(&other.invitation)).await);
}

#[tokio::test]
async fn a_dead_link_takes_no_report_and_says_nothing_about_why() {
    let app = app().await;
    let dead = |token: &str| json!({ "token": token, "reason": "SCAM", "details": "x" });

    // Replaced by the initiator.
    let revoked = app.negotiating().await;
    app.post(
        &revoked.ana,
        &format!("/v1/exchanges/{}/invitation", revoked.exchange),
        json!({ "for_anyone": true }),
    )
    .await
    .ok();

    // Already used.
    let claimed = app.negotiating().await;
    claim(&app, &claimed.ben, &claimed.invitation).await.ok();

    // Run out.
    let expired = app.negotiating().await;
    sqlx::query(
        "UPDATE invitation SET expires_at = now() - interval '1 minute' WHERE exchange_id = $1",
    )
    .bind(id(&expired.exchange))
    .execute(&app.db)
    .await
    .unwrap();

    // Its proposal withdrawn, so there is nothing left to show.
    let withdrawn = app.negotiating().await;
    app.command(
        &withdrawn.ana,
        &withdrawn.exchange,
        json!({ "type": "WITHDRAW", "revision": withdrawn.revision }),
    )
    .await
    .ok();

    // Still live, as a reference for what signing out does.
    let live = app.negotiating().await;

    let carla = app.user("Carla").await;
    let unknown = app
        .post(
            &carla,
            "/v1/invitations/preview",
            json!({ "token": "made-up" }),
        )
        .await;
    unknown.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    let signed_out = report_link(&app, None, dead("made-up")).await;
    signed_out.refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");

    for (token, exchange) in [
        ("made-up", None),
        ("", None),
        (revoked.invitation.as_str(), Some(&revoked.exchange)),
        (claimed.invitation.as_str(), Some(&claimed.exchange)),
        (expired.invitation.as_str(), Some(&expired.exchange)),
        (withdrawn.invitation.as_str(), Some(&withdrawn.exchange)),
    ] {
        // Signed in, and even when the report itself is not valid: exactly
        // what the preview says about a link it cannot show.
        for reply in [
            report_link(&app, Some(&carla), dead(token)).await,
            report_link(
                &app,
                Some(&carla),
                json!({ "token": token, "reason": "OTHER" }),
            )
            .await,
        ] {
            assert_eq!(
                (reply.status, &reply.body),
                (unknown.status, &unknown.body),
                "{token}"
            );
        }
        if let Some(exchange) = exchange {
            assert!(reports(&app, exchange).await.is_empty(), "{token}");
        }
    }

    // Signed out, dead or live, valid or not, it is the same refusal.
    for token in [
        "made-up",
        "",
        revoked.invitation.as_str(),
        claimed.invitation.as_str(),
        live.invitation.as_str(),
    ] {
        for body in [dead(token), json!({ "token": token, "reason": "OTHER" })] {
            let reply = report_link(&app, None, body).await;
            assert_eq!(
                (reply.status, &reply.body),
                (signed_out.status, &signed_out.body),
                "{token}"
            );
        }
    }
    assert!(reports(&app, &live.exchange).await.is_empty());

    // The person who used the link is a party now, and reports as one.
    done(
        report(
            &app,
            &claimed.ben,
            &claimed.exchange,
            json!({ "reason": "SCAM" }),
        )
        .await,
    );
}

// ---- Blocks -----------------------------------------------------------------

#[tokio::test]
async fn a_person_blocks_the_other_party_sees_them_listed_and_unblocks_them() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben, exchange) = (&deal.ana, &deal.ben, deal.exchange.as_str());
    let code = app.view(ana, exchange).await["display_code"].clone();

    assert_eq!(has_blocked(&app, ana, exchange).await, false);
    assert_eq!(blocked_people(&app, ana).await, Vec::<Value>::new());

    block(&app, ana, exchange).await;
    assert_eq!(has_blocked(&app, ana, exchange).await, true);

    // Listed by the name the exchange gives him and its display code, with
    // nothing that identifies his account.
    let listed = blocked_people(&app, ana).await;
    assert_eq!(listed.len(), 1);
    let entry = listed[0].as_object().unwrap();
    assert_eq!(entry["exchange_id"], exchange);
    assert_eq!(entry["display_code"], code);
    assert_eq!(entry["name"], "Ben Ortiz");
    let mut fields: Vec<&str> = entry.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["blocked_at", "display_code", "exchange_id", "name"]
    );
    let text = listed[0].to_string();
    assert!(!text.contains(&ben.id.to_string()) && !text.contains(&ben.email));

    // Blocking again is harmless.
    block(&app, ana, exchange).await;
    assert_eq!(blocked_people(&app, ana).await.len(), 1);

    // Unblocking, and twice is harmless.
    done(block_call(&app, ana, Method::DELETE, exchange).await);
    assert_eq!(has_blocked(&app, ana, exchange).await, false);
    assert_eq!(blocked_people(&app, ana).await, Vec::<Value>::new());
    done(block_call(&app, ana, Method::DELETE, exchange).await);

    // One person, however many exchanges the two share: blocked through any
    // of them, shown by the latest, and unblocked through any of them.
    let second = another_agreement(&app, &deal).await;
    block(&app, ana, exchange).await;
    assert_eq!(has_blocked(&app, ana, &second).await, true);
    let listed = blocked_people(&app, ana).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["exchange_id"], second.as_str());
    done(block_call(&app, ana, Method::DELETE, &second).await);
    assert_eq!(has_blocked(&app, ana, exchange).await, false);

    // With nobody on the other side there is no one to block.
    let draft = app.draft(ana).await;
    let (unclaimed, _, _) = propose(&app, ana).await;
    for exchange in [&draft, &unclaimed] {
        assert_eq!(has_blocked(&app, ana, exchange).await, false);
        for method in [Method::PUT, Method::DELETE] {
            block_call(&app, ana, method, exchange)
                .await
                .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
        }
    }
}

#[tokio::test]
async fn a_block_keeps_the_two_out_of_each_others_new_exchanges_until_it_is_lifted() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben) = (&deal.ana, &deal.ben);
    let carla = app.user("Carla").await;

    let unknown = claim(&app, ben, "made-up").await;
    unknown.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");

    block(&app, ana, &deal.exchange).await;

    // Ben cannot join an exchange Ana starts, nor Ana one that Ben starts.
    // Each is told what anyone with a dead link is told.
    let (from_ana, _, to_ben) = propose(&app, ana).await;
    let (from_ben, _, to_ana) = propose(&app, ben).await;
    for (who, token) in [(ben, &to_ben), (ana, &to_ana)] {
        let reply = claim(&app, who, token).await;
        assert_eq!((reply.status, &reply.body), (unknown.status, &unknown.body));
    }
    for (owner, exchange) in [(ana, &from_ana), (ben, &from_ben)] {
        assert_eq!(app.view(owner, exchange).await["counterparty"], "UNCLAIMED");
    }

    // The links themselves are fine: someone else can use one.
    claim(&app, &carla, &to_ben).await.ok();

    // Lifted, the two can deal again.
    done(block_call(&app, ana, Method::DELETE, &deal.exchange).await);
    let view = claim(&app, ana, &to_ana).await.ok();
    assert_eq!(view["you"], "B");
}

#[tokio::test]
async fn a_block_ends_what_was_waiting_to_be_signed_between_the_two() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben) = (&deal.ana, &deal.ben);
    let carla = app.user("Carla").await;

    // Ana's offer to Ben, which he could otherwise still sign.
    let (offered, offered_revision) = negotiation(&app, ana, ben).await;
    // Ben's offer to Ana, whom he has confirmed, so that she can decline it.
    let (received, _) = negotiation(&app, ben, ana).await;
    app.command(ben, &received, json!({ "type": "CONFIRM_COUNTERPARTY" }))
        .await
        .ok();
    // An amendment Ana proposed to the agreement in force.
    let mut amended = fence_job(deal.repair, deal.payment);
    amended["terms"] = json!("Repair the back fence and the gate.");
    app.send(ana, &deal.exchange, amended).await.ok();
    // An amendment Ben proposed to another agreement of theirs.
    let second = another_agreement(&app, &deal).await;
    let mut amended = app.view(ben, &second).await["in_force_revision"]["terms"].clone();
    amended["terms"] = json!("Repair the back fence by Friday.");
    app.send(ben, &second, amended).await.ok();
    // Ana's offer to someone else, and one nobody has opened.
    let (with_carla, _) = negotiation(&app, ana, &carla).await;
    let (unopened, _, _) = propose(&app, ana).await;

    let agreement = app.view(ben, &deal.exchange).await["in_force_revision"].clone();

    // Through one exchange; the others follow.
    block(&app, ana, &deal.exchange).await;

    // What Ana sent is withdrawn, what she was sent is declined: both closed
    // without agreement, in her name, as if she had done it by hand.
    for (exchange, reason, event, notice) in [
        (
            &offered,
            "WITHDRAWN",
            "REVISION_WITHDRAWN",
            "CLOSED_WITHDRAWN",
        ),
        (
            &received,
            "DECLINED",
            "REVISION_DECLINED",
            "CLOSED_DECLINED",
        ),
    ] {
        let view = app.view(ben, exchange).await;
        assert_eq!(
            (
                &view["state"],
                &view["closed_outcome"],
                &view["closed_reason"]
            ),
            (&json!("CLOSED"), &json!("NOT_AGREED"), &json!(reason)),
            "{exchange}"
        );
        let ana_slot = app.view(ana, exchange).await["you"]
            .as_str()
            .unwrap()
            .to_owned();
        let history = events(&app, exchange).await;
        assert_eq!(
            history[history.len() - 2..],
            [
                (event.to_owned(), Some(ana_slot.clone())),
                ("EXCHANGE_CLOSED".to_owned(), Some(ana_slot)),
            ]
        );
        // Ben is told what he would be told of any withdrawal or refusal.
        assert_eq!(notices(&app, ben, exchange).await.last().unwrap(), notice);
    }
    // A closed negotiation shows no terms; Ana is still told whom she blocked.
    assert_eq!(
        block_call(&app, ana, Method::GET, &offered).await.ok(),
        json!({ "blocked": true, "name": "Ben Ortiz" })
    );
    // Ben can no longer sign the offer Ana left open.
    app.command(ben, &offered, accept(&offered_revision))
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

    // The proposed amendments are gone; the agreements stand as they were.
    for exchange in [&deal.exchange, &second] {
        let view = app.view(ben, exchange).await;
        assert_eq!(
            (&view["state"], &view["open_revision"]),
            (&json!("ACTIVE"), &Value::Null)
        );
    }
    assert_eq!(
        app.view(ben, &deal.exchange).await["in_force_revision"],
        agreement
    );
    assert_eq!(
        notices(&app, ben, &deal.exchange).await.last().unwrap(),
        "AMENDMENT_WITHDRAWN"
    );
    assert_eq!(
        notices(&app, ben, &second).await.last().unwrap(),
        "AMENDMENT_DECLINED"
    );

    // Nothing of Ana's with anyone else is touched.
    assert_eq!(app.view(ana, &with_carla).await["state"], "NEGOTIATING");
    assert_eq!(app.view(ana, &unopened).await["state"], "NEGOTIATING");

    // Blocking again later does not sweep up what came since: Ben proposes
    // an amendment, and it stays for Ana to answer herself.
    let mut amended = fence_job(deal.repair, deal.payment);
    amended["terms"] = json!("Repair the back fence. Paint it too.");
    app.send(ben, &deal.exchange, amended).await.ok();
    block(&app, ana, &deal.exchange).await;
    assert!(app.view(ana, &deal.exchange).await["open_revision"].is_object());
}

#[tokio::test]
async fn a_claim_racing_a_block_never_leaves_an_offer_open_to_the_person_blocked() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben) = (&deal.ana, &deal.ben);

    for round in 0..8 {
        // Ana has an offer out, and blocks Ben at the moment he opens it.
        let (exchange, _, token) = propose(&app, ana).await;
        let (claimed, blocked) = tokio::join!(
            claim(&app, ben, &token),
            block_call(&app, ana, Method::PUT, &deal.exchange),
        );
        done(blocked);

        // Whichever came first, Ben is not left able to sign it: either he
        // never got in, or he did and the block withdrew the offer.
        let view = app.view(ana, &exchange).await;
        if claimed.status == StatusCode::OK {
            assert_eq!(
                (&view["state"], &view["closed_reason"]),
                (&json!("CLOSED"), &json!("WITHDRAWN")),
                "round {round}"
            );
        } else {
            claimed.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
            assert_eq!(view["counterparty"], "UNCLAIMED", "round {round}");
        }

        done(block_call(&app, ana, Method::DELETE, &deal.exchange).await);
    }
}

#[tokio::test]
async fn a_block_by_someone_not_yet_confirmed_takes_them_out_of_the_exchange() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let carla = app.user("Carla").await;

    // Ben has opened Ana's link and signed her offer; it waits for her to
    // confirm who he is. Carla has opened another of Ana's and not signed.
    let (exchange, revision) = negotiation(&app, &ana, &ben).await;
    app.command(&ben, &exchange, accept(&revision)).await.ok();
    let (unsigned, _) = negotiation(&app, &ana, &carla).await;
    let offers = [
        app.view(&ana, &exchange).await["open_revision"]["terms"].clone(),
        app.view(&ana, &unsigned).await["open_revision"]["terms"].clone(),
    ];

    // He cannot decline, and no command takes a signature back. Blocking
    // her must still leave nothing she could bind him with: he leaves.
    block(&app, &ben, &exchange).await;
    block(&app, &carla, &unsigned).await;

    for ((exchange, who), offer) in [(&exchange, &ben), (&unsigned, &carla)]
        .into_iter()
        .zip(offers)
    {
        let view = app.view(&ana, exchange).await;
        assert_eq!(
            (&view["state"], &view["counterparty"], &view["claimant"]),
            (&json!("NEGOTIATING"), &json!("UNCLAIMED"), &Value::Null),
            "her offer stays open, with nobody in the other place"
        );
        assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
        assert_eq!(view["open_revision"]["terms"], offer);
        // Nothing is left for her to confirm.
        app.command(&ana, exchange, json!({ "type": "CONFIRM_COUNTERPARTY" }))
            .await
            .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");

        // She sees someone leave, as anyone might, and nothing of a block.
        let history = events(&app, exchange).await;
        assert_eq!(
            history.last().unwrap(),
            &("COUNTERPARTY_RELEASED".to_owned(), Some("B".to_owned()))
        );
        assert_eq!(
            notices(&app, &ana, exchange).await.last().unwrap(),
            "CLAIMANT_LEFT"
        );
        assert_eq!(
            block_call(&app, &ana, Method::GET, exchange).await.ok(),
            json!({ "blocked": false, "name": "Ben Ortiz" })
        );

        // The exchange is gone for whoever left it.
        app.get(who, &format!("/v1/exchanges/{exchange}"))
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
        block_call(&app, who, Method::GET, exchange)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    }

    // The block stands: a new link from Ana is a dead one to Ben.
    let (_, _, token) = propose(&app, &ana).await;
    claim(&app, &ben, &token)
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");

    // It is still his to see and to lift, through the exchange he left,
    // which is the only one the two ever shared. She has since renamed
    // herself on a new version of the offer; he is shown the name he saw.
    let mut renamed = app.view(&ana, &exchange).await["open_revision"]["terms"].clone();
    renamed["party_a_name"] = json!("A. Ruiz");
    app.send(&ana, &exchange, renamed).await.ok();
    let people = blocked_people(&app, &ben).await;
    assert_eq!(people.len(), 1);
    assert_eq!(
        (
            &people[0]["exchange_id"],
            &people[0]["name"],
            &people[0]["left"]
        ),
        (&json!(exchange), &json!("Ana Ruiz"), &json!(true))
    );
    // Someone who never blocked her gets nothing from that exchange, having
    // been in it or not.
    let dana = app.user("Dana").await;
    let (visited, _) = negotiation(&app, &ana, &dana).await;
    app.call(
        Some(&dana),
        Method::POST,
        &format!("/v1/exchanges/{visited}/leave"),
        None,
        &[],
    )
    .await;
    for (who, exchange) in [(&dana, &visited), (&dana, &exchange)] {
        block_call(&app, who, Method::DELETE, exchange)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    }

    done(block_call(&app, &ben, Method::DELETE, &exchange).await);
    assert_eq!(blocked_people(&app, &ben).await, Vec::<Value>::new());
    block_call(&app, &ben, Method::DELETE, &exchange)
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    // With the block lifted he can open a link from her again. What he
    // signed before he left does not come back with him.
    let token = app
        .post(
            &ana,
            &format!("/v1/exchanges/{exchange}/invitation"),
            json!({ "for_anyone": true }),
        )
        .await
        .ok()["invitation_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let view = claim(&app, &ben, &token).await.ok();
    assert_eq!(view["open_revision"]["accepted_by"], json!(["A"]));
}

#[tokio::test]
async fn an_agreement_in_force_can_still_be_completed_or_closed_by_either_after_blocks() {
    let app = app().await;
    // Completed: each delivers and the other confirms.
    let deal = app.active().await;
    block_each_other(&app, &deal).await;
    let exchange = deal.exchange.as_str();
    app.act(&deal.ana, exchange, deal.repair, "CLAIM")
        .await
        .ok();
    app.act(&deal.ben, exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    app.act(&deal.ben, exchange, deal.payment, "CLAIM")
        .await
        .ok();
    let view = app
        .act(&deal.ana, exchange, deal.payment, "CONFIRM")
        .await
        .ok();
    assert_eq!(
        (&view["state"], &view["closed_outcome"]),
        (&json!("CLOSED"), &json!("COMPLETED"))
    );

    // Disputed, put right, and then amended by agreement: the whole of what
    // an agreement allows is still there.
    let deal = app.active().await;
    block_each_other(&app, &deal).await;
    let exchange = deal.exchange.as_str();
    app.act(&deal.ana, exchange, deal.repair, "CLAIM")
        .await
        .ok();
    app.command(
        &deal.ben,
        exchange,
        json!({ "type": "CONTRIBUTION", "contribution": deal.repair, "action": "DISPUTE", "note": "The gate still sticks." }),
    )
    .await
    .ok();
    let mut amended = fence_job(deal.repair, deal.payment);
    amended["terms"] = json!("Repair the back fence and the gate.");
    let sent = app.send(&deal.ana, exchange, amended).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let view = app
        .command(&deal.ben, exchange, accept(revision))
        .await
        .ok();
    assert_eq!(
        view["in_force_revision"]["terms"]["terms"],
        "Repair the back fence and the gate."
    );
    // Ended by agreement: Ben proposes, Ana agrees.
    app.command(&deal.ben, exchange, json!({ "type": "PROPOSE_END" }))
        .await
        .ok();
    let view = app
        .command(&deal.ana, exchange, json!({ "type": "ACCEPT_END" }))
        .await
        .ok();
    assert_eq!(view["closed_outcome"], "ENDED_BY_AGREEMENT");

    // Closed by one party alone, whichever of the two asks. These are the
    // only close requests this file leaves pending, so running the timers a
    // week ahead touches nothing of any other test here.
    let by_ana = app.active().await;
    let by_ben = app.active().await;
    block_each_other(&app, &by_ana).await;
    block_each_other(&app, &by_ben).await;
    for (who, deal) in [(&by_ana.ana, &by_ana), (&by_ben.ben, &by_ben)] {
        app.command(
            who,
            &deal.exchange,
            json!({ "type": "REQUEST_CLOSE", "note": "We can't settle this." }),
        )
        .await
        .ok();
    }
    let later = OffsetDateTime::now_utc() + app.rules.close_response_window + Duration::hours(1);
    assert_eq!(run_timers(&app.db, &app.rules, later).await.unwrap(), 2);
    for deal in [&by_ana, &by_ben] {
        let view = app.view(&deal.ana, &deal.exchange).await;
        assert_eq!(
            (&view["closed_outcome"], &view["closed_reason"]),
            (&json!("UNRESOLVED"), &json!("CLOSE_REQUEST"))
        );
    }
}

#[tokio::test]
async fn nothing_tells_a_person_they_were_blocked() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben, exchange) = (&deal.ana, &deal.ben, deal.exchange.as_str());

    // What Ben can see before.
    let view = app.view(ben, exchange).await;
    let list = app.get(ben, "/v1/exchanges").await.ok();
    let told = notices(&app, ben, exchange).await;
    let history = events(&app, exchange).await;
    let dead = claim(&app, ben, "made-up").await;

    block(&app, ana, exchange).await;

    // And after: the exchange, his list, his messages and the history are
    // what they were, to the byte.
    let mut seen = vec![
        app.get(ben, &format!("/v1/exchanges/{exchange}")).await,
        app.get(ben, "/v1/exchanges").await,
    ];
    assert_eq!(seen[0].body, view);
    assert_eq!(seen[1].body, list);
    assert_eq!(notices(&app, ben, exchange).await, told);
    assert_eq!(events(&app, exchange).await, history);

    // His own side of blocking says only what he has done himself.
    let status = block_call(&app, ben, Method::GET, exchange).await;
    let own = json!({ "blocked": false, "name": "Ana Ruiz" });
    assert_eq!(status.body, own);
    let people = app.get(ben, "/v1/blocks").await;
    assert_eq!(people.body, json!([]));
    seen.extend([status, people]);

    // An invitation from Ana is a dead link like any other.
    let (_, _, token) = propose(&app, ana).await;
    let refused = claim(&app, ben, &token).await;
    assert_eq!((refused.status, &refused.body), (dead.status, &dead.body));
    seen.push(refused);

    // Everything he can do in the exchange answers as it did: acting on it,
    // reporting it, blocking and unblocking Ana himself.
    seen.push(
        app.command(ben, exchange, json!({ "type": "PROPOSE_END" }))
            .await,
    );
    assert_eq!(seen.last().unwrap().status, StatusCode::OK);
    done(report(&app, ben, exchange, json!({ "reason": "SCAM" })).await);
    block(&app, ben, exchange).await;
    let people = app.get(ben, "/v1/blocks").await;
    assert_eq!(people.body[0]["name"], "Ana Ruiz");
    done(block_call(&app, ben, Method::DELETE, exchange).await);
    // Unblocking Ana lifts his block, not hers.
    assert_eq!(has_blocked(&app, ana, exchange).await, true);
    seen.push(block_call(&app, ben, Method::GET, exchange).await);
    assert_eq!(seen.last().unwrap().body, own);

    // No reply to Ben says anything of a block but his own "blocked: false".
    for reply in &seen {
        let text = reply.body.to_string().to_lowercase();
        let mentions = text.matches("block").count();
        let own = text.matches("\"blocked\":false").count();
        assert_eq!(mentions, own, "{text}");
    }
}

/// `from` proposes the fence job to the one person `to` names, an email
/// address. Returns the invitation token.
async fn bound_proposal(app: &App, from: &User, to: &str) -> String {
    let exchange = app.draft(from).await;
    let sent = app
        .post(
            from,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": consent(),
                "invitation": { "bound_to": to },
            }),
        )
        .await
        .ok();
    sent["invitation_token"].as_str().unwrap().to_owned()
}

/// What each endpoint that takes an invitation token answers `user`, or
/// someone signed out, about `token`: the status, the content type and the
/// body, by the call that gave it.
async fn answers_about(
    app: &App,
    user: Option<&User>,
    token: &str,
) -> Vec<(&'static str, StatusCode, Option<String>, Value)> {
    let calls: [(&'static str, &str, Value); 5] = [
        (
            "preview",
            "/v1/invitations/preview",
            json!({ "token": token }),
        ),
        (
            "report",
            "/v1/invitations/report",
            json!({ "token": token, "reason": "SCAM", "details": "x" }),
        ),
        (
            "report, not valid",
            "/v1/invitations/report",
            json!({ "token": token, "reason": "OTHER" }),
        ),
        (
            "only if yours",
            "/v1/invitations/claim",
            json!({ "token": token, "only_if_yours": true }),
        ),
        ("claim", "/v1/invitations/claim", json!({ "token": token })),
    ];
    let mut answers = Vec::new();
    for (name, path, body) in calls {
        let reply = app.call(user, Method::POST, path, Some(body), &[]).await;
        let content_type = reply
            .headers
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_owned());
        answers.push((name, reply.status, content_type, reply.body));
    }
    answers
}

/// The property behind DESIGN.md §18 item 13a: whatever the person blocked
/// asks about a link from the person who blocked them, signed in or signed
/// out, is answered exactly as the same question about a made-up link.
/// Signed out, nobody learns anything about any link; signed in, a block
/// between the two looks like a dead link everywhere. (A second account is
/// someone else to the service; README, "Reading an invitation".)
#[tokio::test]
async fn a_blocked_person_gets_the_made_up_links_answers_about_the_blockers_links() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben) = (&deal.ana, &deal.ben);
    let carla = app.user("Carla").await;
    block(&app, ana, &deal.exchange).await;

    // Live links from Ana: for whoever opens it, bound to Ben, bound to Carla.
    let (open_exchange, _, open) = propose(&app, ana).await;
    let to_ben = bound_proposal(&app, ana, &ben.email).await;
    let to_carla = bound_proposal(&app, ana, &carla.email).await;

    for user in [None, Some(ben)] {
        let made_up = answers_about(&app, user, "made-up").await;
        for token in [&open, &to_ben, &to_carla] {
            assert_eq!(
                answers_about(&app, user, token).await,
                made_up,
                "signed in: {}",
                user.is_some()
            );
        }
    }
    // Signed out, that answer is the refusal to someone without a session,
    // the same for every link.
    for (call, status, _, body) in answers_about(&app, None, &open).await {
        assert_eq!(
            (status, body),
            (
                StatusCode::UNAUTHORIZED,
                json!({ "code": "UNAUTHENTICATED" })
            ),
            "{call}"
        );
    }

    // The same holds when the block is the other way round: Ben blocking
    // Ana leaves him nothing to compare either.
    let both = app.active().await;
    block(&app, &both.ben, &both.exchange).await;
    let (_, _, hers) = propose(&app, &both.ana).await;
    let made_up = answers_about(&app, Some(&both.ben), "made-up").await;
    assert_eq!(answers_about(&app, Some(&both.ben), &hers).await, made_up);

    // None of it touched the links or filed anything.
    assert!(reports(&app, &open_exchange).await.is_empty());
    // The comparison would have caught a difference: to someone not
    // blocked, a live link answers unlike a made-up one.
    let preview = |token: &str| json!({ "token": token });
    let shown = app
        .post(&carla, "/v1/invitations/preview", preview(&open))
        .await;
    let unknown = app
        .post(&carla, "/v1/invitations/preview", preview("made-up"))
        .await;
    assert_eq!(shown.status, StatusCode::OK);
    assert_ne!((shown.status, shown.body), (unknown.status, unknown.body));
    claim(&app, &carla, &to_carla).await.ok();
}

// ---- Round trips before the answer -------------------------------------------

/// Counts the statements sent to the database while it is on.
struct Statements(Arc<AtomicUsize>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Statements {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        // sqlx reports each statement it has run, once, at this target.
        if event.metadata().target() == "sqlx::query" {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// The body of a request about a link, made from its token.
type Body = fn(&str) -> Value;

/// How many statements the request made, by its reply.
async fn round_trips(app: &App, user: &User, path: &str, body: Value) -> (Reply, usize) {
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(Statements(count.clone()));
    let reply = app.post(user, path, body).with_subscriber(subscriber).await;
    (reply, count.load(Ordering::SeqCst))
}

/// Timing as well as wording: a made-up token, a dead link, a link from
/// someone who blocked the viewer or whom the viewer blocked, and a link from
/// an account that is suspended are each refused after the same number of
/// database round trips, before anything about the exchange is loaded. So nothing tells them apart by how long the
/// refusal takes (the earlier code loaded the exchange, and in a claim locked
/// it, only for a live link).
#[tokio::test]
async fn every_dead_link_is_refused_after_the_same_round_trips_as_a_made_up_one() {
    let app = app().await;
    let deal = app.active().await;
    let (ana, ben) = (&deal.ana, &deal.ben);
    block(&app, ana, &deal.exchange).await;
    let carla = app.user("Carla").await;

    // Live links from Ana, whom Ben has been blocked by.
    let (_, _, blocked_link) = propose(&app, ana).await;
    // A link Ana replaced.
    let (replaced, _, old_link) = propose(&app, ana).await;
    app.post(
        ana,
        &format!("/v1/exchanges/{replaced}/invitation"),
        json!({ "for_anyone": true }),
    )
    .await
    .ok();
    // A link from someone now suspended.
    let sam = app.user("Sam").await;
    let (_, _, suspended_link) = propose(&app, &sam).await;
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1")
        .bind(sam.id)
        .execute(&app.owner)
        .await
        .unwrap();
    // A link someone else already used.
    let (_, _, used_link) = propose(&app, &carla).await;
    claim(&app, &app.user("Dora").await, &used_link).await.ok();

    let calls: [(&str, Body); 4] = [
        ("/v1/invitations/preview", |token| json!({ "token": token })),
        ("/v1/invitations/claim", |token| json!({ "token": token })),
        (
            "/v1/invitations/claim",
            |token| json!({ "token": token, "only_if_yours": true }),
        ),
        (
            "/v1/invitations/report",
            |token| json!({ "token": token, "reason": "SCAM" }),
        ),
    ];
    // Once first, which registers the statement reports with the counter.
    round_trips(&app, ben, calls[0].0, calls[0].1("made-up")).await;
    for (path, body) in calls {
        let (made_up, expected) = round_trips(&app, ben, path, body("made-up")).await;
        assert!(expected > 0, "statements are counted");
        made_up.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
        for (what, token) in [
            ("blocked", &blocked_link),
            ("replaced", &old_link),
            ("suspended", &suspended_link),
            ("used", &used_link),
        ] {
            let (reply, trips) = round_trips(&app, ben, path, body(token)).await;
            reply.refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
            assert_eq!(trips, expected, "{path} {what}");
        }
    }

    // The count is real: a live link to someone it may be shown to goes on
    // to load the exchange, and makes more.
    let (_, _, live) = propose(&app, &carla).await;
    let (_, dead) = round_trips(
        &app,
        ben,
        "/v1/invitations/preview",
        json!({ "token": "made-up" }),
    )
    .await;
    let (shown, more) = round_trips(
        &app,
        ben,
        "/v1/invitations/preview",
        json!({ "token": live }),
    )
    .await;
    assert_eq!(shown.status, StatusCode::OK, "{}", shown.body);
    assert!(more > dead, "{more} > {dead}");
}
