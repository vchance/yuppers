//! The product funnel, counted where each step happens (docs/operations.md,
//! "Telemetry"): accounts created, yups created (a first proposal sent),
//! invitations shared and claimed, agreements come into force, contributions
//! confirmed and disputed, agreements closed, and the codes, texts and emails
//! sent. A claim counts as a share when none was recorded, so there are never
//! more claims than shares. Counts only: no label here ever names a person, an exchange, an
//! address or a number, and every label is from a fixed set.
//!
//! One set of counters for the process, like the historian's, rather than
//! one per `AppState`: the steps happen deep in the exchange service and the
//! deliveries, which have no state to carry a counter in, and a funnel is a
//! property of the service, not of a router. Tests read the process's counts
//! before and after what they do (`backend/tests/telemetry.rs`).
//!
//! Counters start again when a process does, so the worker also takes a
//! **daily snapshot** from the database ([`daily`]): the previous UTC day's
//! counts of the same steps, logged once and served as gauges all day.
//! Aggregate counts only; the queries read nothing about anyone.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::PgPool;
use time::{Date, OffsetDateTime, Time};

use crate::domain::contribution::{Action, Status};
use crate::domain::exchange::{Event, NotAgreed, Outcome, State, Unresolved};
use crate::domain::identity::Identifier;
use crate::domain::revision::{Kind as ItemKind, Revision, Slot};
use crate::metrics::{Kind, Name, Text};

pub const ACCOUNTS_CREATED: Name = Name::new("yuppers.accounts.created", "{account}");
pub const YUPS_CREATED: Name = Name::new("yuppers.yups.created", "{yup}");
pub const INVITATIONS_SHARED: Name = Name::new("yuppers.invitations.shared", "{invitation}");
pub const INVITATIONS_CLAIMED: Name = Name::new("yuppers.invitations.claimed", "{invitation}");
pub const AGREEMENTS_IN_FORCE: Name = Name::new("yuppers.agreements.in_force", "{agreement}");
pub const CONTRIBUTIONS_CONFIRMED: Name =
    Name::new("yuppers.contributions.confirmed", "{contribution}");
pub const CONTRIBUTIONS_DISPUTED: Name =
    Name::new("yuppers.contributions.disputed", "{contribution}");
pub const AGREEMENTS_CLOSED: Name = Name::new("yuppers.agreements.closed", "{agreement}");
pub const CODES_SENT: Name = Name::new("yuppers.codes.sent", "{code}");
pub const TEXTS_SENT: Name = Name::new("yuppers.texts.sent", "{message}");
pub const EMAILS_SENT: Name = Name::new("yuppers.emails.sent", "{message}");
pub const ENTRIES_STARTED: Name = Name::new("yuppers.entries.started", "{yup}");
pub const ENTRIES_SENT: Name = Name::new("yuppers.entries.sent", "{yup}");
pub const ENTRIES_IN_FORCE: Name = Name::new("yuppers.entries.in_force", "{agreement}");
pub const ENTRIES_COMPLETED: Name = Name::new("yuppers.entries.completed", "{agreement}");
pub const INSTALMENT_YUPS: Name = Name::new("yuppers.instalments.yups", "{agreement}");
pub const INSTALMENT_ITEMS: Name = Name::new("yuppers.instalments.items", "{payment}");
pub const STAGE_YUPS: Name = Name::new("yuppers.stages.yups", "{agreement}");
pub const STAGE_ITEMS: Name = Name::new("yuppers.stages.items", "{stage}");
pub const SPLITS_USED: Name = Name::new("yuppers.splits.used", "{sheet}");
pub const MARK_REST_USED: Name = Name::new("yuppers.mark_rest.used", "{command}");
pub const NOTICES_COALESCED: Name = Name::new("yuppers.notices.coalesced", "{message}");
pub const PROGRESS_NOTES_ADDED: Name = Name::new("yuppers.progress_notes.added", "{note}");
pub const DAILY_YUPS_CREATED: Name = Name::new("yuppers.daily.yups_created", "{yup}");
pub const DAILY_INVITATIONS_SHARED: Name =
    Name::new("yuppers.daily.invitations_shared", "{invitation}");
pub const DAILY_INVITATIONS_CLAIMED: Name =
    Name::new("yuppers.daily.invitations_claimed", "{invitation}");
pub const DAILY_AGREEMENTS_IN_FORCE: Name =
    Name::new("yuppers.daily.agreements_in_force", "{agreement}");
pub const DAILY_AGREEMENTS_COMPLETED: Name =
    Name::new("yuppers.daily.agreements_completed", "{agreement}");

/// Where a yup was started from, as a label: one of the shipped templates,
/// the blank form, a copy of an earlier yup, a client that said nothing
/// (`unknown`), or something well formed that this version does not list
/// (`other`). A fixed set, so the label never carries what a client chose
/// to send. The ids are the shared package's (`packages/shared/src/templates.ts`),
/// which a test there checks against this list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Entry {
    JobDepositBalance,
    SellingSomething,
    SwapNoMoney,
    LendingItem,
    PetSittingChildcare,
    SplittingCost,
    Blank,
    Copy,
    Unknown,
    Other,
}

impl Entry {
    pub const ALL: [Self; 10] = [
        Self::JobDepositBalance,
        Self::SellingSomething,
        Self::SwapNoMoney,
        Self::LendingItem,
        Self::PetSittingChildcare,
        Self::SplittingCost,
        Self::Blank,
        Self::Copy,
        Self::Unknown,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::JobDepositBalance => "job-deposit-balance",
            Self::SellingSomething => "selling-something",
            Self::SwapNoMoney => "swap-no-money",
            Self::LendingItem => "lending-item",
            Self::PetSittingChildcare => "pet-sitting-childcare",
            Self::SplittingCost => "splitting-cost",
            Self::Blank => "blank",
            Self::Copy => "copy",
            Self::Unknown => "unknown",
            Self::Other => "other",
        }
    }

    fn of_id(id: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .filter(|entry| !matches!(entry, Self::Unknown | Self::Other))
            .find(|entry| entry.as_str() == id)
            .unwrap_or(Self::Other)
    }
}

/// What a client says a yup was started from (`exchange.started_from`,
/// DESIGN.md section 4.4, "Measurement"): `blank`, `copy`, or a template id
/// and version such as `job-deposit-balance@1`. Only this form is accepted,
/// so the column cannot hold free text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedFrom(String);

impl StartedFrom {
    pub fn parse(text: &str) -> Option<Self> {
        let valid = match text {
            "blank" | "copy" => true,
            _ => text.split_once('@').is_some_and(|(id, version)| {
                (1..=40).contains(&id.len())
                    && id.starts_with(|c: char| c.is_ascii_lowercase())
                    && id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    && (1..=4).contains(&version.len())
                    && version.chars().all(|c| c.is_ascii_digit())
            }),
        };
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The entry, without the version.
    pub fn entry(&self) -> Entry {
        Entry::of_id(self.0.split_once('@').map_or(self.0.as_str(), |(id, _)| id))
    }
}

/// How a person signed up, or a code went: by email address or phone number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Email,
    Phone,
}

impl Channel {
    pub fn of(identifier: &Identifier) -> Self {
        match identifier {
            Identifier::Email(_) => Self::Email,
            Identifier::Phone(_) => Self::Phone,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Phone => "phone",
        }
    }
}

/// What kind of email went out through the outbox. Codes are not sent
/// through it, and count under [`CODES_SENT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmailKind {
    /// A notice about an exchange, a reminder included.
    Notice,
    /// A notice about the account itself (`notifications::outbox`, account
    /// notices): a report waiting for review, accounts combined.
    Account,
}

impl EmailKind {
    const ALL: [Self; 2] = [Self::Notice, Self::Account];

    fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Account => "account",
        }
    }
}

/// Which split sheet of the composer (DESIGN.md §7.1, §7.2), as a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    Instalments,
    Stages,
}

impl Split {
    const ALL: [Self; 2] = [Self::Instalments, Self::Stages];

    fn as_str(self) -> &'static str {
        match self {
            Self::Instalments => "instalments",
            Self::Stages => "stages",
        }
    }
}

/// How an agreement is shaped, as two counts and nothing about what it says
/// (DESIGN.md §7.1, §7.2, "Measures"): the most money items one party owes
/// the other, and the most service and task items one party provides. A
/// count under two is a single payment, or a single job, and is zero here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    /// Money items from one payer, when there are at least two.
    pub instalments: usize,
    /// Service and task items from one provider, when there are at least two.
    pub stages: usize,
}

impl Shape {
    pub fn of(revision: &Revision) -> Self {
        let most = |counts: [usize; 2]| match counts.into_iter().max().unwrap_or(0) {
            0 | 1 => 0,
            many => many,
        };
        let mut payments = [0; 2];
        let mut jobs = [0; 2];
        for contribution in &revision.contributions {
            let side = match contribution.from {
                Slot::A => 0,
                Slot::B => 1,
            };
            match contribution.kind {
                ItemKind::Money { .. } => payments[side] += 1,
                ItemKind::Service | ItemKind::Task => jobs[side] += 1,
                ItemKind::Item | ItemKind::Other => {}
            }
        }
        Self {
            instalments: most(payments),
            stages: most(jobs),
        }
    }
}

/// How an agreement closed, as a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum Closure {
    Withdrawn,
    Declined,
    Expired,
    Discarded,
    Completed,
    EndedByAgreement,
    UnresolvedCloseRequest,
    UnresolvedInactive,
}

impl Closure {
    const ALL: [Self; 8] = [
        Self::Withdrawn,
        Self::Declined,
        Self::Expired,
        Self::Discarded,
        Self::Completed,
        Self::EndedByAgreement,
        Self::UnresolvedCloseRequest,
        Self::UnresolvedInactive,
    ];

    fn of(outcome: Outcome) -> Self {
        match outcome {
            Outcome::NotAgreed(NotAgreed::Withdrawn) => Self::Withdrawn,
            Outcome::NotAgreed(NotAgreed::Declined) => Self::Declined,
            Outcome::NotAgreed(NotAgreed::Expired) => Self::Expired,
            Outcome::NotAgreed(NotAgreed::Discarded) => Self::Discarded,
            Outcome::Completed => Self::Completed,
            Outcome::EndedByAgreement => Self::EndedByAgreement,
            Outcome::Unresolved(Unresolved::CloseRequest) => Self::UnresolvedCloseRequest,
            Outcome::Unresolved(Unresolved::Inactive) => Self::UnresolvedInactive,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Withdrawn => "withdrawn",
            Self::Declined => "declined",
            Self::Expired => "expired",
            Self::Discarded => "discarded",
            Self::Completed => "completed",
            Self::EndedByAgreement => "ended_by_agreement",
            Self::UnresolvedCloseRequest => "unresolved_close_request",
            Self::UnresolvedInactive => "unresolved_inactive",
        }
    }
}

#[derive(Clone, Copy)]
enum Step {
    Sent,
    InForce,
    Completed,
}

/// The previous UTC day's counts, read from the database by [`daily`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Daily {
    /// The day counted, as `2026-10-08`.
    pub day: String,
    pub yups_created: i64,
    pub invitations_shared: i64,
    pub invitations_claimed: i64,
    pub agreements_in_force: i64,
    pub agreements_completed: i64,
}

/// The counts since the process started.
#[derive(Default)]
pub struct Funnel {
    accounts: [AtomicU64; 2],
    yups_created: AtomicU64,
    invitations_shared: AtomicU64,
    invitations_claimed: AtomicU64,
    agreements_in_force: AtomicU64,
    contributions_confirmed: AtomicU64,
    contributions_disputed: AtomicU64,
    closed: [AtomicU64; 8],
    codes: [AtomicU64; 2],
    texts_sent: AtomicU64,
    emails: [AtomicU64; 2],
    entries_started: [AtomicU64; 10],
    entries_sent: [AtomicU64; 10],
    entries_in_force: [AtomicU64; 10],
    entries_completed: [AtomicU64; 10],
    instalment_yups: AtomicU64,
    instalment_items: AtomicU64,
    stage_yups: AtomicU64,
    stage_items: AtomicU64,
    splits_used: [AtomicU64; 2],
    mark_rest_used: AtomicU64,
    notices_coalesced: AtomicU64,
    progress_notes_added: AtomicU64,
    daily: Mutex<Option<Daily>>,
}

/// A plain copy of the counts, for tests to compare before and after.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub accounts_email: u64,
    pub accounts_phone: u64,
    pub yups_created: u64,
    pub invitations_shared: u64,
    pub invitations_claimed: u64,
    pub agreements_in_force: u64,
    pub contributions_confirmed: u64,
    pub contributions_disputed: u64,
    /// By [`Closure`], in its order: withdrawn, declined, expired, discarded,
    /// completed, ended by agreement, unresolved (close request), unresolved
    /// (inactive).
    pub closed: [u64; 8],
    pub codes_email: u64,
    pub codes_phone: u64,
    pub texts_sent: u64,
    /// Notices, account notices.
    pub emails: [u64; 2],
    /// Yups started, sent for the first time, come into force and completed,
    /// each by [`Entry`], in its order.
    pub entries_started: [u64; 10],
    pub entries_sent: [u64; 10],
    pub entries_in_force: [u64; 10],
    pub entries_completed: [u64; 10],
    /// Agreements that first came into force with instalments, and the
    /// payments in them; the same for stages.
    pub instalment_yups: u64,
    pub instalment_items: u64,
    pub stage_yups: u64,
    pub stage_items: u64,
    /// Split sheets completed, by [`Split`]: instalments, stages.
    pub splits_used: [u64; 2],
    pub mark_rest_used: u64,
    pub notices_coalesced: u64,
    pub progress_notes_added: u64,
}

static FUNNEL: Funnel = Funnel {
    accounts: [const { AtomicU64::new(0) }; 2],
    yups_created: AtomicU64::new(0),
    invitations_shared: AtomicU64::new(0),
    invitations_claimed: AtomicU64::new(0),
    agreements_in_force: AtomicU64::new(0),
    contributions_confirmed: AtomicU64::new(0),
    contributions_disputed: AtomicU64::new(0),
    closed: [const { AtomicU64::new(0) }; 8],
    codes: [const { AtomicU64::new(0) }; 2],
    texts_sent: AtomicU64::new(0),
    emails: [const { AtomicU64::new(0) }; 2],
    entries_started: [const { AtomicU64::new(0) }; 10],
    entries_sent: [const { AtomicU64::new(0) }; 10],
    entries_in_force: [const { AtomicU64::new(0) }; 10],
    entries_completed: [const { AtomicU64::new(0) }; 10],
    instalment_yups: AtomicU64::new(0),
    instalment_items: AtomicU64::new(0),
    stage_yups: AtomicU64::new(0),
    stage_items: AtomicU64::new(0),
    splits_used: [const { AtomicU64::new(0) }; 2],
    mark_rest_used: AtomicU64::new(0),
    notices_coalesced: AtomicU64::new(0),
    progress_notes_added: AtomicU64::new(0),
    daily: Mutex::new(None),
};

/// The process's funnel.
pub fn funnel() -> &'static Funnel {
    &FUNNEL
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

fn read(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Relaxed)
}

impl Funnel {
    /// A first sign-in made an account.
    pub fn account_created(&self, channel: Channel) {
        bump(&self.accounts[channel as usize]);
    }

    /// The initiator passed the invitation on (`POST
    /// /v1/exchanges/{id}/invitation/shared`).
    pub fn invitation_shared(&self) {
        bump(&self.invitations_shared);
    }

    /// A one-time code went out.
    pub fn code_sent(&self, channel: Channel) {
        bump(&self.codes[channel as usize]);
    }

    /// An agreement update went out by text.
    pub fn text_sent(&self) {
        bump(&self.texts_sent);
    }

    /// An email went out.
    pub fn email_sent(&self, kind: EmailKind) {
        bump(&self.emails[kind as usize]);
    }

    /// An agreement came into force for the first time with this shape
    /// (`exchanges::service::run_command`). Counts only: how many payments
    /// one party owes the other, and how many stages one provides.
    pub fn shape_in_force(&self, revision: &Revision) {
        let shape = Shape::of(revision);
        if shape.instalments > 0 {
            bump(&self.instalment_yups);
            self.instalment_items
                .fetch_add(shape.instalments as u64, Ordering::Relaxed);
        }
        if shape.stages > 0 {
            bump(&self.stage_yups);
            self.stage_items
                .fetch_add(shape.stages as u64, Ordering::Relaxed);
        }
    }

    /// A proposal was sent after the composer's split sheets were completed
    /// this many times. A client's figure, so each is bounded.
    pub fn splits_used(&self, instalments: u8, stages: u8) {
        self.splits_used[Split::Instalments as usize]
            .fetch_add(u64::from(instalments.min(50)), Ordering::Relaxed);
        self.splits_used[Split::Stages as usize]
            .fetch_add(u64::from(stages.min(50)), Ordering::Relaxed);
    }

    /// "Mark the rest as paid" was used.
    pub fn mark_rest_used(&self) {
        bump(&self.mark_rest_used);
    }

    /// A message that told of several items at once went out by email.
    pub fn notice_coalesced(&self) {
        bump(&self.notices_coalesced);
    }

    /// A draft was made, from this entry (`exchanges::service::create`).
    pub fn yup_started(&self, entry: Entry) {
        bump(&self.entries_started[entry as usize]);
    }

    /// Whether any of these events is a step counted by entry, so that the
    /// store need only be asked what a yup was started from when one is.
    pub fn counts_by_entry(before: State, events: &[Event]) -> bool {
        events
            .iter()
            .any(|event| Self::step(before, event).is_some())
    }

    fn step(before: State, event: &Event) -> Option<Step> {
        match event {
            Event::RevisionSent { .. } if before == State::Draft => Some(Step::Sent),
            Event::AgreementInForce { .. } if before != State::Active => Some(Step::InForce),
            Event::Closed {
                outcome: Outcome::Completed,
                ..
            } => Some(Step::Completed),
            _ => None,
        }
    }

    /// The same steps as [`Funnel::events`] counts, by what the yup was
    /// started from: first sent, first in force, completed. `entry` is
    /// [`Entry::Unknown`] for a yup whose client said nothing.
    pub fn events_by_entry(&self, entry: Entry, before: State, events: &[Event]) {
        for event in events {
            let counter = match Self::step(before, event) {
                Some(Step::Sent) => &self.entries_sent,
                Some(Step::InForce) => &self.entries_in_force,
                Some(Step::Completed) => &self.entries_completed,
                None => continue,
            };
            bump(&counter[entry as usize]);
        }
    }

    /// What an exchange's events say happened, given the state the exchange
    /// was in before them (`exchanges::repo::persist`, where every decision
    /// is stored, whichever service made it). A first proposal sent is a yup
    /// created; an agreement comes into force once, the first time (an
    /// amendment in force is not another agreement); a contribution
    /// confirmed is one the recipient accepted.
    pub fn events(&self, before: State, events: &[Event]) {
        for event in events {
            match event {
                Event::RevisionSent { .. } if before == State::Draft => bump(&self.yups_created),
                Event::CounterpartyClaimed { .. } => bump(&self.invitations_claimed),
                Event::AgreementInForce { .. } if before != State::Active => {
                    bump(&self.agreements_in_force)
                }
                Event::ContributionChanged {
                    action: Action::Confirm,
                    status: Status::Accepted,
                    ..
                } => bump(&self.contributions_confirmed),
                Event::ContributionChanged {
                    action: Action::Dispute,
                    ..
                } => bump(&self.contributions_disputed),
                Event::Closed { outcome, .. } => bump(&self.closed[Closure::of(*outcome) as usize]),
                Event::ProgressNoted { .. } => bump(&self.progress_notes_added),
                _ => {}
            }
        }
    }

    /// Keeps the previous day's counts to serve as gauges.
    pub fn record_daily(&self, daily: Daily) {
        *self
            .daily
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(daily);
    }

    pub fn counts(&self) -> Counts {
        Counts {
            accounts_email: read(&self.accounts[Channel::Email as usize]),
            accounts_phone: read(&self.accounts[Channel::Phone as usize]),
            yups_created: read(&self.yups_created),
            invitations_shared: read(&self.invitations_shared),
            invitations_claimed: read(&self.invitations_claimed),
            agreements_in_force: read(&self.agreements_in_force),
            contributions_confirmed: read(&self.contributions_confirmed),
            contributions_disputed: read(&self.contributions_disputed),
            closed: std::array::from_fn(|index| read(&self.closed[index])),
            codes_email: read(&self.codes[Channel::Email as usize]),
            codes_phone: read(&self.codes[Channel::Phone as usize]),
            texts_sent: read(&self.texts_sent),
            emails: std::array::from_fn(|index| read(&self.emails[index])),
            entries_started: std::array::from_fn(|index| read(&self.entries_started[index])),
            entries_sent: std::array::from_fn(|index| read(&self.entries_sent[index])),
            entries_in_force: std::array::from_fn(|index| read(&self.entries_in_force[index])),
            entries_completed: std::array::from_fn(|index| read(&self.entries_completed[index])),
            instalment_yups: read(&self.instalment_yups),
            instalment_items: read(&self.instalment_items),
            stage_yups: read(&self.stage_yups),
            stage_items: read(&self.stage_items),
            splits_used: std::array::from_fn(|index| read(&self.splits_used[index])),
            mark_rest_used: read(&self.mark_rest_used),
            notices_coalesced: read(&self.notices_coalesced),
            progress_notes_added: read(&self.progress_notes_added),
        }
    }

    /// Every family, with every label value written even at zero, so a
    /// dashboard's rate over a step that has not happened yet reads 0
    /// rather than nothing.
    pub fn render(&self, text: &mut Text) {
        text.family(
            ACCOUNTS_CREATED,
            Kind::Counter,
            "Accounts made by a first sign-in since this process started, by channel: email or phone.",
        );
        for channel in [Channel::Email, Channel::Phone] {
            text.sample(
                ACCOUNTS_CREATED,
                &[("channel", channel.as_str())],
                read(&self.accounts[channel as usize]) as f64,
            );
        }
        text.single(
            YUPS_CREATED,
            Kind::Counter,
            "Yups created: first proposals sent, which open a negotiation.",
            read(&self.yups_created) as f64,
        );
        text.single(
            INVITATIONS_SHARED,
            Kind::Counter,
            "Invitations the initiator passed on (the share step).",
            read(&self.invitations_shared) as f64,
        );
        text.single(
            INVITATIONS_CLAIMED,
            Kind::Counter,
            "Invitations opened and claimed by the other party.",
            read(&self.invitations_claimed) as f64,
        );
        text.single(
            AGREEMENTS_IN_FORCE,
            Kind::Counter,
            "Agreements both parties signed, the first time each came into force.",
            read(&self.agreements_in_force) as f64,
        );
        text.single(
            CONTRIBUTIONS_CONFIRMED,
            Kind::Counter,
            "Contributions the recipient confirmed as delivered.",
            read(&self.contributions_confirmed) as f64,
        );
        text.single(
            CONTRIBUTIONS_DISPUTED,
            Kind::Counter,
            "Contributions disputed.",
            read(&self.contributions_disputed) as f64,
        );
        text.family(
            AGREEMENTS_CLOSED,
            Kind::Counter,
            "Exchanges closed, by outcome: withdrawn, declined, expired, discarded (nothing was agreed), completed, ended_by_agreement, unresolved_close_request, unresolved_inactive.",
        );
        for closure in Closure::ALL {
            text.sample(
                AGREEMENTS_CLOSED,
                &[("outcome", closure.as_str())],
                read(&self.closed[closure as usize]) as f64,
            );
        }
        text.family(
            CODES_SENT,
            Kind::Counter,
            "One-time codes handed to a provider, by channel: email or phone.",
        );
        for channel in [Channel::Email, Channel::Phone] {
            text.sample(
                CODES_SENT,
                &[("channel", channel.as_str())],
                read(&self.codes[channel as usize]) as f64,
            );
        }
        text.single(
            TEXTS_SENT,
            Kind::Counter,
            "Agreement updates sent by text message.",
            read(&self.texts_sent) as f64,
        );
        text.family(
            EMAILS_SENT,
            Kind::Counter,
            "Emails the outbox handed to the provider, by kind: notice (about an exchange, reminders included), account (about the account itself). Codes count under yuppers.codes.sent.",
        );
        for kind in EmailKind::ALL {
            text.sample(
                EMAILS_SENT,
                &[("kind", kind.as_str())],
                read(&self.emails[kind as usize]) as f64,
            );
        }
        for (name, help, counters) in [
            (
                ENTRIES_STARTED,
                "Drafts made, by entry: a template id, blank, copy, unknown (the client said nothing) or other. No version, no person.",
                &self.entries_started,
            ),
            (
                ENTRIES_SENT,
                "Yups whose first proposal was sent, by the entry they were started from.",
                &self.entries_sent,
            ),
            (
                ENTRIES_IN_FORCE,
                "Agreements that first came into force, by the entry they were started from.",
                &self.entries_in_force,
            ),
            (
                ENTRIES_COMPLETED,
                "Agreements completed, by the entry they were started from.",
                &self.entries_completed,
            ),
        ] {
            text.family(name, Kind::Counter, help);
            for entry in Entry::ALL {
                text.sample(
                    name,
                    &[("entry", entry.as_str())],
                    read(&counters[entry as usize]) as f64,
                );
            }
        }
        for (name, help, value) in [
            (
                INSTALMENT_YUPS,
                "Agreements that first came into force with two or more payments from one party (instalments).",
                read(&self.instalment_yups),
            ),
            (
                INSTALMENT_ITEMS,
                "Payments in those agreements, the most one party owes: divide by yuppers.instalments.yups for instalments per yup.",
                read(&self.instalment_items),
            ),
            (
                STAGE_YUPS,
                "Agreements that first came into force with two or more services or tasks from one party (stages).",
                read(&self.stage_yups),
            ),
            (
                STAGE_ITEMS,
                "Services and tasks in those agreements, the most one party provides.",
                read(&self.stage_items),
            ),
            (
                MARK_REST_USED,
                "\"Mark the rest as paid\" used: one command, one claim for each payment.",
                read(&self.mark_rest_used),
            ),
            (
                NOTICES_COALESCED,
                "Emails that told of several claims or confirmations at once.",
                read(&self.notices_coalesced),
            ),
            (
                PROGRESS_NOTES_ADDED,
                "Progress notes added to items under way.",
                read(&self.progress_notes_added),
            ),
        ] {
            text.single(name, Kind::Counter, help, value as f64);
        }
        text.family(
            SPLITS_USED,
            Kind::Counter,
            "Split sheets of the composer completed before a proposal was sent, by kind: instalments, stages.",
        );
        for split in Split::ALL {
            text.sample(
                SPLITS_USED,
                &[("kind", split.as_str())],
                read(&self.splits_used[split as usize]) as f64,
            );
        }

        let daily = self
            .daily
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(daily) = daily {
            let suffix =
                " on the previous UTC day, read from the database once a day by the worker.";
            for (name, help, value) in [
                (
                    DAILY_YUPS_CREATED,
                    "Yups created (first proposals sent)",
                    daily.yups_created,
                ),
                (
                    DAILY_INVITATIONS_SHARED,
                    "Invitations passed on",
                    daily.invitations_shared,
                ),
                (
                    DAILY_INVITATIONS_CLAIMED,
                    "Invitations claimed",
                    daily.invitations_claimed,
                ),
                (
                    DAILY_AGREEMENTS_IN_FORCE,
                    "Agreements that first came into force",
                    daily.agreements_in_force,
                ),
                (
                    DAILY_AGREEMENTS_COMPLETED,
                    "Agreements completed",
                    daily.agreements_completed,
                ),
            ] {
                text.single(name, Kind::Gauge, &format!("{help}{suffix}"), value as f64);
            }
        }
    }
}

/// The funnel steps of one UTC day, counted from the database: yups created
/// (an exchange's first `REVISION_SENT` event), invitations shared and
/// claimed, agreements that first came into force and agreements completed.
/// Five aggregate queries; nothing about anyone is read.
pub async fn daily(db: &PgPool, day: Date) -> Result<Daily, sqlx::Error> {
    let from = OffsetDateTime::new_utc(day, Time::MIDNIGHT);
    let to = from + time::Duration::DAY;

    let yups_created: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exchange_event e
         WHERE e.type = 'REVISION_SENT' AND e.occurred_at >= $1 AND e.occurred_at < $2
           AND NOT EXISTS (
               SELECT 1 FROM exchange_event earlier
               WHERE earlier.exchange_id = e.exchange_id
                 AND earlier.type = 'REVISION_SENT' AND earlier.sequence < e.sequence)",
    )
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;
    let invitations_shared: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invitation WHERE shared_at >= $1 AND shared_at < $2",
    )
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;
    let invitations_claimed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invitation WHERE claimed_at >= $1 AND claimed_at < $2",
    )
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;
    let agreements_in_force: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exchange_event e
         WHERE e.type = 'AGREEMENT_IN_FORCE' AND e.occurred_at >= $1 AND e.occurred_at < $2
           AND NOT EXISTS (
               SELECT 1 FROM exchange_event earlier
               WHERE earlier.exchange_id = e.exchange_id
                 AND earlier.type = 'AGREEMENT_IN_FORCE' AND earlier.sequence < e.sequence)",
    )
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;
    let agreements_completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM exchange
         WHERE closed_outcome = 'COMPLETED' AND closed_at >= $1 AND closed_at < $2",
    )
    .bind(from)
    .bind(to)
    .fetch_one(db)
    .await?;

    let format = time::macros::format_description!("[year]-[month]-[day]");
    Ok(Daily {
        day: day.format(&format).unwrap_or_default(),
        yups_created,
        invitations_shared,
        invitations_claimed,
        agreements_in_force,
        agreements_completed,
    })
}

/// Reads the previous UTC day's counts, logs them (`funnel snapshot`, one
/// INFO line with a field per step) and keeps them for the gauges. The
/// worker calls this once a day, and once at its start.
pub async fn snapshot(db: &PgPool, now: OffsetDateTime) -> Result<Daily, sqlx::Error> {
    let yesterday = now.date().previous_day().unwrap_or(now.date());
    let counts = daily(db, yesterday).await?;
    tracing::info!(
        day = counts.day,
        yups_created = counts.yups_created,
        invitations_shared = counts.invitations_shared,
        invitations_claimed = counts.invitations_claimed,
        agreements_in_force = counts.agreements_in_force,
        agreements_completed = counts.agreements_completed,
        "funnel snapshot"
    );
    funnel().record_daily(counts.clone());
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::amendment::Statuses;
    use crate::domain::revision::{ContributionId, RevisionId, Slot};
    use uuid::Uuid;

    #[test]
    fn the_steps_are_read_from_an_exchange_s_events() {
        let funnel = Funnel::default();
        let revision = RevisionId(Uuid::new_v4());
        let sent = Event::RevisionSent {
            revision,
            by: Slot::A,
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };
        // A first proposal is a yup; a counteroffer is not another.
        funnel.events(State::Draft, std::slice::from_ref(&sent));
        funnel.events(State::Negotiating, &[sent]);
        funnel.events(
            State::Negotiating,
            &[Event::CounterpartyClaimed { confirmed: false }],
        );
        let in_force = Event::AgreementInForce {
            revision,
            statuses: Statuses::default(),
        };
        // The agreement, then an amendment to it.
        funnel.events(State::Negotiating, std::slice::from_ref(&in_force));
        funnel.events(State::Active, &[in_force]);
        let contribution = ContributionId(Uuid::new_v4());
        funnel.events(
            State::Active,
            &[
                Event::ContributionChanged {
                    contribution,
                    action: Action::Claim,
                    by: Slot::A,
                    status: Status::Claimed,
                },
                Event::ContributionChanged {
                    contribution,
                    action: Action::Confirm,
                    by: Slot::B,
                    status: Status::Accepted,
                },
                Event::ContributionChanged {
                    contribution,
                    action: Action::Dispute,
                    by: Slot::B,
                    status: Status::Disputed,
                },
            ],
        );
        funnel.events(
            State::Active,
            &[Event::Closed {
                outcome: Outcome::Completed,
                waived: Vec::new(),
            }],
        );
        funnel.events(
            State::Negotiating,
            &[Event::Closed {
                outcome: Outcome::NotAgreed(NotAgreed::Declined),
                waived: Vec::new(),
            }],
        );
        funnel.account_created(Channel::Phone);
        funnel.code_sent(Channel::Email);
        funnel.code_sent(Channel::Email);
        funnel.text_sent();
        funnel.email_sent(EmailKind::Notice);

        let counts = funnel.counts();
        assert_eq!(
            counts,
            Counts {
                accounts_phone: 1,
                yups_created: 1,
                invitations_claimed: 1,
                agreements_in_force: 1,
                contributions_confirmed: 1,
                contributions_disputed: 1,
                closed: [0, 1, 0, 0, 1, 0, 0, 0],
                codes_email: 2,
                texts_sent: 1,
                emails: [1, 0],
                ..Counts::default()
            }
        );

        let mut text = Text::new();
        funnel.render(&mut text);
        let page = text.finish();
        for line in [
            r#"yuppers_accounts_created_total{channel="email"} 0"#,
            r#"yuppers_accounts_created_total{channel="phone"} 1"#,
            "yuppers_yups_created_total 1",
            "yuppers_invitations_shared_total 0",
            "yuppers_invitations_claimed_total 1",
            "yuppers_agreements_in_force_total 1",
            "yuppers_contributions_confirmed_total 1",
            "yuppers_contributions_disputed_total 1",
            r#"yuppers_agreements_closed_total{outcome="completed"} 1"#,
            r#"yuppers_agreements_closed_total{outcome="declined"} 1"#,
            r#"yuppers_agreements_closed_total{outcome="unresolved_inactive"} 0"#,
            r#"yuppers_codes_sent_total{channel="email"} 2"#,
            "yuppers_texts_sent_total 1",
            r#"yuppers_emails_sent_total{kind="notice"} 1"#,
            r#"yuppers_emails_sent_total{kind="account"} 0"#,
        ] {
            assert!(page.contains(&format!("{line}\n")), "{line} in\n{page}");
        }
        // No snapshot yet: no daily gauges.
        assert!(!page.contains("yuppers_daily_"), "{page}");

        funnel.record_daily(Daily {
            day: "2026-10-08".to_owned(),
            yups_created: 4,
            invitations_shared: 3,
            invitations_claimed: 2,
            agreements_in_force: 1,
            agreements_completed: 0,
        });
        let mut text = Text::new();
        funnel.render(&mut text);
        let page = text.finish();
        for line in [
            "yuppers_daily_yups_created 4",
            "yuppers_daily_invitations_shared 3",
            "yuppers_daily_invitations_claimed 2",
            "yuppers_daily_agreements_in_force 1",
            "yuppers_daily_agreements_completed 0",
        ] {
            assert!(page.contains(&format!("{line}\n")), "{line} in\n{page}");
        }
        assert!(page.contains("# TYPE yuppers_daily_yups_created gauge"));
    }

    #[test]
    fn what_a_yup_was_started_from_is_a_fixed_set_of_entries() {
        for good in [
            "blank",
            "copy",
            "job-deposit-balance@1",
            "selling-something@12",
            "some-future-one@3",
        ] {
            assert!(StartedFrom::parse(good).is_some(), "{good}");
        }
        for bad in [
            "",
            "Blank",
            "job-deposit-balance",
            "job-deposit-balance@",
            "job-deposit-balance@x",
            "job-deposit-balance@12345",
            "-x@1",
            "a b@1",
            "dana@example.com",
            "copy of 3f1c",
            &format!("{}@1", "a".repeat(41)),
        ] {
            assert!(StartedFrom::parse(bad).is_none(), "{bad}");
        }
        let entry = |text: &str| StartedFrom::parse(text).unwrap().entry();
        assert_eq!(entry("job-deposit-balance@1"), Entry::JobDepositBalance);
        assert_eq!(entry("blank"), Entry::Blank);
        assert_eq!(entry("copy"), Entry::Copy);
        // Well formed but not listed: counted, under a label of ours.
        assert_eq!(entry("something-new@1"), Entry::Other);
        assert_eq!(entry("unknown@1"), Entry::Other);
        for (index, entry) in Entry::ALL.iter().enumerate() {
            assert_eq!(*entry as usize, index);
        }
        let labels: std::collections::BTreeSet<&str> =
            Entry::ALL.iter().map(|entry| entry.as_str()).collect();
        assert_eq!(labels.len(), Entry::ALL.len());
    }

    #[test]
    fn the_steps_are_also_counted_by_entry() {
        let funnel = Funnel::default();
        funnel.yup_started(Entry::JobDepositBalance);
        funnel.yup_started(Entry::Blank);
        funnel.yup_started(Entry::Blank);
        let revision = RevisionId(Uuid::new_v4());
        let sent = Event::RevisionSent {
            revision,
            by: Slot::A,
            expires_at: OffsetDateTime::UNIX_EPOCH,
        };
        let in_force = Event::AgreementInForce {
            revision,
            statuses: Statuses::default(),
        };
        let completed = Event::Closed {
            outcome: Outcome::Completed,
            waived: Vec::new(),
        };
        assert!(Funnel::counts_by_entry(
            State::Draft,
            std::slice::from_ref(&sent)
        ));
        // A counteroffer, an amendment in force: not counted by entry.
        assert!(!Funnel::counts_by_entry(
            State::Negotiating,
            std::slice::from_ref(&sent)
        ));
        assert!(!Funnel::counts_by_entry(
            State::Active,
            std::slice::from_ref(&in_force)
        ));
        funnel.events_by_entry(Entry::Blank, State::Draft, &[sent]);
        funnel.events_by_entry(Entry::Blank, State::Negotiating, &[in_force]);
        funnel.events_by_entry(Entry::Blank, State::Active, &[completed]);

        let counts = funnel.counts();
        assert_eq!(counts.entries_started, [1, 0, 0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(counts.entries_sent, [0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(counts.entries_in_force, [0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(counts.entries_completed, [0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);

        let mut text = Text::new();
        funnel.render(&mut text);
        let page = text.finish();
        for line in [
            r#"yuppers_entries_started_total{entry="job-deposit-balance"} 1"#,
            r#"yuppers_entries_started_total{entry="blank"} 2"#,
            r#"yuppers_entries_started_total{entry="copy"} 0"#,
            r#"yuppers_entries_started_total{entry="other"} 0"#,
            r#"yuppers_entries_sent_total{entry="blank"} 1"#,
            r#"yuppers_entries_in_force_total{entry="blank"} 1"#,
            r#"yuppers_entries_completed_total{entry="blank"} 1"#,
        ] {
            assert!(page.contains(&format!("{line}\n")), "{line} in\n{page}");
        }
        // Labels are entries, never a version or anything a client typed.
        assert!(!page.contains('@'), "{page}");
    }

    #[test]
    fn every_closure_has_a_label_of_its_own() {
        let labels: std::collections::BTreeSet<&str> = Closure::ALL
            .iter()
            .map(|closure| closure.as_str())
            .collect();
        assert_eq!(labels.len(), Closure::ALL.len());
        for (index, closure) in Closure::ALL.iter().enumerate() {
            assert_eq!(*closure as usize, index);
        }
    }
}
