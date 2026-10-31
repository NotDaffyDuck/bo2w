//! The numbers behind a steered missile's flight: how fast it turns and how fast it flies.

use crate::SteeredMissile;
use playerstate_t6::game::last_input;

/// The nose may point this far above the ground (degrees of pitch) and no flatter.
pub(super) const PITCH_LIMITS: (f32, f32) = (1.0, 87.0);
/// Degrees a second a full stick turns the nose (pitch, yaw) on the older rule.
const STICK_TURN: (f32, f32) = (15.0, 20.0);
/// Black Ops II turns a missile steered by mouse this fast both ways.
const MOUSE_TURN: f32 = 35.0;

/// Slowest and fastest the missile flies (units a second).
const CRUISE: f32 = 3_000.0;
const BOOSTED: f32 = 6_000.0;
const ACCELERATION: f32 = 2_000.0;
const DECELERATION: f32 = 500.0;
/// Black Ops II counts a boost only if the missile had been at least this much slower than the boosted speed.
const BOOST_NOTICEABLE: f32 = 500.0;
/// A missile that has let its bomblets go drifts on at this speed.
pub(super) const COASTING: f32 = 750.0;

/// How fast the rider turns the nose, as (pitch, yaw) degrees a second at full stick.
pub(super) fn turn_rates(bo2: bool, input_device: u8) -> (f32, f32) {
    if bo2 && input_device != last_input::GAMEPAD {
        (MOUSE_TURN, MOUSE_TURN)
    } else {
        STICK_TURN
    }
}

/// The speed along the nose for the next tick, given the current `speed` and `seconds` to cover.
///
/// Coasting, it drifts (and is no longer armed). With a boost due, it goes full speed; the boost is used up unless Black
/// Ops II would not have felt it. Else it eases back to cruising speed.
pub(super) fn speed_after(link: &mut SteeredMissile, speed: f32, seconds: f32) -> f32 {
    link.armed &= !link.coasting;
    if link.coasting {
        return COASTING;
    }
    if boost_due(link) {
        let felt = BOOSTED - speed > BOOST_NOTICEABLE;
        if felt || !link.t6 {
            link.boosted = true;
        }
        return BOOSTED;
    }
    if speed < CRUISE {
        (speed + ACCELERATION * seconds).min(CRUISE)
    } else {
        (speed - DECELERATION * seconds).max(CRUISE)
    }
}

/// Whether the rider's controls ask for the one boost now. Black Ops II's scripts arm the missile as they link the
/// rider, and holding the control button boosts; the older rule arms it on a release and boosts on fire.
fn boost_due(link: &SteeredMissile) -> bool {
    if link.boosted || !link.armed {
        return false;
    }
    if link.t6 { link.holding } else { link.attack }
}

/// Whether this tick tells everyone to play Black Ops II's boost cue: a boost that counted, or the missile starting to
/// coast while it was still armed. The older rule has none.
pub(super) fn boost_cue_due(before: &SteeredMissile, after: &SteeredMissile) -> bool {
    if !after.t6 {
        return false;
    }
    let boost_counted = after.boosted && !before.boosted;
    let began_coasting_armed = after.coasting && before.armed && !after.armed;
    boost_counted || began_coasting_armed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bo2_link(armed: bool) -> SteeredMissile {
        SteeredMissile {
            t6: true,
            holding: true,
            armed,
            ..Default::default()
        }
    }

    #[test]
    fn mouse_steering_turns_a_bo2_missile_faster_than_a_pad() {
        assert_eq!(turn_rates(true, last_input::MOUSECLICK), (35.0, 35.0));
        assert_eq!(turn_rates(true, last_input::GAMEPAD), STICK_TURN);
        assert_eq!(turn_rates(false, last_input::MOUSECLICK), STICK_TURN);
    }

    #[test]
    fn an_armed_bo2_missile_boosts_once_then_eases_back() {
        let mut link = bo2_link(true);
        assert_eq!(speed_after(&mut link, 1000.0, 0.05), BOOSTED);
        assert!(link.boosted);
        assert_eq!(speed_after(&mut link, 6000.0, 0.05), 5975.0);
    }

    #[test]
    fn a_bo2_missile_that_is_unarmed_or_released_does_not_boost() {
        let mut unarmed = bo2_link(false);
        assert_eq!(speed_after(&mut unarmed, 1000.0, 0.05), 1100.0);
        let mut released = SteeredMissile {
            holding: false,
            ..bo2_link(true)
        };
        assert_eq!(speed_after(&mut released, 1000.0, 0.05), 1100.0);
        assert!(!released.boosted);
    }

    #[test]
    fn a_boost_from_nearly_full_speed_is_not_used_up() {
        let mut link = bo2_link(true);
        assert_eq!(speed_after(&mut link, 5800.0, 0.05), BOOSTED);
        assert!(!link.boosted);
    }

    #[test]
    fn the_older_rule_boosts_on_fire_once_armed() {
        let mut link = SteeredMissile {
            armed: true,
            ..Default::default()
        };
        assert_eq!(speed_after(&mut link, 3000.0, 0.05), 3000.0);
        link = SteeredMissile { attack: true, ..link };
        assert_eq!(speed_after(&mut link, 3000.0, 0.05), BOOSTED);
        assert!(link.boosted);
    }

    #[test]
    fn a_counted_boost_asks_for_the_cue_and_an_uncounted_one_does_not() {
        let before = bo2_link(true);
        let mut counted = before;
        speed_after(&mut counted, 1000.0, 0.05);
        assert!(boost_cue_due(&before, &counted));
        let mut uncounted = before;
        speed_after(&mut uncounted, 5800.0, 0.05);
        assert!(!boost_cue_due(&before, &uncounted));
    }

    #[test]
    fn starting_to_coast_armed_asks_for_the_cue_once() {
        let before = SteeredMissile {
            coasting: true,
            ..bo2_link(true)
        };
        let mut link = before;
        speed_after(&mut link, 3000.0, 0.05);
        assert!(boost_cue_due(&before, &link));
        let settled = link;
        speed_after(&mut link, COASTING, 0.05);
        assert!(!boost_cue_due(&settled, &link));
    }

    #[test]
    fn the_older_rule_never_asks_for_the_cue() {
        let before = SteeredMissile {
            armed: true,
            attack: true,
            ..Default::default()
        };
        let mut link = before;
        speed_after(&mut link, 3000.0, 0.05);
        assert!(link.boosted);
        assert!(!boost_cue_due(&before, &link));
    }

    #[test]
    fn a_coasting_missile_drifts_slowly_and_disarms() {
        let mut link = SteeredMissile {
            coasting: true,
            ..bo2_link(true)
        };
        assert_eq!(speed_after(&mut link, 3000.0, 0.05), COASTING);
        assert!(!link.armed && !link.boosted);
    }
}
