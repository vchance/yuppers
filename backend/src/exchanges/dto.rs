//! The shapes the exchange endpoints accept and return, and their
//! translation to and from the domain types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Date, Month, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use super::repo::{Aggregate, RevisionRecord};
use crate::domain::Rules;
use crate::domain::contribution::{Action, Status};
use crate::domain::exchange::{Counterparty, Outcome, State};
use crate::domain::revision::{
    Contribution, ContributionId, Due, Kind, Quantity, Revision, Settlement, Slot,
};
use crate::error::{ApiError, ErrorCode};

// ---- Revision terms ---------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContributionType {
    Item,
    Service,
    Task,
    Other,
    Money,
}

/// When a contribution falls due.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DueDto {
    /// A calendar date, `YYYY-MM-DD`, in the exchange's timezone.
    Date { date: String },
    /// When the agreement is accepted.
    OnAgreement,
    /// When another contribution in the same revision is accepted.
    AfterContribution { contribution: Uuid },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QuantityDto {
    /// A positive decimal number such as `2` or `1.5`.
    pub amount: String,
    pub unit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ContributionDto {
    /// Chosen by the client for a new contribution; kept unchanged for one
    /// carried over from an earlier revision.
    pub id: Uuid,
    /// The party who owes it.
    pub from: Slot,
    pub r#type: ContributionType,
    pub description: String,
    pub quantity: Option<QuantityDto>,
    pub due: DueDto,
    pub completion_criteria: Option<String>,
    pub required: bool,
    /// Whole minor units of the exchange's currency. Money only.
    pub amount_minor: Option<i64>,
}

/// What a revision says: the part both parties sign.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RevisionTerms {
    pub party_a_name: String,
    pub party_b_name: String,
    pub terms: String,
    pub contributions: Vec<ContributionDto>,
}

fn parse_date(text: &str) -> Option<Date> {
    let mut parts = text.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let month = Month::try_from(month.parse::<u8>().ok()?).ok()?;
    Date::from_calendar_date(year.parse().ok()?, month, day.parse().ok()?).ok()
}

fn format_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

impl RevisionTerms {
    pub fn into_domain(self, note: Option<String>) -> Result<Revision, ApiError> {
        let contributions = self
            .contributions
            .into_iter()
            .map(ContributionDto::into_domain)
            .collect::<Result<_, _>>()?;
        Ok(Revision {
            party_a: self.party_a_name.trim().to_owned(),
            party_b: self.party_b_name.trim().to_owned(),
            terms: self.terms,
            contributions,
            attachments: Vec::new(),
            note: note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()),
        })
    }

    pub fn from_domain(revision: &Revision) -> Self {
        Self {
            party_a_name: revision.party_a.clone(),
            party_b_name: revision.party_b.clone(),
            terms: revision.terms.clone(),
            contributions: revision
                .contributions
                .iter()
                .map(ContributionDto::from_domain)
                .collect(),
        }
    }
}

impl ContributionDto {
    fn into_domain(self) -> Result<Contribution, ApiError> {
        let kind = match (self.r#type, self.amount_minor) {
            // Everything is settled outside the platform until payments exist.
            (ContributionType::Money, Some(amount_minor)) => Kind::Money {
                amount_minor,
                settlement: Settlement::OffPlatform,
            },
            (ContributionType::Item, None) => Kind::Item,
            (ContributionType::Service, None) => Kind::Service,
            (ContributionType::Task, None) => Kind::Task,
            (ContributionType::Other, None) => Kind::Other,
            // Money without an amount, or an amount on something else.
            _ => return Err(ErrorCode::InvalidRequest.into()),
        };
        let due = match self.due {
            DueDto::Date { date } => Due::Date(parse_date(&date).ok_or(ErrorCode::InvalidRequest)?),
            DueDto::OnAgreement => Due::OnAgreement,
            DueDto::AfterContribution { contribution } => Due::After(ContributionId(contribution)),
        };
        Ok(Contribution {
            id: ContributionId(self.id),
            from: self.from,
            kind,
            description: self.description.trim().to_owned(),
            quantity: self.quantity.map(|q| Quantity {
                amount: q.amount,
                unit: q.unit,
            }),
            due,
            completion_criteria: self.completion_criteria,
            required: self.required,
        })
    }

    fn from_domain(contribution: &Contribution) -> Self {
        let (r#type, amount_minor) = match contribution.kind {
            Kind::Item => (ContributionType::Item, None),
            Kind::Service => (ContributionType::Service, None),
            Kind::Task => (ContributionType::Task, None),
            Kind::Other => (ContributionType::Other, None),
            Kind::Money { amount_minor, .. } => (ContributionType::Money, Some(amount_minor)),
        };
        Self {
            id: contribution.id.0,
            from: contribution.from,
            r#type,
            description: contribution.description.clone(),
            quantity: contribution.quantity.as_ref().map(|q| QuantityDto {
                amount: q.amount.clone(),
                unit: q.unit.clone(),
            }),
            due: match contribution.due {
                Due::Date(date) => DueDto::Date {
                    date: format_date(date),
                },
                Due::OnAgreement => DueDto::OnAgreement,
                Due::After(id) => DueDto::AfterContribution { contribution: id.0 },
            },
            completion_criteria: contribution.completion_criteria.clone(),
            required: contribution.required,
            amount_minor,
        }
    }
}

// ---- Requests ---------------------------------------------------------------

/// Which consent wording the signer was shown. Signing is refused unless it
/// is the current version.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct Consent {
    /// The language the wording was shown in, as a supported language tag.
    pub language: String,
    pub version: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateExchange {
    /// IANA timezone name, such as `America/Chicago`. Due dates are read in it.
    pub timezone: String,
    /// How the author began: a template and its version (`job-deposit-balance@1`),
    /// `blank` or `copy`. Kept once for aggregate counts; never returned.
    /// Clients that send nothing leave it unset.
    #[serde(default)]
    pub started_from: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveDraft {
    /// The client's working copy, stored as given.
    #[schema(value_type = Object)]
    pub body: serde_json::Value,
}

/// Who an invitation link is for, said outright: one of `bound_to` and
/// `for_anyone: true`, never both and never neither. A link anyone holding
/// it can claim is never what a request gets by leaving something out.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InvitationOptions {
    /// Names who the invitation is for: only an account that has verified
    /// this email address or phone number can claim it. A phone number is
    /// read as signing in reads it: a US number may leave out the `+1`.
    pub bound_to: Option<String>,
    /// `true` for a link anyone who has it can claim. Required, as `true`,
    /// when `bound_to` is not given.
    #[serde(default)]
    pub for_anyone: bool,
}

/// Sending a revision signs it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendRevision {
    /// The exchange version the client last saw.
    pub expected_version: i64,
    pub terms: RevisionTerms,
    /// A message to the other party. Not part of what is signed.
    pub note: Option<String>,
    pub consent: Consent,
    /// Required when this is the first revision, which also issues the
    /// invitation; ignored after.
    pub invitation: Option<InvitationOptions>,
    /// How many times the composer's split sheets were completed while this
    /// was written (DESIGN.md §7.1, §7.2, "Measures"). Counted and nothing
    /// else: no amount, no description, no date. A client may leave it out.
    #[serde(default)]
    pub splits: Option<SplitsUsed>,
}

/// The split sheets completed while a revision was written.
#[derive(Clone, Copy, Debug, Default, Deserialize, ToSchema)]
pub struct SplitsUsed {
    /// "Split into instalments" completed.
    #[serde(default)]
    pub instalments: u8,
    /// "Split into stages" completed.
    #[serde(default)]
    pub stages: u8,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandDto {
    /// Sign the open revision.
    Accept {
        revision: Uuid,
        consent: Consent,
    },
    Decline {
        revision: Uuid,
    },
    /// Take back a revision you sent, before it takes effect.
    Withdraw {
        revision: Uuid,
    },
    /// Throw away a draft that was never sent. It closes with nothing agreed
    /// and leaves the list. Only the initiator has one, and only until the
    /// first revision is sent.
    Discard,
    /// Claim, confirm, dispute or waive a contribution. A dispute needs a
    /// reason, and a claim after a dispute needs a note.
    Contribution {
        contribution: Uuid,
        action: Action,
        note: Option<String>,
    },
    /// The initiator confirms who claimed the invitation.
    ConfirmCounterparty,
    /// The initiator says whoever claimed the invitation is not who they
    /// invited. That person is removed, anything they signed is void, and a
    /// new invitation link can be issued.
    RejectCounterparty,
    ProposeEnd,
    AcceptEnd,
    CancelEnd,
    RequestClose {
        note: Option<String>,
    },
    RetractClose,
    AddStatement {
        note: String,
    },
    /// Mark several payments as paid at once ("Mark the rest as paid",
    /// DESIGN.md §7.1): one claim for each, in the order given, each exactly
    /// as if claimed on its own, and one message to the other party saying
    /// how many. All or none. Money items you owe, each still pending.
    ClaimRest {
        contributions: Vec<Uuid>,
    },
    /// Note how an item you provide is going (DESIGN.md §7.2). A statement
    /// of your own: it is not a claim, changes no status and asks nothing of
    /// the other party. Only while the item is pending or claimed; a limited
    /// number per item; no text update.
    NoteProgress {
        contribution: Uuid,
        note: String,
    },
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunCommand {
    /// The exchange version the client last saw.
    pub expected_version: i64,
    pub command: CommandDto,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InvitationToken {
    pub token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimInvitation {
    pub token: String,
    /// Only open the exchange if the signed-in account already holds the
    /// place this link gave, and never take it: anything else, a live link
    /// included, is answered `INVITATION_UNAVAILABLE`. For a client that
    /// finds a link spent and wants to take the person who used it back to
    /// their exchange without claiming anything they did not tap for.
    #[serde(default)]
    pub only_if_yours: bool,
}

// ---- Responses --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StateDto {
    Draft,
    Negotiating,
    Active,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OutcomeDto {
    NotAgreed,
    Completed,
    EndedByAgreement,
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CounterpartyDto {
    /// Nobody is in the invited party's place: nobody has opened the
    /// invitation yet, or whoever did was removed or left.
    Unclaimed,
    /// Claimed, waiting for the initiator to confirm who it is. Until then
    /// the claimant can sign or leave, and nothing else.
    Claimed,
    Confirmed,
}

/// Who claimed the invitation, shown to the initiator so they can confirm.
#[derive(Debug, Serialize, ToSchema)]
pub struct Claimant {
    pub display_name: String,
    /// Partly hidden, such as `b•••@example.com`.
    pub identifier: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevisionView {
    pub id: Uuid,
    pub sequence: i32,
    pub author: Slot,
    pub note: Option<String>,
    /// RFC 3339.
    pub expires_at: String,
    /// Who has signed it. The author always has. A signature left by someone
    /// who was removed from the invited party's place is not counted.
    pub accepted_by: Vec<Slot>,
    /// SHA-256 of the signed terms, in hex.
    pub content_hash: String,
    pub terms: RevisionTerms,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContributionStatus {
    pub id: Uuid,
    pub status: Status,
    /// When it came to stand this way, RFC 3339. Not sent where it is not
    /// known, such as on an agreement coming into force in the history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExchangeView {
    pub id: Uuid,
    /// For telling exchanges apart. Not a credential.
    pub display_code: String,
    pub state: StateDto,
    pub closed_outcome: Option<OutcomeDto>,
    pub closed_reason: Option<String>,
    /// Send this back as `expected_version` with the next change.
    pub version: i64,
    pub timezone: String,
    pub currency: String,
    /// Which side the viewer is.
    pub you: Slot,
    pub counterparty: CounterpartyDto,
    pub claimant: Option<Claimant>,
    /// Only for the initiator, and only while nobody is in the invited
    /// party's place: whether an invitation link is out that can still be
    /// used. It is not once the link has been used, even by someone since
    /// removed, or has expired; a new one has to be issued.
    pub invitation_open: Option<bool>,
    /// With `invitation_open`: when the initiator last opened a way to pass
    /// the link on (RFC 3339), such as a text message or the share sheet,
    /// as the apps report it (`POST /v1/exchanges/{id}/invitation/shared`).
    /// Yuppers never sends the link itself, so this says the sender opened a
    /// way to send it, not that it arrived. Null until they have, and for a
    /// link that replaced an earlier one until that one is shared too.
    pub invitation_shared_at: Option<String>,
    /// The other party has deleted their account and can no longer act in
    /// this exchange. Said only while the exchange is still open, which is
    /// when the viewer needs to know it. Always sent; a client may read its
    /// absence as `false`.
    #[schema(required = false)]
    pub other_party_left: bool,
    /// The revision awaiting acceptance, if any.
    pub open_revision: Option<RevisionView>,
    /// The agreement currently binding, if any.
    pub in_force_revision: Option<RevisionView>,
    /// Status of each contribution in the agreement in force.
    pub contributions: Vec<ContributionStatus>,
    pub end_proposed_by: Option<Slot>,
    pub close_requested_by: Option<Slot>,
    pub close_requested_at: Option<String>,
    /// When the close request's response window runs out and the exchange
    /// closes unresolved, unless the request is taken back or what is
    /// outstanding is resolved first (DESIGN.md §5.3). RFC 3339. Computed
    /// here, so every client shows the same moment.
    pub close_request_lapses_at: Option<String>,
    /// The viewer's own unsent working copy.
    #[schema(value_type = Option<Object>)]
    pub draft: Option<serde_json::Value>,
    /// A reviewer has hidden what the parties wrote in this exchange from
    /// the viewer (DESIGN.md §9): the terms, the descriptions, the criteria
    /// and the messages read as one placeholder in the viewer's language,
    /// there is no working copy, and signing or sending terms is refused
    /// with `CONTENT_HIDDEN`. Always sent; a client may read its absence as
    /// `false`.
    #[schema(required = false)]
    pub content_hidden: bool,
    /// Payment options on this agreement (`crate::payments`): whether the
    /// viewer shows theirs, and the other party's where the viewer may see
    /// them. Not part of the terms. Always sent; a client may read its
    /// absence as nothing shown either way.
    #[schema(required = false)]
    pub payment_options: PaymentOptionsView,
}

/// Payment options on one agreement, as one party sees them. Yuppers never
/// moves money: these open someone else's app, and a payment is recorded
/// only when the payer says so and the payee confirms it.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct PaymentOptionsView {
    /// Whether the viewer shows their own payment options to the other
    /// party on this agreement.
    pub shown: bool,
    /// The other party's payment options, only while they show them on
    /// this agreement, it is in force, and the viewer owes them money on it
    /// that is neither marked paid, accepted nor waived. Null otherwise.
    pub theirs: Option<crate::payments::PaymentHandles>,
    /// When each of `theirs` changed, for those that changed after the
    /// agreement came into force, however long ago: the payer is warned
    /// beside them to check with the payee another way. Never the old value.
    /// All null when `theirs` is.
    #[schema(required = false)]
    pub theirs_changed: crate::payments::PaymentHandleChanges,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevisionSent {
    pub exchange: ExchangeView,
    /// The link token for the invited party, returned once, when the first
    /// revision is sent. Only its hash is kept.
    pub invitation_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvitationIssued {
    pub invitation_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExchangeSummary {
    pub id: Uuid,
    pub display_code: String,
    pub state: StateDto,
    pub closed_outcome: Option<OutcomeDto>,
    pub you: Slot,
    pub other_party_name: String,
    pub updated_at: String,
    /// Who is in the invited party's place, as on the exchange's view, so
    /// the initiator's list can say that nobody has joined yet.
    pub counterparty: CounterpartyDto,
    /// As on the exchange's view: only for the initiator while nobody is in
    /// the invited party's place, when they last opened a way to pass the
    /// link on, if they have.
    pub invitation_shared_at: Option<String>,
    /// Where a series of payments stands, when one party owes the other two
    /// or more money items in the agreement (instalments, DESIGN.md §7.1).
    /// Counts only: never an amount. Left out for a single payment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payments: Option<SeriesCount>,
    /// The same for two or more services or tasks from one party (stages,
    /// DESIGN.md §7.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stages: Option<SeriesCount>,
}

/// How many items of a series are confirmed. Counted by the items' own
/// statuses; there is no running total of anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct SeriesCount {
    /// Items in the series: at least two.
    pub total: i64,
    pub confirmed: i64,
    pub disputed: i64,
}

/// What someone holding an invitation link sees before signing in.
#[derive(Debug, Serialize, ToSchema)]
pub struct InvitationPreview {
    pub display_code: String,
    pub expires_at: String,
    /// The invitation names a specific person.
    pub bound: bool,
    /// The exchange's currency, which its amounts are in.
    pub currency: String,
    /// The exchange's IANA timezone, which its due dates are read in.
    pub timezone: String,
    pub revision: RevisionView,
    /// Present when the invitation names an email address or phone number
    /// the signed-in account does not have: only which kind, never the
    /// address, in full or masked. Someone who knows it can type it, and
    /// with a code sent there add it to their account and open the
    /// invitation (`POST /v1/invitations/address/codes`); claiming it as it
    /// is is refused with `INVITATION_NOT_FOR_YOU`.
    pub sent_to: Option<BoundAddress>,
}

/// Whom an invitation names, as someone signed in with another address is
/// told it: the kind alone.
#[derive(Debug, Serialize, ToSchema)]
pub struct BoundAddress {
    pub kind: crate::combine::IdentifierKind,
    /// The account already has another address of this kind: adding this
    /// one replaces it, which needs `replace: true` and a proof of one of
    /// the account's own (`POST /v1/me/identifiers/proof`).
    pub replaces: bool,
}

pub fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339)
        .expect("a UTC timestamp formats as RFC 3339")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn state_dto(state: State) -> (StateDto, Option<OutcomeDto>) {
    match state {
        State::Draft => (StateDto::Draft, None),
        State::Negotiating => (StateDto::Negotiating, None),
        State::Active => (StateDto::Active, None),
        State::Closed(outcome) => (
            StateDto::Closed,
            Some(match outcome {
                Outcome::NotAgreed(_) => OutcomeDto::NotAgreed,
                Outcome::Completed => OutcomeDto::Completed,
                Outcome::EndedByAgreement => OutcomeDto::EndedByAgreement,
                Outcome::Unresolved(_) => OutcomeDto::Unresolved,
            }),
        ),
    }
}

impl RevisionView {
    pub fn from_record(record: &RevisionRecord) -> Self {
        Self {
            id: record.id,
            sequence: record.sequence,
            author: record.author,
            note: record.revision.note.clone(),
            expires_at: rfc3339(record.expires_at),
            accepted_by: record.accepted_by.clone(),
            content_hash: hex(&record.content_hash),
            terms: RevisionTerms::from_domain(&record.revision),
        }
    }
}

/// What the database knows about an exchange beyond the aggregate, gathered
/// for its view.
pub struct ViewContext {
    pub claimant: Option<Claimant>,
    pub invitation_open: Option<bool>,
    pub invitation_shared_at: Option<String>,
    pub other_party_left: bool,
    pub draft: Option<serde_json::Value>,
    /// When each contribution came to its current status.
    pub status_since: BTreeMap<ContributionId, OffsetDateTime>,
    pub payment_options: PaymentOptionsView,
}

impl ExchangeView {
    pub fn build(aggregate: &Aggregate, you: Slot, context: ViewContext, rules: &Rules) -> Self {
        let exchange = &aggregate.exchange;
        let (state, closed_outcome) = state_dto(exchange.state);

        // Only what the agreement in force contains, in its order.
        let contributions = aggregate
            .in_force
            .iter()
            .flat_map(|record| &record.revision.contributions)
            .filter_map(|c| {
                exchange
                    .statuses
                    .get(&c.id)
                    .map(|&status| ContributionStatus {
                        id: c.id.0,
                        status,
                        since: context.status_since.get(&c.id).copied().map(rfc3339),
                    })
            })
            .collect();
        let ViewContext {
            claimant,
            invitation_open,
            invitation_shared_at,
            other_party_left,
            draft,
            payment_options,
            ..
        } = context;

        Self {
            id: aggregate.id,
            display_code: aggregate.display_code.clone(),
            state,
            closed_outcome,
            closed_reason: super::repo::outcome_columns(exchange.state)
                .1
                .map(str::to_owned),
            version: aggregate.version,
            timezone: aggregate.timezone.clone(),
            currency: aggregate.currency.clone(),
            you,
            counterparty: match exchange.counterparty {
                Counterparty::Unclaimed => CounterpartyDto::Unclaimed,
                Counterparty::Claimed => CounterpartyDto::Claimed,
                Counterparty::Confirmed => CounterpartyDto::Confirmed,
            },
            claimant,
            invitation_open,
            invitation_shared_at,
            other_party_left,
            open_revision: aggregate.open.as_ref().map(RevisionView::from_record),
            in_force_revision: aggregate.in_force.as_ref().map(RevisionView::from_record),
            contributions,
            end_proposed_by: exchange.end_proposed_by,
            close_requested_by: exchange.close_request.map(|request| request.by),
            close_requested_at: exchange.close_request.map(|request| rfc3339(request.at)),
            // The same sum the timer uses (`exchange::lapse_close_request`).
            // A moment too far ahead to compute is one that never comes, and
            // is left out rather than made up.
            close_request_lapses_at: exchange
                .close_request
                .and_then(|request| request.at.checked_add(rules.close_response_window))
                .map(rfc3339),
            draft,
            content_hidden: false,
            payment_options,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_strict_calendar_dates() {
        assert_eq!(
            parse_date("2026-11-01").map(format_date).as_deref(),
            Some("2026-11-01")
        );
        for bad in [
            "2026-13-01",
            "2026-02-30",
            "26-11-01",
            "2026-11-1",
            "2026-11-01-",
            "tomorrow",
            "",
        ] {
            assert!(parse_date(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn money_and_amount_go_together() {
        let contribution = |r#type, amount_minor| ContributionDto {
            id: Uuid::nil(),
            from: Slot::A,
            r#type,
            description: "x".into(),
            quantity: None,
            due: DueDto::OnAgreement,
            completion_criteria: None,
            required: true,
            amount_minor,
        };
        assert!(
            contribution(ContributionType::Money, Some(100))
                .into_domain()
                .is_ok()
        );
        assert!(
            contribution(ContributionType::Item, None)
                .into_domain()
                .is_ok()
        );
        assert!(
            contribution(ContributionType::Money, None)
                .into_domain()
                .is_err()
        );
        assert!(
            contribution(ContributionType::Item, Some(100))
                .into_domain()
                .is_err()
        );
    }
}
