use master_protocol::RoomOccupantId;

/// One member the authority has let into the match for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinalizedEntry {
    pub member_id: RoomOccupantId,
    pub round_era: u32,
    pub transfer_ticket: u32,
    pub link_no: u64,

    /// False when the same member confirmed an admission that was already committed.
    pub initial_publish_stamp: bool,
}

/// Names one bootstrap offer: the id it was sent under, the snapshot it carried, and the match
/// epoch it was made for. A member's "applied" message must name the same three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OfferKey {
    pub offer_no: u32,
    pub snap_no: u32,
    pub era: u32,
}

/// How far a peer has got into the match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// Nothing has been offered yet.
    NotYetPublished,
    /// An offer is out and the peer has not confirmed it.
    Pending(OfferKey),
    /// The peer confirmed this bootstrap and is playing.
    Committed { offer_no: u32 },
}

/// What the authority does with an "applied" message from a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// It confirms the open offer: the peer is in.
    Commit,
    /// The peer is already in and is saying so again.
    ReEntry,
    /// It matches nothing that was offered.
    Ignore,
}

impl Admission {
    pub(super) fn judge(&self, applied: OfferKey) -> Verdict {
        match *self {
            Self::Pending(offer) if offer == applied => Verdict::Commit,
            Self::Committed { offer_no } if offer_no == applied.offer_no => {
                Verdict::ReEntry
            }
            _ => Verdict::Ignore,
        }
    }

    pub(super) fn is_committed(&self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(&Self::Committed { offer_no: 0 })
    }

    /// The epoch of the offer that is waiting for an answer, if one is.
    pub(super) fn pending_epoch(&self) -> Option<u32> {
        match self {
            Self::Pending(offer) => Some(offer.era),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: OfferKey = OfferKey {
        offer_no: 4,
        snap_no: 9,
        era: 2,
    };

    #[test]
    fn the_open_offer_is_committed_by_an_exact_match() {
        assert_eq!(Admission::Pending(OFFER).judge(OFFER), Verdict::Commit);
    }

    #[test]
    fn every_part_of_the_key_has_to_match() {
        let pending = Admission::Pending(OFFER);
        for wrong in [
            OfferKey { offer_no: 5, ..OFFER },
            OfferKey { snap_no: 10, ..OFFER },
            OfferKey { era: 3, ..OFFER },
        ] {
            assert_eq!(pending.judge(wrong), Verdict::Ignore, "{wrong:?}");
        }
    }

    #[test]
    fn nothing_offered_means_nothing_to_confirm() {
        assert_eq!(Admission::NotYetPublished.judge(OFFER), Verdict::Ignore);
    }

    #[test]
    fn a_committed_peer_repeating_itself_is_recognised() {
        let committed = Admission::Committed { offer_no: 4 };
        assert_eq!(committed.judge(OFFER), Verdict::ReEntry);
        let other_epoch = OfferKey { era: 99, ..OFFER };
        assert_eq!(committed.judge(other_epoch), Verdict::ReEntry);
    }

    #[test]
    fn a_committed_peer_naming_another_bootstrap_is_ignored() {
        let committed = Admission::Committed { offer_no: 3 };
        assert_eq!(committed.judge(OFFER), Verdict::Ignore);
    }

    #[test]
    fn only_a_committed_peer_counts_as_committed() {
        assert!(!Admission::NotYetPublished.is_committed());
        assert!(!Admission::Pending(OFFER).is_committed());
        assert!(Admission::Committed { offer_no: 1 }.is_committed());
    }

    #[test]
    fn only_a_pending_offer_has_an_epoch() {
        assert_eq!(Admission::Pending(OFFER).pending_epoch(), Some(2));
        assert_eq!(Admission::NotYetPublished.pending_epoch(), None);
        assert_eq!(Admission::Committed { offer_no: 4 }.pending_epoch(), None);
    }
}
