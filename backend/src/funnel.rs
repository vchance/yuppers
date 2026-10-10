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
pub const DAILY_YUPS_CREATED: Name = Name::new("yuppers.daily.yups_created", "{yup}");
pub const DAILY_INVITATIONS_SHARED: Name =
    Name::new("yuppers.daily.invitations_shared", "{invitation}");
pub const DAILY_INVITATIONS_CLAIMED: Name =
    Name::new("yuppers.daily.invitations_claimed", "{invitation}");
pub const DAILY_AGREEMENTS_IN_FORCE: Name =
    Name::new("yuppers.daily.agreements_in_force", "{agreement}");
pub const DAILY_AGREEMENTS_COMPLETED: Name =
    Name::new("yuppers.daily.agreements_completed", "{agreement}");

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
