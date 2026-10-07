//! Reminders end to end: the worker's pass records what each contribution
//! has been reminded about and queues the emails, each once, and delivery
//! sends only those that are still true.
//!
//! A pass looks at every exchange under way in the database at a given
//! moment, so this file has a database to itself, its tests take turns, and
//! each starts by putting away what the ones before it left under way.

mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{App, User, accept};
use serde_json::{Value, json};
use time::{Date, Duration, OffsetDateTime};
use tokio::sync::MutexGuard;
use uuid::Uuid;
use yuppers_backend::auth::SendFuture;
use yuppers_backend::domain::Rules;
use yuppers_backend::exchanges::reminders::run_reminders;
use yuppers_backend::notifications::outbox::{Delivered, Delivery, DeliveryRules, deliver_due};
use yuppers_backend::notifications::wording::Wording;
use yuppers_backend::notifications::{Email, EmailSender};

const DATABASE: &str = "yuppers_test_reminders";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The guard keeps the other tests waiting until this one is done.
async fn app() -> (App, MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    let app = App::start(DATABASE).await;
    // An exchange cannot be deleted, and one an earlier test left under way
    // would be reminded by this test's passes. So they are closed: directly,
    // as the schema owner, with none of the rules, which is not something
    // any test here is about.
    sqlx::query(
        "UPDATE exchange
         SET state = 'CLOSED', closed_at = now(), open_revision_id = NULL,
             closed_outcome = CASE WHEN in_force_revision_id IS NULL
                                   THEN 'NOT_AGREED' ELSE 'UNRESOLVED' END
         WHERE state <> 'CLOSED'",
    )
    .execute(&app.owner)
    .await
    .unwrap();
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
    (app, turn)
}

/// "Due soon" starts two days ahead here, whatever the default becomes.
fn rules() -> Rules {
    Rules {
        due_soon_lead: Duration::days(2),
        ..Rules::default()
    }
}

/// A due date a month from now: far enough ahead that nothing is near it
/// until a test says what time it is.
fn a_month_ahead() -> Date {
    OffsetDateTime::now_utc().date() + Duration::days(30)
}

/// A moment on a day, in UTC.
fn at(date: Date, hour: u8, minute: u8) -> OffsetDateTime {
    date.with_hms(hour, minute, 0).unwrap().assume_utc()
}

/// One pass of the worker's reminders. Returns how many it recorded.
async fn pass(app: &App, at: OffsetDateTime) -> usize {
    run_reminders(&app.db, &rules(), at).await.unwrap()
}

// ---- Agreements ---------------------------------------------------------------

struct Pair {
    ana: User,
    ben: User,
    exchange: String,
}

impl Pair {
    fn id(&self) -> Uuid {
        self.exchange.parse().unwrap()
    }
}

fn on(date: Date) -> Value {
    json!({ "kind": "DATE", "date": date.to_string() })
}

fn after(contribution: Uuid) -> Value {
    json!({ "kind": "AFTER_CONTRIBUTION", "contribution": contribution })
}

/// A contribution owed by `from`. The same call always describes it the same
/// way, so an amendment that repeats it leaves it untouched.
fn owed(id: Uuid, from: &str, due: Value) -> Value {
    json!({
        "id": id,
        "from": from,
        "type": "SERVICE",
        "description": "Repair the back fence",
        "quantity": null,
        "due": due,
        "completion_criteria": null,
        "required": true,
        "amount_minor": null,
    })
}

fn terms(contributions: Vec<Value>) -> Value {
    json!({
        "party_a_name": "Ana Ruiz",
        "party_b_name": "Ben Ortiz",
        "terms": "Repair the back fence.",
        "contributions": contributions,
    })
}

/// Ana proposes `contributions` to Ben, with due dates read in `timezone`.
/// Returns the pair, the proposal and the invitation.
async fn proposed(app: &App, timezone: &str, contributions: Vec<Value>) -> (Pair, String, String) {
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let view = app
        .post(&ana, "/v1/exchanges", json!({ "timezone": timezone }))
        .await
        .ok();
    let exchange = view["id"].as_str().unwrap().to_owned();
    let sent = app.send(&ana, &exchange, terms(contributions)).await.ok();
    let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let invitation = sent["invitation_token"].as_str().unwrap();
    (
        Pair { ana, ben, exchange },
        revision.to_owned(),
        invitation.to_owned(),
    )
}

/// Ben has joined and signed: the proposal is the agreement in force.
async fn agreed(app: &App, timezone: &str, contributions: Vec<Value>) -> Pair {
    let (pair, revision, invitation) = proposed(app, timezone, contributions).await;
    app.post(
        &pair.ben,
        "/v1/invitations/claim",
        json!({ "token": invitation }),
    )
    .await
    .ok();
    app.command(
        &pair.ana,
        &pair.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    let view = app
        .command(&pair.ben, &pair.exchange, accept(&revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    pair
}

/// Ana proposes a change to the agreement in force and Ben signs it.
async fn amend(app: &App, pair: &Pair, contributions: Vec<Value>) {
    let sent = app
        .send(&pair.ana, &pair.exchange, terms(contributions))
        .await
        .ok();
    let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    let view = app
        .command(&pair.ben, &pair.exchange, accept(revision))
        .await
        .ok();
    assert_eq!(view["open_revision"], Value::Null);
}

// ---- What was stored ------------------------------------------------------------

/// A reminder as recorded: the contribution, the kind, the due date.
type Recorded = (Uuid, String, Date);

fn soon(contribution: Uuid, due: Date) -> Recorded {
    (contribution, "DUE_SOON".to_owned(), due)
}

fn overdue(contribution: Uuid, due: Date) -> Recorded {
    (contribution, "OVERDUE".to_owned(), due)
}

/// What an exchange's contributions have been reminded about, in the order
/// it happened.
async fn recorded(app: &App, pair: &Pair) -> Vec<Recorded> {
    sqlx::query_as(
        "SELECT contribution_id, kind, due_date FROM contribution_reminder
         WHERE exchange_id = $1
         ORDER BY reminded_at, due_date, kind, contribution_id",
    )
    .bind(pair.id())
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

/// The reminder emails queued for an exchange, oldest first, as
/// `recipient: NOTICE`. The news of what the parties did is left out.
async fn queued(app: &App, pair: &Pair) -> Vec<String> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT recipient_account_id, payload->>'notice' FROM outbox
         WHERE exchange_id = $1 AND event_sequence IS NULL
         ORDER BY id",
    )
    .bind(pair.id())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(recipient, notice)| {
            let name = if recipient == pair.ana.id {
                "Ana"
            } else if recipient == pair.ben.id {
                "Ben"
            } else {
                "someone else"
            };
            format!("{name}: {notice}")
        })
        .collect()
}

/// A queued reminder: attempts, why it was closed or last failed, and
/// whether it is finished with.
async fn rows(app: &App, pair: &Pair) -> Vec<(i32, Option<String>, bool)> {
    sqlx::query_as(
        "SELECT attempts, last_error, completed_at IS NOT NULL FROM outbox
         WHERE exchange_id = $1 AND event_sequence IS NULL
         ORDER BY id",
    )
    .bind(pair.id())
    .fetch_all(&app.owner)
    .await
    .unwrap()
}

/// What a reminder must leave alone: the exchange's version, its last event,
/// when a party last acted, when it last changed, and how long its history
/// is.
type Untouched = (i64, i64, OffsetDateTime, OffsetDateTime, i64);

async fn untouched(app: &App, pair: &Pair) -> Untouched {
    sqlx::query_as(
        "SELECT version, last_event_seq, last_activity_at, updated_at,
                (SELECT count(*) FROM exchange_event WHERE exchange_id = e.id)
         FROM exchange e WHERE id = $1",
    )
    .bind(pair.id())
    .fetch_one(&app.owner)
    .await
    .unwrap()
}

// ---- Delivery -------------------------------------------------------------------

/// Stands in for an email provider: keeps what it was asked to send.
#[derive(Default)]
struct Inbox(Mutex<Vec<Email>>);

impl EmailSender for Inbox {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(email.clone());
            Ok(())
        })
    }
}

/// Delivers the reminders due at `at`. The news of what the parties did on
/// the way is discarded first; `notifications.rs` is about that.
async fn deliver(app: &App, at: OffsetDateTime) -> (Delivered, Vec<Email>) {
    sqlx::query("DELETE FROM outbox WHERE event_sequence IS NOT NULL")
        .execute(&app.db)
        .await
        .unwrap();
    let inbox = Arc::new(Inbox::default());
    let delivery = Delivery {
        sender: inbox.clone(),
        wording: Wording::embedded().unwrap(),
        web_origin: "https://app.test".to_owned(),
        rules: DeliveryRules::default(),
    };
    let delivered = deliver_due(&app.db, &delivery, at).await.unwrap();
    let sent = inbox.0.lock().unwrap().clone();
    (delivered, sent)
}

fn none() -> Vec<String> {
    Vec::new()
}

// ---- Tests ----------------------------------------------------------------------

#[tokio::test]
async fn the_provider_is_reminded_once_that_it_is_due_soon_and_both_once_that_it_is_overdue() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let day = |n: i64| due + Duration::days(n);
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());
    // Ana owes the repair on a date; Ben owes the payment once it is accepted.
    let pair = agreed(
        &app,
        "UTC",
        vec![
            owed(repair, "A", on(due)),
            owed(payment, "B", after(repair)),
        ],
    )
    .await;

    // Three days ahead is not soon.
    assert_eq!(pass(&app, at(day(-3), 23, 59)).await, 0);
    assert_eq!(queued(&app, &pair).await, none());

    // Two days ahead is. Ana, who owes it, is told; Ben is not.
    assert_eq!(pass(&app, at(day(-2), 0, 0)).await, 1);
    assert_eq!(queued(&app, &pair).await, ["Ana: DUE_SOON"]);
    // Every pass after that, to the end of the due date, finds it said.
    for later in [
        at(day(-2), 0, 0),
        at(day(-2), 12, 0),
        at(day(-1), 12, 0),
        at(due, 23, 59),
    ] {
        assert_eq!(pass(&app, later).await, 0, "{later}");
    }
    assert_eq!(queued(&app, &pair).await, ["Ana: DUE_SOON"]);

    // The day after the due date it is overdue, and both are told: Ana that
    // she owes it, Ben that it is owed to him.
    assert_eq!(pass(&app, at(day(1), 0, 0)).await, 1);
    let both = [
        "Ana: DUE_SOON",
        "Ana: OVERDUE_TO_DELIVER",
        "Ben: OVERDUE_TO_RECEIVE",
    ];
    assert_eq!(queued(&app, &pair).await, both);
    for later in [at(day(1), 0, 0), at(day(2), 12, 0), at(day(300), 12, 0)] {
        assert_eq!(pass(&app, later).await, 0, "{later}");
    }
    assert_eq!(queued(&app, &pair).await, both);

    // The payment has no date, so it was never due soon or overdue.
    assert_eq!(
        recorded(&app, &pair).await,
        [soon(repair, due), overdue(repair, due)]
    );
}

#[tokio::test]
async fn contributions_due_on_different_days_are_each_reminded_of_in_their_turn() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let next = due + Duration::days(1);
    let noon = |days: i64| at(due + Duration::days(days), 12, 0);
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    let pair = agreed(
        &app,
        "UTC",
        vec![owed(first, "A", on(due)), owed(second, "A", on(next))],
    )
    .await;

    // Each pass finds one of the two newly due, with the other already said
    // or not yet true.
    for days in [-2, -1, 1, 2] {
        assert_eq!(pass(&app, noon(days)).await, 1, "{days} days from the date");
        assert_eq!(pass(&app, noon(days)).await, 0);
    }
    assert_eq!(pass(&app, noon(0)).await, 0);
    assert_eq!(
        recorded(&app, &pair).await,
        [
            soon(first, due),
            soon(second, next),
            overdue(first, due),
            overdue(second, next),
        ]
    );
    assert_eq!(
        queued(&app, &pair).await,
        [
            "Ana: DUE_SOON",
            "Ana: DUE_SOON",
            "Ana: OVERDUE_TO_DELIVER",
            "Ben: OVERDUE_TO_RECEIVE",
            "Ana: OVERDUE_TO_DELIVER",
            "Ben: OVERDUE_TO_RECEIVE",
        ]
    );
}

#[tokio::test]
async fn a_reminder_leaves_the_exchange_as_it_was() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let repair = Uuid::new_v4();
    let pair = agreed(&app, "UTC", vec![owed(repair, "A", on(due))]).await;

    // What Ana's screen shows, and what the exchange holds, before any
    // reminder.
    let seen = app.view(&pair.ana, &pair.exchange).await;
    let history = app
        .get(
            &pair.ana,
            &format!("/v1/exchanges/{}/history", pair.exchange),
        )
        .await
        .ok();
    let before = untouched(&app, &pair).await;

    assert_eq!(pass(&app, at(due - Duration::days(1), 12, 0)).await, 1);
    assert_eq!(pass(&app, at(due + Duration::days(1), 12, 0)).await, 1);
    assert_eq!(queued(&app, &pair).await.len(), 3);

    // No version bump, no event, no activity: nothing a rate limit or the
    // inactivity clock could count, and nothing new in the history.
    assert_eq!(untouched(&app, &pair).await, before);
    assert_eq!(app.view(&pair.ana, &pair.exchange).await, seen);
    assert_eq!(
        app.get(
            &pair.ana,
            &format!("/v1/exchanges/{}/history", pair.exchange)
        )
        .await
        .ok(),
        history
    );

    // So what Ana does next, on the strength of what she saw before the
    // reminders, is not refused as stale.
    let reply = app
        .post(
            &pair.ana,
            &format!("/v1/exchanges/{}/commands", pair.exchange),
            json!({
                "expected_version": seen["version"],
                "command": { "type": "CONTRIBUTION", "contribution": repair, "action": "CLAIM" },
            }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn a_reminder_is_in_its_readers_language_and_says_nothing_of_the_terms() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let repair = Uuid::new_v4();
    let pair = agreed(&app, "UTC", vec![owed(repair, "A", on(due))]).await;
    sqlx::query("UPDATE account SET language = 'es' WHERE id = $1")
        .bind(pair.ben.id)
        .execute(&app.db)
        .await
        .unwrap();
    let code = app.view(&pair.ana, &pair.exchange).await["display_code"]
        .as_str()
        .unwrap()
        .to_owned();
    let link = format!("https://app.test/exchanges/{}", pair.exchange);
    let mut emails = Vec::new();

    let early = at(due - Duration::days(1), 12, 0);
    pass(&app, early).await;
    let (delivered, sent) = deliver(&app, early).await;
    assert_eq!(delivered.sent, 1);
    assert_eq!(sent[0].to, pair.ana.email);
    assert_eq!(
        sent[0].subject,
        format!("Something you owe is due soon ({code})")
    );
    emails.extend(sent);

    let late = at(due + Duration::days(1), 12, 0);
    pass(&app, late).await;
    let (delivered, sent) = deliver(&app, late).await;
    assert_eq!(delivered.sent, 2);
    let [to_ana, to_ben] = &sent[..] else {
        panic!("two emails, got {sent:?}");
    };
    assert_eq!(to_ana.to, pair.ana.email);
    assert_eq!(
        to_ana.subject,
        format!("Something you owe is overdue ({code})")
    );
    assert!(to_ana.body.contains(&format!("Open the yup: {link}")));
    assert_eq!(to_ben.to, pair.ben.email);
    assert_eq!(
        to_ben.subject,
        format!("Algo que te deben está atrasado ({code})")
    );
    assert!(to_ben.body.contains(&format!("Abre el yup: {link}")));
    emails.extend(sent);

    // Sent, and so never again.
    for later in [late, late + Duration::days(30)] {
        assert!(deliver(&app, later).await.0.is_empty());
    }

    // No names, no description, no date: only the display code and a link
    // that asks for a sign-in. The link itself is left out of the search,
    // since an exchange ID is random and may spell anything.
    let due = due.to_string();
    for email in &emails {
        assert!(!email.body.contains("/record"), "{}", email.body);
        let text = format!("{}\n{}", email.subject, email.body)
            .replace(&link, "")
            .to_lowercase();
        for private in ["fence", "repair", "ruiz", "ortiz", &due] {
            assert!(!text.contains(private), "{private:?} leaked into {text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn several_workers_at_once_send_each_reminder_once() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let mut pairs = Vec::new();
    for _ in 0..6 {
        // Each party owes the other something on the day.
        let contributions = vec![
            owed(Uuid::new_v4(), "A", on(due)),
            owed(Uuid::new_v4(), "B", on(due)),
        ];
        pairs.push(agreed(&app, "UTC", contributions).await);
    }
    let everyone = [
        "Ana: OVERDUE_TO_DELIVER",
        "Ana: OVERDUE_TO_RECEIVE",
        "Ben: OVERDUE_TO_DELIVER",
        "Ben: OVERDUE_TO_RECEIVE",
    ];

    let late = at(due + Duration::days(1), 12, 0);
    let (first, second, third, fourth) = tokio::join!(
        pass(&app, late),
        pass(&app, late),
        pass(&app, late),
        pass(&app, late),
    );
    assert_eq!(
        first + second + third + fourth,
        12,
        "two overdue contributions in each of six exchanges, each recorded by one worker"
    );
    for pair in &pairs {
        assert_eq!(recorded(&app, pair).await.len(), 2);
        // Two contributions overdue at once are one message to each party
        // about what they owe and one about what they are owed.
        assert_eq!(queued(&app, pair).await, everyone);
    }

    // And again, with everything already said.
    let (first, second) = tokio::join!(pass(&app, late), pass(&app, late));
    assert_eq!(first + second, 0);
    for pair in &pairs {
        assert_eq!(queued(&app, pair).await, everyone);
    }

    // Each queued message goes out once, however many workers deliver.
    let ((one, a), (two, b), (three, c)) = tokio::join!(
        deliver(&app, late),
        deliver(&app, late),
        deliver(&app, late)
    );
    assert_eq!(one.sent + two.sent + three.sent, 24);
    assert_eq!(one.dropped + two.dropped + three.dropped, 0);
    let mut references: Vec<i64> = [a, b, c].concat().iter().map(|e| e.reference).collect();
    references.sort_unstable();
    references.dedup();
    assert_eq!(references.len(), 24, "nothing was sent twice");
}

#[tokio::test]
async fn nothing_is_said_about_what_is_delivered_settled_removed_or_not_in_force() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let [
        claimed,
        disputed,
        accepted,
        waived,
        removed,
        pending,
        only_proposed,
    ] = [(); 7].map(|_| Uuid::new_v4());

    // Ana owes five things on the day and Ben one.
    let kept = vec![
        owed(claimed, "A", on(due)),
        owed(disputed, "A", on(due)),
        owed(accepted, "A", on(due)),
        owed(waived, "A", on(due)),
        owed(pending, "B", on(due)),
    ];
    let mut first = kept.clone();
    first.push(owed(removed, "A", on(due)));
    let pair = agreed(&app, "UTC", first).await;

    for (who, contribution, action, note) in [
        (&pair.ana, claimed, "CLAIM", None),
        (&pair.ana, disputed, "CLAIM", None),
        (
            &pair.ben,
            disputed,
            "DISPUTE",
            Some("The gate still sticks."),
        ),
        (&pair.ben, accepted, "CONFIRM", None),
        (&pair.ben, waived, "WAIVE", None),
    ] {
        let command = json!({
            "type": "CONTRIBUTION", "contribution": contribution, "action": action, "note": note,
        });
        app.command(who, &pair.exchange, command).await.ok();
    }
    // An amendment, signed by both, takes one contribution out.
    amend(&app, &pair, kept.clone()).await;
    // And one Ben has not signed would move his own date a month out and add
    // something of Ana's due on the day. It is a proposal: neither counts.
    let mut proposal = kept.clone();
    proposal[4] = owed(pending, "B", on(due + Duration::days(30)));
    proposal.push(owed(only_proposed, "A", on(due)));
    app.send(&pair.ana, &pair.exchange, terms(proposal))
        .await
        .ok();
    let view = app.view(&pair.ana, &pair.exchange).await;
    assert_eq!(view["state"], "ACTIVE");
    assert_ne!(view["open_revision"], Value::Null);

    // Terms that were offered and never agreed.
    let (offer, _, _) = proposed(&app, "UTC", vec![owed(Uuid::new_v4(), "A", on(due))]).await;
    // An agreement the two then ended.
    let ended = agreed(&app, "UTC", vec![owed(Uuid::new_v4(), "A", on(due))]).await;
    app.command(
        &ended.ana,
        &ended.exchange,
        json!({ "type": "PROPOSE_END" }),
    )
    .await
    .ok();
    let view = app
        .command(&ended.ben, &ended.exchange, json!({ "type": "ACCEPT_END" }))
        .await
        .ok();
    assert_eq!(view["closed_outcome"], "ENDED_BY_AGREEMENT");

    // Of all that, one contribution is in force and still pending: Ben's.
    assert_eq!(pass(&app, at(due - Duration::days(1), 12, 0)).await, 1);
    assert_eq!(queued(&app, &pair).await, ["Ben: DUE_SOON"]);
    assert_eq!(pass(&app, at(due + Duration::days(1), 12, 0)).await, 1);
    assert_eq!(
        queued(&app, &pair).await,
        [
            "Ben: DUE_SOON",
            "Ana: OVERDUE_TO_RECEIVE",
            "Ben: OVERDUE_TO_DELIVER",
        ]
    );
    assert_eq!(
        recorded(&app, &pair).await,
        [soon(pending, due), overdue(pending, due)]
    );

    for other in [&offer, &ended] {
        assert_eq!(recorded(&app, other).await, []);
        assert_eq!(queued(&app, other).await, none());
    }
    assert_eq!(pass(&app, at(due + Duration::days(60), 12, 0)).await, 0);
}

#[tokio::test]
async fn a_due_date_is_read_in_the_exchanges_own_timezone() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let day = |n: i64| due + Duration::days(n);
    // Neither place changes its clocks: Tokyo is nine hours ahead of UTC all
    // year and Honolulu ten behind. The same due date, in each.
    let (east, west) = (Uuid::new_v4(), Uuid::new_v4());
    let tokyo = agreed(&app, "Asia/Tokyo", vec![owed(east, "A", on(due))]).await;
    let honolulu = agreed(&app, "Pacific/Honolulu", vec![owed(west, "A", on(due))]).await;

    // Two days before the due date begins at midnight there: in Tokyo that is
    // 15:00 UTC on the day before, in Honolulu 10:00 UTC on the day itself.
    assert_eq!(pass(&app, at(day(-3), 14, 59)).await, 0);
    assert_eq!(pass(&app, at(day(-3), 15, 0)).await, 1);
    assert_eq!(recorded(&app, &tokyo).await, [soon(east, due)]);
    assert_eq!(recorded(&app, &honolulu).await, []);
    assert_eq!(pass(&app, at(day(-2), 9, 59)).await, 0);
    assert_eq!(pass(&app, at(day(-2), 10, 0)).await, 1);
    assert_eq!(recorded(&app, &honolulu).await, [soon(west, due)]);

    // The due date ends in Tokyo at 15:00 UTC on that same date. In Honolulu
    // it is then five in the morning with the whole day still to go.
    assert_eq!(pass(&app, at(due, 14, 59)).await, 0);
    assert_eq!(pass(&app, at(due, 15, 0)).await, 1);
    assert_eq!(
        recorded(&app, &tokyo).await,
        [soon(east, due), overdue(east, due)]
    );
    assert_eq!(recorded(&app, &honolulu).await, [soon(west, due)]);
    assert_eq!(
        queued(&app, &honolulu).await,
        ["Ana: DUE_SOON"],
        "it is the day after in UTC, and not yet in Honolulu"
    );
    assert_eq!(pass(&app, at(day(1), 0, 30)).await, 0);
    assert_eq!(pass(&app, at(day(1), 9, 59)).await, 0);
    assert_eq!(pass(&app, at(day(1), 10, 0)).await, 1);
    assert_eq!(
        recorded(&app, &honolulu).await,
        [soon(west, due), overdue(west, due)]
    );
}

#[tokio::test]
async fn an_amendment_that_moves_the_due_date_starts_the_reminders_again() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let moved = due + Duration::days(10);
    let noon = |date: Date, days: i64| at(date + Duration::days(days), 12, 0);
    let repair = Uuid::new_v4();
    let pair = agreed(&app, "UTC", vec![owed(repair, "A", on(due))]).await;

    // Ana proposes ten more days. Until Ben signs, the agreed date stands,
    // and it is the one she is reminded of.
    app.send(
        &pair.ana,
        &pair.exchange,
        terms(vec![owed(repair, "A", on(moved))]),
    )
    .await
    .ok();
    assert_eq!(pass(&app, noon(due, -1)).await, 1);
    assert_eq!(recorded(&app, &pair).await, [soon(repair, due)]);

    // Ben signs. The old date passes and nothing is overdue: nothing is owed
    // on it any more.
    let open = app.view(&pair.ben, &pair.exchange).await["open_revision"]["id"].clone();
    app.command(&pair.ben, &pair.exchange, accept(open.as_str().unwrap()))
        .await
        .ok();
    assert_eq!(pass(&app, noon(due, 1)).await, 0);
    assert_eq!(pass(&app, noon(moved, -3)).await, 0);

    // The new date is something she has not been reminded about: due soon
    // again, and then overdue, once each.
    assert_eq!(pass(&app, noon(moved, -2)).await, 1);
    assert_eq!(pass(&app, noon(moved, 0)).await, 0);
    assert_eq!(pass(&app, noon(moved, 1)).await, 1);
    let all = [
        soon(repair, due),
        soon(repair, moved),
        overdue(repair, moved),
    ];
    assert_eq!(recorded(&app, &pair).await, all);
    assert_eq!(
        queued(&app, &pair).await,
        [
            "Ana: DUE_SOON",
            "Ana: DUE_SOON",
            "Ana: OVERDUE_TO_DELIVER",
            "Ben: OVERDUE_TO_RECEIVE",
        ]
    );

    // An amendment that leaves the date alone, here adding something with no
    // date, brings no second reminder about it.
    let extra = Uuid::new_v4();
    amend(
        &app,
        &pair,
        vec![
            owed(repair, "A", on(moved)),
            owed(extra, "B", after(repair)),
        ],
    )
    .await;
    assert_eq!(pass(&app, noon(moved, 1)).await, 0);
    assert_eq!(pass(&app, noon(moved, 5)).await, 0);
    assert_eq!(recorded(&app, &pair).await, all);
}

#[tokio::test]
async fn a_reminder_overtaken_by_events_is_not_sent() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let noon = |days: i64| at(due + Duration::days(days), 12, 0);
    let gone = Some("not sent: what the reminder said is no longer true".to_owned());
    let repair = || owed(Uuid::new_v4(), "A", on(due));

    // Queued while it was due soon, and still waiting when the date has
    // passed: the worker was stopped, or the provider was down.
    let late = agreed(&app, "UTC", vec![repair()]).await;
    assert_eq!(pass(&app, noon(-1)).await, 1);
    let (delivered, emails) = deliver(&app, noon(1)).await;
    assert_eq!((delivered.sent, delivered.dropped), (0, 1));
    assert_eq!(emails, []);
    assert_eq!(rows(&app, &late).await, [(0, gone.clone(), true)]);

    // Three more reminders are queued. Before they go out, one contribution
    // is marked as delivered and another has its date moved.
    let (claimed, shifted) = (Uuid::new_v4(), Uuid::new_v4());
    let delivered_since = agreed(&app, "UTC", vec![owed(claimed, "A", on(due))]).await;
    let moved_since = agreed(&app, "UTC", vec![owed(shifted, "A", on(due))]).await;
    let standing = agreed(&app, "UTC", vec![repair()]).await;
    assert_eq!(pass(&app, noon(-1)).await, 3);
    app.act(
        &delivered_since.ana,
        &delivered_since.exchange,
        claimed,
        "CLAIM",
    )
    .await
    .ok();
    amend(
        &app,
        &moved_since,
        vec![owed(shifted, "A", on(due + Duration::days(30)))],
    )
    .await;

    let (delivered, emails) = deliver(&app, noon(-1)).await;
    assert_eq!((delivered.sent, delivered.dropped), (1, 2));
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].to, standing.ana.email);
    assert_eq!(rows(&app, &standing).await, [(1, None, true)]);
    for pair in [&delivered_since, &moved_since] {
        assert_eq!(rows(&app, pair).await, [(0, gone.clone(), true)]);
    }

    // The same for "overdue": Ben confirms he has the repair before the
    // emails saying it is late have gone.
    assert_eq!(
        pass(&app, noon(1)).await,
        2,
        "the late one and the standing one"
    );
    let repair = recorded(&app, &standing).await[0].0;
    app.act(&standing.ben, &standing.exchange, repair, "CONFIRM")
        .await
        .ok();
    let (delivered, emails) = deliver(&app, noon(1)).await;
    assert_eq!((delivered.sent, delivered.dropped), (2, 2));
    let mut told: Vec<&str> = emails.iter().map(|email| email.to.as_str()).collect();
    told.sort_unstable();
    let mut expected = [late.ana.email.as_str(), late.ben.email.as_str()];
    expected.sort_unstable();
    assert_eq!(told, expected);
}

#[tokio::test]
async fn one_exchange_that_cannot_be_read_does_not_stop_the_rest() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let noon = |days: i64| at(due + Duration::days(days), 12, 0);
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    let broken = agreed(&app, "Asia/Tokyo", vec![owed(first, "A", on(due))]).await;
    let sound = agreed(&app, "UTC", vec![owed(second, "A", on(due))]).await;
    assert_eq!(pass(&app, noon(-1)).await, 2);

    // One exchange's timezone stops being one the database knows. The API
    // never stores such a thing; it is put there as the schema owner.
    let set_timezone = |timezone: &'static str| {
        sqlx::query("UPDATE exchange SET timezone = $2 WHERE id = $1")
            .bind(broken.id())
            .bind(timezone)
            .execute(&app.owner)
    };
    set_timezone("Nowhere/Nothing").await.unwrap();

    // Its queued reminder cannot be checked, so it is not sent, and counts
    // as a failed try to be made again later. The other one goes out.
    let (delivered, emails) = deliver(&app, noon(-1)).await;
    assert_eq!((delivered.sent, delivered.failed), (1, 1));
    assert_eq!(emails[0].to, sound.ana.email);
    let [(attempts, error, done)] = &rows(&app, &broken).await[..] else {
        panic!("one queued reminder");
    };
    assert_eq!((*attempts, *done), (1, false));
    assert!(
        error
            .as_deref()
            .unwrap()
            .starts_with("the reminder could not be checked"),
        "{error:?}"
    );

    // The next pass cannot tell what day it is there. It says so in the log
    // and still reminds everyone else.
    assert_eq!(pass(&app, noon(1)).await, 1);
    assert_eq!(
        recorded(&app, &sound).await,
        [soon(second, due), overdue(second, due)]
    );
    assert_eq!(recorded(&app, &broken).await, [soon(first, due)]);

    // Put right, it is caught up with.
    set_timezone("Asia/Tokyo").await.unwrap();
    assert_eq!(pass(&app, noon(1)).await, 1);
    assert_eq!(
        recorded(&app, &broken).await,
        [soon(first, due), overdue(first, due)]
    );
}

#[tokio::test]
async fn a_party_who_cannot_be_emailed_is_passed_over_and_not_tried_again() {
    let (app, _turn) = app().await;
    let due = a_month_ahead();
    let noon = |days: i64| at(due + Duration::days(days), 12, 0);
    let repair = Uuid::new_v4();
    let pair = agreed(&app, "UTC", vec![owed(repair, "A", on(due))]).await;
    // Ana, who owes the repair, has a phone number and no email address.
    let phone = format!("+1555{:07}", Uuid::new_v4().as_u128() % 10_000_000);
    common::set_phone(&app.db, pair.ana.id, &phone, true).await;

    // The reminder is recorded, with nobody to send it to, and later passes
    // do not keep coming back to it.
    assert_eq!(pass(&app, noon(-1)).await, 1);
    assert_eq!(pass(&app, noon(0)).await, 0);
    assert_eq!(queued(&app, &pair).await, none());

    // Ben can be reached, and is told when it is overdue.
    assert_eq!(pass(&app, noon(1)).await, 1);
    assert_eq!(queued(&app, &pair).await, ["Ben: OVERDUE_TO_RECEIVE"]);
    assert_eq!(
        recorded(&app, &pair).await,
        [soon(repair, due), overdue(repair, due)]
    );
}
