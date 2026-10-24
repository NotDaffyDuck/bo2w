//! What a shooter claims about the moment he fired: which stored collision frames his client was looking at.

use crate::world::Tick;

/// How trustworthy the two frames a shot claims to sit between are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShotSampleQuality {
    /// No claim at all: the shot is judged against the present.
    #[default]
    None,
    /// The client displayed exactly one stored frame.
    Exact,
    /// The client displayed a blend of two stored frames.
    Interpolated,
    /// The client kept showing an old frame because no newer one arrived.
    Held,
    /// The client had nothing to show.
    Starved,
}

impl ShotSampleQuality {
    /// Only the first two qualities ask the server to rewind.
    pub const fn claims_history(self) -> bool {
        matches!(self, Self::Exact | Self::Interpolated)
    }
}

/// The pair of frames a shot was aimed against, with the blend between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShotSampleProvenance {
    pub left: Tick,
    pub right: Tick,
    pub alpha: f32,
    pub quality: ShotSampleQuality,
}

impl ShotSampleProvenance {
    pub const NO_CLAIM: Self = Self { left: Tick(0), right: Tick(0), alpha: 0.0, quality: ShotSampleQuality::None };

    /// The tick the server would rewind to.
    pub const fn requested_tick(self) -> Tick {
        self.left
    }

    /// A claim holds up when it asks for history, its frames are in order, none is from the future, and the blend
    /// is a real fraction.
    pub fn is_valid_for(self, now: Tick) -> bool {
        if !self.quality.claims_history() {
            return false;
        }
        let ordered = self.left.0 <= self.right.0 && self.right.0 <= now.0;
        ordered && self.alpha.is_finite() && (0.0..=1.0).contains(&self.alpha)
    }
}
