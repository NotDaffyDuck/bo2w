//! A ledger of a subsystem's known gaps: parts of the game it does not do
//! yet, or could not do this time. A standing gap is missing for good and
//! always live; any other gap is live from the time a cause raises it until
//! it is cleared. The ledger keeps each gap's last cause and how often it
//! was raised, and writes the live set as a one-line signature, so a report
//! can tell when the set changed.

use core::fmt;

/// One kind of gap. `ALL` names every kind once, and a ledger keeps one slot
/// per entry.
pub trait Gap: Copy + PartialEq + 'static {
    const ALL_KINDS: &'static [Self];

    /// The gap's name in the log.
    fn name(self) -> &'static str;

    /// Missing for good: live from the start, and never cleared.
    fn still_open(self) -> bool;

    /// The gap's slot: its place in `ALL`.
    fn index(self) -> usize {
        let listed = Self::ALL_KINDS.iter().position(|gap| *gap == self);
        listed.expect("every gap is listed in Gap::ALL")
    }
}

/// Why a gap was raised; written after the gap's name.
pub trait ShortfallOrigin: fmt::Display {
    type Gap: Gap;

    fn gap(&self) -> Self::Gap;
}

/// One gap's slot in a ledger: its last cause while live, and its raise count.
struct Slot<Origin> {
    cause: Option<Origin>,
    raised: u64,
}

/// The live gaps, each one's last cause, and how often each was raised.
/// `KINDS` must be `Gap::ALL.len()`.
pub struct LackLedger<Origin: ShortfallOrigin, const KINDS: usize> {
    slots: [Slot<Origin>; KINDS],
}

impl<Origin: ShortfallOrigin, const KINDS: usize> Default for LackLedger<Origin, KINDS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Origin: ShortfallOrigin, const KINDS: usize> LackLedger<Origin, KINDS> {
    pub fn new() -> Self {
        const { assert!(KINDS == Origin::Gap::ALL_KINDS.len(), "a ledger has one slot per Gap::ALL entry") };
        let slots = core::array::from_fn(|_| Slot { cause: None, raised: 0 });
        Self { slots }
    }

    fn slot_for(&self, kind: Origin::Gap) -> &Slot<Origin> {
        &self.slots[kind.index()]
    }

    /// Makes the cause's gap live with it, and counts the raise.
    pub fn raise(&mut self, why: Origin) {
        let entry = &mut self.slots[why.gap().index()];
        entry.raised = entry.raised.saturating_add(1);
        entry.cause = Some(why);
    }

    /// Ends a raised gap. A standing gap stays live, with its last cause.
    pub fn clear(&mut self, hole: Origin::Gap) {
        if hole.still_open() {
            return;
        }
        self.slots[hole.index()].cause = None;
    }

    pub fn is_live(&self, kind: Origin::Gap) -> bool {
        kind.still_open() || self.slot_for(kind).cause.is_some()
    }

    /// The cause the gap was last raised with, while it is live.
    pub fn cause(&self, hole: Origin::Gap) -> Option<&Origin> {
        self.slot_for(hole).cause.as_ref()
    }

    /// How many times the gap was raised.
    pub fn hits(&self, hole: Origin::Gap) -> u64 {
        self.slot_for(hole).raised
    }

    /// The live gaps, in `ALL` order.
    pub fn live(&self) -> impl Iterator<Item = Origin::Gap> + '_ {
        Origin::Gap::ALL_KINDS.iter().copied().filter(|gap| self.is_live(*gap))
    }

    pub fn count(&self) -> usize {
        self.live().count()
    }

    /// Each live gap as `name ` or `name=cause `, in `ALL` order.
    pub fn print_fingerprint(&self, sink: &mut impl fmt::Write) -> fmt::Result {
        self.live().try_for_each(|gap| match self.cause(gap) {
            Some(cause) => write!(sink, "{}={cause} ", gap.name()),
            None => write!(sink, "{} ", gap.name()),
        })
    }
}

impl<Origin: ShortfallOrigin, const KINDS: usize> fmt::Debug for LackLedger<Origin, KINDS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut live_names = String::new();
        self.print_fingerprint(&mut live_names)?;
        write!(f, "GapLedger[{live_names}]")
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Missing {
        Glass,
        Ropes,
        Water,
    }

    impl Gap for Missing {
        const ALL_KINDS: &'static [Self] = &[Self::Glass, Self::Ropes, Self::Water];

        fn name(self) -> &'static str {
            ["glass", "ropes", "water"][self as usize]
        }

        fn still_open(self) -> bool {
            self == Self::Water
        }
    }

    struct Because {
        missing: Missing,
        note: &'static str,
    }

    impl fmt::Display for Because {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.note)
        }
    }

    impl ShortfallOrigin for Because {
        type Gap = Missing;

        fn gap(&self) -> Missing {
            self.missing
        }
    }

    #[test]
    fn standing_gaps_start_live_and_raised_ones_come_and_go() {
        let mut ledger = LackLedger::<Because, 3>::new();
        assert_eq!(ledger.live().collect::<Vec<_>>(), [Missing::Water]);
        ledger.raise(Because { missing: Missing::Glass, note: "pane" });
        ledger.raise(Because { missing: Missing::Glass, note: "shard" });
        ledger.raise(Because { missing: Missing::Water, note: "pool" });
        assert_eq!(ledger.count(), 2);
        assert_eq!(ledger.hits(Missing::Glass), 2);
        assert_eq!(format!("{ledger:?}"), "GapLedger[glass=shard water=pool ]");
        ledger.clear(Missing::Glass);
        ledger.clear(Missing::Water);
        assert!(!ledger.is_live(Missing::Glass));
        assert!(ledger.cause(Missing::Glass).is_none());
        assert_eq!(
            ledger.cause(Missing::Water).map(|cause| cause.note),
            Some("pool")
        );
        assert_eq!(ledger.hits(Missing::Glass), 2);
        assert!(!ledger.is_live(Missing::Ropes));
    }
}
