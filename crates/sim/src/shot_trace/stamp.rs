//! What a shooter claims about the moment he fired: which stored collision frames his client was looking at.

use crate::sim_parts::StepNo;

/// How trustworthy the two frames a shot claims to sit between are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShotRecordGrade {
    /// No claim at all: the shot is judged against the present.
    #[default]
    None,
    /// The client displayed exactly one stored frame.
    Exact,
    /// The client displayed a blend of two stored frames.
    SmoothedBetweenFrames,
    /// The client kept showing an old frame because no newer one arrived.
    Held,
    /// The client had nothing to show.
    RanDry,
}

impl ShotRecordGrade {
    /// Only the first two qualities ask the server to rewind.
    pub const fn asserts_past_record(self) -> bool {
        matches!(self, Self::SmoothedBetweenFrames | Self::Exact)
    }
}

/// The pair of frames a shot was aimed against, with the blend between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShotRecordOrigin {
    pub left: StepNo,
    pub right: StepNo,
    pub opacity: f32,
    pub quality: ShotRecordGrade,
}

impl ShotRecordOrigin {
    pub const UNCLAIMED: Self = Self { left: StepNo(0), right: StepNo(0), opacity: 0.0, quality: ShotRecordGrade::None };

    /// The tick the server would rewind to.
    pub const fn asked_for_tick(self) -> StepNo {
        self.left
    }

    /// The moment the shooter's screen showed, in ms: the blend of the way from the earlier frame to the later one
    /// (BO2 sends this as the command's server time, `client->lastServerTime`).
    pub fn drawn_at_ms(self, step_ms: u32) -> u32 {
        let span = self.right.0.saturating_sub(self.left.0) * step_ms;
        self.left.0 * step_ms + (self.opacity * span as f32).round() as u32
    }

    /// A claim holds up when it asks for history, its frames are in order, none is from the future, and the blend
    /// is a real fraction.
    pub fn still_fresh_at(self, now: StepNo) -> bool {
        if !self.quality.asserts_past_record() {
            return false;
        }
        let ordered = self.left.0 <= self.right.0 && self.right.0 <= now.0;
        ordered && self.opacity.is_finite() && (0.0..=1.0).contains(&self.opacity)
    }
}
