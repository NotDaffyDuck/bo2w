//! The running record of gaps, and the warning when the set of live gaps changes.

use std::ops::{Deref, DerefMut};

use bevy::prelude::Resource;
use diag::gap::{Gap, LackLedger};

use super::cause::LinkDeficiencyOrigin;
use super::kinds::LinkDeficiency;

const GAP_KINDS: usize = <LinkDeficiency as Gap>::ALL_KINDS.len();

/// The net's gap ledger, plus what was last logged about it. The ledger's own
/// methods (`raise`, `still_counting`, `cause`, `hits`, `live`, `count`) are reached
/// through `Deref`.
#[derive(Resource, Debug, Default)]
pub struct LinkIdentityDeficits {
    ledger: LackLedger<LinkDeficiencyOrigin, GAP_KINDS>,
    /// The live gaps as last logged, so an unchanged set is not logged again.
    last_logged: String,
}

impl Deref for LinkIdentityDeficits {
    type Target = LackLedger<LinkDeficiencyOrigin, GAP_KINDS>;

    fn deref(&self) -> &Self::Target {
        &self.ledger
    }
}

impl DerefMut for LinkIdentityDeficits {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ledger
    }
}

impl LinkIdentityDeficits {
    /// Log the raised gaps, once per change in what is live or why.
    pub(crate) fn log_counts(&mut self) {
        let mut now = String::new();
        // Writing into a String cannot fail.
        self.ledger.print_fingerprint(&mut now).unwrap_or(());
        let earlier = std::mem::replace(&mut self.last_logged, now);
        if earlier == self.last_logged {
            return;
        }
        let ledger = &self.ledger;
        let explained = ledger
            .live()
            .filter_map(|gap| ledger.cause(gap).map(|cause| (gap, cause)));
        for (gap, cause) in explained {
            let (name, hits) = (gap.name(), ledger.hits(gap));
            diag::warn!(
                WireLink,
                "net gap {name} (x{hits}): {cause} — rejected instead of aliasing an entity slot"
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn twice_claimed(number: u16) -> LinkDeficiencyOrigin {
        LinkDeficiencyOrigin::NumberClaimedTwice { number }
    }

    #[test]
    fn only_the_standing_gap_is_live_at_first() {
        let gaps = LinkIdentityDeficits::default();
        assert_eq!(gaps.live().collect::<Vec<_>>(), [LinkDeficiency::ObjectStateRecord]);
        assert_eq!(gaps.count(), 1);
        assert_eq!(gaps.cause(LinkDeficiency::ObjectStateRecord), None);
        assert_eq!(gaps.hits(LinkDeficiency::ObjectStateRecord), 0);
    }

    #[test]
    fn raising_a_cause_makes_its_gap_live_with_that_cause() {
        let mut gaps = LinkIdentityDeficits::default();
        assert!(!gaps.is_live(LinkDeficiency::ClientEntityIndex));
        gaps.raise(twice_claimed(5));
        assert!(gaps.is_live(LinkDeficiency::ClientEntityIndex));
        assert_eq!(gaps.cause(LinkDeficiency::ClientEntityIndex), Some(&twice_claimed(5)));
        assert_eq!(gaps.count(), 2);
    }

    #[test]
    fn raising_again_counts_a_hit_and_keeps_the_latest_cause() {
        let mut gaps = LinkIdentityDeficits::default();
        gaps.raise(twice_claimed(5));
        gaps.raise(twice_claimed(8));
        assert_eq!(gaps.hits(LinkDeficiency::ClientEntityIndex), 2);
        assert_eq!(gaps.cause(LinkDeficiency::ClientEntityIndex), Some(&twice_claimed(8)));
        assert_eq!(gaps.hits(LinkDeficiency::ObjectEventRecipient), 0);
    }

    /// Reports, then hands back what is now remembered as reported.
    fn logged(gaps: &mut LinkIdentityDeficits) -> String {
        gaps.log_counts();
        gaps.last_logged.clone()
    }

    #[test]
    fn a_report_is_remembered_so_an_unchanged_set_is_not_repeated() {
        let mut gaps = LinkIdentityDeficits::default();
        assert_eq!(logged(&mut gaps), "entity-state-vector ");
        gaps.raise(twice_claimed(5));
        let first = logged(&mut gaps);
        assert!(first.contains("centity-number=entity number 5 was claimed twice"), "{first}");
        assert_eq!(logged(&mut gaps), first);
        gaps.raise(twice_claimed(6));
        assert_ne!(logged(&mut gaps), first, "a new cause is a change");
    }
}
