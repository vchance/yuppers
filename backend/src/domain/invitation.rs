//! Whether an invitation may be claimed, and by whom (DESIGN.md §8).

use time::OffsetDateTime;

/// Whom an invitation names: an email address or a phone number, as its
/// blind index (`crate::contact`). The same value has the same index
/// wherever it is stored, so comparing indexes compares the values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Email([u8; 32]),
    Phone([u8; 32]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub expires_at: OffsetDateTime,
    pub claimed: bool,
    pub revoked: bool,
    /// Set when the initiator named who the invitation is for.
    pub bound_to: Option<Binding>,
}

/// The verified account trying to claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claimant {
    /// The blind index of its email address, if it has one.
    pub email: Option<[u8; 32]>,
    /// The blind index of its phone number, if it has one.
    pub phone: Option<[u8; 32]>,
    /// The claimant is the person who sent the invitation.
    pub is_initiator: bool,
    /// Either party has blocked the other.
    pub blocked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// The invitation named this person, so the initiator already knows who
    /// they are and need not confirm them.
    pub pre_bound: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("the invitation has been revoked")]
    Revoked,
    #[error("the invitation has already been claimed")]
    AlreadyClaimed,
    #[error("the invitation has expired")]
    Expired,
    #[error("the initiator cannot claim their own invitation")]
    OwnInvitation,
    #[error("the invitation is for someone else")]
    BoundToSomeoneElse,
    #[error("one of the parties has blocked the other")]
    Blocked,
}

/// Decides a claim. An invitation is claimed once; a bound invitation only by
/// an account that has verified the identifier it names.
pub fn claim(
    invitation: &Invitation,
    claimant: &Claimant,
    now: OffsetDateTime,
) -> Result<Claim, Refusal> {
    if invitation.revoked {
        return Err(Refusal::Revoked);
    }
    if invitation.claimed {
        return Err(Refusal::AlreadyClaimed);
    }
    if now >= invitation.expires_at {
        return Err(Refusal::Expired);
    }
    if claimant.is_initiator {
        return Err(Refusal::OwnInvitation);
    }
    if claimant.blocked {
        return Err(Refusal::Blocked);
    }

    match &invitation.bound_to {
        None => Ok(Claim { pre_bound: false }),
        Some(Binding::Email(email)) if claimant.email.as_ref() == Some(email) => {
            Ok(Claim { pre_bound: true })
        }
        Some(Binding::Phone(phone)) if claimant.phone.as_ref() == Some(phone) => {
            Ok(Claim { pre_bound: true })
        }
        Some(_) => Err(Refusal::BoundToSomeoneElse),
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;
    use time::macros::datetime;

    use super::*;

    const NOW: OffsetDateTime = datetime!(2026-10-05 09:00 UTC);

    fn invitation() -> Invitation {
        Invitation {
            expires_at: NOW + Duration::days(10),
            claimed: false,
            revoked: false,
            bound_to: None,
        }
    }

    /// Stand-ins for the blind indexes of Ben's address and number, and of
    /// Carla's address.
    const BEN_EMAIL: [u8; 32] = [1; 32];
    const BEN_PHONE: [u8; 32] = [2; 32];
    const CARLA_EMAIL: [u8; 32] = [3; 32];

    fn ben() -> Claimant {
        Claimant {
            email: Some(BEN_EMAIL),
            phone: Some(BEN_PHONE),
            is_initiator: false,
            blocked: false,
        }
    }

    #[test]
    fn an_unbound_invitation_can_be_claimed_but_needs_confirming() {
        assert_eq!(
            claim(&invitation(), &ben(), NOW),
            Ok(Claim { pre_bound: false })
        );
    }

    #[test]
    fn a_bound_invitation_is_claimed_by_the_person_it_names() {
        for bound in [Binding::Email(BEN_EMAIL), Binding::Phone(BEN_PHONE)] {
            let invitation = Invitation {
                bound_to: Some(bound),
                ..invitation()
            };
            assert_eq!(
                claim(&invitation, &ben(), NOW),
                Ok(Claim { pre_bound: true }),
                "{bound:?}"
            );
        }
    }

    #[test]
    fn a_bound_invitation_is_refused_to_anyone_else() {
        let invitation = Invitation {
            bound_to: Some(Binding::Email(CARLA_EMAIL)),
            ..invitation()
        };
        assert_eq!(
            claim(&invitation, &ben(), NOW),
            Err(Refusal::BoundToSomeoneElse)
        );
        // An address's index is not a number's, whatever its bytes.
        let invitation = Invitation {
            bound_to: Some(Binding::Phone(BEN_EMAIL)),
            ..self::invitation()
        };
        assert_eq!(
            claim(&invitation, &ben(), NOW),
            Err(Refusal::BoundToSomeoneElse)
        );

        // A phone-bound invitation is not satisfied by an account with only an email.
        let invitation = Invitation {
            bound_to: Some(Binding::Phone(BEN_PHONE)),
            ..self::invitation()
        };
        let email_only = Claimant {
            phone: None,
            ..ben()
        };
        assert_eq!(
            claim(&invitation, &email_only, NOW),
            Err(Refusal::BoundToSomeoneElse)
        );
    }

    #[test]
    fn an_invitation_is_claimed_once() {
        let invitation = Invitation {
            claimed: true,
            ..invitation()
        };
        assert_eq!(
            claim(&invitation, &ben(), NOW),
            Err(Refusal::AlreadyClaimed)
        );
    }

    #[test]
    fn revoked_and_expired_invitations_are_dead() {
        let revoked = Invitation {
            revoked: true,
            ..invitation()
        };
        assert_eq!(claim(&revoked, &ben(), NOW), Err(Refusal::Revoked));

        let at_expiry = invitation().expires_at;
        assert_eq!(
            claim(&invitation(), &ben(), at_expiry),
            Err(Refusal::Expired)
        );
        assert!(claim(&invitation(), &ben(), at_expiry - Duration::seconds(1)).is_ok());
    }

    #[test]
    fn the_initiator_cannot_be_their_own_counterparty() {
        let initiator = Claimant {
            is_initiator: true,
            ..ben()
        };
        assert_eq!(
            claim(&invitation(), &initiator, NOW),
            Err(Refusal::OwnInvitation)
        );
    }

    #[test]
    fn a_block_between_the_parties_prevents_the_claim() {
        let blocked = Claimant {
            blocked: true,
            ..ben()
        };
        assert_eq!(claim(&invitation(), &blocked, NOW), Err(Refusal::Blocked));
    }
}
