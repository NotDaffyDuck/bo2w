//! Black Ops II's PM_FoliageSounds: the rustle of pushing through bushes.
//! Each frame on the main move, a player going at least 40 units a second
//! tests a box a little smaller than his own (three quarters as wide, nine
//! tenths as tall) against foliage (contents 0x2: foliage clip brushes and
//! the moss, ivy and lily pad meshes); inside it, he rustles
//! (EV_FOLIAGE_SOUND). The rustle repeats every 1.5 s at 40 units a second,
//! down to every 0.5 s from 180 up. Slower than 40 for more than half a
//! second clears the clock, so the next push rustles at once.
//!
//! BO2 reads the five numbers from dvars (`bg_foliagesnd_*`); these are
//! their defaults, which MP never changes.

use crate::bridge::{GroundTraceInput, MoveBounds, add_predictable_event};
use playerstate_t6::game::{PM_TYPE_DEAD, PlayerState};

use crate::gjk::Bo2Collision;

/// EV_FOLIAGE_SOUND.
pub const EV_FOLIAGE_SOUND: i32 = 1;
/// The foliage contents bit.
pub const CONTENTS_FOLIAGE: u32 = 0x2;
/// bg_foliagesnd_minspeed: slower than this, no rustle.
const MIN_SPEED: f32 = 40.0;
/// bg_foliagesnd_maxspeed: from this up, the fastest pace.
const MAX_SPEED: f32 = 180.0;
/// bg_foliagesnd_slowinterval (ms between rustles at the slowest).
const SLOW_INTERVAL_MS: i32 = 1500;
/// bg_foliagesnd_fastinterval (ms between rustles at the fastest).
const FAST_INTERVAL_MS: i32 = 500;
/// bg_foliagesnd_resetinterval: this long under the slowest speed clears
/// the clock.
const RESET_INTERVAL_MS: i32 = 500;

/// After the frame's move and footsteps: rustle if he is going fast enough
/// and his box is in foliage, at the speed's pace.
pub(crate) fn foliage_sounds<C: Bo2Collision>(
    ps: &mut PlayerState,
    server_time: i32,
    bounds: MoveBounds,
    collision: &C,
) {
    // The footsteps' speed, which a dead player never sets.
    let xyspeed = if ps.pm_type >= PM_TYPE_DEAD {
        0.0
    } else {
        libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1])
    };
    if xyspeed < MIN_SPEED {
        if ps.foliage_sound_time.wrapping_add(RESET_INTERVAL_MS) < server_time {
            ps.foliage_sound_time = 0;
        }
        return;
    }
    let frac = ((xyspeed - MIN_SPEED) / (MAX_SPEED - MIN_SPEED)).min(1.0);
    let interval =
        SLOW_INTERVAL_MS + ((FAST_INTERVAL_MS - SLOW_INTERVAL_MS) as f32 * frac) as i32;
    if ps.foliage_sound_time.wrapping_add(interval) >= server_time {
        return;
    }
    let mins = bounds.mins.map(|v| v * 0.75);
    let maxs = [bounds.maxs[0] * 0.75, bounds.maxs[1] * 0.75, bounds.maxs[2] * 0.9];
    let hit = collision.trace(GroundTraceInput {
        start: ps.origin,
        end: ps.origin,
        mins,
        maxs,
        tracemask: CONTENTS_FOLIAGE,
    });
    if hit.startsolid {
        add_predictable_event(ps, EV_FOLIAGE_SOUND, 0);
        ps.foliage_sound_time = server_time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use crate::bridge::CollisionBackend;
    use trace_t6::Trace;

    /// A bush filling x 100..200 (any y, z); records the last box traced.
    struct Bush {
        last: Cell<Option<GroundTraceInput>>,
    }

    impl Bush {
        fn new() -> Self {
            Self { last: Cell::new(None) }
        }
    }

    impl Bo2Collision for Bush {}

    impl CollisionBackend for Bush {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            self.last.set(Some(input));
            let lo = input.start[0] + input.mins[0];
            let hi = input.start[0] + input.maxs[0];
            let inside = input.tracemask & CONTENTS_FOLIAGE != 0 && hi > 100.0 && lo < 200.0;
            Trace {
                fraction: if inside { 0.0 } else { 1.0 },
                startsolid: inside,
                allsolid: inside,
                ..Trace::default()
            }
        }
    }

    const STAND: MoveBounds = MoveBounds {
        mins: [-15.0, -15.0, 0.0],
        maxs: [15.0, 15.0, 70.0],
        tracemask: 0x0281_0011,
    };

    fn runner(x: f32, speed: f32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.origin = [x, 0.0, 0.0];
        ps.velocity = [speed, 0.0, 0.0];
        ps
    }

    fn rustles(ps: &PlayerState) -> i32 {
        i32::from(ps.predictable_event_sequence)
    }

    #[test]
    fn running_through_a_bush_rustles_every_half_second() {
        let bush = Bush::new();
        let mut ps = runner(150.0, 250.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        assert_eq!(rustles(&ps), 1);
        assert_eq!(ps.predictable_events[0], EV_FOLIAGE_SOUND);
        assert_eq!(ps.predictable_event_parms[0], 0);
        assert_eq!(ps.foliage_sound_time, 10_000);
        foliage_sounds(&mut ps, 10_500, STAND, &bush);
        assert_eq!(rustles(&ps), 1, "not before the half second is over");
        foliage_sounds(&mut ps, 10_501, STAND, &bush);
        assert_eq!(rustles(&ps), 2);
    }

    #[test]
    fn a_slow_walk_rustles_every_one_and_a_half_seconds() {
        let bush = Bush::new();
        let mut ps = runner(150.0, 40.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        foliage_sounds(&mut ps, 11_400, STAND, &bush);
        assert_eq!(rustles(&ps), 1);
        foliage_sounds(&mut ps, 11_501, STAND, &bush);
        assert_eq!(rustles(&ps), 2);
        // Halfway between the speeds, halfway between the paces.
        let mut ps = runner(150.0, 110.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        foliage_sounds(&mut ps, 11_000, STAND, &bush);
        assert_eq!(rustles(&ps), 1);
        foliage_sounds(&mut ps, 11_001, STAND, &bush);
        assert_eq!(rustles(&ps), 2);
    }

    #[test]
    fn the_box_is_smaller_than_him_and_looks_for_foliage_only() {
        let bush = Bush::new();
        let mut ps = runner(150.0, 250.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        let traced = bush.last.get().expect("traced");
        assert_eq!(traced.start, traced.end);
        assert_eq!(traced.mins, [-11.25, -11.25, 0.0]);
        assert_eq!(traced.maxs, [11.25, 11.25, 63.0]);
        assert_eq!(traced.tracemask, CONTENTS_FOLIAGE);
        // Out of the bush: the edge of his own box would touch, the smaller
        // one does not.
        let mut ps = runner(88.0, 250.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        assert_eq!(rustles(&ps), 0);
        assert_eq!(ps.foliage_sound_time, 0);
    }

    #[test]
    fn standing_still_clears_the_clock_after_half_a_second() {
        let bush = Bush::new();
        let mut ps = runner(150.0, 250.0);
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        ps.velocity = [30.0, 0.0, 0.0];
        foliage_sounds(&mut ps, 10_400, STAND, &bush);
        assert_eq!(ps.foliage_sound_time, 10_000);
        foliage_sounds(&mut ps, 10_501, STAND, &bush);
        assert_eq!(ps.foliage_sound_time, 0);
        // The next push rustles at once.
        ps.velocity = [40.0, 0.0, 0.0];
        foliage_sounds(&mut ps, 10_550, STAND, &bush);
        assert_eq!(rustles(&ps), 2);
        assert!(bush.last.get().is_some());
    }

    #[test]
    fn the_dead_never_rustle() {
        let bush = Bush::new();
        let mut ps = runner(150.0, 250.0);
        ps.pm_type = PM_TYPE_DEAD;
        foliage_sounds(&mut ps, 10_000, STAND, &bush);
        assert_eq!(rustles(&ps), 0);
    }
}
