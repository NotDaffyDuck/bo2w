//! Stored collision frames, for rewinding a shot to what its shooter saw.

use std::collections::VecDeque;

use super::COLLISION_HISTORY_TICKS;
use super::entity_caps::EntityCollisionTraceGeom;
use super::poses::PlayerCollisionPose;
use crate::world::Tick;

/// Which part of the tick a frame was taken in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryPhase {
    #[default]
    PostMovement,
}

/// Why a shot was judged against the present although it named a past tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrentAuthorityReason {
    NoSampleClaim,
    SampleAtShot,
    HistoryUnavailable { requested: Tick },
}

/// Why a rewind stopped short of the tick asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryClampReason {
    MaxRewindWindow,
}

/// Why a shot's claim was turned down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryRefusalReason {
    MissingFrame,
    SampleAfterShot,
    SampleMalformed,
}

/// What became of a request to rewind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySampleVerdict {
    CurrentAuthority { tick: Tick, reason: CurrentAuthorityReason },
    Exact { requested: Tick, frame: Tick, phase: HistoryPhase },
    Clamped { requested: Tick, used: Tick, frame: Tick, phase: HistoryPhase, reason: HistoryClampReason },
    Refused { requested: Tick, reason: HistoryRefusalReason },
}

/// Every player's collision pose at one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryFrame {
    pub tick: Tick,
    pub phase: HistoryPhase,
    pub poses: Vec<PlayerCollisionPose>,
}

/// The players a rewound shot is tested against, and how they were chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct HistorySample {
    pub poses: Vec<PlayerCollisionPose>,
    pub verdict: HistorySampleVerdict,
}

/// Every scripted entity's collision at one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionFrame {
    pub tick: Tick,
    pub phase: HistoryPhase,
    pub rows: Vec<EntityCollisionTraceGeom>,
}

/// The entities a rewound shot is tested against, and how they were chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityCollisionSample {
    pub rows: Vec<EntityCollisionTraceGeom>,
    pub verdict: HistorySampleVerdict,
}

/// Players and entities for one shot, taken from the same moment.
#[derive(Clone, Debug, PartialEq)]
pub struct LagcompQuery {
    pub players: HistorySample,
    pub entities: EntityCollisionSample,
}

/// A frame that is filed under a tick and phase.
trait Filed {
    fn filing(&self) -> (Tick, HistoryPhase);
}

impl Filed for HistoryFrame {
    fn filing(&self) -> (Tick, HistoryPhase) {
        (self.tick, self.phase)
    }
}

impl Filed for EntityCollisionFrame {
    fn filing(&self) -> (Tick, HistoryPhase) {
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

    fn file(&mut self, frame: F) {
        match self.frames.back_mut() {
            Some(newest) if newest.filing() == frame.filing() => *newest = frame,
            _ => {
                self.frames.push_back(frame);
                while self.frames.len() > self.capacity {
                    self.frames.pop_front();
                }
            }
        }
    }

    fn at(&self, tick: Tick) -> Option<&F> {
        self.frames.iter().rev().find(|frame| frame.filing().0 == tick)
    }
}

/// Player poses for the last `capacity` ticks.
#[derive(Clone, Debug)]
pub struct CollisionHistory {
    ring: Ring<HistoryFrame>,
}

impl Default for CollisionHistory {
    fn default() -> Self {
        Self::with_capacity(COLLISION_HISTORY_TICKS)
    }
}

impl CollisionHistory {
    pub fn with_capacity(capacity: usize) -> Self {
        Self { ring: Ring::new(capacity) }
    }

    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    pub fn push_frame(&mut self, frame: HistoryFrame) {
        self.ring.file(frame);
    }

    /// Files `poses` under `tick`, taken after movement.
    pub fn push(&mut self, tick: Tick, poses: Vec<PlayerCollisionPose>) {
        self.push_frame(HistoryFrame { tick, phase: HistoryPhase::PostMovement, poses });
    }

    pub fn frame_at(&self, tick: Tick) -> Option<&HistoryFrame> {
        self.ring.at(tick)
    }

    pub fn latest(&self) -> Option<&HistoryFrame> {
        self.ring.frames.back()
    }

    pub fn latest_poses(&self) -> Option<(Tick, &[PlayerCollisionPose])> {
        self.latest().map(|frame| (frame.tick, frame.poses.as_slice()))
    }

    pub fn clear(&mut self) {
        self.ring.frames.clear();
    }
}

/// Entity collision for the last `capacity` ticks.
#[derive(Clone, Debug)]
pub struct EntityCollisionHistory {
    ring: Ring<EntityCollisionFrame>,
}

impl Default for EntityCollisionHistory {
    fn default() -> Self {
        Self::with_capacity(COLLISION_HISTORY_TICKS)
    }
}

impl EntityCollisionHistory {
    pub fn with_capacity(capacity: usize) -> Self {
        Self { ring: Ring::new(capacity) }
    }

    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    pub fn push_frame(&mut self, frame: EntityCollisionFrame) {
        self.ring.file(frame);
    }

    pub fn frame_at(&self, tick: Tick) -> Option<&EntityCollisionFrame> {
        self.ring.at(tick)
    }

    pub fn latest(&self) -> Option<&EntityCollisionFrame> {
        self.ring.frames.back()
    }

    pub fn clear(&mut self) {
        self.ring.frames.clear();
    }
}
