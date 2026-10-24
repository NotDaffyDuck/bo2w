//! bo2mp: Black Ops II's knife lunge (its COD4-style charge, `player_meleeCOD4ChargeMovement`
//! on). The lunge launches early in the move; then the player walks or falls as usual, with
//! the lunge's own friction in place of the ground's.
//!
//! The lunge rides on the same player-state fields as MW2's: the charge flag, the distance
//! sent with the press, and the time left (0 right after the press = launch on this move).

use playerstate_t6::game::{PlayerState, pm_flags};

use crate::bridge::melee_charge::melee_charge_clear;
use crate::bridge::pml::Pml;

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
const ENTITYNUM_NONE: i32 = playerstate_t6::game::ENTITYNUM_NONE;
const MS_TO_SECONDS: f32 = 0.001;

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
pub fn bo2_lunge_clear(ps: &mut PlayerState) {
    if (ps.pm_flags & pm_flags::MELEE_CHARGE) != 0 {
        ps.velocity[2] = ps.velocity[2].min(CHARGE_END_MAX_UP);
    }
    melee_charge_clear(ps);
}

/// Launch speed for a lunge of `dist`: 1200 u/s² stops it exactly `dist` on.
pub fn bo2_lunge_launch_speed(dist: f32) -> f32 {
    libm::sqrtf(2.0 * dist * BO2_CHARGE_FRICTION)
}

fn still_valid(ps: &mut PlayerState, lunge: Bo2Lunge) -> bool {
    if ps.pm_type != PM_NORMAL || (ps.pm_flags & (pm_flags::MANTLE | pm_flags::LADDER)) != 0 {
        return false;
    }
    if !lunge.zombies
        && ps.ground_entity_num == ENTITYNUM_NONE
        && ps.velocity[2] > CHARGE_AIR_MAX_UP
    {
        let speed = len2(ps.velocity);
        let base = ps.speed as f32;
        if speed > base && speed > 0.0 {
            ps.velocity[0] *= base / speed;
            ps.velocity[1] *= base / speed;
        }
        return false;
    }
    true
}

/// Early in the move: launch on the first frame (toward the target), then catch up while
/// the hit is still coming, and count the lunge down.
pub fn bo2_lunge_update(ps: &mut PlayerState, pml: &Pml, lunge: Bo2Lunge) {
    if (ps.pm_flags & pm_flags::MELEE_CHARGE) == 0 {
        return;
    }
    if !still_valid(ps, lunge) {
        bo2_lunge_clear(ps);
        return;
    }
    if ps.melee_charge_time == 0 {
        let v0 = bo2_lunge_launch_speed(ps.melee_charge_dist as f32);
        if let Some(t) = lunge.target {
            let yaw = libm::atan2f(t.origin[1] - ps.origin[1], t.origin[0] - ps.origin[0]);
            ps.velocity[0] = libm::cosf(yaw) * v0;
            ps.velocity[1] = libm::sinf(yaw) * v0;
        }
        ps.melee_charge_time = libm::roundf(v0 / BO2_CHARGE_FRICTION * 1000.0) as i32;
    }
    if let Some(t) = lunge.target {
        catch_up(ps, pml, t, lunge);
    }
    ps.melee_charge_time -= pml.msec;
    if ps.melee_charge_time < 1 {
        bo2_lunge_clear(ps);
    }
}

fn catch_up(ps: &mut PlayerState, pml: &Pml, t: LungeTarget, lunge: Bo2Lunge) {
    let secs = lunge.hit_delay_ms as f32 * MS_TO_SECONDS;
    if secs <= 0.0 {
        return;
    }
    let to = [
        t.origin[0] - ps.origin[0],
        t.origin[1] - ps.origin[1],
        t.origin[2] - ps.origin[2],
    ];
    let d = libm::sqrtf(to[0] * to[0] + to[1] * to[1] + to[2] * to[2]);
    if d <= CHARGE_REACH {
        return;
    }
    let k = (d - CHARGE_REACH) / d;
    let boost = 1.0 + pml.frametime * BO2_CHARGE_FRICTION;
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
    ps.velocity[0] = v[0];
    ps.velocity[1] = v[1];
}

/// The friction while a BO2 lunge runs (in place of the ground's, walking or not): 1200 u/s²,
/// or, once the hit has landed, just enough to stop 15 short of the target.
pub fn bo2_lunge_friction(ps: &mut PlayerState, pml: &Pml, lunge: Bo2Lunge) {
    if (ps.pm_flags & pm_flags::MELEE_CHARGE) == 0 {
        return;
    }
    let speed = if pml.walking != 0 {
        len2(ps.velocity)
    } else {
        let v = ps.velocity;
        libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    };
    if speed < 1.0 {
        ps.velocity = [0.0; 3];
        return;
    }
    let mut drop = BO2_CHARGE_FRICTION * pml.frametime;
    if let Some(t) = lunge.target
        && lunge.hit_delay_ms < 1
        && ps.melee_charge_time > 0
    {
        let dist = len2([t.origin[0] - ps.origin[0], t.origin[1] - ps.origin[1], 0.0])
            - CHARGE_STOP_SHORT;
        let flat = len2(ps.velocity);
        let secs = ps.melee_charge_time as f32 * MS_TO_SECONDS;
        if dist < flat * secs {
            drop = flat - dist / secs;
        }
    }
    let scale = (speed - drop).max(0.0) / speed;
    ps.velocity[0] *= scale;
    ps.velocity[1] *= scale;
    ps.velocity[2] *= scale;
}

fn len2(v: [f32; 3]) -> f32 {
    libm::sqrtf(v[0] * v[0] + v[1] * v[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pml(walking: bool) -> Pml {
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: 0.05,
            msec: 50,
            walking: u32::from(walking),
            ground_plane: u32::from(walking),
            almost_ground_plane: 0,
            ground_trace: [0; 11],
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }

    fn lunging(dist: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags |= pm_flags::MELEE_CHARGE;
        ps.melee_charge_dist = dist;
        ps.ground_entity_num = 0;
        ps.speed = 190;
        ps
    }

    #[test]
    fn launches_toward_the_target_at_sqrt_two_d_friction() {
        let mut ps = lunging(60);
        let target = LungeTarget { origin: [0.0, 90.0, 0.0], velocity: [0.0; 3] };
        let lunge = Bo2Lunge { target: Some(target), hit_delay_ms: 0, zombies: false };
        bo2_lunge_update(&mut ps, &pml(true), lunge);
        let v0 = bo2_lunge_launch_speed(60.0);
        assert!((v0 - 379.47).abs() < 0.1);
        assert!(ps.velocity[0].abs() < 0.01 && (ps.velocity[1] - v0).abs() < 0.01);
        // 316 ms of lunge, one 50 ms frame gone.
        assert_eq!(ps.melee_charge_time, 316 - 50);
    }

    #[test]
    fn catch_up_is_capped() {
        let target = LungeTarget { origin: [400.0, 0.0, 0.0], velocity: [0.0; 3] };
        let lunge = Bo2Lunge { target: Some(target), hit_delay_ms: 100, zombies: false };
        let mut ps = lunging(200);
        bo2_lunge_update(&mut ps, &pml(true), lunge);
        assert!((len2(ps.velocity) - CHARGE_MAX_SPEED_MP).abs() < 0.01);
        let mut ps = lunging(200);
        bo2_lunge_update(&mut ps, &pml(true), Bo2Lunge { zombies: true, ..lunge });
        assert!((len2(ps.velocity) - CHARGE_MAX_SPEED_ZM).abs() < 0.01);
    }

    #[test]
    fn friction_is_the_lunges_then_stops_short_after_the_hit() {
        let mut ps = lunging(60);
        ps.melee_charge_time = 200;
        ps.velocity = [300.0, 0.0, 0.0];
        let far = LungeTarget { origin: [1000.0, 0.0, 0.0], velocity: [0.0; 3] };
        let lunge = Bo2Lunge { target: Some(far), ..Bo2Lunge::default() };
        bo2_lunge_friction(&mut ps, &pml(true), lunge);
        assert!((ps.velocity[0] - 240.0).abs() < 0.01);
        // 40 units from the target: stop 15 short, so cover 25 in the 0.2 s left.
        ps.velocity = [300.0, 0.0, 0.0];
        let near = LungeTarget { origin: [40.0, 0.0, 0.0], velocity: [0.0; 3] };
        let lunge = Bo2Lunge { target: Some(near), ..Bo2Lunge::default() };
        bo2_lunge_friction(&mut ps, &pml(true), lunge);
        assert!((ps.velocity[0] - 125.0).abs() < 0.01);
    }

    #[test]
    fn ending_caps_the_climb_and_an_air_climb_ends_it_in_mp() {
        let mut ps = lunging(60);
        ps.melee_charge_time = 10;
        ps.velocity = [0.0, 0.0, 500.0];
        bo2_lunge_update(&mut ps, &pml(true), Bo2Lunge::default());
        assert_eq!(ps.pm_flags & pm_flags::MELEE_CHARGE, 0);
        assert_eq!(ps.velocity[2], CHARGE_END_MAX_UP);

        let air = |zombies| {
            let mut ps = lunging(60);
            ps.melee_charge_time = 300;
            ps.ground_entity_num = ENTITYNUM_NONE;
            ps.velocity = [400.0, 0.0, 250.0];
            bo2_lunge_update(&mut ps, &pml(false), Bo2Lunge { zombies, ..Bo2Lunge::default() });
            ps
        };
        let mp = air(false);
        assert_eq!(mp.pm_flags & pm_flags::MELEE_CHARGE, 0);
        assert!((mp.velocity[0] - 190.0).abs() < 0.01);
        assert_ne!(air(true).pm_flags & pm_flags::MELEE_CHARGE, 0);
    }
}
