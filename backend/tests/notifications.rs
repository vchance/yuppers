//! Notifications end to end: a change to an exchange queues the message for
//! the other party, and the worker's delivery sends each one once.
//!
//! Delivery acts on every queued message in the database, so the tests here
//! take turns, and each starts with an empty outbox.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use common::{App, Deal, User, accept};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tokio::sync::MutexGuard;
use uuid::Uuid;
use yuppers_backend::auth::SendFuture;
use yuppers_backend::exchanges::service::run_timers;
use yuppers_backend::notifications::outbox::{
    Delivered, Delivery, DeliveryRules, deliver_due, deliver_due_until,
};
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::notifications::{Email, EmailSender};

const DATABASE: &str = "yuppers_test_notifications";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn clear_outbox(app: &App) {
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
}

/// The guard keeps the other tests waiting until this one is done.
async fn app() -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    let app = App::start(DATABASE).await;
    clear_outbox(&app).await;
    (app, turn)
}

/// A deal already under way, with the messages from getting there discarded.
async fn active(app: &App) -> Deal {
    let deal = app.active().await;
    clear_outbox(app).await;
    deal
}

/// Stands in for an email provider: keeps what it was asked to send, and can
/// be told to fail or to be slow.
#[derive(Default)]
struct Provider {
    sent: Mutex<Vec<Email>>,
    attempts: AtomicUsize,
    /// How many sends fail before one succeeds.
    failures: AtomicUsize,
    delay: Option<std::time::Duration>,
    in_flight: AtomicUsize,
    /// The most sends that were ever under way at the same moment.
    most_at_once: AtomicUsize,
}

impl Provider {
    fn failing(failures: usize) -> Self {
        Self {
            failures: AtomicUsize::new(failures),
            ..Self::default()
        }
    }

    fn slow(delay: std::time::Duration) -> Self {
        Self {
            delay: Some(delay),
            ..Self::default()
        }
    }

    fn sent(&self) -> Vec<Email> {
        self.sent.lock().unwrap().clone()
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl EmailSender for Provider {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_at_once.fetch_max(in_flight, Ordering::SeqCst);
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            let failing = self
                .failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok();
            if failing {
                anyhow::bail!("the provider is down");
            }
            self.sent.lock().unwrap().push(email.clone());
            Ok(())
        })
    }
}

fn delivery(provider: &Arc<Provider>, rules: DeliveryRules) -> Delivery {
    Delivery {
        sender: provider.clone(),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules,
    }
}

/// A moment by which everything queued so far is due.
fn soon() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::seconds(5)
}

/// Delivers everything due, through a provider that works.
async fn deliver(app: &App) -> (Delivered, Vec<Email>) {
    let provider = Arc::new(Provider::default());
    let delivered = deliver_due(
        &app.db,
        &delivery(&provider, DeliveryRules::default()),
        soon(),
    )
    .await
    .unwrap();
    (delivered, provider.sent())
}

/// What was queued for an exchange, oldest first, as
/// `recipient: NOTICE (event it is about)`.
async fn queued(app: &App, deal: &Deal) -> Vec<String> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT o.recipient_account_id, o.payload->>'notice', e.type
         FROM outbox o
         JOIN exchange_event e
           ON e.exchange_id = o.exchange_id AND e.sequence = o.event_sequence
         WHERE o.exchange_id = $1 AND o.kind = 'EMAIL'
         ORDER BY o.event_sequence, o.id",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();

    rows.into_iter()
        .map(|(recipient, notice, event)| {
            let name = if recipient == deal.ana.id {
                "Ana"
            } else if recipient == deal.ben.id {
                "Ben"
            } else {
                "someone else"
            };
            format!("{name}: {notice} ({event})")
        })
        .collect()
}

/// One queued row: attempts, last error, whether it is completed, and when
/// it is next due.
type Row = (i32, Option<String>, bool, OffsetDateTime);

async fn rows(app: &App, deal: &Deal) -> Vec<Row> {
    sqlx::query_as(
        "SELECT attempts, last_error, completed_at IS NOT NULL, available_at
         FROM outbox WHERE exchange_id = $1 ORDER BY id",
    )
    .bind(deal.exchange.parse::<Uuid>().unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

async fn only_row(app: &App, deal: &Deal) -> Row {
    let mut rows = rows(app, deal).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows.remove(0)
}

fn contribution(id: Uuid, action: &str, note: Option<&str>) -> Value {
    json!({ "type": "CONTRIBUTION", "contribution": id, "action": action, "note": note })
}

async fn claim(app: &App, deal: &Deal) {
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
}

async fn display_code(app: &App, user: &User, exchange: &str) -> String {
    app.view(user, exchange).await["display_code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn each_party_is_told_what_the_other_did_and_never_their_own_action() {
    let (app, _turn) = app().await;

    // Ana proposes. She is not told about her own proposal, and nobody holds
    // the other slot yet.
    let deal = app.negotiating().await;
    assert_eq!(queued(&app, &deal).await, Vec::<String>::new());

    claim(&app, &deal).await;
    app.command(&deal.ben, &deal.exchange, accept(&deal.revision))
        .await
        .ok();
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();

    let note = "The gate still sticks.";
    for (who, what) in [
        (&deal.ana, contribution(deal.repair, "CLAIM", None)),
        (&deal.ben, contribution(deal.repair, "DISPUTE", Some(note))),
        (
            &deal.ana,
            contribution(deal.repair, "CLAIM", Some("Fixed.")),
        ),
        (&deal.ben, contribution(deal.repair, "CONFIRM", None)),
        (&deal.ben, contribution(deal.payment, "CLAIM", None)),
        (&deal.ana, contribution(deal.payment, "CONFIRM", None)),
    ] {
        app.command(who, &deal.exchange, what).await.ok();
    }
    assert_eq!(
        app.view(&deal.ana, &deal.exchange).await["closed_outcome"],
        "COMPLETED"
    );

    assert_eq!(
        queued(&app, &deal).await,
        [
            "Ana: INVITATION_CLAIMED_UNCONFIRMED (COUNTERPARTY_CLAIMED)",
            "Ana: ACCEPTANCE_WAITING (REVISION_ACCEPTED)",
            // Confirming Ben brought his waiting acceptance into force: two
            // events, one message. It is the one message that also goes to
            // whoever acted, because it tells each signer where their copy
            // of the agreement is.
            "Ana: AGREEMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ben: AGREEMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ben: DELIVERY_CLAIMED (CONTRIBUTION_CLAIMED)",
            "Ana: DISPUTE_OPENED (CONTRIBUTION_DISPUTED)",
            "Ben: DELIVERY_CLAIMED (CONTRIBUTION_CLAIMED)",
            "Ana: DELIVERY_CONFIRMED (CONTRIBUTION_CONFIRMED)",
            "Ana: DELIVERY_CLAIMED (CONTRIBUTION_CLAIMED)",
            // The last confirmation completed the exchange, and that is what
            // Ben is told, once.
            "Ben: CLOSED_COMPLETED (EXCHANGE_CLOSED)",
        ]
    );

    // Each goes to its recipient's address, links to the exchange, and says
    // nothing of what the two agreed or wrote.
    let (delivered, emails) = deliver(&app).await;
    assert_eq!(delivered.sent, 10);
    let code = display_code(&app, &deal.ana, &deal.exchange).await;
    let link = format!("https://app.test/exchanges/{}", deal.exchange);
    let to_ana = emails.iter().filter(|e| e.to == deal.ana.email).count();
    let to_ben = emails.iter().filter(|e| e.to == deal.ben.email).count();
    assert_eq!((to_ana, to_ben), (6, 4));
    for email in &emails {
        assert!(email.subject.contains(&code), "{}", email.subject);
        assert!(email.body.contains(&link), "{}", email.body);
        // The link is left out of the search: an exchange ID is random
        // letters and digits, and may spell anything.
        let text = format!("{}\n{}", email.subject, email.body)
            .replace(&link, "")
            .to_lowercase();
        for private in ["fence", "ruiz", "ortiz", "400", "payment", "gate", "fixed"] {
            assert!(!text.contains(private), "{private:?} leaked into {text}");
        }
    }
}

#[tokio::test]
async fn nothing_is_queued_for_a_slot_nobody_has_claimed() {
    let (app, _turn) = app().await;

    // Ana takes her offer back before anyone opens the link: she did it, and
    // there is no one else.
    let withdrawn = app.negotiating().await;
    app.command(
        &withdrawn.ana,
        &withdrawn.exchange,
        json!({ "type": "WITHDRAW", "revision": withdrawn.revision }),
    )
    .await
    .ok();
    assert_eq!(queued(&app, &withdrawn).await, Vec::<String>::new());

    // An offer nobody opened runs out. The timer's news is for both parties,
    // and only one of them exists.
    let unanswered = app.negotiating().await;
    let later = OffsetDateTime::now_utc() + Duration::days(15);
    run_timers(&app.db, &app.rules, later).await.unwrap();
    assert_eq!(
        queued(&app, &unanswered).await,
        ["Ana: CLOSED_EXPIRED (EXCHANGE_CLOSED)"]
    );

    let (unaddressed, strangers): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE recipient_account_id IS NULL),
                count(*) FILTER (WHERE NOT EXISTS (
                    SELECT 1 FROM participant p
                    WHERE p.exchange_id = o.exchange_id
                      AND p.account_id = o.recipient_account_id))
         FROM outbox o",
    )
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((unaddressed, strangers), (0, 0));

    // Ben, who never claimed the link, hears nothing.
    let (_, emails) = deliver(&app).await;
    assert!(emails.iter().all(|email| email.to != unanswered.ben.email));
    assert!(emails.iter().any(|email| email.to == unanswered.ana.email));
}

#[tokio::test]
async fn what_the_timers_do_is_told_to_both_parties() {
    let (app, _turn) = app().await;
    let now = OffsetDateTime::now_utc();

    let stuck = active(&app).await;
    let idle = active(&app).await;
    app.command(
        &stuck.ana,
        &stuck.exchange,
        json!({ "type": "REQUEST_CLOSE" }),
    )
    .await
    .ok();
    assert_eq!(
        queued(&app, &stuck).await,
        ["Ben: CLOSE_REQUESTED (CLOSE_REQUESTED)"]
    );

    run_timers(&app.db, &app.rules, now + Duration::days(8))
        .await
        .unwrap();
    assert_eq!(
        queued(&app, &stuck).await[1..],
        [
            "Ana: CLOSED_UNRESOLVED (EXCHANGE_CLOSED)",
            "Ben: CLOSED_UNRESOLVED (EXCHANGE_CLOSED)",
        ]
    );

    run_timers(&app.db, &app.rules, now + Duration::days(61))
        .await
        .unwrap();
    assert_eq!(
        queued(&app, &idle).await,
        [
            "Ana: INACTIVITY_PROMPTED (INACTIVITY_PROMPTED)",
            "Ben: INACTIVITY_PROMPTED (INACTIVITY_PROMPTED)",
        ]
    );
}

#[tokio::test]
async fn a_request_that_changes_nothing_queues_nothing() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;

    // Refused: the repair is Ana's to mark as delivered, not Ben's.
    app.act(&deal.ben, &deal.exchange, deal.repair, "CLAIM")
        .await
        .refused(StatusCode::FORBIDDEN, "WRONG_ACTOR");
    assert_eq!(queued(&app, &deal).await, Vec::<String>::new());

    // A retry of a request that already went through is not a second event.
    let version = app.view(&deal.ana, &deal.exchange).await["version"].clone();
    let body = json!({
        "expected_version": version,
        "command": contribution(deal.repair, "CLAIM", None),
    });
    for _ in 0..2 {
        app.call(
            Some(&deal.ana),
            Method::POST,
            &format!("/v1/exchanges/{}/commands", deal.exchange),
            Some(body.clone()),
            &[("idempotency-key", "claim-once")],
        )
        .await
        .ok();
    }
    assert_eq!(
        queued(&app, &deal).await,
        ["Ben: DELIVERY_CLAIMED (CONTRIBUTION_CLAIMED)"]
    );
}

#[tokio::test]
async fn an_account_that_cannot_be_emailed_is_not_emailed() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;

    // Queued while Ben had an address, which he then replaces with a phone
    // number before the worker gets to it.
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    let phone = format!("+1555{:07}", Uuid::new_v4().as_u128() % 10_000_000);
    common::set_phone(&app.db, deal.ben.id, &phone, true).await;

    let (delivered, emails) = deliver(&app).await;
    assert_eq!((delivered.sent, delivered.dropped), (0, 1));
    assert_eq!(emails, []);
    let (attempts, reason, completed, _) = only_row(&app, &deal).await;
    assert_eq!((attempts, completed), (0, true), "closed without a try");
    assert!(reason.as_deref().unwrap().starts_with("not sent"));

    // From here on there is no address to queue anything for.
    app.act(&deal.ana, &deal.exchange, deal.repair, "RETRACT_CLAIM")
        .await
        .ok();
    assert_eq!(queued(&app, &deal).await.len(), 1);

    // Nor is a suspended account told anything.
    let other = active(&app).await;
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1")
        .bind(other.ben.id)
        .execute(&app.db)
        .await
        .unwrap();
    app.act(&other.ana, &other.exchange, other.repair, "CLAIM")
        .await
        .ok();
    assert_eq!(queued(&app, &other).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_message_is_written_in_its_recipients_language() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    // Ben reads Spanish. Ana's preference is a language the product does not
    // have, so she gets the default.
    for (user, language) in [(&deal.ben, "es"), (&deal.ana, "tlh")] {
        sqlx::query("UPDATE account SET language = $2 WHERE id = $1")
            .bind(user.id)
            .bind(language)
            .execute(&app.db)
            .await
            .unwrap();
    }

    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();

    let (_, emails) = deliver(&app).await;
    let code = display_code(&app, &deal.ana, &deal.exchange).await;
    let link = format!("https://app.test/exchanges/{}", deal.exchange);
    let [to_ben, to_ana] = &emails[..] else {
        panic!("two emails, got {emails:?}");
    };

    assert_eq!(to_ben.to, deal.ben.email);
    assert_eq!(
        to_ben.subject,
        format!("Algo se marcó como entregado ({code})")
    );
    assert!(to_ben.body.contains(&format!("Abre el yup: {link}")));

    assert_eq!(to_ana.to, deal.ana.email);
    assert_eq!(to_ana.subject, format!("A delivery was confirmed ({code})"));
    assert!(to_ana.body.contains(&format!("Open the yup: {link}")));
}

#[tokio::test]
async fn each_signer_is_told_where_their_signed_agreement_is_kept() {
    let (app, _turn) = app().await;
    let deal = app.negotiating().await;
    claim(&app, &deal).await;
    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    sqlx::query("UPDATE account SET language = 'es' WHERE id = $1")
        .bind(deal.ben.id)
        .execute(&app.db)
        .await
        .unwrap();
    clear_outbox(&app).await;

    // Ben signs, which brings the agreement into force. Ana is told, and so
    // is Ben, although it was his own doing: it is his copy too.
    app.command(&deal.ben, &deal.exchange, accept(&deal.revision))
        .await
        .ok();
    assert_eq!(
        queued(&app, &deal).await,
        [
            "Ana: AGREEMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ben: AGREEMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
        ]
    );

    let (delivered, emails) = deliver(&app).await;
    assert_eq!(delivered.sent, 2);
    let code = display_code(&app, &deal.ana, &deal.exchange).await;
    let link = format!("https://app.test/exchanges/{}", deal.exchange);
    let record = format!("{link}/record");
    let [to_ana, to_ben] = &emails[..] else {
        panic!("two emails, got {emails:?}");
    };

    // Each in their own language, each with the way to the record, which
    // asks them to sign in, and to the exchange.
    assert_eq!(to_ana.to, deal.ana.email);
    assert_eq!(
        to_ana.subject,
        format!("Your agreement is signed and on record ({code})")
    );
    assert!(to_ana.body.contains(&format!(
        "Sign in to read, print or download your copy at any time: {record}\n"
    )));
    assert!(to_ana.body.contains(&format!("Open the yup: {link}\n")));

    assert_eq!(to_ben.to, deal.ben.email);
    assert_eq!(
        to_ben.subject,
        format!("Tu acuerdo está firmado y registrado ({code})")
    );
    assert!(to_ben.body.contains(&format!(
        "Inicia sesión para leer, imprimir o descargar tu copia cuando quieras: {record}\n"
    )));
    assert!(to_ben.body.contains(&format!("Abre el yup: {link}\n")));

    // The agreement itself does not travel by email: no terms, no names, no
    // amounts, and nothing attached, since an email here is text and nothing
    // more.
    for email in &emails {
        let text = format!("{}\n{}", email.subject, email.body)
            .replace(&link, "")
            .to_lowercase();
        for private in ["fence", "ruiz", "ortiz", "400", "payment"] {
            assert!(!text.contains(private), "{private:?} leaked into {text}");
        }
    }
}

#[tokio::test]
async fn an_amendment_coming_into_force_sends_each_signer_their_copy_again() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;

    // Ana proposes a change of wording; Ben is told there is one to look at.
    let mut amended = common::fence_job(deal.repair, deal.payment);
    amended["terms"] = json!("Repair the back fence and oil the gate.");
    let sent = app.send(&deal.ana, &deal.exchange, amended).await.ok();
    let amendment = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    assert_eq!(
        queued(&app, &deal).await,
        ["Ben: AMENDMENT_PROPOSED (REVISION_SENT)"]
    );

    // Ben signs it. Both have now signed the changed agreement, which is what
    // the record holds from here on, so both are told again where it is.
    app.command(&deal.ben, &deal.exchange, accept(amendment))
        .await
        .ok();
    assert_eq!(
        queued(&app, &deal).await[1..],
        [
            "Ana: AMENDMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ben: AMENDMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
        ]
    );
    let (_, emails) = deliver(&app).await;
    let code = display_code(&app, &deal.ana, &deal.exchange).await;
    let record = format!("https://app.test/exchanges/{}/record", deal.exchange);
    for (email, to) in emails[1..].iter().zip([&deal.ana, &deal.ben]) {
        assert_eq!(email.to, to.email);
        assert_eq!(
            email.subject,
            format!("The change to your agreement is signed and on record ({code})")
        );
        assert!(email.body.contains(&record), "{}", email.body);
        assert!(!email.body.to_lowercase().contains("gate"));
    }

    // An amendment that settles everything is two events, the agreement
    // changing and the exchange closing: each signer gets their copy, and
    // the one who did not act is also told that it is over.
    app.act(&deal.ben, &deal.exchange, deal.repair, "CONFIRM")
        .await
        .ok();
    clear_outbox(&app).await;
    let mut settled = common::fence_job(deal.repair, deal.payment);
    settled["terms"] = json!("Repair the back fence and oil the gate.");
    settled["contributions"].as_array_mut().unwrap().truncate(1);
    let sent = app.send(&deal.ana, &deal.exchange, settled).await.ok();
    let amendment = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let view = app
        .command(&deal.ben, &deal.exchange, accept(amendment))
        .await
        .ok();
    assert_eq!(view["closed_outcome"], "COMPLETED");
    assert_eq!(
        queued(&app, &deal).await[1..],
        [
            "Ana: AMENDMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ben: AMENDMENT_IN_FORCE (AGREEMENT_IN_FORCE)",
            "Ana: CLOSED_COMPLETED (EXCHANGE_CLOSED)",
        ]
    );
}

#[tokio::test]
async fn a_delivered_message_is_not_sent_again() {
    let (app, _turn) = app().await;
    let deal = app.active().await;
    // Three pieces of news on the way to an agreement, and the copy of it
    // that goes to the one who signed last.
    assert_eq!(rows(&app, &deal).await.len(), 4);

    let provider = Arc::new(Provider::default());
    let delivery = delivery(&provider, DeliveryRules::default());
    assert_eq!(
        deliver_due(&app.db, &delivery, soon()).await.unwrap(),
        Delivered {
            sent: 4,
            ..Delivered::default()
        }
    );
    // Later passes, however late, find nothing to do.
    for at in [soon(), soon() + Duration::days(30)] {
        assert!(
            deliver_due(&app.db, &delivery, at)
                .await
                .unwrap()
                .is_empty()
        );
    }

    assert_eq!(provider.sent().len(), 4);
    for (attempts, error, completed, _) in rows(&app, &deal).await {
        assert_eq!((attempts, error, completed), (1, None, true));
    }
}

#[tokio::test]
async fn workers_draining_at_once_send_each_message_once() {
    let (app, _turn) = app().await;
    for _ in 0..4 {
        app.active().await;
    }
    let queued: Vec<i64> = sqlx::query_scalar("SELECT id FROM outbox ORDER BY id")
        .fetch_all(&app.owner)
        .await
        .unwrap();
    assert_eq!(queued.len(), 16);

    // Slow enough that every worker is mid-send while the others look for
    // work.
    let provider = Arc::new(Provider::slow(std::time::Duration::from_millis(20)));
    let delivery = delivery(&provider, DeliveryRules::default());
    let at = soon();
    let (first, second, third) = tokio::join!(
        deliver_due(&app.db, &delivery, at),
        deliver_due(&app.db, &delivery, at),
        deliver_due(&app.db, &delivery, at),
    );
    let workers = [first.unwrap(), second.unwrap(), third.unwrap()];

    let sent: Vec<i64> = provider.sent().iter().map(|e| e.reference).collect();
    assert_eq!(sent.len(), 16, "nothing was sent twice");
    assert_eq!(
        sent.iter().copied().collect::<BTreeSet<i64>>(),
        queued.into_iter().collect::<BTreeSet<i64>>(),
        "and nothing was missed"
    );
    assert_eq!(workers.iter().map(|w| w.sent).sum::<usize>(), 16);
    assert!(
        workers.iter().all(|worker| worker.sent > 0),
        "the work was shared: {workers:?}"
    );
    // A worker passes over a message another is sending instead of waiting
    // for it, so all three were sending at the same moment.
    assert_eq!(provider.most_at_once.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_failing_send_is_retried_with_longer_waits_and_then_given_up_on() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    let provider = Arc::new(Provider::failing(usize::MAX));
    let rules = DeliveryRules {
        max_attempts: 3,
        retry_after: Duration::minutes(1),
        ..DeliveryRules::default()
    };
    let delivery = delivery(&provider, rules);
    let failed = Delivered {
        failed: 1,
        ..Delivered::default()
    };
    let start = soon();
    let state = || async {
        let (attempts, error, completed, available_at) = only_row(&app, &deal).await;
        assert!(!completed);
        (attempts, error, available_at)
    };
    // The wait is measured from when the send failed, which is a little
    // after the pass started.
    let waited = |from: OffsetDateTime, until: OffsetDateTime, minutes: i64| {
        let wait = until - from;
        wait >= Duration::minutes(minutes)
            && wait < Duration::minutes(minutes) + Duration::seconds(5)
    };

    // The first try fails. The failure is recorded and the next try put off.
    assert_eq!(
        deliver_due(&app.db, &delivery, start).await.unwrap(),
        failed
    );
    let (attempts, error, first_retry) = state().await;
    assert_eq!(attempts, 1);
    assert_eq!(error.as_deref(), Some("the provider is down"));
    assert!(waited(start, first_retry, 1), "{first_retry}");

    // Not before its time.
    let early = start + Duration::seconds(30);
    assert!(
        deliver_due(&app.db, &delivery, early)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(provider.attempts(), 1);

    // The second failure waits twice as long.
    assert_eq!(
        deliver_due(&app.db, &delivery, first_retry).await.unwrap(),
        failed
    );
    let (attempts, _, second_retry) = state().await;
    assert_eq!(attempts, 2);
    assert!(waited(first_retry, second_retry, 2), "{second_retry}");
    assert!(
        deliver_due(&app.db, &delivery, first_retry + Duration::minutes(1))
            .await
            .unwrap()
            .is_empty()
    );

    // The third is the last.
    assert_eq!(
        deliver_due(&app.db, &delivery, second_retry).await.unwrap(),
        Delivered {
            failed: 1,
            given_up: 1,
            ..Delivered::default()
        }
    );
    let later = start + Duration::days(30);
    assert!(
        deliver_due(&app.db, &delivery, later)
            .await
            .unwrap()
            .is_empty()
    );

    // It stays as it was left, for someone to look at.
    let (attempts, error, _) = state().await;
    assert_eq!(attempts, 3);
    assert_eq!(error.as_deref(), Some("the provider is down"));
    assert_eq!(provider.attempts(), 3);
    assert_eq!(provider.sent(), []);
}

#[tokio::test]
async fn a_send_that_fails_and_then_works_is_delivered_once() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    let provider = Arc::new(Provider::failing(1));
    let delivery = delivery(&provider, DeliveryRules::default());
    let start = soon();
    assert_eq!(
        deliver_due(&app.db, &delivery, start).await.unwrap().failed,
        1
    );
    // The wait runs from when the send failed, a moment after `start`.
    let retry = start + Duration::minutes(1) + Duration::seconds(5);
    assert_eq!(
        deliver_due(&app.db, &delivery, retry).await.unwrap().sent,
        1
    );

    let (attempts, error, completed, _) = only_row(&app, &deal).await;
    assert_eq!((attempts, error, completed), (2, None, true));
    assert_eq!(provider.sent().len(), 1);
    assert_eq!(provider.sent()[0].to, deal.ben.email);
}

#[tokio::test]
async fn a_provider_that_does_not_answer_counts_as_a_failed_send() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    let provider = Arc::new(Provider::slow(std::time::Duration::from_secs(5)));
    let rules = DeliveryRules {
        send_timeout: std::time::Duration::from_millis(50),
        ..DeliveryRules::default()
    };
    let delivered = deliver_due(&app.db, &delivery(&provider, rules), soon())
        .await
        .unwrap();
    assert_eq!(delivered.failed, 1);

    let (attempts, error, completed, _) = only_row(&app, &deal).await;
    assert_eq!((attempts, completed), (1, false));
    assert!(error.unwrap().starts_with("no answer within"));
    assert_eq!(provider.sent(), []);
}

#[tokio::test]
async fn a_pass_takes_no_new_message_once_its_time_budget_is_spent() {
    let (app, _turn) = app().await;
    for _ in 0..2 {
        app.active().await;
    }
    let queued = 8;

    // Each send takes 100 ms and the pass may take new messages for 250 ms:
    // it takes three, finishing the one under way, and leaves the rest.
    let provider = Arc::new(Provider::slow(std::time::Duration::from_millis(100)));
    let rules = DeliveryRules {
        batch_budget: std::time::Duration::from_millis(250),
        ..DeliveryRules::default()
    };
    let delivery = delivery(&provider, rules);
    let first = deliver_due(&app.db, &delivery, soon()).await.unwrap();
    assert!(first.cut_short, "{first:?}");
    assert!((2..=4).contains(&first.sent), "{first:?}");

    // The next pass goes on where it stopped.
    let mut sent = first.sent;
    while sent < queued {
        let next = deliver_due(&app.db, &delivery, soon()).await.unwrap();
        assert!(next.sent > 0, "{next:?}");
        sent += next.sent;
    }
    assert_eq!(provider.sent().len(), queued);
}

#[tokio::test]
async fn a_pass_stops_between_messages_when_the_worker_is_stopping() {
    let (app, _turn) = app().await;
    app.active().await;

    let provider = Arc::new(Provider::default());
    let delivery = delivery(&provider, DeliveryRules::default());
    // Asked to stop once the first message is on its way.
    let delivered = deliver_due_until(&app.db, &delivery, soon(), || provider.attempts() >= 1)
        .await
        .unwrap();
    assert_eq!((delivered.sent, delivered.cut_short), (1, true));
    assert_eq!(provider.sent().len(), 1);
}

#[tokio::test]
async fn a_retry_waits_from_when_the_send_failed_not_from_when_the_pass_began() {
    let (app, _turn) = app().await;
    let deal = active(&app).await;
    app.act(&deal.ana, &deal.exchange, deal.repair, "CLAIM")
        .await
        .ok();

    // A send that takes a second and then fails.
    let provider = Arc::new(Provider {
        failures: AtomicUsize::new(usize::MAX),
        delay: Some(std::time::Duration::from_secs(1)),
        ..Provider::default()
    });
    let delivery = delivery(&provider, DeliveryRules::default());
    let start = soon();
    assert_eq!(
        deliver_due(&app.db, &delivery, start).await.unwrap().failed,
        1
    );
    let (_, _, _, available_at) = only_row(&app, &deal).await;
    let wait = available_at - start;
    assert!(
        wait >= Duration::minutes(1) + Duration::seconds(1)
            && wait < Duration::minutes(1) + Duration::seconds(5),
        "{wait}"
    );
}
