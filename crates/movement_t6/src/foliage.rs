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

use crate::bridge::{SweepAsk, HullExtents, queue_forecast_happening};
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::AvatarState;

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
    avatar: &mut AvatarState,
    authority_clock_ms: i32,
    bounds: HullExtents,
    solids: &C,
) {
    // The footsteps' speed, which a dead player never sets.
    let ground_velocity = if avatar.movement_mode >= MOVE_KIND_DEAD {
        0.0
    } else {
        libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1])
    };
    if ground_velocity < MIN_SPEED {
        if avatar.foliage_sound_time.wrapping_add(RESET_INTERVAL_MS) < authority_clock_ms {
            avatar.foliage_sound_time = 0;
        }
        return;
    }
    let frac = ((ground_velocity - MIN_SPEED) / (MAX_SPEED - MIN_SPEED)).min(1.0);
    let interval =
        SLOW_INTERVAL_MS + ((FAST_INTERVAL_MS - SLOW_INTERVAL_MS) as f32 * frac) as i32;
    if avatar.foliage_sound_time.wrapping_add(interval) >= authority_clock_ms {
        return;
    }
    let box_lo = bounds.lo_corner.map(|v| v * 0.75);
    let box_hi = [bounds.hi_corner[0] * 0.75, bounds.hi_corner[1] * 0.75, bounds.hi_corner[2] * 0.9];
    let strike = solids.trace(SweepAsk {
        start: avatar.origin,
        end: avatar.origin,
        lo_corner: box_lo,
        hi_corner: box_hi,
        collide_filter: CONTENTS_FOLIAGE,
    });
    if strike.starts_inside {
        queue_forecast_happening(avatar, EV_FOLIAGE_SOUND, 0);
        avatar.foliage_sound_time = authority_clock_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use crate::bridge::SolidProbe;
    use trace_t6::Trace;

    /// A bush filling x 100..200 (any y, z); records the last box traced.
    struct Bush {
        last: Cell<Option<SweepAsk>>,
    }

    impl Bush {
        fn new() -> Self {
            Self { last: Cell::new(None) }
        }
    }

    impl Bo2Collision for Bush {}

    impl SolidProbe for Bush {
        fn trace(&self, input: SweepAsk) -> Trace {
            self.last.set(Some(input));
            let lo = input.start[0] + input.lo_corner[0];
            let hi = input.start[0] + input.hi_corner[0];
            let inside = input.collide_filter & CONTENTS_FOLIAGE != 0 && hi > 100.0 && lo < 200.0;
            if !inside {
                return Trace::new();
            }
            Trace { fraction: 0.0, starts_inside: true, fully_embedded: true, ..Trace::new() }
        }
    }

    const UPRIGHT: HullExtents = HullExtents {
        lo_corner: [-15.0_f32, -15.0, 0.0],
        hi_corner: [15.0_f32, 15.0, 70.0],
        collide_filter: 0x0281_0011,
    };

    fn runner(x: f32, speed: f32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.origin = [x, 0.0, 0.0];
        avatar.velocity = [speed, 0.0, 0.0];
        avatar
    }

    fn rustles(avatar: &AvatarState) -> i32 {
        i32::from(avatar.predictable_event_sequence)
    }

    #[test]
    fn running_through_a_bush_rustles_every_half_second() {
        let bush = Bush::new();
        let mut avatar = runner(150.0, 250.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 1);
        assert_eq!(avatar.predictable_events[0], EV_FOLIAGE_SOUND);
        assert_eq!(avatar.predictable_event_parms[0], 0);
        assert_eq!(avatar.foliage_sound_time, 10_000);
        foliage_sounds(&mut avatar, 10_500, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 1, "not before the half second is over");
        foliage_sounds(&mut avatar, 10_501, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 2);
    }

    #[test]
    fn a_slow_walk_rustles_every_one_and_a_half_seconds() {
        let bush = Bush::new();
        let mut avatar = runner(150.0, 40.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        foliage_sounds(&mut avatar, 11_400, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 1);
        foliage_sounds(&mut avatar, 11_501, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 2);
        // Halfway between the speeds, halfway between the paces.
        let mut avatar = runner(150.0, 110.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        foliage_sounds(&mut avatar, 11_000, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 1);
        foliage_sounds(&mut avatar, 11_001, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 2);
    }

    #[test]
    fn the_box_is_smaller_than_him_and_looks_for_foliage_only() {
        let bush = Bush::new();
        let mut avatar = runner(150.0, 250.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        let traced = bush.last.get().expect("traced");
        assert_eq!(traced.start, traced.end);
        assert_eq!(traced.lo_corner, [-11.25, -11.25, 0.0]);
        assert_eq!(traced.hi_corner, [11.25, 11.25, 63.0]);
        assert_eq!(traced.collide_filter, CONTENTS_FOLIAGE);
        // Out of the bush: the edge of his own box would touch, the smaller
        // one does not.
        let mut avatar = runner(88.0, 250.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 0);
        assert_eq!(avatar.foliage_sound_time, 0);
    }

    #[test]
    fn standing_still_clears_the_clock_after_half_a_second() {
        let bush = Bush::new();
        let mut avatar = runner(150.0, 250.0);
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        avatar.velocity = [30.0, 0.0, 0.0];
        foliage_sounds(&mut avatar, 10_400, UPRIGHT, &bush);
        assert_eq!(avatar.foliage_sound_time, 10_000);
        foliage_sounds(&mut avatar, 10_501, UPRIGHT, &bush);
        assert_eq!(avatar.foliage_sound_time, 0);
        // The next push rustles at once.
        avatar.velocity = [40.0, 0.0, 0.0];
        foliage_sounds(&mut avatar, 10_550, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 2);
        assert!(bush.last.get().is_some());
    }

    #[test]
    fn the_dead_never_rustle() {
        let bush = Bush::new();
        let mut avatar = runner(150.0, 250.0);
        avatar.movement_mode = MOVE_KIND_DEAD;
        foliage_sounds(&mut avatar, 10_000, UPRIGHT, &bush);
        assert_eq!(rustles(&avatar), 0);
    }
}
