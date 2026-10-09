//! The record of an exchange, read back: its history, and the document a
//! party takes away as their copy (DESIGN.md §10, §14.1).
//!
//! Everything here only reads. The record is what the append-only tables
//! already hold: revisions, their signatures, and the events every state was
//! derived from. Like the exchange itself, it is visible only to its two
//! parties.
//!
//! One answer is bounded, because a history can grow without limit: see
//! [`Limits`]. A record too long for one document continues in another, and
//! each says where it starts and whether more follows.

pub mod dto;
pub mod notices;

use std::collections::HashMap;

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use self::dto::{
    Actor, ConsentShown, Continuation, ContributionRef, EventType, FORMAT, FORMAT_VERSION,
    HistoryPage, Part, Parties, RecordContribution, RecordDocument, RecordEvent, RecordExchange,
    RecordRevision, ReviewRecord, RevisionRef, RevisionStanding, RevisionStatus, Signature,
    Verification, VerificationMethod, VoidSignature,
};
use super::dto::{ContributionStatus, CounterpartyDto, OutcomeDto, rfc3339, state_dto};
use super::repo::{self, Aggregate};
use crate::domain::canonical::canonical_document;
use crate::domain::contribution::Status;
use crate::domain::exchange::Counterparty;
use crate::domain::revision::Slot;
use crate::error::{ApiError, ErrorCode};
use crate::http::extract::Session;
use crate::review;

/// How much one answer may hold.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Events in a page of history when the caller does not say.
    pub history_default: i64,
    /// The most events a page of history may hold.
    pub history_max: i64,
    /// The most events one record document holds.
    pub events: i64,
    /// The most revisions one record document holds.
    pub revisions: usize,
    /// A record document takes no further revision once the ones it holds
    /// come to this much text. It always takes at least one.
    pub revision_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            history_default: 50,
            history_max: 200,
            events: 500,
            revisions: 50,
            revision_bytes: 1_000_000,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn slot(text: &str) -> Slot {
    if text == "A" { Slot::A } else { Slot::B }
}

/// One snapshot for the several queries an answer takes, so it never shows
/// parts of two different moments.
async fn snapshot(db: &PgPool) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Which side of the exchange an account is, and what the two sides are
/// called. Anyone who is not a party is told the exchange does not exist,
/// exactly as when it does not.
async fn party(
    conn: &mut PgConnection,
    exchange: Uuid,
    account: Uuid,
) -> Result<(Slot, Parties), ApiError> {
    let rows: Vec<(String, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT slot, account_id, display_name FROM participant WHERE exchange_id = $1",
    )
    .bind(exchange)
    .fetch_all(&mut *conn)
    .await?;

    let mut you = None;
    let mut parties = Parties {
        a: String::new(),
        b: String::new(),
    };
    for (held, holder, name) in rows {
        let held = slot(&held);
        if holder == Some(account) {
            you = Some(held);
        }
        match held {
            Slot::A => parties.a = name,
            Slot::B => parties.b = name,
        }
    }
    Ok((you.ok_or(ErrorCode::NotFound)?, parties))
}

// ---- Events -----------------------------------------------------------------

/// Which stretch of the history to read.
#[derive(Clone, Copy)]
enum Stretch {
    /// The first events after this sequence number.
    After(i64),
    /// The last events before this sequence number.
    Before(i64),
}

/// Reads up to `limit` events, oldest first, and says whether the history
/// goes on beyond them in the direction read.
///
/// Only the kinds of event in [`EventType`] are read, and only what a party
/// or the service did. Anything else stored against an exchange is not the
/// parties' to see until this module says what it is.
async fn events(
    conn: &mut PgConnection,
    exchange: Uuid,
    stretch: Stretch,
    limit: i64,
) -> Result<(Vec<RecordEvent>, bool), sqlx::Error> {
    let (comparison, order, cursor) = match stretch {
        Stretch::After(sequence) => (">", "ASC", sequence),
        Stretch::Before(sequence) => ("<", "DESC", sequence),
    };
    // The message sent with a revision is kept on the revision. A
    // contribution is described as the agreement in force at the time
    // described it.
    //
    // Someone can be taken out of the invited party's place only before they
    // are confirmed, and the place has one holder at a time. So whatever
    // was done from that place up to the last such removal was done by
    // someone since removed, and by nobody the record names.
    let query = format!(
        "SELECT e.sequence, e.type, e.actor_slot, e.occurred_at, e.data,
                CASE WHEN e.type = 'REVISION_SENT' THEN r.note ELSE e.note END AS note,
                e.revision_id, r.sequence AS revision_sequence,
                e.contribution_id, s.description AS contribution_description,
                (e.actor_slot IS NOT DISTINCT FROM 'B' AND EXISTS (
                    SELECT 1 FROM exchange_event x
                    WHERE x.exchange_id = e.exchange_id AND x.sequence >= e.sequence
                      AND x.type IN ('COUNTERPARTY_REJECTED', 'COUNTERPARTY_RELEASED')
                )) AS by_removed_claimant
         FROM exchange_event e
         LEFT JOIN revision r ON r.exchange_id = e.exchange_id AND r.id = e.revision_id
         LEFT JOIN contribution_snapshot s
                ON s.revision_id = e.revision_id AND s.contribution_id = e.contribution_id
         WHERE e.exchange_id = $1
           AND e.type = ANY($2)
           AND e.actor_kind IN ('PARTICIPANT', 'SYSTEM')
           AND e.sequence {comparison} $3
         ORDER BY e.sequence {order}
         LIMIT $4"
    );
    let kinds: Vec<&str> = EventType::ALL.iter().map(|kind| kind.as_str()).collect();
    let rows = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(exchange)
        .bind(kinds)
        .bind(cursor)
        // One more than asked for, to learn whether there is more.
        .bind(limit + 1)
        .fetch_all(&mut *conn)
        .await?;

    let more = rows.len() as i64 > limit;
    let mut events: Vec<RecordEvent> = rows.iter().take(limit as usize).filter_map(event).collect();
    events.sort_by_key(|event| event.sequence);
    Ok((events, more))
}

fn outcome(text: &str) -> Option<OutcomeDto> {
    match text {
        "NOT_AGREED" => Some(OutcomeDto::NotAgreed),
        "COMPLETED" => Some(OutcomeDto::Completed),
        "ENDED_BY_AGREEMENT" => Some(OutcomeDto::EndedByAgreement),
        "UNRESOLVED" => Some(OutcomeDto::Unresolved),
        _ => None,
    }
}

fn status(value: &Value) -> Option<Status> {
    serde_json::from_value(value.clone()).ok()
}

/// One stored event as the parties may read it. The stored details are
/// picked from by name, never passed through: the event that records who
/// claimed an invitation also holds their account ID.
fn event(row: &PgRow) -> Option<RecordEvent> {
    let kind = EventType::parse(row.get("type"))?;
    let data: Value = row.get("data");

    let mut event = RecordEvent {
        sequence: row.get("sequence"),
        r#type: kind,
        actor: match row.get::<Option<&str>, _>("actor_slot") {
            Some("A") => Actor::A,
            Some(_) => Actor::B,
            None => Actor::System,
        },
        by_removed_claimant: row.get::<bool, _>("by_removed_claimant").then_some(true),
        at: rfc3339(row.get("occurred_at")),
        note: row.get("note"),
        revision: row
            .get::<Option<Uuid>, _>("revision_id")
            .zip(row.get::<Option<i32>, _>("revision_sequence"))
            .map(|(id, sequence)| RevisionRef { id, sequence }),
        contribution: row
            .get::<Option<Uuid>, _>("contribution_id")
            .map(|id| ContributionRef {
                id,
                description: row
                    .get::<Option<String>, _>("contribution_description")
                    .unwrap_or_default(),
            }),
        status: None,
        statuses: None,
        invitation_named_them: None,
        signature_void: None,
        outcome: None,
        reason: None,
        waived: None,
    };

    match kind {
        EventType::CounterpartyClaimed => {
            event.invitation_named_them = data["confirmed"].as_bool();
        }
        // The account that was removed is stored with these too, and stays
        // there.
        EventType::CounterpartyRejected | EventType::CounterpartyReleased => {
            event.signature_void = data["signature_void"].as_bool();
        }
        EventType::ContributionClaimed
        | EventType::ContributionClaimRetracted
        | EventType::ContributionConfirmed
        | EventType::ContributionDisputed
        | EventType::ContributionWaived => {
            event.status = status(&data["status"]);
        }
        EventType::AgreementInForce => {
            // Stored for every contribution the exchange has ever had; what
            // an amendment removed stays `REMOVED` in every later event, so
            // that part grows without limit and says nothing new.
            event.statuses = data["statuses"].as_object().map(|statuses| {
                statuses
                    .iter()
                    .filter_map(|(id, value)| Some((id.parse().ok()?, status(value)?)))
                    .filter(|(_, status)| *status != Status::Removed)
                    .map(|(id, status)| ContributionStatus {
                        id,
                        status,
                        since: None,
                    })
                    .collect()
            });
        }
        EventType::ExchangeClosed => {
            event.outcome = data["outcome"].as_str().and_then(outcome);
            event.reason = data["reason"].as_str().map(str::to_owned);
            event.waived = data["waived"]
                .as_array()
                .map(|waived| {
                    waived
                        .iter()
                        .filter_map(|id| id.as_str()?.parse().ok())
                        .collect::<Vec<Uuid>>()
                })
                .filter(|waived| !waived.is_empty());
        }
        _ => {}
    }
    Some(event)
}

/// The history of an exchange, for one of its parties: the latest events, or
/// the ones before a given event, oldest first.
pub async fn history(
    db: &PgPool,
    session: &Session,
    exchange: Uuid,
    before: Option<i64>,
    limit: Option<i64>,
    limits: &Limits,
) -> Result<HistoryPage, ApiError> {
    let limit = match limit {
        Some(limit) if limit < 1 => return Err(ErrorCode::InvalidRequest.into()),
        Some(limit) => limit.min(limits.history_max),
        None => limits.history_default,
    };

    let mut tx = snapshot(db).await?;
    let (you, parties) = party(&mut tx, exchange, session.account_id).await?;
    let stretch = Stretch::Before(before.unwrap_or(i64::MAX));
    let (mut events, more) = events(&mut tx, exchange, stretch, limit).await?;

    // What a reviewer hid from the reader reads as a placeholder.
    let hidden = review::hidden_text(&mut tx, exchange, session.account_id).await?;
    if let Some(placeholder) = &hidden {
        review::hide_in_events(&mut events, placeholder);
    }

    Ok(HistoryPage {
        you,
        parties,
        earlier: events.first().map(|first| first.sequence).filter(|_| more),
        events,
        content_hidden: hidden.is_some(),
    })
}

// ---- Revisions --------------------------------------------------------------

/// What the events say became of each of some revisions.
async fn standings(
    conn: &mut PgConnection,
    aggregate: &Aggregate,
    revisions: &[(Uuid, OffsetDateTime)],
) -> Result<HashMap<Uuid, RevisionStanding>, sqlx::Error> {
    let ids: Vec<Uuid> = revisions.iter().map(|(id, _)| *id).collect();

    // The event that ended each one, or brought it into force, with the
    // event that came straight after: a newer revision being sent, or the
    // exchange closing.
    let rows = sqlx::query(
        "SELECT e.revision_id, e.type, e.occurred_at, n.type AS next_type,
                nr.id AS next_revision, nr.sequence AS next_revision_sequence
         FROM exchange_event e
         LEFT JOIN exchange_event n
                ON n.exchange_id = e.exchange_id AND n.sequence = e.sequence + 1
         LEFT JOIN revision nr ON nr.exchange_id = n.exchange_id AND nr.id = n.revision_id
         WHERE e.exchange_id = $1 AND e.revision_id = ANY($2)
           AND e.type IN ('REVISION_SUPERSEDED', 'REVISION_DECLINED', 'REVISION_WITHDRAWN',
                          'REVISION_EXPIRED', 'AGREEMENT_IN_FORCE')
         ORDER BY e.sequence",
    )
    .bind(aggregate.id)
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;

    // For each that came into force, the next one that did.
    let successors = sqlx::query(
        "SELECT DISTINCT ON (f.revision_id)
                f.revision_id, n.occurred_at, r.id AS successor, r.sequence AS successor_sequence
         FROM exchange_event f
         JOIN exchange_event n
           ON n.exchange_id = f.exchange_id AND n.type = 'AGREEMENT_IN_FORCE'
          AND n.sequence > f.sequence
         JOIN revision r ON r.exchange_id = n.exchange_id AND r.id = n.revision_id
         WHERE f.exchange_id = $1 AND f.type = 'AGREEMENT_IN_FORCE' AND f.revision_id = ANY($2)
         ORDER BY f.revision_id, n.sequence",
    )
    .bind(aggregate.id)
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;

    let mut found = HashMap::new();
    for row in &rows {
        let revision: Uuid = row.get("revision_id");
        let at = rfc3339(row.get("occurred_at"));
        let standing = match row.get::<&str, _>("type") {
            "AGREEMENT_IN_FORCE" => RevisionStanding {
                status: RevisionStatus::InForce,
                since: at.clone(),
                in_force_at: Some(at),
                replaced_by: None,
            },
            ended => {
                let (status, replaced_by) = match (ended, row.get::<Option<&str>, _>("next_type")) {
                    ("REVISION_DECLINED", _) => (RevisionStatus::Declined, None),
                    ("REVISION_WITHDRAWN", _) => (RevisionStatus::Withdrawn, None),
                    ("REVISION_EXPIRED", _) => (RevisionStatus::Expired, None),
                    (_, Some("EXCHANGE_CLOSED")) => (RevisionStatus::Voided, None),
                    _ => (
                        RevisionStatus::Superseded,
                        row.get::<Option<Uuid>, _>("next_revision")
                            .zip(row.get::<Option<i32>, _>("next_revision_sequence"))
                            .map(|(id, sequence)| RevisionRef { id, sequence }),
                    ),
                };
                RevisionStanding {
                    status,
                    since: at,
                    in_force_at: None,
                    replaced_by,
                }
            }
        };
        found.insert(revision, standing);
    }

    for row in &successors {
        if let Some(standing) = found.get_mut(&row.get::<Uuid, _>("revision_id")) {
            standing.status = RevisionStatus::Replaced;
            standing.since = rfc3339(row.get("occurred_at"));
            standing.replaced_by = Some(RevisionRef {
                id: row.get("successor"),
                sequence: row.get("successor_sequence"),
            });
        }
    }

    // Whatever no event has ended is still open.
    for (id, sent_at) in revisions {
        found.entry(*id).or_insert_with(|| {
            if aggregate.open.as_ref().map(|open| open.id) != Some(*id) {
                tracing::error!(revision = %id, "a revision is neither open nor accounted for by an event");
            }
            RevisionStanding {
                status: RevisionStatus::Open,
                since: rfc3339(*sent_at),
                in_force_at: None,
                replaced_by: None,
            }
        });
    }
    Ok(found)
}

/// The signatures on one revision: those that count, and those left by
/// someone removed from the invited party's place before being confirmed.
#[derive(Default)]
struct Signed {
    signatures: Vec<(Slot, Signature)>,
    void: Vec<VoidSignature>,
}

/// Every signature on some revisions, in the order they were made.
async fn signatures(
    conn: &mut PgConnection,
    exchange: Uuid,
    revisions: &[Uuid],
    wording: &notices::Export,
) -> Result<HashMap<Uuid, Signed>, sqlx::Error> {
    // A signature stands or falls with the holding it was made under: the
    // signer's time in that place. One whose holding ended is void from then,
    // unless it ended only because the signer's account was combined into
    // the one that holds the place now (`holding_void_since`, migration
    // 0028): the same person, whose signature stands as written.
    let rows = sqlx::query(
        "SELECT a.revision_id, a.slot, a.content_hash, a.auth_method, a.authenticated_at,
                a.consent_language, a.consent_version, a.accepted_at,
                holding_void_since(a.exchange_id, a.slot, a.holding) AS void_since
         FROM acceptance a
         WHERE a.exchange_id = $1 AND a.revision_id = ANY($2)
         ORDER BY a.accepted_at, a.slot",
    )
    .bind(exchange)
    .bind(revisions)
    .fetch_all(&mut *conn)
    .await?;

    let mut found: HashMap<Uuid, Signed> = HashMap::new();
    for row in &rows {
        let party = slot(row.get("slot"));
        // The database allows only these two; anything else would be a
        // method this code has no words for, and is not passed off as one.
        let Some(method) = VerificationMethod::parse(row.get("auth_method")) else {
            tracing::error!("a signature was verified by a method the record cannot describe");
            continue;
        };
        let signed_at = rfc3339(row.get("accepted_at"));
        let content_hash = hex(&row.get::<Vec<u8>, _>("content_hash"));
        let verification = Verification {
            method,
            verified_at: rfc3339(row.get("authenticated_at")),
            description: wording.verification(method),
        };
        let consent = ConsentShown {
            language: row.get("consent_language"),
            version: row.get("consent_version"),
        };
        let signed = found.entry(row.get("revision_id")).or_default();
        match row.get::<Option<OffsetDateTime>, _>("void_since") {
            Some(void_since) => signed.void.push(VoidSignature {
                party,
                signed_at,
                content_hash,
                verification,
                consent,
                void_since: rfc3339(void_since),
            }),
            None => signed.signatures.push((
                party,
                Signature {
                    party,
                    // Filled in by the caller, which has the revision.
                    name: String::new(),
                    signed_at,
                    content_hash,
                    verification,
                    consent,
                },
            )),
        }
    }
    Ok(found)
}

/// Reads revisions after a sequence number, oldest first, until the
/// document is full, and says whether more follow.
async fn revisions(
    conn: &mut PgConnection,
    aggregate: &Aggregate,
    after: i32,
    limits: &Limits,
    wording: &notices::Export,
) -> Result<(Vec<RecordRevision>, bool), sqlx::Error> {
    let listed = sqlx::query(
        "SELECT r.id, r.created_at, p.id AS parent, p.sequence AS parent_sequence
         FROM revision r
         LEFT JOIN revision p ON p.exchange_id = r.exchange_id AND p.id = r.parent_revision_id
         WHERE r.exchange_id = $1 AND r.sequence > $2
         ORDER BY r.sequence
         LIMIT $3",
    )
    .bind(aggregate.id)
    .bind(after)
    .bind(limits.revisions as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;

    let mut bytes = 0;
    let mut loaded = Vec::new();
    for row in listed.iter().take(limits.revisions) {
        if bytes >= limits.revision_bytes {
            break;
        }
        let record = repo::load_revision(conn, row.get("id")).await?;
        let signed = canonical_document(
            aggregate.id,
            &aggregate.currency,
            &aggregate.timezone,
            &record.revision,
        );
        // What is stored must still say what was signed. If it ever does
        // not, the copy shows both as they are and anyone can see it.
        if Sha256::digest(signed.as_bytes()).as_slice() != record.content_hash.as_slice() {
            tracing::error!(revision = %record.id, "stored terms no longer reproduce the signed hash");
        }
        bytes += signed.len() + record.revision.note.as_ref().map_or(0, String::len);
        loaded.push((row, record, signed));
    }
    let more = loaded.len() < listed.len();

    let ids: Vec<Uuid> = loaded.iter().map(|(_, record, _)| record.id).collect();
    let sent: Vec<(Uuid, OffsetDateTime)> = loaded
        .iter()
        .map(|(row, record, _)| (record.id, row.get("created_at")))
        .collect();
    let mut standings = standings(conn, aggregate, &sent).await?;
    let mut signatures = signatures(conn, aggregate.id, &ids, wording).await?;

    let revisions = loaded
        .into_iter()
        .map(|(row, record, signed)| {
            let on_it = signatures.remove(&record.id).unwrap_or_default();
            let signatures = on_it
                .signatures
                .into_iter()
                .map(|(party, signature)| Signature {
                    name: match party {
                        Slot::A => record.revision.party_a.clone(),
                        Slot::B => record.revision.party_b.clone(),
                    },
                    ..signature
                })
                .collect();
            RecordRevision {
                id: record.id,
                sequence: record.sequence,
                answers: row
                    .get::<Option<Uuid>, _>("parent")
                    .zip(row.get::<Option<i32>, _>("parent_sequence"))
                    .map(|(id, sequence)| RevisionRef { id, sequence }),
                author: record.author,
                sent_at: rfc3339(row.get("created_at")),
                expires_at: rfc3339(record.expires_at),
                note: record.revision.note.clone(),
                standing: standings
                    .remove(&record.id)
                    .expect("every revision asked about has a standing"),
                content_hash: hex(&record.content_hash),
                signed: Some(
                    serde_json::from_str(&signed).expect("the canonical document is JSON"),
                ),
                redacted: None,
                signatures,
                void_signatures: on_it.void,
            }
        })
        .collect();
    Ok((revisions, more))
}

// ---- The record -------------------------------------------------------------

/// Where each contribution of the agreement stands, in the agreement's order.
async fn contributions(
    conn: &mut PgConnection,
    aggregate: &Aggregate,
) -> Result<Vec<RecordContribution>, sqlx::Error> {
    let Some(in_force) = &aggregate.in_force else {
        return Ok(Vec::new());
    };
    let ids: Vec<Uuid> = in_force
        .revision
        .contributions
        .iter()
        .map(|contribution| contribution.id.0)
        .collect();
    let since: HashMap<Uuid, OffsetDateTime> = sqlx::query_as(
        "SELECT c.id, e.occurred_at
         FROM contribution c
         JOIN exchange_event e
           ON e.exchange_id = c.exchange_id AND e.sequence = c.last_event_seq
         WHERE c.exchange_id = $1 AND c.id = ANY($2)",
    )
    .bind(aggregate.id)
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();

    Ok(in_force
        .revision
        .contributions
        .iter()
        .filter_map(|contribution| {
            let status = *aggregate.exchange.statuses.get(&contribution.id)?;
            Some(RecordContribution {
                id: contribution.id.0,
                from: contribution.from,
                description: contribution.description.clone(),
                required: contribution.required,
                status,
                since: since.get(&contribution.id.0).copied().map(rfc3339),
            })
        })
        .collect())
}

async fn standing(
    conn: &mut PgConnection,
    aggregate: &Aggregate,
) -> Result<RecordExchange, sqlx::Error> {
    let (created_at, closed_at): (OffsetDateTime, Option<OffsetDateTime>) =
        sqlx::query_as("SELECT created_at, closed_at FROM exchange WHERE id = $1")
            .bind(aggregate.id)
            .fetch_one(&mut *conn)
            .await?;

    let exchange = &aggregate.exchange;
    let (state, closed_outcome) = state_dto(exchange.state);
    let named = |record: &repo::RevisionRecord| RevisionRef {
        id: record.id,
        sequence: record.sequence,
    };
    Ok(RecordExchange {
        id: aggregate.id,
        display_code: aggregate.display_code.clone(),
        timezone: aggregate.timezone.clone(),
        currency: aggregate.currency.clone(),
        created_at: rfc3339(created_at),
        state,
        counterparty: match exchange.counterparty {
            Counterparty::Unclaimed => CounterpartyDto::Unclaimed,
            Counterparty::Claimed => CounterpartyDto::Claimed,
            Counterparty::Confirmed => CounterpartyDto::Confirmed,
        },
        closed_outcome,
        closed_reason: repo::outcome_columns(exchange.state).1.map(str::to_owned),
        closed_at: closed_at.map(rfc3339),
        in_force_revision: aggregate.in_force.as_ref().map(named),
        open_revision: aggregate.open.as_ref().map(named),
        end_proposed_by: exchange.end_proposed_by,
        close_requested_by: exchange.close_request.map(|request| request.by),
        close_requested_at: exchange.close_request.map(|request| rfc3339(request.at)),
        last_event: aggregate.last_event_seq,
    })
}

/// The record of an exchange as one document, for one of its parties:
/// everything from `from` on that fits, and where the next document starts
/// if the record goes on.
pub async fn record(
    db: &PgPool,
    session: &Session,
    exchange: Uuid,
    from: Continuation,
    limits: &Limits,
) -> Result<RecordDocument, ApiError> {
    if from.revisions_after < 0 || from.events_after < 0 {
        return Err(ErrorCode::InvalidRequest.into());
    }

    let mut tx = snapshot(db).await?;
    let (you, parties) = party(&mut tx, exchange, session.account_id).await?;
    let aggregate = repo::load(&mut tx, exchange, false)
        .await?
        .ok_or(ErrorCode::NotFound)?;

    // In the reader's own language, like everything else they are shown.
    let preference: String = sqlx::query_scalar("SELECT language FROM account WHERE id = $1")
        .bind(session.account_id)
        .fetch_one(&mut *tx)
        .await?;
    let (language, wording) = notices::wording(&preference);

    let (revisions, more_revisions) =
        revisions(&mut tx, &aggregate, from.revisions_after, limits, wording).await?;
    let (events, more_events) = events(
        &mut tx,
        exchange,
        Stretch::After(from.events_after),
        limits.events,
    )
    .await?;

    let next = (more_revisions || more_events).then(|| Continuation {
        revisions_after: revisions
            .last()
            .map_or(from.revisions_after, |revision| revision.sequence),
        events_after: events
            .last()
            .map_or(from.events_after, |event| event.sequence),
    });
    let first = Continuation {
        revisions_after: 0,
        events_after: 0,
    };

    let mut contributions = contributions(&mut tx, &aggregate).await?;
    let (mut revisions, mut events) = (revisions, events);
    // What a reviewer hid from the reader reads as a placeholder, in this
    // copy too: it is the reader's.
    let hidden = review::hidden_text(&mut tx, exchange, session.account_id).await?;
    if let Some(placeholder) = &hidden {
        review::hide_in_events(&mut events, placeholder);
        review::hide_in_revisions(&mut revisions, placeholder);
        for contribution in &mut contributions {
            contribution.description = placeholder.clone();
        }
    }

    Ok(RecordDocument {
        format: FORMAT.to_owned(),
        format_version: FORMAT_VERSION,
        generated_at: rfc3339(OffsetDateTime::now_utc()),
        language: language.to_owned(),
        notices: wording.notices(),
        prepared_for: you,
        exchange: standing(&mut tx, &aggregate).await?,
        parties,
        contributions,
        revisions,
        events,
        part: Part {
            complete: from == first && next.is_none(),
            from,
            next,
        },
        content_hidden: hidden.is_some().then_some(true),
    })
}

/// The record of an exchange as a reviewer reads it (`crate::review`): its
/// beginning, as much as one document holds, with nothing hidden and for
/// neither party. Whether the reviewer may read it at all is the caller's
/// to decide. `language` is the reviewer's, for the descriptions of how
/// each signer was checked.
pub async fn for_review(
    conn: &mut PgConnection,
    exchange: Uuid,
    language: &str,
    limits: &Limits,
) -> Result<Option<ReviewRecord>, sqlx::Error> {
    let Some(aggregate) = repo::load(conn, exchange, false).await? else {
        return Ok(None);
    };
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT slot, display_name FROM participant WHERE exchange_id = $1")
            .bind(exchange)
            .fetch_all(&mut *conn)
            .await?;
    let mut parties = Parties {
        a: String::new(),
        b: String::new(),
    };
    for (held, name) in rows {
        match slot(&held) {
            Slot::A => parties.a = name,
            Slot::B => parties.b = name,
        }
    }
    let (_, wording) = notices::wording(language);
    let (revisions, more_revisions) = revisions(conn, &aggregate, 0, limits, wording).await?;
    let (events, more_events) = events(conn, exchange, Stretch::After(0), limits.events).await?;
    Ok(Some(ReviewRecord {
        exchange: standing(conn, &aggregate).await?,
        parties,
        contributions: contributions(conn, &aggregate).await?,
        revisions,
        events,
        complete: !more_revisions && !more_events,
    }))
}

#[cfg(test)]
mod tests {
    use super::dto::SignedDocument;
    use super::*;
    use crate::domain::revision::tests::fence_job;
    use crate::domain::revision::{Quantity, Settlement};

    #[test]
    fn the_signed_document_is_described_field_for_field() {
        // The record hands out the canonical document as it is. Its
        // published shape refuses unknown fields, so a change to what is
        // hashed fails here until the shape says so too.
        let mut revision = fence_job();
        revision.contributions[0].quantity = Some(Quantity {
            amount: "1.50".into(),
            unit: Some("days".into()),
        });
        revision.contributions[0].completion_criteria = Some("The gate swings freely".into());
        revision.attachments.push([7; 32]);
        let text = canonical_document(Uuid::from_u128(0xE), "USD", "America/Chicago", &revision);

        let signed: SignedDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(signed.parties.a, "Ana");
        assert_eq!(signed.contributions.len(), 2);
        assert_eq!(signed.attachments, [hex(&[7; 32])]);
        assert!(matches!(
            signed.contributions[1].settlement,
            Some(dto::SettlementDto::OffPlatform)
        ));
        // And nothing is lost on the way back out.
        let again: Value = serde_json::to_value(&signed).unwrap();
        assert_eq!(again, serde_json::from_str::<Value>(&text).unwrap());

        let mut processor = fence_job();
        processor.contributions[1].kind = crate::domain::revision::Kind::Money {
            amount_minor: 1,
            settlement: Settlement::Processor,
        };
        let text = canonical_document(Uuid::nil(), "USD", "UTC", &processor);
        assert!(serde_json::from_str::<SignedDocument>(&text).is_ok());
    }

    #[test]
    fn every_kind_of_event_has_a_name_that_reads_back() {
        for kind in EventType::ALL {
            assert_eq!(EventType::parse(kind.as_str()), Some(kind));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                Value::String(kind.as_str().to_owned()),
                "the API names it as the database does"
            );
        }
        assert_eq!(EventType::parse("ACCOUNT_REPORTED"), None);
    }
}
