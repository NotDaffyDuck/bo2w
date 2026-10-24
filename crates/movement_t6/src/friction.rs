//! Black Ops II's PM_Friction: how the ground (and sliding ground, a melee
//! charge, water, spectating) slows the player each frame. More than waist
//! deep (water level 2 and up) the ground's own grip and the sliding
//! ground's are gone, and standing in water grips by its level instead.

use crate::bridge::{Bo2Lunge, Pml, bo2_lunge_friction};
use playerstate_t6::game::{PlayerState, pm_flags};

/// stopspeed: below this the ground stops you as if you were going this fast.
const STOP_SPEED: f32 = 100.0;

/// The ground's friction (BG_GetFriction).
pub(crate) const GROUND_FRICTION: f32 = 5.5;

/// Spectators slow down this much times their speed each second.
const SPECTATOR_FRICTION: f32 = 5.0;

/// Flags that skip the ground's grip: the knockback timer (0x100) and the
/// forced slick flag.
const NO_GRIP_FLAGS: u32 = 0x4000_0100;

/// The hard-landing stun (pm_flags 0x80).
const TIME_HARDLANDING: u32 = 0x80;

/// Sliding ground (the ground trace's surface flag 2).
pub(crate) const SURF_SLIDE: u32 = 2;

pub(crate) fn friction(ps: &mut PlayerState, pml: &Pml, jump_slowdown: bool, lunge: Option<Bo2Lunge>) {
    friction_scaled(ps, pml, jump_slowdown, lunge, |_| 1.0);
}

/// `ground_scale` scales the ground's grip (a dive's slide, BO2's
/// Dtp_ReduceFriction); it is asked only when that grip applies, after a hard
/// landing's or a jump's own scale. `jump_slowdown`: a landed jump grips
/// harder for a moment (jump_slowdownEnable). `lunge`: the knife lunge's
/// target, when the game plays BO2's COD4-style lunge.
pub(crate) fn friction_scaled(
    ps: &mut PlayerState,
    pml: &Pml,
    jump_slowdown: bool,
    lunge: Option<Bo2Lunge>,
    ground_scale: impl FnOnce(&mut PlayerState) -> f32,
) {
    // A knife lunge with player_meleeCOD4ChargeMovement on (bo2mp's) slows
    // by the lunge's own rule (PM_GetMeleeChargeFriction) and skips the
    // ground's grip.
    if let Some(lunge) = lunge
        && ps.pm_flags & pm_flags::MELEE_CHARGE != 0
    {
        // Water's grip comes on top, from the speed before the lunge's.
        let before = speed(ps, pml);
        bo2_lunge_friction(ps, pml, lunge);
        let water = crate::water::friction_drop(ps, pml, before);
        let left = speed(ps, pml);
        if water > 0.0 && left > 0.0 {
            let scale = (left - water).max(0.0) / left;
            ps.velocity = ps.velocity.map(|v| v * scale);
        }
        return;
    }

    let speed = speed(ps, pml);
    if speed < 1.0 {
        ps.velocity = [0.0; 3];
        return;
    }

    let flags = ps.pm_flags;
    let sliding = pml.ground_trace[4] & SURF_SLIDE != 0;
    let mut drop = 0.0_f32;
    if flags & pm_flags::MELEE_CHARGE != 0 {
        // The charge's own time to stop. Before that time is set (0) the
        // drop is endless and the player stops, as in BO2.
        drop = speed / (ps.melee_charge_time as f32 * 0.001) * pml.frametime;
    } else if ps.waterlevel <= 1 {
        if pml.walking != 0 && flags & NO_GRIP_FLAGS == 0 && !sliding {
            let mut control = speed.max(STOP_SPEED);
            if flags & TIME_HARDLANDING != 0 {
                control *= crate::feel::HARD_LANDING_FRICTION_SCALE;
            } else if flags & pm_flags::JUMPING != 0 {
                control *= walking_scale(ps, jump_slowdown);
            }
            control *= ground_scale(ps);
            drop = GROUND_FRICTION * control * pml.frametime;
        }
        if sliding {
            drop += crate::feel::SLIDING_FRICTION * speed * pml.frametime;
        }
    }
    drop += crate::water::friction_drop(ps, pml, speed);

    if ps.pm_type == 5 {
        drop += speed * SPECTATOR_FRICTION * pml.frametime;
    }

    let new_speed = (speed - drop).max(0.0);
    let scale = new_speed / speed;
    ps.velocity[0] *= scale;
    ps.velocity[1] *= scale;
    ps.velocity[2] *= scale;
}

/// The speed friction works on: walking, only the speed along the ground.
fn speed(ps: &PlayerState, pml: &Pml) -> f32 {
    let z = if pml.walking != 0 { 0.0 } else { ps.velocity[2] };
    libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1] + z * z)
}

/// Jump_ReduceFriction: just after landing a jump the ground grips harder
/// (up to 2.5 times), easing back over the landing's time. Once that time
/// is up the jump is over.
fn walking_scale(ps: &mut PlayerState, jump_slowdown: bool) -> f32 {
    if ps.pm_time < 0x709 {
        if jump_slowdown {
            if ps.pm_time > 0x6a3 {
                return 2.5;
            }
            return ps.pm_time as f32 * 1.5 * 0.0005882352706976235 + 1.0;
        }
    } else {
        ps.pm_flags &= !pm_flags::JUMPING;
        ps.jump_origin_z = 0.0;
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pml(walking: bool, surface: u32) -> Pml {
        let mut ground_trace = [0; 11];
        ground_trace[4] = surface;
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: 0.05,
            msec: 50,
            walking: u32::from(walking),
            ground_plane: u32::from(walking),
            almost_ground_plane: u32::from(walking),
            ground_trace,
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }

    #[test]
    fn walking_friction_matches_bo2() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [200.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        // 200 - 5.5 x 200 x 0.05 = 145.
        assert!((ps.velocity[0] - 145.0).abs() < 1e-3, "{:?}", ps.velocity);
    }

    #[test]
    fn slow_speeds_grip_as_if_at_stopspeed() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [50.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        // 50 - 5.5 x 100 x 0.05 = 22.5 (at its own speed it would keep 36.25).
        assert!((ps.velocity[0] - 22.5).abs() < 1e-3, "{:?}", ps.velocity);
        ps.velocity = [20.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn sliding_ground_only_drags() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [200.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, SURF_SLIDE), true, None);
        // 200 - 1.5 x 200 x 0.05 = 185.
        assert!((ps.velocity[0] - 185.0).abs() < 1e-3, "{:?}", ps.velocity);
    }

    #[test]
    fn forced_slick_has_no_grip() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = 0x4000_0000;
        ps.velocity = [200.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        assert_eq!(ps.velocity[0], 200.0);
    }

    #[test]
    fn water_takes_over_from_the_ground() {
        // Ankle deep: the ground's grip and the water's (level 1) both.
        let mut ps = PlayerState::ZERO;
        ps.waterlevel = 1;
        ps.velocity = [200.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        // 200 - 5.5 x 200 x 0.05 - 5.5 x 200 x 1 x 0.05 = 90.
        assert!((ps.velocity[0] - 90.0).abs() < 1e-3, "{:?}", ps.velocity);
        // Waist deep: only the water's, 1.7 times; sliding ground no longer drags.
        ps.waterlevel = 2;
        ps.velocity = [200.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, SURF_SLIDE), true, None);
        // 200 - 5.5 x 200 x 1.7 x 0.05 = 106.5.
        assert!((ps.velocity[0] - 106.5).abs() < 1e-3, "{:?}", ps.velocity);
        // Swimming off the ground nothing slows the player.
        ps.waterlevel = 5;
        ps.velocity = [200.0, 0.0, 50.0];
        friction(&mut ps, &pml(false, 0), true, None);
        assert_eq!(ps.velocity, [200.0, 0.0, 50.0]);
    }

    #[test]
    fn walking_ignores_the_fall_speed() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [0.5, 0.0, -300.0];
        friction(&mut ps, &pml(true, 0), true, None);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn a_landed_jump_grips_harder() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.pm_time = 0x708;
        ps.velocity = [400.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        // 400 - 5.5 x 400 x 2.5 x 0.05 = 125.
        assert!((ps.velocity[0] - 125.0).abs() < 1e-2, "{:?}", ps.velocity);
    }

    #[test]
    fn a_charge_stops_in_its_own_time() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::MELEE_CHARGE;
        ps.melee_charge_time = 200;
        ps.velocity = [400.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, None);
        // 400 - 400 / 0.2 x 0.05 = 300.
        assert!((ps.velocity[0] - 300.0).abs() < 1e-2, "{:?}", ps.velocity);
    }

    #[test]
    fn a_lunge_slows_by_its_own_rule_once() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::MELEE_CHARGE;
        ps.velocity = [400.0, 0.0, 0.0];
        friction(&mut ps, &pml(true, 0), true, Some(Bo2Lunge::default()));
        let want = 400.0 - crate::bridge::BO2_CHARGE_FRICTION * 0.05;
        assert!((ps.velocity[0] - want).abs() < 1e-2, "{:?}", ps.velocity);
    }
}
