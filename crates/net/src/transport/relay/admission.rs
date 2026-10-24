use master_protocol::MemberId;

/// One member the authority has let into the match for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedAdmission {
    pub member_id: MemberId,
    pub epoch: u32,
    pub bootstrap_id: u32,
    pub connection_id: u64,

    /// False when the same member confirmed an admission that was already committed.
    pub first_commit: bool,
}

/// Names one bootstrap offer: the id it was sent under, the snapshot it carried, and the match
/// epoch it was made for. A member's "applied" message must name the same three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OfferKey {
    pub bootstrap_id: u32,
    pub snapshot_seq: u32,
    pub epoch: u32,
}

/// How far a peer has got into the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// Nothing has been offered yet.
    Uncommitted,
    /// An offer is out and the peer has not confirmed it.
    Pending(OfferKey),
    /// The peer confirmed this bootstrap and is playing.
    Committed { bootstrap_id: u32 },
}

/// What the authority does with an "applied" message from a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// It confirms the open offer: the peer is in.
    Commit,
    /// The peer is already in and is saying so again.
    RepeatEnter,
    /// It matches nothing that was offered.
    Ignore,
}

impl Admission {
    pub(super) fn judge(&self, applied: OfferKey) -> Verdict {
        match *self {
            Self::Pending(offer) if offer == applied => Verdict::Commit,
            Self::Committed { bootstrap_id } if bootstrap_id == applied.bootstrap_id => {
                Verdict::RepeatEnter
            }
            _ => Verdict::Ignore,
        }
    }

    pub(super) fn is_committed(&self) -> bool {
        matches!(self, Self::Committed { .. })
    }

    /// The epoch of the offer that is waiting for an answer, if one is.
    pub(super) fn pending_epoch(&self) -> Option<u32> {
        match self {
            Self::Pending(offer) => Some(offer.epoch),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: OfferKey = OfferKey {
        bootstrap_id: 4,
        snapshot_seq: 9,
        epoch: 2,
    };

    #[test]
    fn the_open_offer_is_committed_by_an_exact_match() {
        assert_eq!(Admission::Pending(OFFER).judge(OFFER), Verdict::Commit);
    }

    #[test]
    fn every_part_of_the_key_has_to_match() {
        let pending = Admission::Pending(OFFER);
        for wrong in [
            OfferKey { bootstrap_id: 5, ..OFFER },
            OfferKey { snapshot_seq: 10, ..OFFER },
            OfferKey { epoch: 3, ..OFFER },
        ] {
            assert_eq!(pending.judge(wrong), Verdict::Ignore, "{wrong:?}");
        }
    }

    #[test]
    fn nothing_offered_means_nothing_to_confirm() {
        assert_eq!(Admission::Uncommitted.judge(OFFER), Verdict::Ignore);
    }

    #[test]
    fn a_committed_peer_repeating_itself_is_recognised() {
        let committed = Admission::Committed { bootstrap_id: 4 };
        assert_eq!(committed.judge(OFFER), Verdict::RepeatEnter);
        let other_epoch = OfferKey { epoch: 99, ..OFFER };
        assert_eq!(committed.judge(other_epoch), Verdict::RepeatEnter);
    }

    #[test]
    fn a_committed_peer_naming_another_bootstrap_is_ignored() {
        let committed = Admission::Committed { bootstrap_id: 3 };
        assert_eq!(committed.judge(OFFER), Verdict::Ignore);
    }

    #[test]
    fn only_a_committed_peer_counts_as_committed() {
        assert!(!Admission::Uncommitted.is_committed());
        assert!(!Admission::Pending(OFFER).is_committed());
        assert!(Admission::Committed { bootstrap_id: 1 }.is_committed());
    }

    #[test]
    fn only_a_pending_offer_has_an_epoch() {
        assert_eq!(Admission::Pending(OFFER).pending_epoch(), Some(2));
        assert_eq!(Admission::Uncommitted.pending_epoch(), None);
        assert_eq!(Admission::Committed { bootstrap_id: 4 }.pending_epoch(), None);
    }
}
