//! The record of an exchange, read back through the API: its history, and
//! the copy a party takes away.

mod common;

use axum::http::{Method, StatusCode};
use common::{App, Deal, User, accept, consent, fence_job};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use yuppers_backend::exchanges::record::dto::Continuation;
use yuppers_backend::exchanges::record::{self, Limits};
use yuppers_backend::exchanges::service::run_timers;
use yuppers_backend::http::extract::Session;

const DATABASE: &str = "yuppers_test_record";

async fn app() -> App {
    App::start(DATABASE).await
}

async fn send_with_note(app: &App, user: &User, exchange: &str, terms: Value, note: &str) -> Value {
    let version = app.view(user, exchange).await["version"].clone();
    app.post(
        user,
        &format!("/v1/exchanges/{exchange}/revisions"),
        json!({
            "expected_version": version, "terms": terms, "note": note, "consent": consent(),
            "invitation": { "for_anyone": true },
        }),
    )
    .await
    .ok()
}

async fn contribution(app: &App, user: &User, exchange: &str, id: Uuid, action: &str, note: &str) {
    app.command(
        user,
        exchange,
        json!({ "type": "CONTRIBUTION", "contribution": id, "action": action, "note": note }),
    )
    .await
    .ok();
}

async fn record_of(app: &App, user: &User, exchange: &str) -> Value {
    app.get(user, &format!("/v1/exchanges/{exchange}/record"))
        .await
        .ok()
}

async fn history_of(app: &App, user: &User, exchange: &str, query: &str) -> Value {
    app.get(user, &format!("/v1/exchanges/{exchange}/history{query}"))
        .await
        .ok()
}

fn list(value: &Value) -> &Vec<Value> {
    value.as_array().expect("a list")
}

fn texts<'a>(values: &'a Value, field: &str) -> Vec<&'a str> {
    list(values)
        .iter()
        .map(|value| value[field].as_str().unwrap_or(""))
        .collect()
}

/// Joins the invited party and has the initiator confirm them.
async fn join(app: &App, deal: &Deal) {
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

// ---- An independent reading of the hash ---------------------------------------

/// RFC 8785 for the values a signed document holds, written here from the
/// RFC and not borrowed from the service, the way a third party checking a
/// copy would have to.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            out.push_str(&number.as_i64().expect("whole numbers only").to_string())
        }
        Value::String(text) => canonical_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_string(key, out);
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

fn canonical_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The content hash of a revision in a copy, from the copy alone.
fn recomputed(revision: &Value) -> String {
    let mut text = String::new();
    canonical(&revision["signed"], &mut text);
    sha256_hex(&text)
}

// ---- The tests ----------------------------------------------------------------

#[tokio::test]
async fn the_record_of_a_whole_exchange_tells_all_of_it() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    // Something else of Ben's that must never be in anyone's copy.
    common::set_phone(&app.db, ben.id, "+15555550123", false).await;
    let exchange = app.draft(&ana).await;
    let (repair, payment, cleanup) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

    // Ana proposes, with a message. Ben joins and counters at a lower price.
    let sent = send_with_note(
        &app,
        &ana,
        &exchange,
        fence_job(repair, payment),
        "Does Saturday work?",
    )
    .await;
    let deal = Deal {
        ana,
        ben,
        exchange: exchange.clone(),
        repair,
        payment,
        revision: String::new(),
        invitation: sent["invitation_token"].as_str().unwrap().to_owned(),
    };
    join(&app, &deal).await;
    let (ana, ben) = (&deal.ana, &deal.ben);

    let mut counter = fence_job(repair, payment);
    counter["contributions"][1]["amount_minor"] = json!(35000);
    let second = app.send(ben, &exchange, counter).await.ok()["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.command(ana, &exchange, accept(&second)).await.ok();

    // The work is claimed, disputed with a reason, put right, and confirmed.
    contribution(
        &app,
        ana,
        &exchange,
        repair,
        "CLAIM",
        "Finished this morning.",
    )
    .await;
    contribution(
        &app,
        ben,
        &exchange,
        repair,
        "DISPUTE",
        "The gate still sticks.",
    )
    .await;
    contribution(&app, ana, &exchange, repair, "CLAIM", "Rehung the gate.").await;
    app.act(ben, &exchange, repair, "CONFIRM").await.ok();

    // An amendment adds a task and is accepted; another is declined.
    let mut amended = fence_job(repair, payment);
    amended["contributions"][1]["amount_minor"] = json!(38000);
    amended["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": cleanup, "from": "A", "type": "TASK",
            "description": "Haul away the old panels", "quantity": null,
            "due": { "kind": "AFTER_CONTRIBUTION", "contribution": repair },
            "completion_criteria": null, "required": true, "amount_minor": null,
        }));
    let third =
        app.send(ana, &exchange, amended.clone()).await.ok()["exchange"]["open_revision"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
    app.command(ben, &exchange, accept(&third)).await.ok();

    let mut cheaper = amended.clone();
    cheaper["contributions"][1]["amount_minor"] = json!(20000);
    let fourth = app.send(ben, &exchange, cheaper.clone()).await.ok()["exchange"]["open_revision"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.command(
        ana,
        &exchange,
        json!({ "type": "DECLINE", "revision": fourth }),
    )
    .await
    .ok();

    // Ana asks to close; both add statements; Ben tries one more amendment,
    // which is still open when the time to respond runs out.
    app.command(
        ana,
        &exchange,
        json!({ "type": "REQUEST_CLOSE", "note": "Never paid." }),
    )
    .await
    .ok();
    app.command(
        ben,
        &exchange,
        json!({ "type": "ADD_STATEMENT", "note": "The price was too high." }),
    )
    .await
    .ok();
    app.command(
        ana,
        &exchange,
        json!({ "type": "ADD_STATEMENT", "note": "It was the price we signed." }),
    )
    .await
    .ok();
    cheaper["contributions"][1]["amount_minor"] = json!(25000);
    app.send(ben, &exchange, cheaper).await.ok();
    let later = OffsetDateTime::now_utc() + Duration::days(8);
    assert_eq!(run_timers(&app.db, &app.rules, later).await.unwrap(), 1);

    let record = record_of(&app, ana, &exchange).await;

    // What it is, and that it is all of it.
    assert_eq!(record["format"], "exchange-record");
    assert_eq!(record["format_version"], 2);
    assert_eq!(record["language"], "en");
    assert_eq!(record["prepared_for"], "A");
    assert_eq!(
        record["part"],
        json!({
            "from": { "revisions_after": 0, "events_after": 0 },
            "next": null,
            "complete": true,
        })
    );

    // How the exchange stands.
    let standing = &record["exchange"];
    assert_eq!(standing["id"], exchange.as_str());
    assert_eq!(
        (
            &standing["state"],
            &standing["closed_outcome"],
            &standing["closed_reason"]
        ),
        (
            &json!("CLOSED"),
            &json!("UNRESOLVED"),
            &json!("CLOSE_REQUEST")
        )
    );
    assert!(standing["closed_at"].is_string());
    assert_eq!(standing["in_force_revision"]["sequence"], 3);
    assert_eq!(standing["in_force_revision"]["id"], third.as_str());
    assert!(standing.get("open_revision").is_none());
    assert_eq!(
        (&standing["timezone"], &standing["currency"]),
        (&json!("America/Chicago"), &json!("USD"))
    );
    assert_eq!(
        record["parties"],
        json!({ "A": "Ana Ruiz", "B": "Ben Ortiz" })
    );

    // Each contribution of the agreement, as the parties left it.
    let contributions = &record["contributions"];
    assert_eq!(
        texts(contributions, "status"),
        ["ACCEPTED", "PENDING", "PENDING"]
    );
    assert_eq!(
        texts(contributions, "description"),
        [
            "Repair the back fence",
            "Payment on completion",
            "Haul away the old panels"
        ]
    );
    assert_eq!(texts(contributions, "from"), ["A", "B", "A"]);
    assert!(list(contributions).iter().all(|c| c["since"].is_string()));

    // Every revision, in order, with what became of it.
    let revisions = list(&record["revisions"]);
    assert_eq!(revisions.len(), 5);
    let standings: Vec<&str> = revisions
        .iter()
        .map(|revision| revision["standing"]["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        standings,
        ["SUPERSEDED", "REPLACED", "IN_FORCE", "DECLINED", "VOIDED"]
    );
    assert_eq!(
        texts(&record["revisions"], "author"),
        ["A", "B", "A", "B", "B"]
    );
    for (index, revision) in revisions.iter().enumerate() {
        assert_eq!(revision["sequence"], index + 1);
        assert!(revision["sent_at"].is_string() && revision["expires_at"].is_string());
        assert!(revision["standing"]["since"].is_string());
    }
    assert_eq!(revisions[0]["note"], "Does Saturday work?");
    assert!(revisions[1].get("note").is_none());
    assert!(revisions[0].get("answers").is_none());
    assert_eq!(revisions[1]["answers"]["sequence"], 1);
    assert_eq!(revisions[2]["answers"]["sequence"], 2);
    assert_eq!(revisions[4]["answers"]["sequence"], 3);
    assert_eq!(revisions[0]["standing"]["replaced_by"]["sequence"], 2);
    assert_eq!(revisions[1]["standing"]["replaced_by"]["sequence"], 3);
    assert!(revisions[1]["standing"]["in_force_at"].is_string());
    assert!(revisions[2]["standing"]["in_force_at"].is_string());
    assert!(revisions[3]["standing"].get("in_force_at").is_none());
    assert_eq!(
        revisions[1]["signed"]["contributions"][1]["amount_minor"],
        35000
    );
    assert_eq!(
        revisions[2]["signed"]["contributions"][2]["description"],
        "Haul away the old panels"
    );
    assert_eq!(
        revisions[2]["signed"]["parties"],
        json!({ "A": "Ana Ruiz", "B": "Ben Ortiz" })
    );

    // Every signature: who, on what, when, and on what evidence.
    let signers: Vec<Vec<&str>> = revisions
        .iter()
        .map(|revision| texts(&revision["signatures"], "party"))
        .collect();
    assert_eq!(
        signers,
        [
            vec!["A"],
            vec!["B", "A"],
            vec!["A", "B"],
            vec!["B"],
            vec!["B"]
        ]
    );
    for revision in revisions {
        assert_eq!(revision["content_hash"], recomputed(revision).as_str());
        for signature in list(&revision["signatures"]) {
            assert_eq!(signature["content_hash"], revision["content_hash"]);
            assert!(signature["signed_at"].is_string());
            assert_eq!(
                signature["consent"],
                json!({ "language": "en", "version": common::CONSENT_VERSION })
            );
            let verification = &signature["verification"];
            assert_eq!(verification["method"], "EMAIL_OTP");
            assert!(verification["verified_at"].is_string());
            // In words, and claiming no more than there was.
            let said = verification["description"].as_str().unwrap();
            assert!(said.contains("single one-time code") && said.contains("email address"));
            let name = if signature["party"] == "A" {
                "Ana Ruiz"
            } else {
                "Ben Ortiz"
            };
            assert_eq!(signature["name"], name);
        }
    }

    // The document says, in itself, what its contents can and cannot show.
    let notices = &record["notices"];
    assert!(
        notices["signatures"]
            .as_str()
            .unwrap()
            .contains("No other proof of identity")
    );
    assert!(
        notices["statements"]
            .as_str()
            .unwrap()
            .contains("did not check")
    );
    let how = notices["content_hash"].as_str().unwrap();
    assert!(how.contains("SHA-256") && how.contains("RFC 8785"));

    // The history: everything stored, in order, with who did it and what
    // they wrote.
    let id: Uuid = exchange.parse().unwrap();
    let stored: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT sequence, type, actor_slot FROM exchange_event
         WHERE exchange_id = $1 ORDER BY sequence",
    )
    .bind(id)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    let events = list(&record["events"]);
    assert_eq!(events.len(), stored.len());
    for (event, (sequence, kind, actor)) in events.iter().zip(&stored) {
        assert_eq!(event["sequence"], *sequence);
        assert_eq!(event["type"], kind.as_str());
        assert_eq!(event["actor"], actor.as_deref().unwrap_or("SYSTEM"));
        assert!(event["at"].is_string());
    }
    assert_eq!(
        texts(&record["events"], "type"),
        [
            "REVISION_SENT",
            "COUNTERPARTY_CLAIMED",
            "COUNTERPARTY_CONFIRMED",
            "REVISION_SUPERSEDED",
            "REVISION_SENT",
            "REVISION_ACCEPTED",
            "AGREEMENT_IN_FORCE",
            "CONTRIBUTION_CLAIMED",
            "CONTRIBUTION_DISPUTED",
            "CONTRIBUTION_CLAIMED",
            "CONTRIBUTION_CONFIRMED",
            "REVISION_SENT",
            "REVISION_ACCEPTED",
            "AGREEMENT_IN_FORCE",
            "REVISION_SENT",
            "REVISION_DECLINED",
            "CLOSE_REQUESTED",
            "STATEMENT_ADDED",
            "STATEMENT_ADDED",
            "REVISION_SENT",
            "REVISION_SUPERSEDED",
            "EXCHANGE_CLOSED",
        ]
    );
    let notes: Vec<(i64, &str)> = events
        .iter()
        .filter_map(|event| Some((event["sequence"].as_i64()?, event.get("note")?.as_str()?)))
        .collect();
    assert_eq!(
        notes,
        [
            (1, "Does Saturday work?"),
            (8, "Finished this morning."),
            (9, "The gate still sticks."),
            (10, "Rehung the gate."),
            (17, "Never paid."),
            (18, "The price was too high."),
            (19, "It was the price we signed."),
        ]
    );
    assert_eq!(events[0]["revision"]["sequence"], 1);
    assert_eq!(events[1]["invitation_named_them"], false);
    assert_eq!(events[3]["revision"]["sequence"], 1);
    assert_eq!(events[6]["revision"]["sequence"], 2);
    assert_eq!(
        events[6]["statuses"],
        json!([
            { "id": repair, "status": "PENDING" },
            { "id": payment, "status": "PENDING" },
        ])
        .as_array()
        .map(|statuses| {
            let mut statuses = statuses.clone();
            statuses.sort_by_key(|status| status["id"].as_str().unwrap().to_owned());
            Value::Array(statuses)
        })
        .unwrap()
    );
    // A dispute: who, about what as the agreement then described it, and why.
    let dispute = &events[8];
    assert_eq!(
        (&dispute["actor"], &dispute["status"]),
        (&json!("B"), &json!("DISPUTED"))
    );
    assert_eq!(
        dispute["contribution"],
        json!({ "id": repair, "description": "Repair the back fence" })
    );
    assert_eq!(dispute["revision"]["sequence"], 2);
    let closed = events.last().unwrap();
    assert_eq!(
        (&closed["actor"], &closed["outcome"], &closed["reason"]),
        (
            &json!("SYSTEM"),
            &json!("UNRESOLVED"),
            &json!("CLOSE_REQUEST")
        )
    );

    // Ben's copy holds the same record.
    let mut bens = record_of(&app, ben, &exchange).await;
    assert_eq!(bens["prepared_for"], "B");
    let mut anas = record.clone();
    for copy in [&mut anas, &mut bens] {
        copy["prepared_for"] = Value::Null;
        copy["generated_at"] = Value::Null;
    }
    assert_eq!(anas, bens);

    // Neither copy says who either person is beyond the name they signed
    // with: no email address, no phone number, no account ID.
    let mut found = Vec::new();
    strings(&record, &mut found);
    let secrets = [
        ana.email.clone(),
        ben.email.clone(),
        ana.id.to_string(),
        ben.id.to_string(),
        "+15555550123".to_owned(),
    ];
    for text in &found {
        for secret in &secrets {
            assert!(!text.contains(secret.as_str()), "{text}");
        }
        assert!(!text.contains("@example.test"), "{text}");
        assert!(text != "account" && text != "email" && text != "phone");
    }

    // The same history, a page at a time, latest first.
    let page = history_of(&app, ana, &exchange, "?limit=5").await;
    assert_eq!(page["you"], "A");
    assert_eq!(page["parties"], record["parties"]);
    assert_eq!(page["events"], json!(events[17..]));
    assert_eq!(page["earlier"], 18);
    let mut pages = vec![page];
    while let Some(before) = pages.last().unwrap()["earlier"].as_i64() {
        pages.push(history_of(&app, ana, &exchange, &format!("?limit=5&before={before}")).await);
    }
    assert_eq!(pages.len(), 5);
    let joined: Vec<Value> = pages
        .iter()
        .rev()
        .flat_map(|page| list(&page["events"]).clone())
        .collect();
    assert_eq!(&joined, events);
    // Without a limit it is the latest fifty, which here is all of it.
    let whole = history_of(&app, ben, &exchange, "").await;
    assert_eq!(&whole["events"], &record["events"]);
    assert_eq!(whole["earlier"], Value::Null);
    let mut found = Vec::new();
    strings(&whole, &mut found);
    assert!(
        found
            .iter()
            .all(|text| secrets.iter().all(|secret| !text.contains(secret.as_str())))
    );
}

#[tokio::test]
async fn an_exchange_that_was_never_agreed_keeps_its_record() {
    let app = app().await;

    // Declined by the invited party.
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    let exchange = app.draft(&ana).await;
    let sent = send_with_note(
        &app,
        &ana,
        &exchange,
        fence_job(Uuid::new_v4(), Uuid::new_v4()),
        "Let me know.",
    )
    .await;
    let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
    app.post(
        &ben,
        "/v1/invitations/claim",
        json!({ "token": sent["invitation_token"] }),
    )
    .await
    .ok();
    // Until Ana has confirmed him he could only sign or leave.
    app.command(&ana, &exchange, json!({ "type": "CONFIRM_COUNTERPARTY" }))
        .await
        .ok();
    app.command(
        &ben,
        &exchange,
        json!({ "type": "DECLINE", "revision": revision }),
    )
    .await
    .ok();

    for reader in [&ana, &ben] {
        let record = record_of(&app, reader, &exchange).await;
        assert_eq!(record["part"]["complete"], true);
        let standing = &record["exchange"];
        assert_eq!(
            (
                &standing["state"],
                &standing["closed_outcome"],
                &standing["closed_reason"]
            ),
            (&json!("CLOSED"), &json!("NOT_AGREED"), &json!("DECLINED"))
        );
        assert!(standing.get("in_force_revision").is_none());
        assert_eq!(record["contributions"], json!([]));

        // What was offered is still there to read, with who signed it.
        let revisions = list(&record["revisions"]);
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0]["standing"]["status"], "DECLINED");
        assert_eq!(revisions[0]["note"], "Let me know.");
        assert_eq!(revisions[0]["signed"]["terms"], "Repair the back fence.");
        assert_eq!(texts(&revisions[0]["signatures"], "party"), ["A"]);
        assert_eq!(
            revisions[0]["content_hash"],
            recomputed(&revisions[0]).as_str()
        );

        assert_eq!(
            texts(&record["events"], "type"),
            [
                "REVISION_SENT",
                "COUNTERPARTY_CLAIMED",
                "COUNTERPARTY_CONFIRMED",
                "REVISION_DECLINED",
                "EXCHANGE_CLOSED"
            ]
        );
        assert_eq!(texts(&record["events"], "actor"), ["A", "B", "A", "B", "B"]);
        let closed = &record["events"][4];
        assert_eq!(
            (&closed["outcome"], &closed["reason"]),
            (&json!("NOT_AGREED"), &json!("DECLINED"))
        );
        let history = history_of(&app, reader, &exchange, "").await;
        assert_eq!(history["events"], record["events"]);
    }

    // Withdrawn before anyone opened the link: the record is its sender's.
    let alone = app.negotiating().await;
    app.command(
        &alone.ana,
        &alone.exchange,
        json!({ "type": "WITHDRAW", "revision": alone.revision }),
    )
    .await
    .ok();
    let record = record_of(&app, &alone.ana, &alone.exchange).await;
    assert_eq!(record["exchange"]["closed_reason"], "WITHDRAWN");
    assert_eq!(record["exchange"]["counterparty"], "UNCLAIMED");
    assert_eq!(record["revisions"][0]["standing"]["status"], "WITHDRAWN");
    assert_eq!(
        texts(&record["events"], "type"),
        ["REVISION_SENT", "REVISION_WITHDRAWN", "EXCHANGE_CLOSED"]
    );
    app.get(
        &alone.ben,
        &format!("/v1/exchanges/{}/record", alone.exchange),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    // A draft has nothing in its record yet, and says so without failing.
    let draft = app.draft(&alone.ana).await;
    let record = record_of(&app, &alone.ana, &draft).await;
    assert_eq!(record["exchange"]["state"], "DRAFT");
    assert_eq!(
        (&record["revisions"], &record["events"]),
        (&json!([]), &json!([]))
    );
    assert_eq!(record["part"]["complete"], true);
}

#[tokio::test]
async fn a_signature_that_waited_for_confirmation_is_dated_as_it_was_made() {
    let app = app().await;
    let deal = app.negotiating().await;
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();

    // Ben signs before Ana has confirmed who he is.
    app.command(&deal.ben, &deal.exchange, accept(&deal.revision))
        .await
        .ok();
    let record = record_of(&app, &deal.ben, &deal.exchange).await;
    let revision = &record["revisions"][0];
    assert_eq!(revision["standing"]["status"], "OPEN");
    assert_eq!(revision["standing"]["since"], revision["sent_at"]);
    assert_eq!(texts(&revision["signatures"], "party"), ["A", "B"]);
    assert_eq!(record["exchange"]["open_revision"]["sequence"], 1);
    assert_eq!(record["exchange"]["counterparty"], "CLAIMED");

    app.command(
        &deal.ana,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    let record = record_of(&app, &deal.ben, &deal.exchange).await;
    let revision = &record["revisions"][0];
    assert_eq!(revision["standing"]["status"], "IN_FORCE");
    assert_eq!(
        texts(&record["events"], "type"),
        [
            "REVISION_SENT",
            "COUNTERPARTY_CLAIMED",
            "REVISION_ACCEPTED",
            "COUNTERPARTY_CONFIRMED",
            "AGREEMENT_IN_FORCE"
        ]
    );
    // The signature keeps its own time; the agreement took effect later.
    let signed_at = revision["signatures"][1]["signed_at"].as_str().unwrap();
    let in_force_at = revision["standing"]["in_force_at"].as_str().unwrap();
    assert_eq!(record["events"][2]["at"], signed_at);
    assert_eq!(record["events"][4]["at"], in_force_at);
    assert_ne!(signed_at, in_force_at);
    assert_eq!(
        texts(&record["contributions"], "status"),
        ["PENDING", "PENDING"]
    );
}

#[tokio::test]
async fn only_the_two_parties_can_read_a_record_and_only_that_exchange_is_in_it() {
    let app = app().await;
    let deal = app.active().await;
    let stranger = app.user("Carla").await;

    for path in ["record", "history"] {
        let path = format!("/v1/exchanges/{}/{path}", deal.exchange);
        app.get(&stranger, &path)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
        app.call(None, Method::GET, &path, None, &[])
            .await
            .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
        app.get(&deal.ana, &path).await.ok();
        app.get(&deal.ben, &path).await.ok();
    }
    for path in [
        "/v1/exchanges/not-an-id/record",
        "/v1/exchanges/not-an-id/history",
    ] {
        app.get(&deal.ana, path)
            .await
            .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
    }
    let missing = Uuid::new_v4();
    app.get(&deal.ana, &format!("/v1/exchanges/{missing}/record"))
        .await
        .refused(StatusCode::NOT_FOUND, "NOT_FOUND");

    // The same two people in a second exchange, and two other people in a
    // third: nothing of either reaches the first one's record.
    let second = app.draft(&deal.ana).await;
    let mut shed = fence_job(Uuid::new_v4(), Uuid::new_v4());
    shed["terms"] = json!("Build the gazebo.");
    shed["contributions"][0]["description"] = json!("Build a gazebo");
    let sent = send_with_note(&app, &deal.ana, &second, shed, "About the gazebo.").await;
    app.post(
        &deal.ben,
        "/v1/invitations/claim",
        json!({ "token": sent["invitation_token"] }),
    )
    .await
    .ok();
    let third = app.negotiating().await;

    let record = record_of(&app, &deal.ben, &deal.exchange).await;
    let mut found = Vec::new();
    strings(&record, &mut found);
    for text in &found {
        assert!(!text.to_lowercase().contains("gazebo"), "{text}");
        for other in [&second, &third.exchange] {
            assert!(!text.contains(other.as_str()), "{text}");
        }
    }
    assert_eq!(list(&record["revisions"]).len(), 1);
    // And the first one's parties cannot read the third.
    app.get(
        &deal.ben,
        &format!("/v1/exchanges/{}/history", third.exchange),
    )
    .await
    .refused(StatusCode::NOT_FOUND, "NOT_FOUND");
}

#[tokio::test]
async fn the_hash_can_be_recomputed_from_the_copy_alone() {
    let app = app().await;
    let ana = app.user("Ana").await;
    let exchange = app.draft(&ana).await;
    let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());

    // Every kind of field, and text that a careless reading would change.
    let mut terms = fence_job(repair, payment);
    terms["party_a_name"] = json!("Ana María Ruiz-Peña");
    terms["party_b_name"] = json!("Ben \"Benny\" Ortiz 😀");
    terms["terms"] =
        json!("Reparar la cerca.\nLine two with \"quotes\", a tab\t, a \\ and \u{1} \u{7f} €.");
    terms["contributions"][0]["quantity"] = json!({ "amount": "1.50", "unit": "días" });
    terms["contributions"][0]["completion_criteria"] = json!("The gate swings freely");
    terms["contributions"][0]["due"] = json!({ "kind": "DATE", "date": "2026-11-01" });
    terms["contributions"][1]["required"] = json!(false);
    let sent = send_with_note(
        &app,
        &ana,
        &exchange,
        terms.clone(),
        "Not signed: this note.",
    )
    .await;

    let record = record_of(&app, &ana, &exchange).await;
    let revision = &record["revisions"][0];
    let signed = &revision["signed"];

    // The copy carries what was signed, word for word.
    assert_eq!(signed["v"], 1);
    assert_eq!(signed["exchange"], exchange.as_str());
    assert_eq!(
        (&signed["currency"], &signed["timezone"]),
        (
            &record["exchange"]["currency"],
            &record["exchange"]["timezone"]
        )
    );
    assert_eq!(
        signed["parties"],
        json!({ "A": terms["party_a_name"], "B": terms["party_b_name"] })
    );
    assert_eq!(signed["terms"], terms["terms"]);
    assert_eq!(signed["attachments"], json!([]));
    let first = &signed["contributions"][0];
    assert_eq!(
        first,
        &json!({
            "id": repair, "from": "A", "type": "SERVICE",
            "description": "Repair the back fence",
            "quantity": { "amount": "1.50", "unit": "días" },
            "due": { "kind": "DATE", "date": "2026-11-01" },
            "completion_criteria": "The gate swings freely",
            "required": true, "amount_minor": null, "settlement": null,
        })
    );
    assert_eq!(
        signed["contributions"][1],
        json!({
            "id": payment, "from": "B", "type": "MONEY",
            "description": "Payment on completion", "quantity": null,
            "due": { "kind": "AFTER_CONTRIBUTION", "contribution": repair },
            "completion_criteria": null,
            "required": false, "amount_minor": 40000, "settlement": "OFF_PLATFORM",
        })
    );
    // The note and the expiry are about the offer, and are not signed.
    assert!(signed.get("note").is_none() && signed.get("expires_at").is_none());

    // From that alone, by the method the copy itself names, comes the hash
    // the signature is on, which is the one the service showed when signing.
    let hash = recomputed(revision);
    assert_eq!(revision["content_hash"], hash.as_str());
    assert_eq!(revision["signatures"][0]["content_hash"], hash.as_str());
    assert_eq!(
        sent["exchange"]["open_revision"]["content_hash"],
        hash.as_str()
    );

    // Any change to what was signed gives a different hash.
    let mut altered = revision.clone();
    altered["signed"]["contributions"][1]["amount_minor"] = json!(40001);
    assert_ne!(recomputed(&altered), hash);
}

#[tokio::test]
async fn a_long_record_comes_in_parts_that_join_up() {
    let rules = yuppers_backend::domain::Rules {
        changes_per_minute: 1000,
        ..Default::default()
    };
    let app = App::start_with(DATABASE, rules).await;
    let deal = app.active().await;

    // Three more revisions and a good many events.
    for price in [41000, 42000, 43000] {
        let mut amended = fence_job(deal.repair, deal.payment);
        amended["contributions"][1]["amount_minor"] = json!(price);
        let sent = app.send(&deal.ana, &deal.exchange, amended).await.ok();
        let revision = sent["exchange"]["open_revision"]["id"].as_str().unwrap();
        app.command(&deal.ben, &deal.exchange, accept(revision))
            .await
            .ok();
    }
    for _ in 0..4 {
        for kind in ["PROPOSE_END", "CANCEL_END"] {
            app.command(&deal.ben, &deal.exchange, json!({ "type": kind }))
                .await
                .ok();
        }
    }

    let whole = record_of(&app, &deal.ana, &deal.exchange).await;
    assert_eq!(whole["part"]["complete"], true);
    assert_eq!(list(&whole["revisions"]).len(), 4);
    assert_eq!(list(&whole["events"]).len(), 22);

    // The service's own reading, with room for less in each document.
    let session = Session {
        id: Uuid::new_v4(),
        account_id: deal.ana.id,
        auth_method: "EMAIL_OTP".to_owned(),
        authenticated_at: OffsetDateTime::now_utc(),
    };
    let id: Uuid = deal.exchange.parse().unwrap();
    let small = [
        Limits {
            events: 5,
            revisions: 3,
            ..Default::default()
        },
        // No room for a second revision's text: one to a document.
        Limits {
            events: 100,
            revision_bytes: 1,
            ..Default::default()
        },
    ];
    for (limits, expected_parts) in small.iter().zip([5, 4]) {
        let mut from = Continuation {
            revisions_after: 0,
            events_after: 0,
        };
        let (mut revisions, mut events, mut parts) = (Vec::new(), Vec::new(), 0);
        loop {
            let part = record::record(&app.db, &session, id, from, limits)
                .await
                .unwrap();
            let part = serde_json::to_value(&part).unwrap();
            parts += 1;
            assert!(list(&part["events"]).len() <= limits.events as usize);
            assert!(list(&part["revisions"]).len() <= limits.revisions);
            assert_eq!(part["part"]["complete"], false);
            assert_eq!(part["part"]["from"], serde_json::to_value(from).unwrap());
            // Every part can be read alone: it says what it is part of.
            assert_eq!(part["exchange"], whole["exchange"]);
            assert_eq!(part["notices"], whole["notices"]);
            revisions.extend(list(&part["revisions"]).clone());
            events.extend(list(&part["events"]).clone());

            let next = &part["part"]["next"];
            if next.is_null() {
                break;
            }
            from = Continuation {
                revisions_after: next["revisions_after"].as_i64().unwrap() as i32,
                events_after: next["events_after"].as_i64().unwrap(),
            };
            assert!(parts < 50, "the parts never end");
        }
        assert_eq!(parts, expected_parts);
        assert_eq!(&revisions, list(&whole["revisions"]));
        assert_eq!(&events, list(&whole["events"]));
    }

    // Over HTTP a continuation is asked for by the same two numbers.
    let rest = app
        .get(
            &deal.ana,
            &format!(
                "/v1/exchanges/{}/record?revisions_after=3&events_after=20",
                deal.exchange
            ),
        )
        .await
        .ok();
    assert_eq!(
        texts(&rest["revisions"], "id"),
        [whole["revisions"][3]["id"].as_str().unwrap()]
    );
    assert_eq!(rest["events"], json!(list(&whole["events"])[20..]));
    assert_eq!(
        rest["part"],
        json!({
            "from": { "revisions_after": 3, "events_after": 20 },
            "next": null,
            "complete": false,
        })
    );

    // A query that is not what the endpoint takes is refused as such.
    for query in [
        "record?events_after=soon",
        "record?revisions_after=-1",
        "history?limit=0",
        "history?before=x",
    ] {
        app.get(
            &deal.ana,
            &format!("/v1/exchanges/{}/{query}", deal.exchange),
        )
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_REQUEST");
    }
    // Asking for more than a page may hold gets a page.
    let page = history_of(&app, &deal.ana, &deal.exchange, "?limit=100000").await;
    assert_eq!(list(&page["events"]).len(), 22);
}

#[tokio::test]
async fn the_copy_speaks_the_readers_language_and_leaves_the_parties_words_alone() {
    let app = app().await;
    let deal = app.active().await;
    contribution(
        &app,
        &deal.ana,
        &deal.exchange,
        deal.repair,
        "CLAIM",
        "Finished this morning.",
    )
    .await;
    let english = record_of(&app, &deal.ben, &deal.exchange).await;

    let updated = app
        .call(
            Some(&deal.ben),
            Method::PATCH,
            "/v1/me",
            Some(json!({ "language": "es-MX" })),
            &[],
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    let spanish = record_of(&app, &deal.ben, &deal.exchange).await;

    assert_eq!(
        (&english["language"], &spanish["language"]),
        (&json!("en"), &json!("es"))
    );
    for notice in ["about", "signatures", "statements", "content_hash"] {
        assert_ne!(english["notices"][notice], spanish["notices"][notice]);
        assert!(!spanish["notices"][notice].as_str().unwrap().is_empty());
    }
    let described = |record: &Value| {
        record["revisions"][0]["signatures"][0]["verification"]["description"].clone()
    };
    assert_ne!(described(&english), described(&spanish));
    assert!(
        described(&spanish)
            .as_str()
            .unwrap()
            .contains("código de un solo uso")
    );

    // Everything else is the record, the same in any language.
    assert_eq!(english["events"], spanish["events"]);
    assert_eq!(english["contributions"], spanish["contributions"]);
    assert_eq!(
        english["revisions"][0]["signed"],
        spanish["revisions"][0]["signed"]
    );
    assert_eq!(spanish["events"][5]["note"], "Finished this morning.");
    // Ana's copy is still in hers.
    assert_eq!(
        record_of(&app, &deal.ana, &deal.exchange).await["language"],
        "en"
    );
}
