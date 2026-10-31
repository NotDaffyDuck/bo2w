//! Black Ops II's PM_Friction: how the ground (and sliding ground, a melee
//! charge, water, spectating) slows the player each frame. More than waist
//! deep (water level 2 and up) the ground's own grip and the sliding
//! ground's are gone, and standing in water grips by its level instead.

use crate::bridge::{Bo2Lunge, TickLocals, bo2_lunge_friction};
use playerstate_t6::game::{AvatarState, movement_state_bits};

/// stopspeed: below this the ground stops you as if you were going this fast.
const STOP_SPEED: f32 = 100.0;

/// The ground's friction (BG_GetFriction).
pub(crate) const GROUND_FRICTION: f32 = 5.5;

/// Spectators slow down this much times their speed each second.
const SPECTATOR_FRICTION: f32 = 5.0;

/// Flags that skip the ground's grip: the knockback timer (0x100) and the
/// forced slick flag.
const NO_GRIP_FLAGS: u32 = 0x4000_0100;

/// The hard-landing stun (movement_state_bits 0x80).
const TIME_HARDLANDING: u32 = 0x80;

/// Sliding ground (the ground trace's surface flag 2).
pub(crate) const SURF_SLIDE: u32 = 2;

pub(crate) fn friction(avatar: &mut AvatarState, tick_vars: &TickLocals, jump_slowdown: bool, lunge: Option<Bo2Lunge>) {
    friction_scaled(avatar, tick_vars, jump_slowdown, lunge, |_| 1.0);
}

/// `ground_scale` scales the ground's grip (a dive's slide, BO2's
/// Dtp_ReduceFriction); it is asked only when that grip applies, after a hard
/// landing's or a jump's own scale. `jump_slowdown`: a landed jump grips
/// harder for a moment (jump_slowdownEnable). `lunge`: the knife lunge's
/// target, when the game plays BO2's COD4-style lunge.
pub(crate) fn friction_scaled(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    jump_slowdown: bool,
    lunge: Option<Bo2Lunge>,
    ground_scale: impl FnOnce(&mut AvatarState) -> f32,
) {
    // A knife lunge with player_meleeCOD4ChargeMovement on (bo2mp's) slows
    // by the lunge's own rule (PM_GetMeleeChargeFriction) and skips the
    // ground's grip.
    if let Some(lunge) = lunge
        && avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE != 0
    {
        // Water's grip comes on top, from the speed before the lunge's.
        let before = speed(avatar, tick_vars);
        bo2_lunge_friction(avatar, tick_vars, lunge);
        let water = crate::water::friction_drop(avatar, tick_vars, before);
        let left = speed(avatar, tick_vars);
        if water > 0.0 && left > 0.0 {
            let scale = (left - water).max(0.0) / left;
            avatar.velocity = avatar.velocity.map(|v| v * scale);
        }
        return;
    }

    let speed = speed(avatar, tick_vars);
    if 1.0 > speed {
        avatar.velocity = Default::default();
        return;
    }

    let flags = avatar.movement_state_bits;
    let sliding = tick_vars.floor_probe[4] & SURF_SLIDE != 0;
    let mut shed = 0.0_f32;
    if flags & movement_state_bits::MELEE_CHARGE != 0 {
        // The charge's own time to stop. Before that time is set (0) the
        // drop is endless and the player stops, as in BO2.
        shed = speed / (avatar.lunge_clock as f32 * 0.001) * tick_vars.step_millis_elapsed;
    } else if avatar.waterlevel <= 1 {
        if tick_vars.walking != 0 && flags & NO_GRIP_FLAGS == 0 && !sliding {
            let mut control = speed.max(STOP_SPEED);
            if flags & TIME_HARDLANDING != 0 {
                control *= crate::feel::HARD_LANDING_FRICTION_SCALE;
            } else if flags & movement_state_bits::JUMPING != 0 {
                control *= walking_scale(avatar, jump_slowdown);
            }
            control *= ground_scale(avatar);
            shed = GROUND_FRICTION * control * tick_vars.step_millis_elapsed;
        }
        if sliding {
            shed += crate::feel::SLIDING_FRICTION * speed * tick_vars.step_millis_elapsed;
        }
    }
    shed += crate::water::friction_drop(avatar, tick_vars, speed);

    if avatar.movement_mode == 5 {
        shed += speed * SPECTATOR_FRICTION * tick_vars.step_millis_elapsed;
    }

    let updated_pace = (speed - shed).max(0.0);
    let scale = updated_pace / speed;
    avatar.velocity = avatar.velocity.map(|axis| axis * scale);
}

/// The speed friction works on: walking, only the speed along the ground.
fn speed(avatar: &AvatarState, tick_vars: &TickLocals) -> f32 {
    let z = if tick_vars.walking != 0 { 0.0 } else { avatar.velocity[2] };
    libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1] + z * z)
}

/// Jump_ReduceFriction: just after landing a jump the ground grips harder
/// (up to 2.5 times), easing back over the landing's time. Once that time
/// is up the jump is over.
fn walking_scale(avatar: &mut AvatarState, jump_slowdown: bool) -> f32 {
    if avatar.move_timer < 0x709 {
        if jump_slowdown {
            if avatar.move_timer > 0x6a3 {
                return 2.5;
            }
            return avatar.move_timer as f32 * 1.5 * 0.0005882352706976235 + 1.0;
        }
    } else {
        avatar.movement_state_bits &= !movement_state_bits::JUMPING;
        avatar.takeoff_height = 0.0;
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_vars(walking: bool, surface: u32) -> TickLocals {
        let mut floor_probe = [0; 11];
        floor_probe[4] = surface;
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: u32::from(walking),
            standing_on_floor: u32::from(walking),
            near_floor_contact: u32::from(walking),
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    #[test]
    fn walking_friction_matches_bo2() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [200.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        // 200 - 5.5 x 200 x 0.05 = 145.
        assert!((avatar.velocity[0] - 145.0).abs() < 1e-3, "{:?}", avatar.velocity);
    }

    #[test]
    fn slow_speeds_grip_as_if_at_stopspeed() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [50.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        // 50 - 5.5 x 100 x 0.05 = 22.5 (at its own speed it would keep 36.25).
        assert!((avatar.velocity[0] - 22.5).abs() < 1e-3, "{:?}", avatar.velocity);
        avatar.velocity = [20.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        assert_eq!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn sliding_ground_only_drags() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [200.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, SURF_SLIDE), true, None);
        // 200 - 1.5 x 200 x 0.05 = 185.
        assert!((avatar.velocity[0] - 185.0).abs() < 1e-3, "{:?}", avatar.velocity);
    }

    #[test]
    fn forced_slick_has_no_grip() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = 0x4000_0000;
        avatar.velocity = [200.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        assert_eq!(avatar.velocity[0], 200.0);
    }

    #[test]
    fn water_takes_over_from_the_ground() {
        // Ankle deep: the ground's grip and the water's (level 1) both.
        let mut avatar = AvatarState::ZERO;
        avatar.waterlevel = 1;
        avatar.velocity = [200.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        // 200 - 5.5 x 200 x 0.05 - 5.5 x 200 x 1 x 0.05 = 90.
        assert!((avatar.velocity[0] - 90.0).abs() < 1e-3, "{:?}", avatar.velocity);
        // Waist deep: only the water's, 1.7 times; sliding ground no longer drags.
        avatar.waterlevel = 2;
        avatar.velocity = [200.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, SURF_SLIDE), true, None);
        // 200 - 5.5 x 200 x 1.7 x 0.05 = 106.5.
        assert!((avatar.velocity[0] - 106.5).abs() < 1e-3, "{:?}", avatar.velocity);
        // Swimming off the ground nothing slows the player.
        avatar.waterlevel = 5;
        avatar.velocity = [200.0, 0.0, 50.0];
        friction(&mut avatar, &tick_vars(false, 0), true, None);
        assert_eq!(avatar.velocity, [200.0, 0.0, 50.0]);
    }

    #[test]
    fn walking_ignores_the_fall_speed() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [0.5, 0.0, -300.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        assert_eq!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn a_landed_jump_grips_harder() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.move_timer = 1_800;
        avatar.velocity = [400.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        // 400 - 5.5 x 400 x 2.5 x 0.05 = 125.
        assert!((avatar.velocity[0] - 125.0).abs() < 1e-2, "{:?}", avatar.velocity);
    }

    #[test]
    fn a_charge_stops_in_its_own_time() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::MELEE_CHARGE;
        avatar.lunge_clock = 200;
        avatar.velocity = [400.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, None);
        // 400 - 400 / 0.2 x 0.05 = 300.
        assert!((avatar.velocity[0] - 300.0).abs() < 1e-2, "{:?}", avatar.velocity);
    }

    #[test]
    fn a_lunge_slows_by_its_own_rule_once() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::MELEE_CHARGE;
        avatar.velocity = [400.0, 0.0, 0.0];
        friction(&mut avatar, &tick_vars(true, 0), true, Some(Bo2Lunge::default()));
        let want = 400.0 - crate::bridge::BO2_CHARGE_FRICTION * 0.05;
        assert!((avatar.velocity[0] - want).abs() < 1e-2, "{:?}", avatar.velocity);
    }
}
