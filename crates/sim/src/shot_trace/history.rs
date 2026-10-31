//! Stored collision frames, for rewinding a shot to what its shooter saw.

use std::collections::VecDeque;

use super::HITBOX_TIMELINE_LENGTH;
use super::entity_caps::EntityRayShape;
use super::poses::PlayerHitboxPose;
use crate::sim_parts::StepNo;

/// Which part of the tick a frame was taken in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RewindStage {
    #[default]
    AfterPlayerMove,
}

/// Why a shot was judged against the present although it named a past tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveStateCause {
    ShooterNamedNoTick,
    NamedTickIsShotTick,
    /// BO2 does not rewind a shot that is no more than `ANTILAG_SKIP_MS` old.
    TooRecentToRewind { requested: StepNo },
    NothingStored { requested: StepNo },
}

/// Why a rewind stopped short of the tick asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewindLimitCause {
    BeyondRewindLimit,
}

/// Why a shot's claim was turned down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewindDenialCause {
    NoFrameForTick,
    TickAfterShot,
    ClaimUnreadable,
}

/// What became of a request to rewind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewindSampleRuling {
    LiveNow { tick: StepNo, reason: LiveStateCause },
    Exact { requested: StepNo, frame: StepNo, phase: RewindStage },
    Clamped { requested: StepNo, used: StepNo, frame: StepNo, phase: RewindStage, reason: RewindLimitCause },
    Declined { requested: StepNo, reason: RewindDenialCause },
}

/// Every player's collision pose at one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindFrame {
    pub tick: StepNo,
    pub phase: RewindStage,
    pub poses: Vec<PlayerHitboxPose>,
}

/// The players a rewound shot is tested against, and how they were chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindSample {
    pub bodies_then: Vec<PlayerHitboxPose>,
    pub verdict: RewindSampleRuling,
}

/// Every scripted entity's collision at one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityHitboxSnapshot {
    pub tick: StepNo,
    pub phase: RewindStage,
    pub rows: Vec<EntityRayShape>,
}

/// The entities a rewound shot is tested against, and how they were chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityHitboxReading {
    pub rows: Vec<EntityRayShape>,
    pub verdict: RewindSampleRuling,
}

/// Players and entities for one shot. Both carry the shot's verdict; the entities stand at the very moment the
/// shooter drew, as BO2's movers do, and each row's epoch names the kept frame it was taken from.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindRequest {
    pub players: RewindSample,
    pub entities: EntityHitboxReading,
}

/// A frame that is filed under a tick and phase.
trait Filed {
    fn filing(&self) -> (StepNo, RewindStage);
}

impl Filed for RewindFrame {
    fn filing(&self) -> (StepNo, RewindStage) {
        (self.tick, self.phase)
    }
}

impl Filed for EntityHitboxSnapshot {
    fn filing(&self) -> (StepNo, RewindStage) {
        (self.tick, self.phase)
    }
}

/// The newest `capacity` frames. A frame filed under the same tick and phase as the newest one replaces it.
#[derive(Clone, Debug)]
struct Ring<F> {
    frames: VecDeque<F>,
    capacity: usize,
}

impl<F: Filed> Ring<F> {
    fn new(capacity: usize) -> Self {
        Self { frames: VecDeque::new(), capacity: capacity.max(1) }
    }

    fn file(&mut self, entry: F) {
        match self.frames.back_mut() {
            Some(newest) if newest.filing() == entry.filing() => *newest = entry,
            _ => {
                self.frames.push_back(entry);
                let excess = self.frames.len().saturating_sub(self.capacity);
                self.frames.drain(..excess);
            }
        }
    }

    fn at(&self, tick: StepNo) -> Option<&F> {
        self.frames.iter().rev().find(|frame| frame.filing().0 == tick)
    }
}

/// Player poses for the last `capacity` ticks.
#[derive(Clone, Debug)]
pub struct HitboxTimeline {
    ring: Ring<RewindFrame>,
}

impl Default for HitboxTimeline {
    fn default() -> Self {
        Self { ring: Ring::new(HITBOX_TIMELINE_LENGTH) }
    }
}

impl HitboxTimeline {
    pub fn with_capacity(slots: usize) -> Self {
        Self { ring: Ring::new(slots) }
    }

    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    pub fn record_sample(&mut self, incoming: RewindFrame) {
        self.ring.file(incoming);
    }

    /// Files `snapshot` under the tick `at`, as the poses stood after movement.
    pub fn push(&mut self, at: StepNo, snapshot: Vec<PlayerHitboxPose>) {
        self.record_sample(RewindFrame { tick: at, phase: RewindStage::AfterPlayerMove, poses: snapshot });
    }

    pub fn sample_for(&self, wanted: StepNo) -> Option<&RewindFrame> {
        self.ring.at(wanted)
    }

    pub fn latest(&self) -> Option<&RewindFrame> {
        self.ring.frames.back()
    }

    pub fn latest_poses(&self) -> Option<(StepNo, &[PlayerHitboxPose])> {
        self.latest().map(|frame| (frame.tick, frame.poses.as_slice()))
    }

    pub fn clear(&mut self) {
        self.ring.frames.clear();
    }
}

/// Entity collision for the last `capacity` ticks.
#[derive(Clone, Debug)]
pub struct EntityHitboxTimeline {
    ring: Ring<EntityHitboxSnapshot>,
}

impl Default for EntityHitboxTimeline {
    fn default() -> Self {
        Self { ring: Ring::new(HITBOX_TIMELINE_LENGTH) }
    }
}

impl EntityHitboxTimeline {
    pub fn with_capacity(slots: usize) -> Self {
        Self { ring: Ring::new(slots) }
    }

    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    pub fn record_sample(&mut self, incoming: EntityHitboxSnapshot) {
        self.ring.file(incoming);
    }

    pub fn sample_for(&self, wanted: StepNo) -> Option<&EntityHitboxSnapshot> {
        self.ring.at(wanted)
    }

    pub fn latest(&self) -> Option<&EntityHitboxSnapshot> {
        self.ring.frames.back()
    }

    /// The kept frames, oldest first.
    pub fn frames(&self) -> impl Iterator<Item = &EntityHitboxSnapshot> {
        self.ring.frames.iter()
    }

    pub fn clear(&mut self) {
        self.ring.frames.clear();
    }
}
