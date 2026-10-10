//! The shapes of an exchange's record: its history, and the document a party
//! takes away as their copy.
//!
//! Nothing here identifies an account. A party is `A` or `B` and is named as
//! the agreement names them; email addresses, phone numbers and account IDs
//! have no field to travel in.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::domain::contribution::Status;
use crate::domain::revision::Slot;
use crate::exchanges::dto::{
    ContributionStatus, ContributionType, CounterpartyDto, DueDto, OutcomeDto, QuantityDto,
    StateDto,
};

/// Names this kind of document, so a file found later says what it is.
pub const FORMAT: &str = "exchange-record";

/// Bumped whenever the document changes in a way a reader must know about.
///
/// 2: an event from party B's place may have been made by someone who was
/// removed from it before being confirmed (`by_removed_claimant`), who is
/// not the party the document names as B; and a signature such a person
/// left is listed apart, under `void_signatures`. `signatures` still holds
/// only the signatures that count.
pub const FORMAT_VERSION: u32 = 2;

// ---- Requests ---------------------------------------------------------------

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryQuery {
    /// Return events before this sequence number. Leave out for the latest.
    pub before: Option<i64>,
    /// How many events to return: 50 unless said, 200 at most.
    pub limit: Option<i64>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RecordQuery {
    /// Continue after the revision with this sequence number. Taken from
    /// `part.next` of the part before.
    pub revisions_after: Option<i32>,
    /// Continue after the event with this sequence number. Taken from
    /// `part.next` of the part before.
    pub events_after: Option<i64>,
}

// ---- What both answers share ------------------------------------------------

/// Each party's name as the agreement writes it: the agreement in force, or
/// the first proposal until there is one.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Parties {
    #[serde(rename = "A")]
    pub a: String,
    #[serde(rename = "B")]
    pub b: String,
}

/// Who did something: one of the two parties, or the service acting on a
/// timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Actor {
    A,
    B,
    System,
}

/// Everything that can happen to an exchange. Events of any other kind are
/// not part of what the parties are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    /// The invited party opened the invitation and took their place.
    CounterpartyClaimed,
    /// The initiator confirmed who took it.
    CounterpartyConfirmed,
    /// The initiator said whoever took it was not who they invited. That
    /// person was removed, and the place was free to be taken again.
    CounterpartyRejected,
    /// Whoever took it left before the initiator had confirmed them.
    CounterpartyReleased,
    /// A revision was sent, which signs it. Its note is the message sent
    /// with it.
    RevisionSent,
    /// An open revision stopped being open without both signing: a newer one
    /// replaced it, or the exchange closed.
    RevisionSuperseded,
    /// The other party signed the open revision.
    RevisionAccepted,
    RevisionDeclined,
    RevisionWithdrawn,
    RevisionExpired,
    /// A revision signed by both became the agreement.
    AgreementInForce,
    /// The provider said they delivered. A claim, not a confirmation.
    ContributionClaimed,
    ContributionClaimRetracted,
    /// The recipient said they received it.
    ContributionConfirmed,
    /// The recipient said why they do not accept it; the note is the reason.
    ContributionDisputed,
    ContributionWaived,
    EndProposed,
    EndProposalCancelled,
    /// One party asked to close without agreement; the note is their
    /// statement.
    CloseRequested,
    CloseRequestRetracted,
    /// A statement about a close request.
    StatementAdded,
    InactivityPrompted,
    ExchangeClosed,
}

impl EventType {
    pub const ALL: [EventType; 23] = [
        EventType::CounterpartyClaimed,
        EventType::CounterpartyConfirmed,
        EventType::CounterpartyRejected,
        EventType::CounterpartyReleased,
        EventType::RevisionSent,
        EventType::RevisionSuperseded,
        EventType::RevisionAccepted,
        EventType::RevisionDeclined,
        EventType::RevisionWithdrawn,
        EventType::RevisionExpired,
        EventType::AgreementInForce,
        EventType::ContributionClaimed,
        EventType::ContributionClaimRetracted,
        EventType::ContributionConfirmed,
        EventType::ContributionDisputed,
        EventType::ContributionWaived,
        EventType::EndProposed,
        EventType::EndProposalCancelled,
        EventType::CloseRequested,
        EventType::CloseRequestRetracted,
        EventType::StatementAdded,
        EventType::InactivityPrompted,
        EventType::ExchangeClosed,
    ];

    /// The name the event is stored under.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::CounterpartyClaimed => "COUNTERPARTY_CLAIMED",
            EventType::CounterpartyConfirmed => "COUNTERPARTY_CONFIRMED",
            EventType::CounterpartyRejected => "COUNTERPARTY_REJECTED",
            EventType::CounterpartyReleased => "COUNTERPARTY_RELEASED",
            EventType::RevisionSent => "REVISION_SENT",
            EventType::RevisionSuperseded => "REVISION_SUPERSEDED",
            EventType::RevisionAccepted => "REVISION_ACCEPTED",
            EventType::RevisionDeclined => "REVISION_DECLINED",
            EventType::RevisionWithdrawn => "REVISION_WITHDRAWN",
            EventType::RevisionExpired => "REVISION_EXPIRED",
            EventType::AgreementInForce => "AGREEMENT_IN_FORCE",
            EventType::ContributionClaimed => "CONTRIBUTION_CLAIMED",
            EventType::ContributionClaimRetracted => "CONTRIBUTION_CLAIM_RETRACTED",
            EventType::ContributionConfirmed => "CONTRIBUTION_CONFIRMED",
            EventType::ContributionDisputed => "CONTRIBUTION_DISPUTED",
            EventType::ContributionWaived => "CONTRIBUTION_WAIVED",
            EventType::EndProposed => "END_PROPOSED",
            EventType::EndProposalCancelled => "END_PROPOSAL_CANCELLED",
            EventType::CloseRequested => "CLOSE_REQUESTED",
            EventType::CloseRequestRetracted => "CLOSE_REQUEST_RETRACTED",
            EventType::StatementAdded => "STATEMENT_ADDED",
            EventType::InactivityPrompted => "INACTIVITY_PROMPTED",
            EventType::ExchangeClosed => "EXCHANGE_CLOSED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

/// A revision, named both ways: by ID, and by the number people see.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RevisionRef {
    pub id: Uuid,
    /// 1 for the first proposal, counting up.
    pub sequence: i32,
}

/// A contribution, with its description as the parties wrote it in the
/// revision the event happened under.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContributionRef {
    pub id: Uuid,
    pub description: String,
}

/// One entry in the history. The history is append-only: nothing in it is
/// ever edited or removed, and a correction is a later entry.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecordEvent {
    /// Position in the exchange's history, counting from 1.
    pub sequence: i64,
    pub r#type: EventType,
    pub actor: Actor,
    /// Set when the actor was someone who had opened the invitation and was
    /// removed, or left, before the initiator confirmed them. `actor` is
    /// then `B` for the place they were in; they are not the party this
    /// record names as B, and the record does not say who they were.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_removed_claimant: Option<bool>,
    /// RFC 3339, UTC.
    pub at: String,
    /// What the actor wrote with it, exactly as written: the message sent
    /// with a revision, a note on a claim, the reason for a dispute, or a
    /// statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The revision the event is about; for an event about a contribution,
    /// the agreement that was in force.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<RevisionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution: Option<ContributionRef>,
    /// Where the contribution stood as a result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    /// When a revision came into force: where each of its contributions
    /// stood as a result. A contribution of the agreement before that is not
    /// listed was removed by the amendment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statuses: Option<Vec<ContributionStatus>>,
    /// When the invited party joined: whether the invitation named them, so
    /// that no confirmation by the initiator was needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invitation_named_them: Option<bool>,
    /// When whoever opened the invitation was removed or left: whether they
    /// had signed the revision then open, named in `revision`. If so, that
    /// signature is void: it never took effect and never can.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_void: Option<bool>,
    /// When the exchange closed: how.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<OutcomeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the exchange ended by agreement: what was released, undelivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waived: Option<Vec<Uuid>>,
}

// ---- The history ------------------------------------------------------------

/// A stretch of an exchange's history, oldest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryPage {
    /// Which side the reader is.
    pub you: Slot,
    pub parties: Parties,
    pub events: Vec<RecordEvent>,
    /// Set when there is history before this page: pass it as `before` to
    /// read the page before.
    pub earlier: Option<i64>,
    /// A reviewer has hidden what the parties wrote in this exchange from
    /// the reader: every note in this page reads as the same placeholder, in
    /// the reader's language (DESIGN.md §9). Always sent; a client may read
    /// its absence as `false`.
    #[schema(required = false)]
    pub content_hidden: bool,
}

// ---- The record -------------------------------------------------------------

/// What became of a revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RevisionStatus {
    /// Waiting for the other party's signature.
    Open,
    /// Signed by both: the agreement in force, or the one that was in force
    /// when the exchange closed.
    InForce,
    /// Signed by both and in force until an amendment replaced it.
    Replaced,
    /// A newer revision replaced it before both had signed.
    Superseded,
    /// Still open when the exchange closed.
    Voided,
    Declined,
    Withdrawn,
    Expired,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RevisionStanding {
    pub status: RevisionStatus,
    /// When it came to stand this way. RFC 3339, UTC.
    pub since: String,
    /// When it came into force, if it ever did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_force_at: Option<String>,
    /// The revision that took its place, where one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<RevisionRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationMethod {
    /// A one-time code sent to an email address.
    EmailOtp,
    /// A one-time code sent by text message to a phone number.
    PhoneOtp,
}

/// The evidence behind a signature: how the signer had shown the service who
/// they are. It is all the evidence there is.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Verification {
    pub method: VerificationMethod,
    /// When the signer entered that code, which is when they signed in and
    /// can be earlier than the signature. RFC 3339, UTC.
    pub verified_at: String,
    /// The same, in words, in the document's language.
    pub description: String,
    /// The kind of identifier the signer had signed in with, and how long
    /// before signing, in the document's language. Never the address or
    /// number. Left out for a signature made before this was kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

/// Which consent wording the signer was shown before signing.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ConsentShown {
    pub language: String,
    pub version: String,
}

/// One party's signature on one exact revision.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Signature {
    pub party: Slot,
    /// The signer's name as that revision writes it.
    pub name: String,
    /// RFC 3339, UTC.
    pub signed_at: String,
    /// The hash that was signed, in hex. It is the revision's own.
    pub content_hash: String,
    pub verification: Verification,
    pub consent: ConsentShown,
}

/// A signature that never took effect and never can. It was made by someone
/// who had opened the invitation and was removed, or left, before the
/// initiator confirmed them. It is kept because it happened. It names
/// nobody: that person is not the party the revision names.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct VoidSignature {
    /// The place the signer was in.
    pub party: Slot,
    /// RFC 3339, UTC.
    pub signed_at: String,
    /// The hash that was signed, in hex.
    pub content_hash: String,
    pub verification: Verification,
    pub consent: ConsentShown,
    /// When the signer was removed or left. RFC 3339, UTC.
    pub void_since: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettlementDto {
    /// Paid by any outside means and recorded by claim and confirmation.
    OffPlatform,
    Processor,
}

/// A contribution as it is signed.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedContribution {
    /// The same contribution keeps its ID from one revision to the next.
    pub id: Uuid,
    /// The party who owes it; the other receives it.
    pub from: Slot,
    pub r#type: ContributionType,
    pub description: String,
    pub quantity: Option<QuantityDto>,
    pub due: DueDto,
    pub completion_criteria: Option<String>,
    pub required: bool,
    /// Whole minor units of the exchange's currency. Money only.
    pub amount_minor: Option<i64>,
    /// Money only.
    pub settlement: Option<SettlementDto>,
}

/// Exactly what a signature covers. A revision's content hash is the SHA-256
/// of this object written as canonical JSON (RFC 8785).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedDocument {
    /// The version of this layout.
    pub v: u8,
    pub exchange: Uuid,
    pub currency: String,
    /// The timezone due dates are read in.
    pub timezone: String,
    pub parties: Parties,
    pub terms: String,
    pub contributions: Vec<SignedContribution>,
    /// SHA-256 of each attached file, in hex.
    pub attachments: Vec<String>,
}

/// A revision that was sent: what it says, who sent and signed it, and what
/// became of it.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RecordRevision {
    pub id: Uuid,
    /// 1 for the first proposal, counting up.
    pub sequence: i32,
    /// The revision this one answered: the offer it replaced, or the
    /// agreement it proposed to amend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<RevisionRef>,
    pub author: Slot,
    /// RFC 3339, UTC.
    pub sent_at: String,
    /// When it would lapse if not signed by both. RFC 3339, UTC.
    pub expires_at: String,
    /// The message sent with it. Not part of what is signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub standing: RevisionStanding,
    /// SHA-256 of `signed` as canonical JSON (RFC 8785), in hex.
    pub content_hash: String,
    /// What the signatures cover, word for word. Left out when a reviewer
    /// has hidden what was written in the exchange from the reader
    /// (`content_hidden`); `redacted` is there instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<SignedDocument>)]
    pub signed: Option<serde_json::Value>,
    /// Only when `signed` is left out: the signed document with every piece
    /// of free text (the terms, and each contribution's description,
    /// completion criteria and unit of quantity) replaced by the placeholder
    /// that says it was hidden by review. Names and amounts are as signed.
    /// It is not what was signed and does not hash to `content_hash`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<SignedDocument>)]
    pub redacted: Option<serde_json::Value>,
    /// The author's, made by sending it, then the other party's if they
    /// accepted. Only signatures that count are here.
    pub signatures: Vec<Signature>,
    /// Signatures that do not count and never will, kept apart so that
    /// nobody reading `signatures` takes one for the other party's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub void_signatures: Vec<VoidSignature>,
}

/// Where a contribution of the agreement stands.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RecordContribution {
    pub id: Uuid,
    /// The party who owes it.
    pub from: Slot,
    pub description: String,
    pub required: bool,
    pub status: Status,
    /// When it came to stand this way. RFC 3339, UTC.
    pub since: Option<String>,
}

/// How the exchange stands.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RecordExchange {
    pub id: Uuid,
    /// For telling exchanges apart. Not a credential.
    pub display_code: String,
    /// The timezone due dates are read in.
    pub timezone: String,
    pub currency: String,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub state: StateDto,
    pub counterparty: CounterpartyDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_outcome: Option<OutcomeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// The agreement: in force now, or in force when the exchange closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_force_revision: Option<RevisionRef>,
    /// The revision waiting to be signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_revision: Option<RevisionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_proposed_by: Option<Slot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_requested_by: Option<Slot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_requested_at: Option<String>,
    /// The sequence number of the latest event when this was written.
    pub last_event: i64,
}

/// What a reader of the document needs to be told, in its language.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Notices {
    /// What this document is.
    pub about: String,
    /// What a signature here rests on.
    pub signatures: String,
    /// That what the parties recorded about delivery is theirs alone.
    pub statements: String,
    /// How to recompute a content hash.
    pub content_hash: String,
}

/// Where to continue reading a record too long for one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Continuation {
    pub revisions_after: i32,
    pub events_after: i64,
}

/// Which stretch of the record a document holds.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Part {
    /// Where this document starts. Zero and zero for the first.
    pub from: Continuation,
    /// Where the next one starts, when the record goes on.
    pub next: Option<Continuation>,
    /// This one document holds the whole record.
    pub complete: bool,
}

/// The record of one exchange, as one self-contained document: a party's
/// copy to keep or to hand to someone else.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecordDocument {
    /// Always `exchange-record`.
    pub format: String,
    pub format_version: u32,
    /// RFC 3339, UTC.
    pub generated_at: String,
    /// The language `notices` and the descriptions are written in. What the
    /// parties wrote is never translated.
    pub language: String,
    pub notices: Notices,
    /// Which party this copy was made for. Both parties' copies hold the
    /// same record.
    pub prepared_for: Slot,
    pub exchange: RecordExchange,
    pub parties: Parties,
    /// Where each contribution of the agreement stands. These are the
    /// parties' own statements about delivery; the service did not check
    /// them.
    pub contributions: Vec<RecordContribution>,
    /// Every revision sent, oldest first.
    pub revisions: Vec<RecordRevision>,
    /// Everything that happened, oldest first.
    pub events: Vec<RecordEvent>,
    pub part: Part,
    /// Set when a reviewer has hidden what the parties wrote in this
    /// exchange from the reader (DESIGN.md §9). The terms, the descriptions,
    /// the criteria and every note then read as one placeholder, in
    /// `language`, in this copy, `signed` included, so a fingerprint no longer
    /// matches what is shown. The record itself is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hidden: Option<bool>,
    /// That the history is chained, and the fingerprint of its last entry,
    /// in the document's language: a printed copy anchors the history with
    /// it (`crate::chain`). Left out while the history has entries from
    /// before the chain that have not been chained yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_chain: Option<String>,
}

/// The record of an exchange as a reviewer reads it while a report about
/// it is open: everything a party's copy holds, for neither party, and with
/// nothing hidden.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReviewRecord {
    pub exchange: RecordExchange,
    pub parties: Parties,
    pub contributions: Vec<RecordContribution>,
    pub revisions: Vec<RecordRevision>,
    pub events: Vec<RecordEvent>,
    /// False when the record is longer than one document holds; what is
    /// here is its beginning.
    pub complete: bool,
}
