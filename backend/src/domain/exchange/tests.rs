use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::*;
use crate::domain::contribution::Action;
use crate::domain::revision::tests::{contribution, fence_job, id, money};
use crate::domain::revision::{Due, Kind};

const START: OffsetDateTime = datetime!(2026-10-01 12:00 UTC);
const A: Actor = Actor::Party(Slot::A);
const B: Actor = Actor::Party(Slot::B);

fn day(n: i64) -> OffsetDateTime {
    START + Duration::days(n)
}

fn rev(n: u128) -> RevisionId {
    RevisionId(Uuid::from_u128(n))
}

fn send(n: u128, revision: Revision) -> Command {
    Command::Send {
        id: rev(n),
        revision,
    }
}

fn act(n: u128, action: Action) -> Command {
    Command::Contribution { id: id(n), action }
}

/// Runs commands against one exchange, keeping the events of the last one.
struct Scenario {
    exchange: Exchange,
    events: Vec<Event>,
    rules: Rules,
}

impl Scenario {
    fn draft() -> Self {
        Self {
            exchange: Exchange::draft(START),
            events: Vec::new(),
            rules: Rules::default(),
        }
    }

    /// A has sent the fence job as revision 1; nobody has claimed slot B.
    fn negotiating() -> Self {
        let mut scenario = Self::draft();
        scenario.ok(A, send(1, fence_job()), START);
        scenario
    }

    /// B was named in the invitation, claimed it and accepted revision 1.
    fn active() -> Self {
        let mut scenario = Self::negotiating();
        scenario.ok(B, Command::ClaimCounterparty { pre_bound: true }, START);
        scenario.ok(B, Command::Accept { revision: rev(1) }, START);
        scenario
    }

    fn ok(&mut self, actor: Actor, command: Command, now: OffsetDateTime) -> &mut Self {
        let decision = decide(&self.exchange, actor, command.clone(), now, &self.rules)
            .unwrap_or_else(|refusal| panic!("{command:?} by {actor:?} was refused: {refusal}"));
        self.exchange = decision.exchange;
        self.events = decision.events;
        self
    }

    fn refused(&self, actor: Actor, command: Command, now: OffsetDateTime) -> Refusal {
        match decide(&self.exchange, actor, command.clone(), now, &self.rules) {
            Err(refusal) => refusal,
            Ok(_) => panic!("{command:?} by {actor:?} should have been refused"),
        }
    }

    fn status(&self, n: u128) -> Status {
        self.exchange.statuses[&id(n)]
    }

    fn closed(&self) -> Option<Outcome> {
        match self.exchange.state {
            State::Closed(outcome) => Some(outcome),
            _ => None,
        }
    }
}

// ---- Reaching agreement ---------------------------------------------------

#[test]
fn sending_the_first_revision_opens_the_negotiation() {
    let scenario = Scenario::negotiating();

    assert_eq!(scenario.exchange.state, State::Negotiating);
    let open = scenario.exchange.open.as_ref().unwrap();
    assert_eq!(
        (open.id, open.author, open.expires_at),
        (rev(1), Slot::A, day(14))
    );
    assert_eq!(
        scenario.events,
        vec![Event::RevisionSent {
            revision: rev(1),
            by: Slot::A,
            expires_at: day(14)
        }]
    );
}

#[test]
fn a_draft_never_sent_can_be_discarded_by_its_initiator() {
    let mut scenario = Scenario::draft();
    assert_eq!(
        scenario.refused(B, Command::Discard, START),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(Actor::System, Command::Discard, START),
        Refusal::WrongActor
    );

    scenario.ok(A, Command::Discard, START);
    assert_eq!(
        scenario.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Discarded))
    );
    assert_eq!(
        scenario.events,
        [Event::Closed {
            outcome: Outcome::NotAgreed(NotAgreed::Discarded),
            waived: Vec::new(),
        }]
    );
    assert_eq!(scenario.exchange.open, None);
    assert_eq!(scenario.exchange.in_force, None);
    // Closed is closed: the draft cannot be revived by sending.
    assert_eq!(
        scenario.refused(A, send(1, fence_job()), START),
        Refusal::NotAllowed
    );
}

#[test]
fn once_something_has_been_sent_there_is_no_discarding() {
    let scenario = Scenario::negotiating();
    assert_eq!(
        scenario.refused(A, Command::Discard, START),
        Refusal::NotAllowed
    );
    let scenario = Scenario::active();
    assert_eq!(
        scenario.refused(A, Command::Discard, START),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(B, Command::Discard, START),
        Refusal::NotAllowed
    );
}

#[test]
fn only_the_initiator_can_send_from_a_draft() {
    let scenario = Scenario::draft();
    assert_eq!(
        scenario.refused(B, send(1, fence_job()), START),
        Refusal::WrongActor
    );
}

#[test]
fn an_invalid_revision_is_not_sent() {
    let scenario = Scenario::draft();
    let mut revision = fence_job();
    revision.contributions.clear();

    assert_eq!(
        scenario.refused(A, send(1, revision), START),
        Refusal::InvalidRevision(vec![Invalid::NoRequiredContribution])
    );
}

#[test]
fn a_prebound_counterparty_accepting_makes_the_agreement_binding() {
    let scenario = Scenario::active();

    assert_eq!(scenario.exchange.state, State::Active);
    assert_eq!(scenario.exchange.open, None);
    assert_eq!(scenario.exchange.in_force.as_ref().unwrap().id, rev(1));
    assert_eq!(
        (scenario.status(1), scenario.status(2)),
        (Status::Pending, Status::Pending)
    );
    assert_eq!(
        scenario.events,
        vec![
            Event::RevisionAccepted {
                revision: rev(1),
                by: Slot::B
            },
            Event::AgreementInForce {
                revision: rev(1),
                statuses: scenario.exchange.statuses.clone(),
            },
        ]
    );
}

#[test]
fn nobody_can_act_for_slot_b_before_it_is_claimed() {
    let scenario = Scenario::negotiating();
    for command in [
        Command::Accept { revision: rev(1) },
        Command::Decline { revision: rev(1) },
        send(2, fence_job()),
    ] {
        assert_eq!(scenario.refused(B, command, START), Refusal::NotAllowed);
    }
}

#[test]
fn an_unconfirmed_acceptance_waits_for_the_initiator() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, day(1));
    scenario.ok(B, Command::Accept { revision: rev(1) }, day(1));

    // Signed by both, but the initiator has not said who B is.
    assert_eq!(scenario.exchange.state, State::Negotiating);
    assert!(scenario.exchange.open.as_ref().unwrap().accepted);
    assert_eq!(
        scenario.events,
        vec![Event::RevisionAccepted {
            revision: rev(1),
            by: Slot::B
        }]
    );

    scenario.ok(A, Command::ConfirmCounterparty, day(2));
    assert_eq!(scenario.exchange.state, State::Active);
    assert!(matches!(
        scenario.events[..],
        [Event::CounterpartyConfirmed, Event::AgreementInForce { .. }]
    ));
}

#[test]
fn confirming_after_the_offer_ran_out_does_not_revive_it() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, day(1));
    scenario.ok(B, Command::Accept { revision: rev(1) }, day(1));

    scenario.ok(A, Command::ConfirmCounterparty, day(14));
    assert_eq!(scenario.exchange.state, State::Negotiating);

    scenario.ok(Actor::System, Command::ExpireRevision, day(14));
    assert_eq!(
        scenario.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Expired))
    );
}

#[test]
fn a_counteroffer_has_to_wait_until_the_initiator_has_confirmed_the_claimant() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, day(1));
    assert_eq!(
        scenario.refused(B, send(2, fence_job()), day(1)),
        Refusal::AwaitingConfirmation
    );

    scenario.ok(A, Command::ConfirmCounterparty, day(2));
    scenario.ok(B, send(2, fence_job()), day(2));
    scenario.ok(A, Command::Accept { revision: rev(2) }, day(2));
    assert_eq!(scenario.exchange.state, State::Active);
}

// ---- A claimant the initiator has not confirmed ---------------------------

/// Someone has claimed an invitation that named nobody.
fn claimed() -> Scenario {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, day(1));
    scenario
}

/// An exchange from before unconfirmed claimants were stopped from
/// countering: the claimant's own offer is the one on the table.
fn claimed_with_the_claimants_offer_open() -> Scenario {
    let mut scenario = claimed();
    let open = scenario.exchange.open.as_mut().unwrap();
    open.id = rev(2);
    open.author = Slot::B;
    scenario
}

#[test]
fn an_unconfirmed_claimant_can_sign_or_leave_and_nothing_else() {
    let scenario = claimed();
    let refusals = [
        (send(2, fence_job()), Refusal::AwaitingConfirmation),
        (
            Command::Decline { revision: rev(1) },
            Refusal::AwaitingConfirmation,
        ),
        (act(1, Action::Confirm), Refusal::AwaitingConfirmation),
        (Command::ProposeEnd, Refusal::AwaitingConfirmation),
        (Command::AcceptEnd, Refusal::AwaitingConfirmation),
        (Command::CancelEnd, Refusal::AwaitingConfirmation),
        (Command::RequestClose, Refusal::AwaitingConfirmation),
        (Command::RetractClose, Refusal::AwaitingConfirmation),
        (Command::AddStatement, Refusal::AwaitingConfirmation),
        // Not theirs to do, confirmed or not.
        (Command::Withdraw { revision: rev(1) }, Refusal::WrongActor),
        (Command::ConfirmCounterparty, Refusal::WrongActor),
        (Command::RejectCounterparty, Refusal::WrongActor),
        (
            Command::ClaimCounterparty { pre_bound: false },
            Refusal::NotAllowed,
        ),
        (Command::ExpireRevision, Refusal::WrongActor),
        (Command::LapseCloseRequest, Refusal::WrongActor),
        (Command::PromptInactivity, Refusal::WrongActor),
        (Command::CloseInactive, Refusal::WrongActor),
    ];
    for (command, refusal) in refusals {
        assert_eq!(
            scenario.refused(B, command.clone(), day(1)),
            refusal,
            "{command:?}"
        );
    }

    for command in [Command::Accept { revision: rev(1) }, Command::ReleaseClaim] {
        let mut scenario = claimed();
        scenario.ok(B, command, day(1));
        assert_eq!(scenario.exchange.state, State::Negotiating);
    }
}

#[test]
fn the_limits_end_when_the_initiator_confirms_and_never_apply_to_someone_the_invitation_named() {
    let mut confirmed = claimed();
    confirmed.ok(A, Command::ConfirmCounterparty, day(2));
    let mut named = Scenario::negotiating();
    named.ok(B, Command::ClaimCounterparty { pre_bound: true }, day(1));

    for scenario in [confirmed, named] {
        for command in [Command::Decline { revision: rev(1) }, send(2, fence_job())] {
            let decision = decide(
                &scenario.exchange,
                B,
                command.clone(),
                day(2),
                &scenario.rules,
            );
            assert!(decision.is_ok(), "{command:?}");
        }
    }
}

#[test]
fn the_initiator_can_say_the_claimant_is_not_who_they_invited() {
    let mut scenario = claimed();
    let offer = scenario.exchange.open.clone();

    scenario.ok(A, Command::RejectCounterparty, day(2));
    assert_eq!(
        scenario.events,
        vec![Event::CounterpartyRejected { voided: None }]
    );
    assert_eq!(scenario.exchange.counterparty, Counterparty::Unclaimed);
    assert_eq!(scenario.exchange.state, State::Negotiating);
    assert_eq!(
        scenario.exchange.open, offer,
        "the initiator's offer stands, signed by them and nobody else"
    );

    // The slot is empty again: nobody can act for it, and there is nobody
    // to confirm or to remove a second time.
    assert_eq!(
        scenario.refused(B, Command::Accept { revision: rev(1) }, day(2)),
        Refusal::NotAllowed
    );
    for command in [Command::ConfirmCounterparty, Command::RejectCounterparty] {
        assert_eq!(scenario.refused(A, command, day(2)), Refusal::NotAllowed);
    }

    // Someone else claims it, signs, and is confirmed.
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, day(3));
    scenario.ok(B, Command::Accept { revision: rev(1) }, day(3));
    scenario.ok(A, Command::ConfirmCounterparty, day(4));
    assert_eq!(scenario.exchange.state, State::Active);
}

#[test]
fn a_removed_claimants_signature_never_takes_effect() {
    for removal in [(A, Command::RejectCounterparty), (B, Command::ReleaseClaim)] {
        let (by, command) = removal.clone();
        let mut scenario = claimed();
        scenario.ok(B, Command::Accept { revision: rev(1) }, day(1));
        assert!(scenario.exchange.open.as_ref().unwrap().accepted);

        scenario.ok(by, command.clone(), day(2));
        let voided = Some(rev(1));
        assert_eq!(
            scenario.events,
            vec![if by == A {
                Event::CounterpartyRejected { voided }
            } else {
                Event::CounterpartyReleased { voided }
            }],
            "the event says which signature went with them"
        );
        assert!(!scenario.exchange.open.as_ref().unwrap().accepted);

        // Whoever comes next, however they come, the old signature is not
        // theirs: confirming them, or their having been named, binds nobody
        // until they sign for themselves.
        for pre_bound in [false, true] {
            let mut next = Scenario {
                exchange: scenario.exchange.clone(),
                events: Vec::new(),
                rules: Rules::default(),
            };
            next.ok(B, Command::ClaimCounterparty { pre_bound }, day(3));
            if !pre_bound {
                next.ok(A, Command::ConfirmCounterparty, day(3));
                assert_eq!(next.events, vec![Event::CounterpartyConfirmed]);
            }
            assert_eq!(next.exchange.state, State::Negotiating, "{command:?}");
            assert_eq!(next.exchange.in_force, None);

            next.ok(B, Command::Accept { revision: rev(1) }, day(3));
            assert_eq!(next.exchange.state, State::Active);
        }
    }
}

#[test]
fn a_claimant_can_leave_before_being_confirmed() {
    let mut scenario = claimed();
    let offer = scenario.exchange.open.clone();

    scenario.ok(B, Command::ReleaseClaim, day(2));
    assert_eq!(
        scenario.events,
        vec![Event::CounterpartyReleased { voided: None }]
    );
    assert_eq!(scenario.exchange.counterparty, Counterparty::Unclaimed);
    assert_eq!(scenario.exchange.state, State::Negotiating);
    assert_eq!(scenario.exchange.open, offer);
    assert_eq!(
        scenario.refused(B, Command::ReleaseClaim, day(2)),
        Refusal::NotAllowed,
        "nobody is there to leave"
    );
}

#[test]
fn only_an_unconfirmed_claimant_can_be_removed_or_leave() {
    // Nobody has claimed the slot.
    let unclaimed = Scenario::negotiating();
    // The initiator confirmed the claimant.
    let mut confirmed = claimed();
    confirmed.ok(A, Command::ConfirmCounterparty, day(2));
    // The invitation named them.
    let mut named = Scenario::negotiating();
    named.ok(B, Command::ClaimCounterparty { pre_bound: true }, day(1));
    // The agreement is in force.
    let active = Scenario::active();
    // The exchange closed with the claimant still unconfirmed.
    let mut closed = claimed();
    closed.ok(A, Command::Withdraw { revision: rev(1) }, day(2));

    for scenario in [unclaimed, confirmed, named, active, closed] {
        assert_eq!(
            scenario.refused(A, Command::RejectCounterparty, day(3)),
            Refusal::NotAllowed
        );
        assert_eq!(
            scenario.refused(B, Command::ReleaseClaim, day(3)),
            Refusal::NotAllowed
        );
    }

    // Removing is the initiator's to do and leaving is the claimant's.
    let scenario = claimed();
    assert_eq!(
        scenario.refused(B, Command::RejectCounterparty, day(2)),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(A, Command::ReleaseClaim, day(2)),
        Refusal::WrongActor
    );
    for command in [Command::RejectCounterparty, Command::ReleaseClaim] {
        assert_eq!(
            scenario.refused(Actor::System, command, day(2)),
            Refusal::WrongActor
        );
    }
}

#[test]
fn removing_a_claimant_leaves_the_offer_to_run_out_on_its_own_date() {
    let mut scenario = claimed();
    scenario.ok(B, Command::Accept { revision: rev(1) }, day(1));
    scenario.ok(A, Command::RejectCounterparty, day(2));

    assert_eq!(scenario.exchange.open.as_ref().unwrap().expires_at, day(14));
    scenario.ok(Actor::System, Command::ExpireRevision, day(14));
    assert_eq!(
        scenario.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Expired))
    );
}

#[test]
fn a_claimants_offer_from_before_the_rule_has_to_go_before_they_can() {
    let scenario = claimed_with_the_claimants_offer_open();

    // Taking the claimant out would leave a negotiation with nothing on the
    // table.
    assert_eq!(
        scenario.refused(A, Command::RejectCounterparty, day(2)),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(B, Command::ReleaseClaim, day(2)),
        Refusal::NotAllowed
    );
    // The initiator is still never bound to someone they have not confirmed.
    assert_eq!(
        scenario.refused(A, Command::Accept { revision: rev(2) }, day(2)),
        Refusal::CounterpartyNotConfirmed
    );

    // The claimant can take their offer back, which ends it as it always did.
    let mut withdrawn = claimed_with_the_claimants_offer_open();
    withdrawn.ok(B, Command::Withdraw { revision: rev(2) }, day(2));
    assert_eq!(
        withdrawn.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Withdrawn))
    );

    // Or the initiator answers it with their own, and can then remove them.
    let mut answered = claimed_with_the_claimants_offer_open();
    answered.ok(A, send(3, fence_job()), day(2));
    answered.ok(A, Command::RejectCounterparty, day(2));
    assert_eq!(answered.exchange.counterparty, Counterparty::Unclaimed);
    assert_eq!(answered.exchange.open.as_ref().unwrap().id, rev(3));
}

#[test]
fn only_the_initiator_confirms_and_only_the_invited_party_claims() {
    let mut scenario = Scenario::negotiating();
    assert_eq!(
        scenario.refused(A, Command::ClaimCounterparty { pre_bound: true }, START),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(A, Command::ConfirmCounterparty, START),
        Refusal::NotAllowed
    );

    scenario.ok(B, Command::ClaimCounterparty { pre_bound: false }, START);
    assert_eq!(
        scenario.refused(B, Command::ConfirmCounterparty, START),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(B, Command::ClaimCounterparty { pre_bound: false }, START),
        Refusal::NotAllowed
    );
}

// ---- Counteroffers, withdrawal, expiry ------------------------------------

#[test]
fn a_counteroffer_supersedes_the_open_revision() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: true }, day(1));
    scenario.ok(B, send(2, fence_job()), day(1));

    assert_eq!(
        scenario.events,
        vec![
            Event::RevisionSuperseded { revision: rev(1) },
            Event::RevisionSent {
                revision: rev(2),
                by: Slot::B,
                expires_at: day(15)
            },
        ]
    );

    // Accepting what was on screen a moment ago is refused, not applied to
    // the new terms.
    assert_eq!(
        scenario.refused(A, Command::Accept { revision: rev(1) }, day(1)),
        Refusal::StaleRevision
    );
    scenario.ok(A, Command::Accept { revision: rev(2) }, day(1));
    assert_eq!(scenario.exchange.in_force.as_ref().unwrap().id, rev(2));
}

#[test]
fn the_author_has_already_signed_and_cannot_accept_or_decline() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: true }, START);

    assert_eq!(
        scenario.refused(A, Command::Accept { revision: rev(1) }, START),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(A, Command::Decline { revision: rev(1) }, START),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(B, Command::Withdraw { revision: rev(1) }, START),
        Refusal::WrongActor
    );
}

#[test]
fn withdrawing_or_declining_during_negotiation_ends_it_without_agreement() {
    let mut withdrawn = Scenario::negotiating();
    withdrawn.ok(A, Command::Withdraw { revision: rev(1) }, day(1));
    assert_eq!(
        withdrawn.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Withdrawn))
    );
    assert_eq!(
        withdrawn.events,
        vec![
            Event::RevisionWithdrawn {
                revision: rev(1),
                by: Slot::A
            },
            Event::Closed {
                outcome: Outcome::NotAgreed(NotAgreed::Withdrawn),
                waived: vec![]
            },
        ]
    );

    let mut declined = Scenario::negotiating();
    declined.ok(B, Command::ClaimCounterparty { pre_bound: true }, day(1));
    declined.ok(B, Command::Decline { revision: rev(1) }, day(1));
    assert_eq!(
        declined.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Declined))
    );
}

#[test]
fn an_offer_cannot_be_accepted_once_it_has_expired() {
    let mut scenario = Scenario::negotiating();
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: true }, day(1));

    assert_eq!(
        scenario.refused(B, Command::Accept { revision: rev(1) }, day(14)),
        Refusal::RevisionExpired
    );
    // One second earlier it still could.
    scenario.ok(
        B,
        Command::Accept { revision: rev(1) },
        day(14) - Duration::seconds(1),
    );
    assert_eq!(scenario.exchange.state, State::Active);
}

#[test]
fn expiry_is_the_systems_to_run_and_only_when_due() {
    let mut scenario = Scenario::negotiating();

    assert_eq!(
        scenario.refused(A, Command::ExpireRevision, day(14)),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(Actor::System, Command::ExpireRevision, day(13)),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(Actor::System, Command::Accept { revision: rev(1) }, START),
        Refusal::WrongActor
    );

    scenario.ok(Actor::System, Command::ExpireRevision, day(14));
    assert_eq!(
        scenario.closed(),
        Some(Outcome::NotAgreed(NotAgreed::Expired))
    );
}

// ---- Fulfillment ----------------------------------------------------------

#[test]
fn claims_and_confirmations_complete_the_exchange() {
    let mut scenario = Scenario::active();

    scenario.ok(A, act(1, Action::Claim), day(20));
    assert_eq!(scenario.status(1), Status::Claimed);
    assert_eq!(
        scenario.events,
        vec![Event::ContributionChanged {
            contribution: id(1),
            action: Action::Claim,
            by: Slot::A,
            status: Status::Claimed,
        }]
    );

    scenario.ok(B, act(1, Action::Confirm), day(21));
    scenario.ok(B, act(2, Action::Claim), day(21));
    assert_eq!(scenario.exchange.state, State::Active);

    scenario.ok(A, act(2, Action::Confirm), day(22));
    assert_eq!(scenario.closed(), Some(Outcome::Completed));
    assert!(matches!(
        scenario.events[..],
        [
            Event::ContributionChanged {
                status: Status::Accepted,
                ..
            },
            Event::Closed {
                outcome: Outcome::Completed,
                ..
            }
        ]
    ));
}

#[test]
fn roles_follow_each_contribution_not_the_slot() {
    let scenario = Scenario::active();

    // A provides the repair and receives the payment.
    assert_eq!(
        scenario.refused(B, act(1, Action::Claim), day(1)),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(A, act(2, Action::Claim), day(1)),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(A, act(1, Action::Confirm), day(1)),
        Refusal::WrongActor
    );
}

#[test]
fn a_dispute_can_be_remedied_or_waived() {
    let mut scenario = Scenario::active();
    scenario.ok(A, act(1, Action::Claim), day(20));
    scenario.ok(B, act(1, Action::Dispute), day(21));
    assert_eq!(scenario.status(1), Status::Disputed);
    assert_eq!(scenario.exchange.state, State::Active);

    scenario.ok(A, act(1, Action::Claim), day(22));
    scenario.ok(B, act(1, Action::Confirm), day(23));
    scenario.ok(A, act(2, Action::Waive), day(23));
    assert_eq!(scenario.closed(), Some(Outcome::Completed));
}

#[test]
fn optional_contributions_do_not_hold_up_completion() {
    let mut revision = fence_job();
    let mut extra = contribution(3, Slot::A, Kind::Task, Due::OnAgreement);
    extra.required = false;
    revision.contributions.push(extra);

    let mut scenario = Scenario::draft();
    scenario.ok(A, send(1, revision), START);
    scenario.ok(B, Command::ClaimCounterparty { pre_bound: true }, START);
    scenario.ok(B, Command::Accept { revision: rev(1) }, START);

    scenario.ok(B, act(1, Action::Confirm), day(1));
    scenario.ok(A, act(2, Action::Confirm), day(1));
    assert_eq!(scenario.closed(), Some(Outcome::Completed));
    assert_eq!(scenario.status(3), Status::Pending);
}

#[test]
fn fulfillment_needs_an_agreement_in_force_and_a_known_contribution() {
    let negotiating = Scenario::negotiating();
    assert_eq!(
        negotiating.refused(A, act(1, Action::Claim), START),
        Refusal::NotAllowed
    );

    let active = Scenario::active();
    assert_eq!(
        active.refused(A, act(9, Action::Claim), START),
        Refusal::UnknownContribution(id(9))
    );
}

// ---- Amendments -----------------------------------------------------------

/// The fence job with a higher price and a clean-up task added.
fn amended() -> Revision {
    let mut revision = fence_job();
    revision.contributions[1].kind = money(60_000);
    revision
        .contributions
        .push(contribution(3, Slot::A, Kind::Task, Due::After(id(1))));
    revision
}

#[test]
fn a_proposed_amendment_changes_nothing_until_accepted() {
    let mut scenario = Scenario::active();
    scenario.ok(A, act(1, Action::Claim), day(5));
    scenario.ok(A, send(2, amended()), day(6));

    assert_eq!(scenario.exchange.state, State::Active);
    assert_eq!(scenario.exchange.in_force.as_ref().unwrap().id, rev(1));
    assert_eq!(scenario.exchange.open.as_ref().unwrap().id, rev(2));
    assert_eq!(scenario.status(1), Status::Claimed);

    // Fulfillment continues under the agreement in force.
    scenario.ok(B, act(1, Action::Dispute), day(7));
    assert_eq!(scenario.status(1), Status::Disputed);
}

#[test]
fn an_accepted_amendment_replaces_the_agreement_and_resets_what_changed() {
    let mut scenario = Scenario::active();
    scenario.ok(A, act(1, Action::Claim), day(5));
    scenario.ok(B, act(2, Action::Claim), day(5));
    scenario.ok(A, send(2, amended()), day(6));
    scenario.ok(B, Command::Accept { revision: rev(2) }, day(7));

    assert_eq!(scenario.exchange.in_force.as_ref().unwrap().id, rev(2));
    assert_eq!(scenario.exchange.open, None);
    assert_eq!(scenario.status(1), Status::Claimed, "untouched");
    assert_eq!(scenario.status(2), Status::Pending, "price changed");
    assert_eq!(scenario.status(3), Status::Pending, "new");
}

#[test]
fn an_amendment_can_remove_a_contribution() {
    let mut scenario = Scenario::active();
    let mut revision = fence_job();
    revision.contributions.truncate(1);

    scenario.ok(B, send(2, revision), day(1));
    scenario.ok(A, Command::Accept { revision: rev(2) }, day(1));
    assert_eq!(scenario.status(2), Status::Removed);

    assert_eq!(
        scenario.refused(B, act(2, Action::Claim), day(2)),
        Refusal::UnknownContribution(id(2))
    );
    // Its ID is retired.
    assert_eq!(
        scenario.refused(A, send(3, fence_job()), day(2)),
        Refusal::InvalidRevision(vec![Invalid::ReusedContribution(id(2))])
    );
}

#[test]
fn an_accepted_contribution_is_locked_against_amendment() {
    let mut scenario = Scenario::active();
    scenario.ok(B, act(1, Action::Confirm), day(5));

    let mut revision = fence_job();
    revision.contributions[0].description = "Repair and paint".into();
    assert_eq!(
        scenario.refused(A, send(2, revision), day(6)),
        Refusal::ContributionLocked(id(1))
    );
}

#[test]
fn the_lock_is_checked_again_when_the_amendment_is_accepted() {
    let mut scenario = Scenario::active();
    let mut revision = fence_job();
    revision.contributions[0].description = "Repair and paint".into();
    scenario.ok(A, send(2, revision), day(5));

    // The repair is accepted under the old terms while the amendment waits.
    scenario.ok(B, act(1, Action::Confirm), day(6));
    assert_eq!(
        scenario.refused(B, Command::Accept { revision: rev(2) }, day(7)),
        Refusal::ContributionLocked(id(1))
    );
    assert_eq!(scenario.exchange.in_force.as_ref().unwrap().id, rev(1));
}

#[test]
fn an_amendment_that_goes_nowhere_leaves_the_agreement_as_it_was() {
    let outcomes: [(Actor, Command, OffsetDateTime); 3] = [
        (A, Command::Withdraw { revision: rev(2) }, day(2)),
        (B, Command::Decline { revision: rev(2) }, day(2)),
        (Actor::System, Command::ExpireRevision, day(15)),
    ];
    for (actor, command, now) in outcomes {
        let mut scenario = Scenario::active();
        scenario.ok(A, send(2, amended()), day(1));
        scenario.ok(actor, command.clone(), now);

        assert_eq!(scenario.exchange.state, State::Active, "{command:?}");
        assert_eq!(scenario.exchange.open, None, "{command:?}");
        assert_eq!(
            scenario.exchange.in_force.as_ref().unwrap().id,
            rev(1),
            "{command:?}"
        );
    }
}

#[test]
fn an_amendment_that_settles_everything_completes_the_exchange() {
    let mut scenario = Scenario::active();
    scenario.ok(B, act(1, Action::Confirm), day(5));

    // The parties drop the payment; only the accepted repair remains.
    let mut revision = fence_job();
    revision.contributions.truncate(1);
    scenario.ok(A, send(2, revision), day(6));
    scenario.ok(B, Command::Accept { revision: rev(2) }, day(6));

    assert_eq!(scenario.closed(), Some(Outcome::Completed));
}

#[test]
fn completing_the_exchange_voids_a_pending_amendment() {
    let mut scenario = Scenario::active();
    scenario.ok(B, act(1, Action::Confirm), day(5));
    let mut revision = fence_job();
    revision.contributions[1].kind = money(60_000);
    scenario.ok(A, send(2, revision), day(5));

    scenario.ok(A, act(2, Action::Confirm), day(6));
    assert_eq!(scenario.closed(), Some(Outcome::Completed));
    assert_eq!(scenario.exchange.open, None);
    assert!(
        scenario
            .events
            .contains(&Event::RevisionSuperseded { revision: rev(2) })
    );
}

// ---- Ending by agreement --------------------------------------------------

#[test]
fn ending_by_agreement_releases_what_is_outstanding() {
    let mut scenario = Scenario::active();
    scenario.ok(B, act(1, Action::Confirm), day(5));
    scenario.ok(B, act(2, Action::Claim), day(5));

    scenario.ok(A, Command::ProposeEnd, day(6));
    assert_eq!(
        scenario.refused(A, Command::AcceptEnd, day(6)),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(B, Command::ProposeEnd, day(6)),
        Refusal::NotAllowed
    );

    scenario.ok(B, Command::AcceptEnd, day(7));
    assert_eq!(scenario.closed(), Some(Outcome::EndedByAgreement));
    assert_eq!(
        scenario.status(1),
        Status::Accepted,
        "what was accepted stays accepted"
    );
    assert_eq!(scenario.status(2), Status::Waived);
    assert_eq!(
        scenario.events,
        vec![Event::Closed {
            outcome: Outcome::EndedByAgreement,
            waived: vec![id(2)]
        }]
    );
}

#[test]
fn an_end_proposal_can_be_cancelled_by_either_party() {
    for canceller in [A, B] {
        let mut scenario = Scenario::active();
        scenario.ok(A, Command::ProposeEnd, day(1));
        scenario.ok(canceller, Command::CancelEnd, day(2));

        assert_eq!(scenario.exchange.end_proposed_by, None);
        assert_eq!(
            scenario.refused(B, Command::AcceptEnd, day(3)),
            Refusal::NotAllowed
        );
    }
    assert_eq!(
        Scenario::active().refused(A, Command::CancelEnd, day(1)),
        Refusal::NotAllowed
    );
}

// ---- Closing without agreement --------------------------------------------

#[test]
fn a_close_request_closes_unresolved_when_the_window_lapses() {
    let mut scenario = Scenario::active();
    scenario.ok(A, act(1, Action::Claim), day(20));
    scenario.ok(A, Command::RequestClose, day(40));
    scenario.ok(B, Command::AddStatement, day(41));
    scenario.ok(A, Command::AddStatement, day(42));

    assert_eq!(
        scenario.refused(Actor::System, Command::LapseCloseRequest, day(46)),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(A, Command::LapseCloseRequest, day(47)),
        Refusal::WrongActor
    );

    scenario.ok(Actor::System, Command::LapseCloseRequest, day(47));
    assert_eq!(
        scenario.closed(),
        Some(Outcome::Unresolved(Unresolved::CloseRequest))
    );
    // The record shows "claimed, not confirmed".
    assert_eq!(scenario.status(1), Status::Claimed);
    assert_eq!(scenario.status(2), Status::Pending);
}

#[test]
fn only_the_requester_can_retract_a_close_request() {
    let mut scenario = Scenario::active();
    assert_eq!(
        scenario.refused(A, Command::RetractClose, day(1)),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(A, Command::AddStatement, day(1)),
        Refusal::NotAllowed
    );

    scenario.ok(A, Command::RequestClose, day(1));
    assert_eq!(
        scenario.refused(B, Command::RetractClose, day(2)),
        Refusal::WrongActor
    );
    assert_eq!(
        scenario.refused(B, Command::RequestClose, day(2)),
        Refusal::NotAllowed
    );

    scenario.ok(A, Command::RetractClose, day(2));
    assert_eq!(scenario.exchange.close_request, None);
    assert_eq!(
        scenario.refused(Actor::System, Command::LapseCloseRequest, day(30)),
        Refusal::NotAllowed
    );
}

#[test]
fn a_close_request_is_not_an_offer_to_release_everything() {
    let mut scenario = Scenario::active();
    scenario.ok(A, act(1, Action::Claim), day(1));
    scenario.ok(A, Command::RequestClose, day(2));

    // Ana wants the record kept as it stands: her work claimed, Ben's payment
    // owed. Ben cannot turn that into a mutual release by "agreeing".
    for party in [A, B] {
        assert_eq!(
            scenario.refused(party, Command::AcceptEnd, day(3)),
            Refusal::NotAllowed
        );
    }
    assert_eq!(scenario.status(2), Status::Pending);

    // Ending by agreement still takes a proposal from one and consent from
    // the other.
    scenario.ok(B, Command::ProposeEnd, day(3));
    scenario.ok(A, Command::AcceptEnd, day(4));
    assert_eq!(scenario.closed(), Some(Outcome::EndedByAgreement));
}

#[test]
fn a_date_too_far_ahead_to_compute_never_comes_due() {
    // Validation refuses such a date; this is the rule holding on its own if
    // one ever reaches it, where it used to overflow.
    for far in [time::macros::date!(9999 - 11 - 03), time::Date::MAX] {
        let mut scenario = Scenario::active();
        let in_force = scenario.exchange.in_force.as_mut().unwrap();
        in_force.revision.contributions[0].due = Due::Date(far);

        assert_eq!(
            scenario.refused(Actor::System, Command::PromptInactivity, day(100_000)),
            Refusal::NotAllowed,
            "{far}"
        );
    }
}

#[test]
fn resolving_the_open_items_during_the_window_completes_instead() {
    let mut scenario = Scenario::active();
    scenario.ok(A, Command::RequestClose, day(1));
    scenario.ok(B, act(1, Action::Confirm), day(2));
    scenario.ok(A, act(2, Action::Confirm), day(2));

    assert_eq!(scenario.closed(), Some(Outcome::Completed));
}

#[test]
fn an_idle_exchange_is_prompted_then_closed() {
    let mut scenario = Scenario::active();
    let system = Actor::System;

    // The repair is due on 1 November, so the clock starts when that day ends.
    let idle_from = datetime!(2026-11-02 00:00 UTC);
    assert_eq!(scenario.exchange.idle_since(), idle_from);
    assert_eq!(
        scenario.refused(
            system,
            Command::PromptInactivity,
            idle_from + Duration::days(59)
        ),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(
            system,
            Command::CloseInactive,
            idle_from + Duration::days(200)
        ),
        Refusal::NotAllowed,
        "never closed without a prompt first"
    );

    let prompted = idle_from + Duration::days(60);
    scenario.ok(system, Command::PromptInactivity, prompted);
    assert_eq!(scenario.events, vec![Event::InactivityPrompted]);
    assert_eq!(
        scenario.refused(
            system,
            Command::PromptInactivity,
            prompted + Duration::days(1)
        ),
        Refusal::NotAllowed
    );
    assert_eq!(
        scenario.refused(
            system,
            Command::CloseInactive,
            prompted + Duration::days(29)
        ),
        Refusal::NotAllowed
    );

    scenario.ok(
        system,
        Command::CloseInactive,
        prompted + Duration::days(30),
    );
    assert_eq!(
        scenario.closed(),
        Some(Outcome::Unresolved(Unresolved::Inactive))
    );
}

#[test]
fn any_activity_after_the_prompt_restarts_the_clock() {
    let mut scenario = Scenario::active();
    let prompted = datetime!(2027-01-01 00:00 UTC);
    scenario.ok(Actor::System, Command::PromptInactivity, prompted);

    let acted = prompted + Duration::days(10);
    scenario.ok(A, act(1, Action::Claim), acted);
    assert_eq!(scenario.exchange.inactivity_prompted_at, None);
    assert_eq!(scenario.exchange.idle_since(), acted);
    assert_eq!(
        scenario.refused(
            Actor::System,
            Command::CloseInactive,
            prompted + Duration::days(30)
        ),
        Refusal::NotAllowed
    );
}

// ---- Closed is final ------------------------------------------------------

#[test]
fn a_closed_exchange_accepts_nothing() {
    let mut scenario = Scenario::active();
    scenario.ok(A, Command::ProposeEnd, day(1));
    scenario.ok(B, Command::AcceptEnd, day(1));

    let commands = [
        (A, send(2, fence_job())),
        (A, act(1, Action::Claim)),
        (B, Command::ProposeEnd),
        (B, Command::RequestClose),
        (Actor::System, Command::PromptInactivity),
        (Actor::System, Command::ExpireRevision),
    ];
    for (actor, command) in commands {
        assert_eq!(
            scenario.refused(actor, command, day(400)),
            Refusal::NotAllowed
        );
    }
}

#[test]
fn a_refusal_changes_nothing() {
    let scenario = Scenario::active();
    let before = scenario.exchange.clone();
    scenario.refused(A, act(1, Action::Confirm), day(1));
    scenario.refused(B, Command::Accept { revision: rev(9) }, day(1));
    assert_eq!(scenario.exchange, before);
}

// ---- Progress notes (DESIGN.md §7.2) --------------------------------------

fn note_progress(n: u128) -> Command {
    Command::NoteProgress { id: id(n) }
}

#[test]
fn the_provider_can_note_progress_while_an_item_is_pending_or_claimed_and_nothing_changes() {
    let mut scenario = Scenario::active();
    let before = scenario.exchange.statuses.clone();

    scenario.ok(A, note_progress(1), day(1));
    assert_eq!(
        scenario.events,
        vec![Event::ProgressNoted {
            contribution: id(1),
            by: Slot::A
        }]
    );
    assert_eq!(scenario.exchange.statuses, before, "no status moves");
    assert_eq!(scenario.status(1), Status::Pending, "and it is not a claim");

    scenario.ok(A, act(1, Action::Claim), day(2));
    scenario.ok(A, note_progress(1), day(3));
    assert_eq!(scenario.status(1), Status::Claimed);
}

#[test]
fn a_progress_note_is_the_providers_alone_and_only_while_the_item_is_under_way() {
    let mut scenario = Scenario::active();
    assert_eq!(
        scenario.refused(B, note_progress(1), day(1)),
        Refusal::WrongActor,
        "the recipient has no note to add"
    );
    assert_eq!(
        scenario.refused(A, note_progress(9), day(1)),
        Refusal::UnknownContribution(id(9))
    );

    scenario.ok(A, act(1, Action::Claim), day(1));
    scenario.ok(B, act(1, Action::Dispute), day(2));
    assert_eq!(
        scenario.refused(A, note_progress(1), day(3)),
        Refusal::NotAllowed,
        "a dispute has its own note"
    );
    scenario.ok(A, act(1, Action::Claim), day(4));
    scenario.ok(B, act(1, Action::Confirm), day(5));
    assert_eq!(
        scenario.refused(A, note_progress(1), day(6)),
        Refusal::NotAllowed
    );
}

#[test]
fn a_progress_note_needs_an_agreement_in_force() {
    let scenario = Scenario::negotiating();
    assert_eq!(
        scenario.refused(A, note_progress(1), day(1)),
        Refusal::NotAllowed
    );
}

#[test]
fn a_progress_note_changes_nothing_in_the_exchange_but_counts_as_activity() {
    let mut scenario = Scenario::active();
    scenario.ok(A, note_progress(1), day(20));
    assert_eq!(scenario.exchange.state, State::Active);
    assert_eq!(scenario.exchange.last_activity_at, day(20));
}
