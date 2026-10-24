//! A ledger of a subsystem's known gaps: parts of the game it does not do
//! yet, or could not do this time. A standing gap is missing for good and
//! always live; any other gap is live from the time a cause raises it until
//! it is cleared. The ledger keeps each gap's last cause and how often it
//! was raised, and writes the live set as a one-line signature, so a report
//! can tell when the set changed.

use core::fmt::{self, Write as _};

/// One kind of gap. `ALL` names every kind once, and a ledger keeps one slot
/// per entry.
pub trait Gap: Copy + PartialEq + 'static {
    const ALL: &'static [Self];

    /// The gap's name in the log.
    fn name(self) -> &'static str;

    /// Missing for good: live from the start, and never cleared.
    fn is_standing(self) -> bool;

    /// The gap's slot: its place in `ALL`.
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|gap| *gap == self)
            .expect("every gap is listed in Gap::ALL")
    }
}

/// Why a gap was raised; written after the gap's name.
pub trait GapCause: fmt::Display {
    type Gap: Gap;

    fn gap(&self) -> Self::Gap;
}

/// The live gaps, each one's last cause, and how often each was raised.
/// `N` must be `Gap::ALL.len()`.
pub struct GapLedger<C: GapCause, const N: usize> {
    causes: [Option<C>; N],
    hits: [u64; N],
}

impl<C: GapCause, const N: usize> Default for GapLedger<C, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: GapCause, const N: usize> GapLedger<C, N> {
    pub fn new() -> Self {
        const {
            assert!(
                N == C::Gap::ALL.len(),
                "a ledger has one slot per Gap::ALL entry"
            )
        };
        Self {
            causes: core::array::from_fn(|_| None),
            hits: [0; N],
        }
    }

    /// Makes the cause's gap live with it, and counts the raise.
    pub fn raise(&mut self, cause: C) {
        let slot = cause.gap().index();
        self.hits[slot] = self.hits[slot].saturating_add(1);
        self.causes[slot] = Some(cause);
    }

    /// Ends a raised gap. A standing gap stays live, with its last cause.
    pub fn clear(&mut self, gap: C::Gap) {
        if !gap.is_standing() {
            self.causes[gap.index()] = None;
        }
    }

    pub fn is_live(&self, gap: C::Gap) -> bool {
        gap.is_standing() || self.causes[gap.index()].is_some()
    }

    /// The cause the gap was last raised with, while it is live.
    pub fn cause(&self, gap: C::Gap) -> Option<&C> {
        self.causes[gap.index()].as_ref()
    }

    /// How many times the gap was raised.
    pub fn hits(&self, gap: C::Gap) -> u64 {
        self.hits[gap.index()]
    }

    /// The live gaps, in `ALL` order.
    pub fn live(&self) -> impl Iterator<Item = C::Gap> + '_ {
        C::Gap::ALL.iter().copied().filter(|gap| self.is_live(*gap))
    }

    pub fn count(&self) -> usize {
        self.live().count()
    }

    /// Each live gap as `name ` or `name=cause `, in `ALL` order.
    pub fn write_signature(&self, out: &mut impl fmt::Write) -> fmt::Result {
        for gap in self.live() {
            out.write_str(gap.name())?;
            if let Some(cause) = self.cause(gap) {
                write!(out, "={cause}")?;
            }
            out.write_char(' ')?;
        }
        Ok(())
    }
}

impl<C: GapCause, const N: usize> fmt::Debug for GapLedger<C, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GapLedger[")?;
        self.write_signature(f)?;
        f.write_char(']')
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
        const ALL: &'static [Self] = &[Self::Glass, Self::Ropes, Self::Water];

        fn name(self) -> &'static str {
            match self {
                Self::Glass => "glass",
                Self::Ropes => "ropes",
                Self::Water => "water",
            }
        }

        fn is_standing(self) -> bool {
            self == Self::Water
        }
    }

    struct Because(Missing, &'static str);

    impl fmt::Display for Because {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.1)
        }
    }

    impl GapCause for Because {
        type Gap = Missing;

        fn gap(&self) -> Missing {
            self.0
        }
    }

    #[test]
    fn standing_gaps_start_live_and_raised_ones_come_and_go() {
        let mut ledger = GapLedger::<Because, 3>::new();
        assert_eq!(ledger.live().collect::<Vec<_>>(), [Missing::Water]);
        ledger.raise(Because(Missing::Glass, "pane"));
        ledger.raise(Because(Missing::Glass, "shard"));
        ledger.raise(Because(Missing::Water, "pool"));
        assert_eq!(ledger.count(), 2);
        assert_eq!(ledger.hits(Missing::Glass), 2);
        assert_eq!(format!("{ledger:?}"), "GapLedger[glass=shard water=pool ]");
        ledger.clear(Missing::Glass);
        ledger.clear(Missing::Water);
        assert!(!ledger.is_live(Missing::Glass));
        assert!(ledger.cause(Missing::Glass).is_none());
        assert_eq!(
            ledger.cause(Missing::Water).map(|cause| cause.1),
            Some("pool")
        );
        assert_eq!(ledger.hits(Missing::Glass), 2);
        assert!(!ledger.is_live(Missing::Ropes));
    }
}
