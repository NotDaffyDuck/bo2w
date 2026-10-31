//! bo2mp: Black Ops II's knife lunge (its COD4-style charge, `player_meleeCOD4ChargeMovement`
//! on). The lunge launches early in the move; then the player walks or falls as usual, with
//! the lunge's own friction in place of the ground's.
//!
//! The lunge rides on the same player-state fields as MW2's: the charge flag, the distance
//! sent with the press, and the time left (0 right after the press = launch on this move).

use playerstate_t6::game::{AvatarState, movement_state_bits};

use crate::bridge::melee_reach::melee_charge_clear;
use crate::bridge::pml::TickLocals;

/// Lunge slow-down, units/s² (`player_meleeChargeFriction`).
pub const BO2_CHARGE_FRICTION: f32 = 1200.0;
/// Catch-up speed cap: multiplayer, Zombies (`player_meleeChargeMaxSpeed`).
const CHARGE_MAX_SPEED_MP: f32 = 800.0;
const CHARGE_MAX_SPEED_ZM: f32 = 600.0;
/// After the hit, the lunge stops this far short of the target.
const CHARGE_STOP_SHORT: f32 = 15.0;
/// Ending a lunge never leaves more upward speed than this.
const CHARGE_END_MAX_UP: f32 = 300.0;
/// Multiplayer: a lunge in the air rising faster than this ends.
const CHARGE_AIR_MAX_UP: f32 = 240.0;
/// The catch-up aims for the swing's reach in front of the target.
const CHARGE_REACH: f32 = 55.0;
const PM_NORMAL: i32 = 0;
const NO_ENTITY_SLOT: i32 = playerstate_t6::game::NO_ENTITY_SLOT;
const MILLIS_PER_SEC_INV: f32 = 0.001;

/// The lunge target this move.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LungeTarget {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
}

/// What a BO2 lunge needs from outside the move.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bo2Lunge {
    /// None: the target is gone (the lunge keeps its launch and slows down).
    pub target: Option<LungeTarget>,
    /// ms until the swing's hit lands; 0 once it has.
    pub hit_delay_ms: i32,
    pub zombies: bool,
}

/// End a BO2 lunge; it never leaves the player rising fast.
pub fn bo2_lunge_clear(avatar: &mut AvatarState) {
    if (avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE) != 0 {
        avatar.velocity[2] = avatar.velocity[2].min(CHARGE_END_MAX_UP);
    }
    melee_charge_clear(avatar);
}

/// Launch speed for a lunge of `dist`: 1200 u/s² stops it exactly `dist` on.
pub fn bo2_lunge_launch_speed(dist: f32) -> f32 {
    libm::sqrtf(2.0 * dist * BO2_CHARGE_FRICTION)
}

fn still_valid(avatar: &mut AvatarState, lunge: Bo2Lunge) -> bool {
    if avatar.movement_mode != PM_NORMAL || (avatar.movement_state_bits & (movement_state_bits::MANTLE | movement_state_bits::LADDER)) != 0 {
        return false;
    }
    if !lunge.zombies
        && avatar.standing_on == NO_ENTITY_SLOT
        && avatar.velocity[2] > CHARGE_AIR_MAX_UP
    {
        let speed = len2(avatar.velocity);
        let base = avatar.speed as f32;
        if speed > base && speed > 0.0 {
            avatar.velocity[0] *= base / speed;
            avatar.velocity[1] *= base / speed;
        }
        return false;
    }
    true
}

/// Early in the move: launch on the first frame (toward the target), then catch up while
/// the hit is still coming, and count the lunge down.
pub fn bo2_lunge_update(avatar: &mut AvatarState, tick_vars: &TickLocals, lunge: Bo2Lunge) {
    if (avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE) == 0 {
        return;
    }
    if !still_valid(avatar, lunge) {
        bo2_lunge_clear(avatar);
        return;
    }
    if avatar.lunge_clock == 0 {
        let v0 = bo2_lunge_launch_speed(avatar.lunge_reach as f32);
        if let Some(t) = lunge.target {
            let yaw = libm::atan2f(t.origin[1] - avatar.origin[1], t.origin[0] - avatar.origin[0]);
            avatar.velocity[0] = libm::cosf(yaw) * v0;
            avatar.velocity[1] = libm::sinf(yaw) * v0;
        }
        avatar.lunge_clock = libm::roundf(v0 / BO2_CHARGE_FRICTION * 1000.0) as i32;
    }
    if let Some(t) = lunge.target {
        catch_up(avatar, tick_vars, t, lunge);
    }
    avatar.lunge_clock -= tick_vars.frame_ms;
    if avatar.lunge_clock < 1 {
        bo2_lunge_clear(avatar);
    }
}

fn catch_up(avatar: &mut AvatarState, tick_vars: &TickLocals, t: LungeTarget, lunge: Bo2Lunge) {
    let secs = lunge.hit_delay_ms as f32 * MILLIS_PER_SEC_INV;
    if secs <= 0.0 {
        return;
    }
    let to = [
        t.origin[0] - avatar.origin[0],
        t.origin[1] - avatar.origin[1],
        t.origin[2] - avatar.origin[2],
    ];
    let d = libm::sqrtf(to[0] * to[0] + to[1] * to[1] + to[2] * to[2]);
    if d <= CHARGE_REACH {
        return;
    }
    let k = (d - CHARGE_REACH) / d;
    let boost = 1.0 + tick_vars.step_millis_elapsed * BO2_CHARGE_FRICTION;
    let mut v = [
        (t.velocity[0] * secs + to[0] * k) / secs * boost,
        (t.velocity[1] * secs + to[1] * k) / secs * boost,
    ];
    let cap = if lunge.zombies {
        CHARGE_MAX_SPEED_ZM
    } else {
        CHARGE_MAX_SPEED_MP
    };
    let s = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    if s > cap {
        v = [v[0] * cap / s, v[1] * cap / s];
    }
    avatar.velocity[0] = v[0];
    avatar.velocity[1] = v[1];
}

/// The friction while a BO2 lunge runs (in place of the ground's, walking or not): 1200 u/s²,
/// or, once the hit has landed, just enough to stop 15 short of the target.
pub fn bo2_lunge_friction(avatar: &mut AvatarState, tick_vars: &TickLocals, lunge: Bo2Lunge) {
    if (avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE) == 0 {
        return;
    }
    let speed = if tick_vars.walking != 0 {
        len2(avatar.velocity)
    } else {
        let v = avatar.velocity;
        libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    };
    if 1.0 > speed {
        avatar.velocity = Default::default();
        return;
    }
    let mut drop = BO2_CHARGE_FRICTION * tick_vars.step_millis_elapsed;
    if let Some(t) = lunge.target
        && lunge.hit_delay_ms < 1
        && avatar.lunge_clock > 0
    {
        let dist = len2([t.origin[0] - avatar.origin[0], t.origin[1] - avatar.origin[1], 0.0])
            - CHARGE_STOP_SHORT;
        let flat = len2(avatar.velocity);
        let secs = avatar.lunge_clock as f32 * MILLIS_PER_SEC_INV;
        if dist < flat * secs {
            drop = flat - dist / secs;
        }
    }
    let scale = (speed - drop).max(0.0) / speed;
    avatar.velocity = avatar.velocity.map(|axis| axis * scale);
}

fn len2(v: [f32; 3]) -> f32 {
    libm::sqrtf(v[0] * v[0] + v[1] * v[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_vars(walking: bool) -> TickLocals {
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: u32::from(walking),
            standing_on_floor: u32::from(walking),
            near_floor_contact: 0,
            floor_probe: [0; 11],
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    fn lunging(dist: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits |= movement_state_bits::MELEE_CHARGE;
        avatar.lunge_reach = dist;
        avatar.standing_on = 0;
        avatar.speed = 190;
        avatar
    }

    #[test]
    fn launches_toward_the_target_at_sqrt_two_d_friction() {
        let mut avatar = lunging(60);
        let target = LungeTarget { velocity: [0.0_f32; 3], origin: [0.0_f32, 90.0, 0.0] };
        let lunge = Bo2Lunge { target: Some(target), hit_delay_ms: 0, zombies: false };
        bo2_lunge_update(&mut avatar, &tick_vars(true), lunge);
        let v0 = bo2_lunge_launch_speed(60.0);
        assert!((v0 - 379.47).abs() < 0.1);
        assert!(avatar.velocity[0].abs() < 0.01 && (avatar.velocity[1] - v0).abs() < 0.01);
        // 316 ms of lunge, one 50 ms frame gone.
        assert_eq!(avatar.lunge_clock, 316 - 50);
    }

    #[test]
    fn catch_up_is_capped() {
        let target = LungeTarget { velocity: [0.0_f32; 3], origin: [400.0, 0.0, 0.0] };
        let lunge = Bo2Lunge { target: Some(target), hit_delay_ms: 100, zombies: false };
        let mut avatar = lunging(200);
        bo2_lunge_update(&mut avatar, &tick_vars(true), lunge);
        assert!((len2(avatar.velocity) - CHARGE_MAX_SPEED_MP).abs() < 0.01);
        let mut avatar = lunging(200);
        bo2_lunge_update(&mut avatar, &tick_vars(true), Bo2Lunge { zombies: true, ..lunge });
        assert!((len2(avatar.velocity) - CHARGE_MAX_SPEED_ZM).abs() < 0.01);
    }

    #[test]
    fn friction_is_the_lunges_then_stops_short_after_the_hit() {
        let mut avatar = lunging(60);
        avatar.lunge_clock = 200;
        avatar.velocity = [300.0, 0.0, 0.0];
        let far = LungeTarget { velocity: [0.0_f32; 3], origin: [1000.0, 0.0, 0.0] };
        let lunge = Bo2Lunge { target: Some(far), ..Bo2Lunge::default() };
        bo2_lunge_friction(&mut avatar, &tick_vars(true), lunge);
        assert!((avatar.velocity[0] - 240.0).abs() < 0.01);
        // 40 units from the target: stop 15 short, so cover 25 in the 0.2 s left.
        avatar.velocity = [300.0, 0.0, 0.0];
        let near = LungeTarget { velocity: [0.0_f32; 3], origin: [40.0, 0.0, 0.0] };
        let lunge = Bo2Lunge { target: Some(near), ..Bo2Lunge::default() };
        bo2_lunge_friction(&mut avatar, &tick_vars(true), lunge);
        assert!((avatar.velocity[0] - 125.0).abs() < 0.01);
    }

    #[test]
    fn ending_caps_the_climb_and_an_air_climb_ends_it_in_mp() {
        let mut avatar = lunging(60);
        avatar.lunge_clock = 10;
        avatar.velocity = [0.0_f32, 0.0, 500.0];
        bo2_lunge_update(&mut avatar, &tick_vars(true), Bo2Lunge::default());
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE, 0);
        assert_eq!(avatar.velocity[2], CHARGE_END_MAX_UP);

        let air = |zombies| {
            let mut avatar = lunging(60);
            avatar.lunge_clock = 300;
            avatar.standing_on = NO_ENTITY_SLOT;
            avatar.velocity = [400.0, 0.0, 250.0];
            bo2_lunge_update(&mut avatar, &tick_vars(false), Bo2Lunge { zombies, ..Bo2Lunge::default() });
            avatar
        };
        let mp = air(false);
        assert_eq!(mp.movement_state_bits & movement_state_bits::MELEE_CHARGE, 0);
        assert!((mp.velocity[0] - 190.0).abs() < 0.01);
        assert_ne!(air(true).movement_state_bits & movement_state_bits::MELEE_CHARGE, 0);
    }
}
