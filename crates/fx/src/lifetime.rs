//! Short-lived local entities (tracers) and the presentation clock they run on.

use crate::system::FxSystemHost;

/// At most this many local entities live at once; a new one past it drops
/// the oldest.
pub const LOCAL_ENTITY_POOL_CAPACITY: usize = 128;

/// The entity type of a tracer that flies from muzzle to target.
pub const LE_MOVING_TRACER: i32 = 0;

/// Trajectory type for straight-line motion (`entity_t6::tr_type::LINEAR`).
pub const LE_TR_LINEAR: u8 = 3;

/// A tracer that has not yet reached its start is kept if it starts within
/// this many milliseconds.
const START_GRACE_MSEC: i32 = 1000;

/// A time on the presentation clock, in milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FxMsec(pub i32);

impl FxMsec {
    pub fn from_host(host: &FxSystemHost) -> Self {
        Self(host.msec_now)
    }
}

#[derive(Clone, Debug)]
pub struct LocalEntitySlot {
    pub le_type: i32,
    /// Trajectory: start time, kind, duration, per-second delta and base.
    pub pos_tr_time: i32,
    pub pos_tr_type: u8,
    pub pos_tr_duration: i32,
    pub pos_tr_delta: [f32; 3],
    pub pos_tr_base: [f32; 3],
    pub end_time: i32,
    pub material: Option<usize>,
    pub tracer_clip_dist: f32,
    pub beam_length: f32,
    pub beam_width: f32,
    pub screw_dist: f32,
    pub screw_radius: f32,
    pub colors: [[f32; 4]; 5],
    pub own_shot: bool,
    /// A smoke tracer's fade, ending at `end_time`; 0 for one that flies.
    pub fade_msec: i32,
    /// The flight starts on the first frame drawn at or after `pos_tr_time`,
    /// so a player's own tracer waits out the muzzle flash and then leaves
    /// the barrel whole.
    pub starts_when_drawn: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LocalEntityPool {
    live: Vec<LocalEntitySlot>,
}

impl LocalEntityPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn live(&self) -> &[LocalEntitySlot] {
        &self.live
    }

    pub fn live_mut(&mut self) -> &mut [LocalEntitySlot] {
        &mut self.live
    }

    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    pub fn tr_time_min(&self) -> Option<i32> {
        self.live.iter().map(|le| le.pos_tr_time).min()
    }

    pub fn end_time_min(&self) -> Option<i32> {
        self.live.iter().map(|le| le.end_time).min()
    }

    pub fn alloc(&mut self, slot: LocalEntitySlot) {
        if self.live.len() >= LOCAL_ENTITY_POOL_CAPACITY {
            self.live.remove(0);
        }
        self.live.push(slot);
    }

    /// Drops what has ended, keeping what is running and what is about to
    /// start.
    pub fn free_expired(&mut self, clock: FxMsec) {
        self.live.retain(|le| {
            local_entity_is_live(le.pos_tr_time, le.end_time, clock)
                || (le.pos_tr_time > clock.0 && le.pos_tr_time - clock.0 <= START_GRACE_MSEC)
        });
    }
}

/// Running when `tr_time <= clock < end_time`.
#[inline]
pub fn local_entity_is_live(tr_time: i32, end_time: i32, clock: FxMsec) -> bool {
    (tr_time..end_time).contains(&clock.0)
}

/// Milliseconds a tracer takes to cover `dist` at `speed` units per second;
/// `None` when either is not positive or the trip rounds to nothing.
pub fn tracer_travel_msec(dist: f32, speed: f32) -> Option<i32> {
    if dist <= 0.0 || speed <= 0.0 {
        return None;
    }
    let msec = (dist * 1000.0 / speed).round() as i32;
    (msec > 0).then_some(msec)
}

pub fn set_presentation_clock(host: &mut FxSystemHost, clock: FxMsec) {
    host.set_msec_now(clock.0);
}
