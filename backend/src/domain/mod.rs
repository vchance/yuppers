//! Pure domain rules. Nothing in this module touches the database or the
//! network: each state machine is a function from (current state, action,
//! actor) to a new state or a typed refusal (DESIGN.md §13.3).

use time::Duration;

pub mod amendment;
pub mod canonical;
pub mod contribution;
pub mod exchange;
pub mod identity;
pub mod invitation;
pub mod notification;
pub mod reminder;
pub mod revision;
pub mod risk;

/// The tunable numbers the rules depend on. They are configuration, not
/// constants: the defaults are the decisions recorded in DESIGN.md.
#[derive(Clone, Debug)]
pub struct Rules {
    /// How long a sent revision stays open for acceptance (§6).
    pub revision_ttl: Duration,
    /// How long the other party has to answer a close request (§5.3).
    pub close_response_window: Duration,
    /// Idle time after which both parties are prompted (§5.3).
    pub inactivity_prompt_after: Duration,
    /// Time after the prompt at which an idle exchange closes (§5.3).
    pub inactivity_close_after: Duration,
    /// How long before its due date the party who owes a contribution is
    /// reminded that it is due soon (§12). Counted in whole days, because a
    /// due date is a calendar date. A placeholder.
    pub due_soon_lead: Duration,
    /// Longest note, in characters: on a revision, a claim, a dispute, a close
    /// request, a statement or a progress note (§6, §7.2).
    pub note_max_chars: usize,
    /// Total money in an agreement above which stronger verification is
    /// required, in minor units (§8).
    pub tier_one_threshold_minor: i64,
    /// How recent a one-time code must be to count as fresh at signing (§8).
    pub fresh_code_window: Duration,
    /// How long an invitation link can be claimed (§8).
    pub invitation_ttl: Duration,
    /// Exchanges one account may create per day (§9). A placeholder.
    pub exchanges_per_day: i64,
    /// Changes one party may make to one exchange per minute (§9). Keeps a
    /// party from flooding the permanent history, and from changing the
    /// exchange so fast the other can never act on what they see.
    /// A placeholder.
    pub changes_per_minute: i64,
    /// Invitation links one exchange may be issued per day (§9). A placeholder.
    pub invitations_per_day: i64,
    /// How far ahead a due date may be set. A placeholder.
    pub due_date_horizon: Duration,
    /// How long a signature\'s network address and user agent are kept
    /// (DESIGN.md §14). The signature itself is permanent.
    pub network_metadata_retention: Duration,
    /// How long the record of consent to text updates is kept after the
    /// updates it covers have ended (README, "Text updates"): proof that a
    /// person asked for the texts they were sent. Four years, the time
    /// within which a claim about unwanted texts can usually be brought in
    /// the US. A placeholder for counsel to confirm.
    pub sms_consent_retention: Duration,
    /// Progress notes one contribution may carry (DESIGN.md §7.2). A
    /// placeholder from the design.
    pub progress_notes_per_contribution: i64,
    /// Size limits on what a revision may contain. Placeholders.
    pub limits: Limits,
}

/// How much a revision may hold. Agreement history is permanent, so
/// everything written into it is bounded.
#[derive(Clone, Debug)]
pub struct Limits {
    pub contributions: usize,
    pub party_name_chars: usize,
    pub terms_chars: usize,
    /// A contribution's description, and its completion criteria.
    pub description_chars: usize,
    pub unit_chars: usize,
    /// The unsent working copy, as stored.
    pub draft_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            contributions: 50,
            party_name_chars: 200,
            terms_chars: 20_000,
            description_chars: 2_000,
            unit_chars: 50,
            draft_bytes: 200_000,
        }
    }
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            revision_ttl: Duration::days(14),
            close_response_window: Duration::days(7),
            inactivity_prompt_after: Duration::days(60),
            inactivity_close_after: Duration::days(30),
            due_soon_lead: Duration::days(2),
            note_max_chars: 1000,
            tier_one_threshold_minor: 50_000,
            fresh_code_window: Duration::minutes(10),
            invitation_ttl: Duration::days(14),
            exchanges_per_day: 20,
            changes_per_minute: 20,
            invitations_per_day: 5,
            due_date_horizon: Duration::days(3650),
            network_metadata_retention: Duration::days(90),
            sms_consent_retention: Duration::days(365 * 4),
            progress_notes_per_contribution: 20,
            limits: Limits::default(),
        }
    }
}
