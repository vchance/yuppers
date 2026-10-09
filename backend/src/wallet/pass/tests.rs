use time::macros::date;
use uuid::Uuid;

use super::*;
use crate::domain::revision::Slot;
use crate::exchanges::dto::{
    ContributionDto, ContributionStatus, ContributionType, CounterpartyDto, RevisionTerms,
    RevisionView,
};
use crate::wallet::wording::WalletWording;

const REPAIR: Uuid = Uuid::from_u128(1);
const PAYMENT: Uuid = Uuid::from_u128(2);

fn contribution(id: Uuid, from: Slot, due: DueDto, amount_minor: Option<i64>) -> ContributionDto {
    ContributionDto {
        id,
        from,
        r#type: if amount_minor.is_some() {
            ContributionType::Money
        } else {
            ContributionType::Service
        },
        description: if amount_minor.is_some() {
            "Payment for the fence".to_owned()
        } else {
            "Repair the back fence".to_owned()
        },
        quantity: None,
        due,
        completion_criteria: Some("Gate closes cleanly".to_owned()),
        required: true,
        amount_minor,
    }
}

fn revision(author: Slot) -> RevisionView {
    RevisionView {
        id: Uuid::from_u128(9),
        sequence: 1,
        author,
        note: Some("See you Saturday".to_owned()),
        expires_at: "2026-10-20T00:00:00Z".to_owned(),
        accepted_by: vec![Slot::A, Slot::B],
        content_hash: "ab".repeat(32),
        terms: RevisionTerms {
            party_a_name: "Ana Ruiz".to_owned(),
            party_b_name: "Ben Ortiz".to_owned(),
            terms: "Repair the back fence, then Ben pays.".to_owned(),
            contributions: vec![
                contribution(
                    REPAIR,
                    Slot::A,
                    DueDto::Date {
                        date: "2026-10-09".to_owned(),
                    },
                    None,
                ),
                contribution(
                    PAYMENT,
                    Slot::B,
                    DueDto::AfterContribution {
                        contribution: REPAIR,
                    },
                    Some(40_000),
                ),
            ],
        },
    }
}

/// The fence job in force, seen by Ben (slot B), both contributions open.
fn view() -> ExchangeView {
    ExchangeView {
        id: Uuid::from_u128(7),
        display_code: "AB12-CD34".to_owned(),
        state: StateDto::Active,
        closed_outcome: None,
        closed_reason: None,
        version: 4,
        timezone: "America/Chicago".to_owned(),
        currency: "USD".to_owned(),
        you: Slot::B,
        counterparty: CounterpartyDto::Confirmed,
        claimant: None,
        invitation_open: None,
        invitation_shared_at: None,
        other_party_left: false,
        open_revision: None,
        in_force_revision: Some(revision(Slot::A)),
        contributions: vec![
            ContributionStatus {
                id: REPAIR,
                status: Status::Pending,
                since: None,
            },
            ContributionStatus {
                id: PAYMENT,
                status: Status::Pending,
                since: None,
            },
        ],
        end_proposed_by: None,
        close_requested_by: None,
        close_requested_at: None,
        close_request_lapses_at: None,
        draft: None,
        content_hidden: false,
        // Ben owes the payment, and Ana shows her payment options: none of
        // them is ever on a pass.
        payment_options: crate::exchanges::dto::PaymentOptionsView {
            shown: false,
            theirs: Some(crate::payments::PaymentHandles {
                venmo: Some("ana-venmo".to_owned()),
                cash_app: Some("AnaCash".to_owned()),
                paypal: Some("AnaPayPal".to_owned()),
                zelle: Some("ana@zelle.test".to_owned()),
            }),
            theirs_changed: Default::default(),
        },
    }
}

fn context() -> PassContext {
    PassContext {
        today: date!(2026 - 10 - 01),
        closed_on: None,
        other_party_alias: None,
        link: "https://app.test/exchanges/7".to_owned(),
        due_soon_days: 2,
        status_on_face: StatusOnFace::Detailed,
    }
}

fn set(view: &mut ExchangeView, id: Uuid, status: Status) {
    for item in &mut view.contributions {
        if item.id == id {
            item.status = status;
        }
    }
}

fn wording() -> WalletWording {
    WalletWording::embedded().unwrap()
}

/// Every text on the pass, front and back.
fn texts(model: &PassModel) -> Vec<String> {
    let fields = [
        Some(&model.status),
        model.reference.as_ref(),
        model.next_due.as_ref(),
        model.outstanding.as_ref(),
        model.other_party.as_ref(),
        model.closed_on.as_ref(),
    ];
    let mut texts: Vec<String> = fields
        .into_iter()
        .flatten()
        .flat_map(|field| [field.label.clone(), field.value.clone()])
        .collect();
    texts.extend([
        model.product.clone(),
        model.description.clone(),
        model.note.clone(),
    ]);
    if let Some(link) = &model.link {
        texts.extend([link.label.clone(), link.url.clone()]);
    }
    texts
}

/// Nothing from the agreement is on a pass: no names, terms, descriptions,
/// amounts or notes (the lock-screen rule, DESIGN.md §11, §12).
#[track_caller]
fn holds_nothing_from_the_agreement(model: &PassModel) {
    let all = texts(model).join("\n");
    for secret in [
        "Ana",
        "Ben",
        "Ruiz",
        "Ortiz",
        "fence",
        "Fence",
        "Payment",
        "Gate",
        "400",
        "Saturday",
        "ana-venmo",
        "AnaCash",
        "AnaPayPal",
        "zelle",
    ] {
        assert!(!all.contains(secret), "{secret:?} is on the pass:\n{all}");
    }
}

#[test]
fn an_agreement_in_force_shows_its_reference_what_is_open_and_the_next_date() {
    let wording = wording();
    let model = render(&view(), &context(), wording.language("en"));
    assert_eq!(model.standing, Standing::InForce);
    assert_eq!(model.product, "Yuppers");
    assert_eq!(
        (model.status.label.as_str(), model.status.value.as_str()),
        ("Status", "In force")
    );
    assert_eq!(model.reference.as_ref().unwrap().value, "AB12-CD34");
    assert_eq!(model.next_due.as_ref().unwrap().value, "Oct 9, 2026");
    assert_eq!(model.next_due.as_ref().unwrap().label, "Next due");
    assert_eq!(model.outstanding.as_ref().unwrap().value, "2");
    assert_eq!(model.other_party, None, "no alias, so nobody is named");
    assert_eq!(model.closed_on, None);
    assert_eq!(
        model.link.as_ref().unwrap().url,
        "https://app.test/exchanges/7"
    );
    assert_eq!(model.description, "Yuppers yup AB12-CD34");
    assert!(model.note.contains("Yuppers"));
    assert!(!model.voided() && !model.finished());
    holds_nothing_from_the_agreement(&model);
}

#[test]
fn the_same_pass_in_spanish() {
    let wording = wording();
    let model = render(&view(), &context(), wording.language("es"));
    assert_eq!(model.language, "es");
    assert_eq!(
        (model.status.label.as_str(), model.status.value.as_str()),
        ("Estado", "En vigor")
    );
    assert_eq!(model.reference.as_ref().unwrap().label, "Referencia");
    assert_eq!(model.next_due.as_ref().unwrap().value, "9 oct 2026");
    assert_eq!(model.outstanding.as_ref().unwrap().label, "Pendiente");
    assert_eq!(model.link.as_ref().unwrap().label, "Abrir el yup");
    holds_nothing_from_the_agreement(&model);

    // A regional tag falls back to its language, and an unknown one to the
    // default.
    assert_eq!(
        render(&view(), &context(), wording.language("es-MX")).language,
        "es"
    );
    assert_eq!(
        render(&view(), &context(), wording.language("fr")).language,
        "en"
    );
}

#[test]
fn something_marked_delivered_to_the_holder_waits_for_them() {
    let wording = wording();
    let mut seen = view();
    // Ana (A) marked the repair she owes Ben as delivered.
    set(&mut seen, REPAIR, Status::Claimed);
    let model = render(&seen, &context(), wording.language("en"));
    assert_eq!(model.standing, Standing::WaitingForYou);
    assert_eq!(model.status.value, "Waiting for you");
    assert_eq!(
        render(&seen, &context(), wording.language("es"))
            .status
            .value,
        "Te toca a ti"
    );
    holds_nothing_from_the_agreement(&model);

    // To Ana, who is waiting on Ben, it is not.
    seen.you = Slot::A;
    assert_eq!(
        render(&seen, &context(), wording.language("en")).standing,
        Standing::InForce
    );
}

#[test]
fn a_proposal_from_the_other_party_waits_for_the_holder() {
    let wording = wording();
    let en = wording.language("en");
    for change in [
        |view: &mut ExchangeView| view.open_revision = Some(revision(Slot::A)),
        |view: &mut ExchangeView| view.end_proposed_by = Some(Slot::A),
        |view: &mut ExchangeView| view.close_requested_by = Some(Slot::A),
    ] {
        let mut seen = view();
        change(&mut seen);
        assert_eq!(
            render(&seen, &context(), en).standing,
            Standing::WaitingForYou
        );
        // Their own proposal waits for the other party, not for them.
        seen.you = Slot::A;
        assert_ne!(
            render(&seen, &context(), en).standing,
            Standing::WaitingForYou
        );
    }
}

#[test]
fn a_dispute_is_said_before_any_date() {
    let wording = wording();
    let mut seen = view();
    set(&mut seen, REPAIR, Status::Disputed);
    let model = render(
        &seen,
        &PassContext {
            today: date!(2026 - 10 - 20),
            ..context()
        },
        wording.language("en"),
    );
    assert_eq!(model.standing, Standing::Disputed);
    assert_eq!(model.status.value, "Disputed");
}

#[test]
fn dates_bring_due_soon_then_overdue_while_something_is_still_pending() {
    let wording = wording();
    let en = wording.language("en");
    let on = |today, seen: &ExchangeView| render(seen, &PassContext { today, ..context() }, en);
    let seen = view();
    assert_eq!(on(date!(2026 - 10 - 06), &seen).standing, Standing::InForce);
    assert_eq!(on(date!(2026 - 10 - 07), &seen).standing, Standing::DueSoon);
    assert_eq!(on(date!(2026 - 10 - 09), &seen).status.value, "Due soon");
    assert_eq!(on(date!(2026 - 10 - 10), &seen).standing, Standing::Overdue);
    assert_eq!(on(date!(2026 - 10 - 10), &seen).status.value, "Overdue");

    // Delivered and confirmed: nothing dated is open any more.
    let mut done = view();
    set(&mut done, REPAIR, Status::Accepted);
    let model = on(date!(2026 - 10 - 10), &done);
    assert_eq!(model.standing, Standing::InForce);
    assert_eq!(model.next_due, None);
    assert_eq!(model.outstanding.as_ref().unwrap().value, "1");
}

#[test]
fn a_completed_agreement_says_so_with_the_day_it_closed() {
    let wording = wording();
    let mut seen = view();
    seen.state = StateDto::Closed;
    seen.closed_outcome = Some(OutcomeDto::Completed);
    set(&mut seen, REPAIR, Status::Accepted);
    set(&mut seen, PAYMENT, Status::Accepted);
    let closed = PassContext {
        closed_on: Some(date!(2026 - 10 - 12)),
        ..context()
    };
    let model = render(&seen, &closed, wording.language("en"));
    assert_eq!(model.standing, Standing::Completed);
    assert_eq!(model.status.value, "Completed");
    assert_eq!(model.closed_on.as_ref().unwrap().label, "Closed on");
    assert_eq!(model.closed_on.as_ref().unwrap().value, "Oct 12, 2026");
    assert!(model.finished());
    assert_eq!((model.next_due, model.outstanding), (None, None));

    let es = render(&seen, &closed, wording.language("es"));
    assert_eq!(es.status.value, "Completado");
    assert_eq!(es.closed_on.unwrap().value, "12 oct 2026");

    seen.closed_outcome = Some(OutcomeDto::EndedByAgreement);
    assert_eq!(
        render(&seen, &closed, wording.language("en")).status.value,
        "Ended by agreement"
    );
    seen.closed_outcome = Some(OutcomeDto::Unresolved);
    assert_eq!(
        render(&seen, &closed, wording.language("es")).status.value,
        "Cerrado"
    );
}

#[test]
fn the_other_party_is_named_only_by_an_alias() {
    let wording = wording();
    let with_alias = PassContext {
        other_party_alias: Some("  Neighbour ".to_owned()),
        ..context()
    };
    let model = render(&view(), &with_alias, wording.language("en"));
    let other = model.other_party.unwrap();
    assert_eq!(
        (other.label.as_str(), other.value.as_str()),
        ("With", "Neighbour")
    );

    let blank = PassContext {
        other_party_alias: Some("   ".to_owned()),
        ..context()
    };
    assert_eq!(
        render(&view(), &blank, wording.language("en")).other_party,
        None
    );
}

#[test]
fn a_void_pass_says_only_that_it_is_no_longer_in_use() {
    let wording = wording();
    for (language, status) in [("en", "No longer in use"), ("es", "Ya no está en uso")] {
        let model = void(wording.language(language));
        assert!(model.voided());
        assert_eq!(model.status.value, status);
        assert_eq!(
            (
                &model.reference,
                &model.next_due,
                &model.outstanding,
                &model.other_party,
                &model.closed_on,
                &model.link
            ),
            (&None, &None, &None, &None, &None, &None)
        );
        assert!(!texts(&model).join(" ").contains("AB12"));
    }
}

#[test]
fn the_face_changes_only_when_what_it_shows_changes() {
    let wording = wording();
    let en = wording.language("en");
    let before = render(&view(), &context(), en);
    let mut later = view();
    // A new version (a statement, say) that changes nothing shown.
    later.version += 1;
    later.close_requested_at = Some("2026-10-02T10:00:00Z".to_owned());
    assert_eq!(render(&later, &context(), en), before);
}

#[test]
fn a_neutral_status_line_says_in_force_and_no_date_whatever_presses() {
    let wording = wording();
    let en = wording.language("en");
    let neutral = |today, seen: &ExchangeView| {
        render(
            seen,
            &PassContext {
                today,
                status_on_face: StatusOnFace::Neutral,
                ..context()
            },
            en,
        )
    };
    let mut seen = view();
    for today in [
        date!(2026 - 10 - 06),
        date!(2026 - 10 - 09),
        date!(2026 - 10 - 20),
    ] {
        let model = neutral(today, &seen);
        assert_eq!(model.standing, Standing::InForce);
        assert_eq!(model.status.value, "In force");
        assert_eq!(model.next_due, None);
        assert!(model.outstanding.is_some());
    }
    set(&mut seen, REPAIR, Status::Disputed);
    assert_eq!(
        neutral(date!(2026 - 10 - 06), &seen).standing,
        Standing::InForce
    );
    // The detailed line, a deployment's choice, says it.
    assert_eq!(render(&seen, &context(), en).standing, Standing::Disputed);
}
