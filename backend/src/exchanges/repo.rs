//! Loading an exchange into the domain model and storing what a decision
//! changed. Every function here runs inside the caller's transaction.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use time::{Date, OffsetDateTime};
use tracing::Instrument;
use uuid::Uuid;

use crate::chain;
use crate::domain::amendment::Statuses;
use crate::domain::contribution::{Action, Status};
use crate::domain::exchange::{
    Actor, CloseRequest, Counterparty, Decision, Event, Exchange, InForce, NotAgreed, Open,
    Outcome, State, Unresolved,
};
use crate::domain::notification::notifications;
use crate::domain::revision::{
    Contribution, ContributionId, Due, Kind, Quantity, Revision, RevisionId, Settlement, Slot,
};
use crate::notifications::{outbox, sms_updates};
use crate::wallet;

/// A stored revision: what it says, plus what the database knows about it.
#[derive(Clone, Debug)]
pub struct RevisionRecord {
    pub id: Uuid,
    pub sequence: i32,
    pub author: Slot,
    pub expires_at: OffsetDateTime,
    pub content_hash: Vec<u8>,
    pub accepted_by: Vec<Slot>,
    pub revision: Revision,
}

/// An exchange as stored: the domain state and the row data around it.
#[derive(Clone, Debug)]
pub struct Aggregate {
    pub id: Uuid,
    pub display_code: String,
    pub version: i64,
    pub last_event_seq: i64,
    pub timezone: String,
    pub currency: String,
    /// The account holding slot A and slot B.
    pub accounts: [Option<Uuid>; 2],
    pub open: Option<RevisionRecord>,
    pub in_force: Option<RevisionRecord>,
    pub exchange: Exchange,
}

impl Aggregate {
    pub fn slot_of(&self, account: Uuid) -> Option<Slot> {
        match self.accounts {
            [Some(a), _] if a == account => Some(Slot::A),
            [_, Some(b)] if b == account => Some(Slot::B),
            _ => None,
        }
    }

    /// The account holding a slot, if anyone does yet.
    pub fn account_of(&self, slot: Slot) -> Option<Uuid> {
        match slot {
            Slot::A => self.accounts[0],
            Slot::B => self.accounts[1],
        }
    }
}

// ---- Text forms stored in the database --------------------------------------

pub fn slot_str(slot: Slot) -> &'static str {
    slot.as_str()
}

fn parse_slot(text: &str) -> Slot {
    if text == "A" { Slot::A } else { Slot::B }
}

pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Pending => "PENDING",
        Status::Claimed => "CLAIMED",
        Status::Disputed => "DISPUTED",
        Status::Accepted => "ACCEPTED",
        Status::Waived => "WAIVED",
        Status::Removed => "REMOVED",
    }
}

fn parse_status(text: &str) -> Status {
    match text {
        "CLAIMED" => Status::Claimed,
        "DISPUTED" => Status::Disputed,
        "ACCEPTED" => Status::Accepted,
        "WAIVED" => Status::Waived,
        "REMOVED" => Status::Removed,
        _ => Status::Pending,
    }
}

fn state_str(state: State) -> &'static str {
    match state {
        State::Draft => "DRAFT",
        State::Negotiating => "NEGOTIATING",
        State::Active => "ACTIVE",
        State::Closed(_) => "CLOSED",
    }
}

/// The `closed_outcome` and `closed_reason` columns for a state.
pub fn outcome_columns(state: State) -> (Option<&'static str>, Option<&'static str>) {
    let State::Closed(outcome) = state else {
        return (None, None);
    };
    match outcome {
        Outcome::NotAgreed(reason) => (
            Some("NOT_AGREED"),
            Some(match reason {
                NotAgreed::Withdrawn => "WITHDRAWN",
                NotAgreed::Declined => "DECLINED",
                NotAgreed::Expired => "EXPIRED",
                NotAgreed::Discarded => "DISCARDED",
            }),
        ),
        Outcome::Completed => (Some("COMPLETED"), None),
        Outcome::EndedByAgreement => (Some("ENDED_BY_AGREEMENT"), None),
        Outcome::Unresolved(reason) => (
            Some("UNRESOLVED"),
            Some(match reason {
                Unresolved::CloseRequest => "CLOSE_REQUEST",
                Unresolved::Inactive => "INACTIVE",
            }),
        ),
    }
}

pub fn parse_state(state: &str, outcome: Option<&str>, reason: Option<&str>) -> State {
    match (state, outcome) {
        ("DRAFT", _) => State::Draft,
        ("NEGOTIATING", _) => State::Negotiating,
        ("ACTIVE", _) => State::Active,
        (_, Some("COMPLETED")) => State::Closed(Outcome::Completed),
        (_, Some("ENDED_BY_AGREEMENT")) => State::Closed(Outcome::EndedByAgreement),
        (_, Some("UNRESOLVED")) => State::Closed(Outcome::Unresolved(match reason {
            Some("INACTIVE") => Unresolved::Inactive,
            _ => Unresolved::CloseRequest,
        })),
        _ => State::Closed(Outcome::NotAgreed(match reason {
            Some("WITHDRAWN") => NotAgreed::Withdrawn,
            Some("DECLINED") => NotAgreed::Declined,
            Some("DISCARDED") => NotAgreed::Discarded,
            _ => NotAgreed::Expired,
        })),
    }
}

// ---- Loading ----------------------------------------------------------------

/// Loads an exchange. With `lock`, the row is locked until the transaction
/// ends, which is what puts concurrent changes to one exchange in order.
pub async fn load(
    conn: &mut PgConnection,
    id: Uuid,
    lock: bool,
) -> Result<Option<Aggregate>, sqlx::Error> {
    load_in(conn, id, lock)
        .instrument(crate::db::span("exchange.load"))
        .await
}

async fn load_in(
    conn: &mut PgConnection,
    id: Uuid,
    lock: bool,
) -> Result<Option<Aggregate>, sqlx::Error> {
    let query = format!(
        "SELECT display_code, state, closed_outcome, closed_reason, open_revision_id,
                in_force_revision_id, timezone, currency, version, last_event_seq,
                end_proposed_by, close_requested_by, close_requested_at,
                inactivity_prompted_at, last_activity_at
         FROM exchange WHERE id = $1 {}",
        if lock { "FOR UPDATE" } else { "" }
    );
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(None);
    };

    let participants: Vec<(String, Option<Uuid>, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT slot, account_id, initiator_confirmed_at FROM participant WHERE exchange_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut accounts = [None, None];
    let mut counterparty = Counterparty::Unclaimed;
    for (slot, account, confirmed_at) in participants {
        match parse_slot(&slot) {
            Slot::A => accounts[0] = account,
            Slot::B => {
                accounts[1] = account;
                counterparty = match (account, confirmed_at) {
                    (None, _) => Counterparty::Unclaimed,
                    (Some(_), None) => Counterparty::Claimed,
                    (Some(_), Some(_)) => Counterparty::Confirmed,
                };
            }
        }
    }

    // A contribution has a status only once it has been in force.
    let statuses: Statuses = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, status FROM contribution
         WHERE exchange_id = $1 AND last_event_seq IS NOT NULL",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, status)| (ContributionId(id), parse_status(&status)))
    .collect();

    let open = match row.get::<Option<Uuid>, _>("open_revision_id") {
        Some(revision) => Some(load_revision(conn, revision).await?),
        None => None,
    };
    let in_force = match row.get::<Option<Uuid>, _>("in_force_revision_id") {
        Some(revision) => Some(load_revision(conn, revision).await?),
        None => None,
    };

    let close_request = match (
        row.get::<Option<String>, _>("close_requested_by"),
        row.get::<Option<OffsetDateTime>, _>("close_requested_at"),
    ) {
        (Some(by), Some(at)) => Some(CloseRequest {
            by: parse_slot(&by),
            at,
        }),
        _ => None,
    };

    let exchange = Exchange {
        state: parse_state(
            row.get("state"),
            row.get::<Option<&str>, _>("closed_outcome"),
            row.get::<Option<&str>, _>("closed_reason"),
        ),
        counterparty,
        open: open.as_ref().map(|record| Open {
            id: RevisionId(record.id),
            author: record.author,
            expires_at: record.expires_at,
            accepted: record.accepted_by.contains(&record.author.other()),
            revision: record.revision.clone(),
        }),
        in_force: in_force.as_ref().map(|record| InForce {
            id: RevisionId(record.id),
            revision: record.revision.clone(),
        }),
        statuses,
        end_proposed_by: row
            .get::<Option<String>, _>("end_proposed_by")
            .map(|slot| parse_slot(&slot)),
        close_request,
        inactivity_prompted_at: row.get("inactivity_prompted_at"),
        last_activity_at: row.get("last_activity_at"),
    };

    Ok(Some(Aggregate {
        id,
        display_code: row.get("display_code"),
        version: row.get("version"),
        last_event_seq: row.get("last_event_seq"),
        timezone: row.get("timezone"),
        currency: row.get("currency"),
        accounts,
        open,
        in_force,
        exchange,
    }))
}

pub async fn load_revision(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<RevisionRecord, sqlx::Error> {
    let row = sqlx::query(
        "SELECT sequence, author_slot, note, terms, expires_at, content_hash,
                party_a_name, party_b_name
         FROM revision WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;

    let snapshots = sqlx::query(
        "SELECT contribution_id, from_slot, type, description, quantity::text AS quantity, unit,
                due_kind, due_date, due_after_contribution_id, completion_criteria, required,
                amount_minor, settlement_mode
         FROM contribution_snapshot WHERE revision_id = $1 ORDER BY position",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;

    let contributions = snapshots
        .iter()
        .map(|s| {
            let kind = match s.get::<&str, _>("type") {
                "ITEM" => Kind::Item,
                "SERVICE" => Kind::Service,
                "TASK" => Kind::Task,
                "MONEY" => Kind::Money {
                    amount_minor: s.get("amount_minor"),
                    settlement: match s.get::<&str, _>("settlement_mode") {
                        "PROCESSOR" => Settlement::Processor,
                        _ => Settlement::OffPlatform,
                    },
                },
                _ => Kind::Other,
            };
            let due = match s.get::<&str, _>("due_kind") {
                "DATE" => Due::Date(s.get::<Date, _>("due_date")),
                "AFTER_CONTRIBUTION" => {
                    Due::After(ContributionId(s.get("due_after_contribution_id")))
                }
                _ => Due::OnAgreement,
            };
            Contribution {
                id: ContributionId(s.get("contribution_id")),
                from: parse_slot(s.get("from_slot")),
                kind,
                description: s.get("description"),
                quantity: s
                    .get::<Option<String>, _>("quantity")
                    .map(|amount| Quantity {
                        amount,
                        unit: s.get("unit"),
                    }),
                due,
                completion_criteria: s.get("completion_criteria"),
                required: s.get("required"),
            }
        })
        .collect();

    // A signature counts for as long as the signer holds the slot they
    // signed in, or the account they were combined into holds it
    // (`holding_void_since`, migration 0028). One left behind by a claimant
    // who was removed before being confirmed is still in the record, and is
    // nobody's acceptance.
    let accepted_by = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT a.slot FROM acceptance a
         WHERE a.revision_id = $1
           AND holding_void_since(a.exchange_id, a.slot, a.holding) IS NULL
         ORDER BY a.slot",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|slot| parse_slot(slot))
    .collect();

    Ok(RevisionRecord {
        id,
        sequence: row.get("sequence"),
        author: parse_slot(row.get("author_slot")),
        expires_at: row.get("expires_at"),
        content_hash: row.get("content_hash"),
        accepted_by,
        revision: Revision {
            party_a: row.get("party_a_name"),
            party_b: row.get("party_b_name"),
            terms: row.get("terms"),
            contributions,
            // Attachments are not accepted by the API yet.
            attachments: Vec::new(),
            note: row.get("note"),
        },
    })
}

// ---- Storing ----------------------------------------------------------------

pub struct NewRevision<'a> {
    pub id: Uuid,
    pub exchange: &'a Aggregate,
    pub author: Slot,
    pub expires_at: OffsetDateTime,
    pub content_hash: [u8; 32],
    pub revision: &'a Revision,
}

/// Stores a revision and its contribution snapshots. A contribution ID that
/// is new gets its stable row here; one that belongs to another exchange
/// fails the foreign key.
pub async fn insert_revision(
    conn: &mut PgConnection,
    new: NewRevision<'_>,
) -> Result<(), sqlx::Error> {
    insert_revision_in(conn, new)
        .instrument(crate::db::span("revision.insert"))
        .await
}

async fn insert_revision_in(
    conn: &mut PgConnection,
    new: NewRevision<'_>,
) -> Result<(), sqlx::Error> {
    let exchange = new.exchange;
    // The revision this one answers: the one it replaces, else the agreement
    // it would amend.
    let parent = exchange
        .open
        .as_ref()
        .or(exchange.in_force.as_ref())
        .map(|record| record.id);

    sqlx::query(
        "INSERT INTO revision
            (id, exchange_id, sequence, parent_revision_id, author_slot, note, terms, expires_at,
             content_hash, party_a_name, party_b_name)
         VALUES ($1, $2,
                 (SELECT coalesce(max(sequence), 0) + 1 FROM revision WHERE exchange_id = $2),
                 $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(new.id)
    .bind(exchange.id)
    .bind(parent)
    .bind(slot_str(new.author))
    .bind(&new.revision.note)
    .bind(&new.revision.terms)
    .bind(new.expires_at)
    .bind(new.content_hash.as_slice())
    .bind(&new.revision.party_a)
    .bind(&new.revision.party_b)
    .execute(&mut *conn)
    .await?;

    for (position, contribution) in new.revision.contributions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO contribution (id, exchange_id) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(contribution.id.0)
        .bind(exchange.id)
        .execute(&mut *conn)
        .await?;

        let (kind, amount_minor, settlement) = match contribution.kind {
            Kind::Item => ("ITEM", None, None),
            Kind::Service => ("SERVICE", None, None),
            Kind::Task => ("TASK", None, None),
            Kind::Other => ("OTHER", None, None),
            Kind::Money {
                amount_minor,
                settlement,
            } => (
                "MONEY",
                Some(amount_minor),
                Some(match settlement {
                    Settlement::OffPlatform => "OFF_PLATFORM",
                    Settlement::Processor => "PROCESSOR",
                }),
            ),
        };
        let (due_kind, due_date, due_after) = match contribution.due {
            Due::Date(date) => ("DATE", Some(date), None),
            Due::OnAgreement => ("ON_AGREEMENT", None, None),
            Due::After(id) => ("AFTER_CONTRIBUTION", None, Some(id.0)),
        };

        sqlx::query(
            "INSERT INTO contribution_snapshot
                (exchange_id, revision_id, contribution_id, position, from_slot, to_slot, type,
                 description, quantity, unit, due_kind, due_date, due_after_contribution_id,
                 completion_criteria, required, amount_minor, currency, settlement_mode)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::numeric, $10, $11, $12, $13, $14, $15,
                     $16, $17, $18)",
        )
        .bind(exchange.id)
        .bind(new.id)
        .bind(contribution.id.0)
        .bind(position as i32)
        .bind(slot_str(contribution.from))
        .bind(slot_str(contribution.from.other()))
        .bind(kind)
        .bind(&contribution.description)
        .bind(contribution.quantity.as_ref().map(|q| q.amount.as_str()))
        .bind(
            contribution
                .quantity
                .as_ref()
                .and_then(|q| q.unit.as_deref()),
        )
        .bind(due_kind)
        .bind(due_date)
        .bind(due_after)
        .bind(&contribution.completion_criteria)
        .bind(contribution.required)
        .bind(amount_minor)
        .bind(amount_minor.map(|_| exchange.currency.as_str()))
        .bind(settlement)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The stored form of an event. `claimant` is the account holding the
/// invited party's slot, recorded on the event that put them there and on
/// the one that took them out again.
fn event_row(
    event: &Event,
    claimant: Option<Uuid>,
) -> (&'static str, Option<Uuid>, Option<Uuid>, Value) {
    let none = json!({});
    match event {
        Event::CounterpartyClaimed { confirmed } => (
            "COUNTERPARTY_CLAIMED",
            None,
            None,
            json!({ "confirmed": confirmed, "account": claimant }),
        ),
        Event::CounterpartyConfirmed => ("COUNTERPARTY_CONFIRMED", None, None, none),
        // Who was removed is kept here for the same reason as who claimed.
        // The revision is the one they had signed, if they had.
        Event::CounterpartyRejected { voided } => (
            "COUNTERPARTY_REJECTED",
            voided.map(|revision| revision.0),
            None,
            json!({ "account": claimant, "signature_void": voided.is_some() }),
        ),
        Event::CounterpartyReleased { voided } => (
            "COUNTERPARTY_RELEASED",
            voided.map(|revision| revision.0),
            None,
            json!({ "account": claimant, "signature_void": voided.is_some() }),
        ),
        Event::RevisionSent { revision, .. } => ("REVISION_SENT", Some(revision.0), None, none),
        Event::RevisionSuperseded { revision } => {
            ("REVISION_SUPERSEDED", Some(revision.0), None, none)
        }
        Event::RevisionAccepted { revision, .. } => {
            ("REVISION_ACCEPTED", Some(revision.0), None, none)
        }
        Event::RevisionDeclined { revision, .. } => {
            ("REVISION_DECLINED", Some(revision.0), None, none)
        }
        Event::RevisionWithdrawn { revision, .. } => {
            ("REVISION_WITHDRAWN", Some(revision.0), None, none)
        }
        Event::RevisionExpired { revision } => ("REVISION_EXPIRED", Some(revision.0), None, none),
        Event::AgreementInForce { revision, statuses } => {
            let statuses: BTreeMap<String, &str> = statuses
                .iter()
                .map(|(id, status)| (id.0.to_string(), status_str(*status)))
                .collect();
            (
                "AGREEMENT_IN_FORCE",
                Some(revision.0),
                None,
                json!({ "statuses": statuses }),
            )
        }
        Event::ContributionChanged {
            contribution,
            action,
            status,
            ..
        } => {
            let kind = match action {
                Action::Claim => "CONTRIBUTION_CLAIMED",
                Action::RetractClaim => "CONTRIBUTION_CLAIM_RETRACTED",
                Action::Confirm => "CONTRIBUTION_CONFIRMED",
                Action::Dispute => "CONTRIBUTION_DISPUTED",
                Action::Waive => "CONTRIBUTION_WAIVED",
            };
            (
                kind,
                None,
                Some(contribution.0),
                json!({ "status": status_str(*status) }),
            )
        }
        Event::EndProposed { .. } => ("END_PROPOSED", None, None, none),
        Event::EndProposalCancelled { .. } => ("END_PROPOSAL_CANCELLED", None, None, none),
        Event::CloseRequested { .. } => ("CLOSE_REQUESTED", None, None, none),
        Event::CloseRequestRetracted { .. } => ("CLOSE_REQUEST_RETRACTED", None, None, none),
        Event::StatementAdded { .. } => ("STATEMENT_ADDED", None, None, none),
        Event::InactivityPrompted => ("INACTIVITY_PROMPTED", None, None, none),
        Event::Closed { outcome, waived } => {
            let (outcome, reason) = outcome_columns(State::Closed(*outcome));
            let waived: Vec<String> = waived.iter().map(|id| id.0.to_string()).collect();
            (
                "EXCHANGE_CLOSED",
                None,
                None,
                json!({ "outcome": outcome, "reason": reason, "waived": waived }),
            )
        }
    }
}

/// Takes an unconfirmed claimant out of the invited party's slot, leaving it
/// free to be claimed again.
///
/// Emptying the participant row is what ends their holding of the slot (the
/// database does that itself, see `0006_slot_holdings.sql`), and with it any
/// signature they gave: signatures count only while the holding they were
/// made under is open. Nothing of what they did is removed. Their claim,
/// their signature and the holding stay as they were, under their account.
///
/// What does go is what was theirs alone and binds nobody: a working copy,
/// messages still waiting to be sent to them about an exchange that is no
/// longer theirs, text updates they turned on for it, whose ending is
/// recorded as any other's, and showing their payment options on it.
async fn vacate(
    conn: &mut PgConnection,
    exchange: Uuid,
    removed: Uuid,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE participant SET account_id = NULL, removal_requested_at = NULL
         WHERE exchange_id = $1 AND slot = 'B' AND account_id = $2",
    )
    .bind(exchange)
    .bind(removed)
    .execute(&mut *conn)
    .await?;

    sqlx::query("DELETE FROM exchange_draft WHERE exchange_id = $1 AND account_id = $2")
        .bind(exchange)
        .bind(removed)
        .execute(&mut *conn)
        .await?;

    // One the worker is sending at this moment is locked by it and is let go:
    // waiting for it would hold up the exchange for as long as a send takes.
    sqlx::query(
        "UPDATE outbox SET completed_at = $3, last_error = $4
         WHERE id IN (SELECT id FROM outbox
                      WHERE exchange_id = $1 AND recipient_account_id = $2
                        AND completed_at IS NULL
                      FOR UPDATE SKIP LOCKED)",
    )
    .bind(exchange)
    .bind(removed)
    .bind(now)
    .bind(outbox::NO_LONGER_A_PARTY)
    .execute(&mut *conn)
    .await?;

    sms_updates::turn_off(conn, removed, exchange, sms_updates::Source::NoLongerAParty).await?;
    // Nor does whoever takes the place next see their payment options.
    sqlx::query("DELETE FROM payment_offer WHERE exchange_id = $1 AND account_id = $2")
        .bind(exchange)
        .bind(removed)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Stores a decision: appends its events, queues the messages about them,
/// and brings the exchange row, the contribution statuses, the counterparty
/// confirmation and who holds the invited party's slot up to date. The
/// caller holds the lock on the exchange row.
///
/// `note` is what the actor wrote with the command (a dispute reason, a
/// statement) and is kept on the first event.
pub async fn persist(
    conn: &mut PgConnection,
    before: &Aggregate,
    decision: &Decision,
    actor: Actor,
    note: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    persist_in(conn, before, decision, actor, note, now)
        .instrument(crate::db::span("exchange.persist"))
        .await?;
    // Every decision about an exchange is stored here, whichever service
    // made it, so this is where the funnel reads what happened. Counted
    // once the rows are written; a transaction that fails to commit after
    // this counts one step too many, which is rare and does not matter to
    // a funnel.
    crate::funnel::funnel().events(before.exchange.state, &decision.events);
    // The same steps by what the yup was started from, which is asked of the
    // store only when one of them happened.
    if crate::funnel::Funnel::counts_by_entry(before.exchange.state, &decision.events) {
        let started_from: Option<String> =
            sqlx::query_scalar("SELECT started_from FROM exchange WHERE id = $1")
                .bind(before.id)
                .fetch_one(&mut *conn)
                .await?;
        let entry = started_from
            .as_deref()
            .and_then(crate::funnel::StartedFrom::parse)
            .map_or(crate::funnel::Entry::Unknown, |from| from.entry());
        crate::funnel::funnel().events_by_entry(entry, before.exchange.state, &decision.events);
    }
    Ok(())
}

async fn persist_in(
    conn: &mut PgConnection,
    before: &Aggregate,
    decision: &Decision,
    actor: Actor,
    note: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let (actor_kind, actor_slot) = match actor {
        Actor::Party(slot) => ("PARTICIPANT", Some(slot_str(slot))),
        Actor::System => ("SYSTEM", None),
    };
    // Fulfillment events are recorded against the agreement they happened under.
    let in_force = before.in_force.as_ref().map(|record| record.id);

    let mut sequence = before.last_event_seq;
    // What the next event chains from (`crate::chain`): the last row's hash,
    // read in this transaction. None while the history still has rows from
    // before the chain, which then go on without one until the owner's
    // backfill.
    let mut previous = chain::head(&mut *conn, before.id, sequence).await?;
    // Which event gave each contribution its status.
    let mut caused_by: BTreeMap<ContributionId, i64> = BTreeMap::new();

    for (index, event) in decision.events.iter().enumerate() {
        sequence += 1;
        let (kind, revision, contribution, data) = event_row(event, before.accounts[1]);
        let fields = chain::EventFields {
            exchange_id: before.id,
            sequence,
            kind: kind.to_owned(),
            actor_kind: actor_kind.to_owned(),
            actor_slot: actor_slot.map(str::to_owned),
            contribution_id: contribution,
            revision_id: revision.or(contribution.and(in_force)),
            note: note.filter(|_| index == 0).map(str::to_owned),
            evidence_attachment_id: None,
            data,
            occurred_at: chain::to_micros(now),
        };
        let hash = previous.map(|previous| chain::link(&previous, &fields));
        sqlx::query(
            "INSERT INTO exchange_event
                (exchange_id, sequence, type, actor_kind, actor_slot, contribution_id, revision_id,
                 note, data, occurred_at, chain_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(fields.exchange_id)
        .bind(fields.sequence)
        .bind(&fields.kind)
        .bind(&fields.actor_kind)
        .bind(&fields.actor_slot)
        .bind(fields.contribution_id)
        .bind(fields.revision_id)
        .bind(&fields.note)
        .bind(&fields.data)
        .bind(fields.occurred_at)
        .bind(hash.as_ref().map(|hash| hash.as_slice()))
        .execute(&mut *conn)
        .await?;
        previous = hash;

        match event {
            Event::ContributionChanged { contribution, .. } => {
                caused_by.insert(*contribution, sequence);
            }
            Event::AgreementInForce { statuses, .. } => {
                caused_by.extend(statuses.keys().map(|id| (*id, sequence)));
            }
            Event::Closed { waived, .. } => {
                caused_by.extend(waived.iter().map(|id| (*id, sequence)));
            }
            _ => {}
        }
    }

    // An unconfirmed claimant was removed, or left. Before anyone is told
    // anything, so that what is closed here is only what was waiting for
    // them from before.
    let after = &decision.exchange;
    if let (Counterparty::Claimed, Counterparty::Unclaimed, Some(removed)) = (
        before.exchange.counterparty,
        after.counterparty,
        before.accounts[1],
    ) {
        vacate(conn, before.id, removed, now).await?;
    }

    // Who is told (DESIGN.md §12). Queued here, with the events, so that a
    // message exists exactly when the event it is about does.
    for notification in notifications(&before.exchange, actor, &decision.events) {
        let event_sequence = before.last_event_seq + 1 + notification.event as i64;
        for slot in notification.to {
            // An unclaimed slot is nobody yet: there is no one the platform
            // may message (invariant 7).
            if let Some(account) = before.account_of(slot) {
                outbox::enqueue(
                    conn,
                    before.id,
                    event_sequence,
                    account,
                    notification.notice,
                )
                .await?;
            }
        }
    }

    // Its Wallet passes are brought up to date by the worker, marked here so
    // that they are exactly when something happened (`crate::wallet::store`).
    wallet::store::mark_exchange_changed(conn, before.id).await?;

    for (id, status) in &after.statuses {
        if before.exchange.statuses.get(id) != Some(status) {
            sqlx::query(
                "UPDATE contribution SET status = $3, last_event_seq = $4
                 WHERE exchange_id = $1 AND id = $2",
            )
            .bind(before.id)
            .bind(id.0)
            .bind(status_str(*status))
            .bind(caused_by.get(id).copied().unwrap_or(sequence))
            .execute(&mut *conn)
            .await?;
        }
    }

    if before.exchange.counterparty != Counterparty::Confirmed
        && after.counterparty == Counterparty::Confirmed
    {
        sqlx::query(
            "UPDATE participant SET initiator_confirmed_at = $2
             WHERE exchange_id = $1 AND slot = 'B'",
        )
        .bind(before.id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    }

    let (outcome, reason) = outcome_columns(after.state);
    let closing = matches!(after.state, State::Closed(_));
    sqlx::query(
        "UPDATE exchange
         SET state = $2, closed_outcome = $3, closed_reason = $4,
             closed_at = CASE WHEN $5 THEN coalesce(closed_at, $6) END,
             open_revision_id = $7, in_force_revision_id = $8,
             end_proposed_by = $9, close_requested_by = $10, close_requested_at = $11,
             inactivity_prompted_at = $12, last_activity_at = $13,
             version = version + 1, last_event_seq = $14, updated_at = $6
         WHERE id = $1",
    )
    .bind(before.id)
    .bind(state_str(after.state))
    .bind(outcome)
    .bind(reason)
    .bind(closing)
    .bind(now)
    .bind(after.open.as_ref().map(|open| open.id.0))
    .bind(after.in_force.as_ref().map(|in_force| in_force.id.0))
    .bind(after.end_proposed_by.map(slot_str))
    .bind(after.close_request.map(|request| slot_str(request.by)))
    .bind(after.close_request.map(|request| request.at))
    .bind(after.inactivity_prompted_at)
    .bind(after.last_activity_at)
    .bind(sequence)
    .execute(&mut *conn)
    .await?;

    Ok(())
}
