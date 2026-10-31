//! The vocabulary of remote-player sampling: what was asked, and how the answer was reached.

use playerstate_t6::game::AvatarState;

/// How many snapshots a proxy keeps.
pub const SMOOTHER_FRAME_CAPACITY: usize = 32;

/// How far behind the newest snapshot remote players are shown.
pub const STANDIN_LAG_MILLIS: i32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmootherStrategy {
    ConstantLag,
    SessionNewest,
}

/// The time a sample was taken at: render time minus the proxy delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SmoothingInstant(pub i32);

/// Which version of the sampling rules produced a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmootherPolicyGeneration(pub u16);

pub const CONSTANT_LAG_POLICY_GENERATION: SmootherPolicyGeneration = SmootherPolicyGeneration(2);

/// Why a sample had nothing to interpolate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteBufferShortfall {
    NothingBuffered,
    PriorToEarliestFrame,
    PastNewestFrame,
    EntityEntryAbsent,
}

impl RemoteBufferShortfall {
    pub fn trace_dump_caption(self) -> &'static str {
        match self {
            Self::NothingBuffered => "empty",
            Self::PriorToEarliestFrame => "before",
            Self::PastNewestFrame => "after",
            Self::EntityEntryAbsent => "missing",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmoothingSampleVerdict {
    Exact {
        whole_frame: sim::Tick,
    },
    SmoothedBetweenFrames {
        from_tick: sim::Tick,
        to_tick: sim::Tick,
        alpha: f32,
    },
    RanDry {
        reason: RemoteBufferShortfall,
        retained_world_state: Option<sim::Tick>,
    },
}

impl SmoothingSampleVerdict {
    pub fn dump_variety_name(self) -> &'static str {
        match self {
            Self::Exact { .. } => "exact",
            Self::SmoothedBetweenFrames { .. } => "interpolated",
            Self::RanDry { .. } => "starved",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothingSampleOrigin {
    pub policy_revision: SmootherPolicyGeneration,
    pub resolved_clock: SmoothingInstant,
    pub outcome: SmoothingSampleVerdict,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmootherReading {
    Pose {
        ps: AvatarState,
        sample_origin_tag: SmoothingSampleOrigin,
    },
    RanDry {
        held: Option<AvatarState>,
        sample_origin_tag: SmoothingSampleOrigin,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseFillIn {
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_labels_are_the_short_words_the_dumps_use() {
        assert_eq!(RemoteBufferShortfall::NothingBuffered.trace_dump_caption(), "empty");
        assert_eq!(RemoteBufferShortfall::PriorToEarliestFrame.trace_dump_caption(), "before");
        assert_eq!(RemoteBufferShortfall::PastNewestFrame.trace_dump_caption(), "after");
        assert_eq!(RemoteBufferShortfall::EntityEntryAbsent.trace_dump_caption(), "missing");
    }

    #[test]
    fn dump_kinds_name_the_outcome() {
        let tick = sim::Tick(1);
        assert_eq!(
            SmoothingSampleVerdict::Exact { whole_frame: tick }.dump_variety_name(),
            "exact"
        );
        assert_eq!(
            SmoothingSampleVerdict::SmoothedBetweenFrames {
                from_tick: tick,
                to_tick: tick,
                alpha: 0.5
            }
            .dump_variety_name(),
            "interpolated"
        );
        assert_eq!(
            SmoothingSampleVerdict::RanDry {
                reason: RemoteBufferShortfall::NothingBuffered,
                retained_world_state: None
            }
            .dump_variety_name(),
            "starved"
        );
    }
}
