//! Short-lived local entities (tracers) and the presentation clock they run on.

use crate::host::EffectSystemHost;

/// At most this many local entities live at once; a new one past it drops
/// the oldest.
pub const CLIENT_THING_LIMIT: usize = 128;

/// The entity type of a tracer that flies from muzzle to target.
pub const THING_FLYING_STREAK: i32 = 0;

/// Trajectory type for straight-line motion (`entity_t6::tr_type::LINEAR`).
pub const PATH_STRAIGHT_LINE: u8 = 3;

/// A tracer that has not yet reached its start is kept if it starts within
/// this many milliseconds.
const START_GRACE_MSEC: i32 = 1000;

/// A time on the presentation clock, in milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EffectClockMs(pub i32);

impl EffectClockMs {
    pub fn from_host(host: &EffectSystemHost) -> Self {
        Self(host.clock_ms_now)
    }
}

#[derive(Clone, Debug)]
pub struct ClientOnlyThingSlot {
    pub thing_category: i32,
    /// Trajectory: start time, kind, duration, per-second delta and base.
    pub path_start_ms: i32,
    pub pos_tr_type: u8,
    pub path_length_ms: i32,
    pub path_drift: [f32; 3],
    pub path_start_point: [f32; 3],
    pub finish_stamp: i32,
    pub shader_ix: Option<usize>,
    pub tracer_cut: f32,
    pub ray_extent: f32,
    pub ray_thickness: f32,
    pub spiral_pitch_distance: f32,
    pub spiral_radius: f32,
    pub colors: [[f32; 4]; 5],
    pub fired_by_me: bool,
    /// A smoke tracer's fade, ending at `finish_stamp`; 0 for one that flies.
    pub fade_msec: i32,
    /// The flight starts on the first frame drawn at or after `path_start_ms`,
    /// so a player's own tracer waits out the muzzle flash and then leaves
    /// the barrel whole.
    pub starts_when_drawn: bool,
}

/// The live local entities, oldest first.
#[derive(Clone, Debug, Default)]
pub struct ClientOnlyThingBank(Vec<ClientOnlyThingSlot>);

impl ClientOnlyThingBank {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn live(&self) -> &[ClientOnlyThingSlot] {
        self.0.as_slice()
    }

    pub fn live_mut(&mut self) -> &mut [ClientOnlyThingSlot] {
        self.0.as_mut_slice()
    }

    pub fn active_total(&self) -> usize {
        self.live().len()
    }

    /// The earliest of one time field over the live entities.
    fn earliest(&self, field: fn(&ClientOnlyThingSlot) -> i32) -> Option<i32> {
        self.live().iter().map(field).min()
    }

    pub fn tracer_min_ms(&self) -> Option<i32> {
        self.earliest(|le| le.path_start_ms)
    }

    pub fn soonest_finish(&self) -> Option<i32> {
        self.earliest(|le| le.finish_stamp)
    }

    /// Adds an entity; when the pool is full the oldest one makes room.
    pub fn claim_slot(&mut self, slot: ClientOnlyThingSlot) {
        let over = (self.0.len() + 1).saturating_sub(CLIENT_THING_LIMIT);
        self.0.drain(..over);
        self.0.push(slot);
    }

    /// Drops what has ended, keeping what is running and what is about to
    /// start.
    pub fn reclaim_finished(&mut self, clock: EffectClockMs) {
        let now = clock.0;
        let starts_soon = |start: i32| start > now && start - now <= START_GRACE_MSEC;
        self.0
            .retain(|le| client_thing_alive(le.path_start_ms, le.finish_stamp, clock) || starts_soon(le.path_start_ms));
    }
}

/// Running when `tr_time <= clock < finish_stamp`.
#[inline]
pub fn client_thing_alive(tr_time: i32, finish_stamp: i32, clock: EffectClockMs) -> bool {
    (tr_time..finish_stamp).contains(&clock.0)
}

/// Milliseconds a tracer takes to cover `dist` at `speed` units per second;
/// `None` when either is not positive or the trip rounds to nothing.
pub fn tracer_flight_ms(span: f32, pace: f32) -> Option<i32> {
    let usable = span > 0.0 && pace > 0.0;
    usable
        .then(|| (span * 1000.0 / pace).round() as i32)
        .filter(|&msec| msec > 0)
}

pub fn set_show_clock(host: &mut EffectSystemHost, clock: EffectClockMs) {
    host.store_effect_clock(clock.0);
}
