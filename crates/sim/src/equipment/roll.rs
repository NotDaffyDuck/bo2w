//! Black Ops II rolling grenades (frag, flash, concussion, EMP): their bounce rule and the per-tick roll along a floor.

use entity_t6::{Trajectory, evaluate_trajectory, trajectory::tr_type::GRAVITY as TR_GRAVITY};

use super::state::ProjectileState;
use super::stick::stick_missile;
use super::vec3::{cross, dot, norm, unit};
use crate::{Tick, level_time_ms};

/// bo2zm fix list 2: under its body: how far below its resting height a
/// rolling grenade still finds the ground it rolls on.
pub(super) const ROLL_SNAP: f32 = 2.0;
/// bo2mp window 5: Black Ops II's rolling grenade rules (its engine defaults):
/// a hit slower than this along the surface normal rolls instead of bouncing.
pub(super) const ROLL_HIT_SPEED: f32 = 25.0;
/// Speed along the ground below which each hit takes the low friction.
pub(super) const ROLL_FRICTION_THRESH: f32 = 100.0;
/// Fraction of its speed a hit takes away: slow (rolling) and fast.
pub(super) const ROLL_FRICTION_LOW: f32 = 0.01;
pub(super) const ROLL_FRICTION_HIGH: f32 = 0.4;
/// The most of a hard hit's normal speed it bounces back with on a wall.
pub(super) const ROLL_RESTITUTION_MAX: f32 = 0.3;
/// Slower than this on a floor, it comes to rest.
pub(super) const ROLL_REST_SPEED: f32 = 20.0;
/// Bumps: chance per hit, size (fraction of its speed) and the cap.
pub(super) const ROLL_BUMP_CHANCE: f32 = 0.3;
pub(super) const ROLL_BUMP_MAG: f32 = 0.4;
pub(super) const ROLL_BUMP_MAX: f32 = 100.0;
/// Wobble: cycles per unit rolled, forward speed swing, side swing, and how
/// fast the side swing fades with speed.
pub(super) const ROLL_WOBBLE_FREQ: f32 = 0.08;
pub(super) const ROLL_WOBBLE_FWD: f32 = 10.0;
pub(super) const ROLL_WOBBLE_SIDE: f32 = 1.0;
pub(super) const ROLL_WOBBLE_SIDE_DAMP: f32 = 0.01;
/// The most sideways speed its curve adds per hit (each grenade curves its
/// own random way between minus and plus this).
pub(super) const ROLL_CURVE_MAX: f32 = 4.0;
/// The radius its spin is drawn for.
pub(super) const ROLL_SPIN_RADIUS: f32 = 5.0;
pub(super) const ROLL_GRAVITY: f32 = 800.0;
pub(super) const ROLL_TICK_S: f32 = crate::MATCH_TICK_MS as f32 / 1000.0;


/// bo2mp window 5: a Black Ops II rolling grenade's hit (the frag, flash,
/// concussion, EMP): its speed into the surface is gone, each hit takes a
/// share of its speed (a little when slow, a lot when fast), and only a hard
/// hit (25 units/s or more into it) bounces back, by its perpendicular
/// bounce, capped by how upright the surface is.
pub(super) fn bo2_rolling_bounce(incoming: [f32; 3], normal: [f32; 3], perpendicular: f32) -> [f32; 3] {
    let Some(n) = unit(normal) else {
        return [0.0, 0.0, 0.0];
    };
    let into = dot(incoming, n);
    let mut v: [f32; 3] = core::array::from_fn(|i| incoming[i] - into * n[i]);
    if norm(incoming) <= 0.0 || into > 0.0 {
        return v;
    }
    v = roll_friction(v);
    if into.abs() >= ROLL_HIT_SPEED {
        let back = (1.0 - (1.0 - perpendicular) * n[2]).min(ROLL_RESTITUTION_MAX.max(perpendicular));
        v = core::array::from_fn(|i| v[i] - back * into * n[i]);
    }
    v
}

fn roll_friction(v: [f32; 3]) -> [f32; 3] {
    let friction = if norm(v) > ROLL_FRICTION_THRESH {
        ROLL_FRICTION_HIGH
    } else {
        ROLL_FRICTION_LOW
    };
    v.map(|c| c * (1.0 - friction))
}

/// Each grenade's own curve, between -1 and 1, fixed for its life.
fn roll_curve(projectile: &ProjectileState) -> f32 {
    let h = (projectile.spawn_time_ms as u32).wrapping_mul(2_654_435_761)
        ^ (projectile.entnum as u32).wrapping_mul(40_503);
    let h = h ^ (h >> 15);
    (h >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
}

/// bo2zm fix list 2 + bo2mp window 5: one tick of a Black Ops II rolling
/// grenade (`isRollingGrenade`) after the shared bounce ran. Whatever it
/// touched it touched with its body (`radius` off the surface, not its
/// centre on it). On a floor it hits the ground every tick, as Black Ops II's
/// does under gravity: its speed into the ground is gone, gravity pulls it
/// down a slope, each hit takes its friction, now and then it bumps up, it
/// wobbles and curves, and it spins as it rolls, until it is slower than its
/// rest speed or rolls off an edge and falls. `pre_vel` is its velocity
/// when it touched, before this tick's bounce; `bump_roll` is a random 0..1 for this tick.
/// True when it came to rest.
pub(super) fn roll_grenade(
    tick: Tick,
    projectile: &mut ProjectileState,
    radius: f32,
    contact_normal: Option<[f32; 3]>,
    pre_vel: [f32; 3],
    bump_roll: f32,
    probe: &dyn Fn([f32; 3], [f32; 3]) -> Option<([f32; 3], [f32; 3])>,
) -> bool {
    let time = level_time_ms(tick);
    let rolling = projectile.pos.tr_type == entity_t6::trajectory::tr_type::LINEAR;
    let mut origin = projectile.origin;
    let mut v = projectile.velocity_at(time);
    if let Some(n) = contact_normal {
        origin = core::array::from_fn(|i| origin[i] + n[i] * radius);
        // Rolling onto a rise is a hit like any other tick's.
        if rolling && n[2] > 0.7 {
            v = pre_vel;
        }
    }
    let angles = evaluate_trajectory(&projectile.apos, time);
    let mut hit_this_tick = rolling;
    if !rolling {
        let floor = contact_normal.filter(|n| n[2] > 0.7);
        let Some(n) = floor else {
            // Still in the air, or off a wall: just off what it hit.
            if contact_normal.is_some() {
                projectile.origin = origin;
                projectile.pos.tr_base = origin;
            }
            return false;
        };
        if norm(v) < ROLL_REST_SPEED {
            stick_missile(tick, projectile, origin);
            return true;
        }
        if dot(pre_vel, n).abs() >= ROLL_HIT_SPEED {
            // A hard landing: it bounced. It hops from where it touched, so
            // it comes down as fast as it went up, as Black Ops II's does;
            // lifting it by its body here would add a fall to every hop and
            // keep it hopping (never rolling) on flat ground.
            return false;
        }
        // It landed softly: the bounce already took this tick's hit.
        hit_this_tick = false;
    }
    let below = [origin[0], origin[1], origin[2] - (radius + ROLL_SNAP)];
    let Some((ground, n)) = probe(origin, below).filter(|(_, n)| n[2] > 0.7) else {
        // Rolled off an edge: it falls.
        projectile.origin = origin;
        projectile.pos = Trajectory {
            tr_type: TR_GRAVITY,
            tr_time: time,
            tr_duration: 0,
            tr_base: origin,
            tr_delta: v,
        };
        return false;
    };
    origin = core::array::from_fn(|i| ground[i] + n[i] * radius);
    // bo2mp window 5: rolling on a floor is being on the ground, so a
    // ground-only fuse (EMP, concussion, smoke) goes off as it rolls or rests;
    // the flight step clears it every tick a rolling grenade moves.
    projectile.grounded = true;
    // Its speed into the ground is gone; gravity pulls it down the slope.
    let g = [0.0, 0.0, -ROLL_GRAVITY];
    let gn = dot(g, n);
    let vn = dot(v, n);
    v = core::array::from_fn(|i| v[i] - n[i] * vn + (g[i] - n[i] * gn) * ROLL_TICK_S);
    let speed = norm(v);
    let mut airborne = false;
    if hit_this_tick && speed > 0.0 {
        let dir = v.map(|c| c / speed);
        v = roll_friction(v);
        if bump_roll < ROLL_BUMP_CHANCE {
            let bump = (ROLL_BUMP_MAG * speed).min(ROLL_BUMP_MAX);
            if bump > ROLL_GRAVITY * 0.05 {
                v = core::array::from_fn(|i| v[i] + n[i] * bump);
                airborne = true;
            }
        }
        // The wobble's phase is the distance rolled (the tick's move already
        // added it), so it turns at its cycles per unit as Black Ops II's does.
        let tau = core::f32::consts::TAU;
        let phase = (projectile.travel_distance * ROLL_WOBBLE_FREQ * tau) % tau;
        let next = phase + ROLL_WOBBLE_FREQ * speed * ROLL_TICK_S * tau;
        let swing = next.sin() - phase.sin();
        if speed > 0.01 {
            let scale = (ROLL_WOBBLE_FWD * swing + speed) / speed;
            v = v.map(|c| c * scale);
        }
        let side = cross(dir, n);
        let damp = 1.0 - ROLL_WOBBLE_SIDE_DAMP * speed;
        if damp > 0.0 {
            let off = ROLL_WOBBLE_SIDE * swing * damp;
            let to: [f32; 3] = core::array::from_fn(|i| origin[i] + side[i] * off);
            origin = probe(origin, to).map_or(to, |(end, _)| end);
        }
        let curve = ROLL_CURVE_MAX * roll_curve(projectile);
        v = core::array::from_fn(|i| v[i] + side[i] * curve);
    }
    let speed = norm(v);
    if !airborne && speed < ROLL_REST_SPEED {
        stick_missile(tick, projectile, origin);
        projectile.apos.tr_base = angles;
        return true;
    }
    projectile.origin = origin;
    projectile.velocity = v;
    projectile.pos = Trajectory {
        tr_type: if airborne {
            TR_GRAVITY
        } else {
            entity_t6::trajectory::tr_type::LINEAR
        },
        tr_time: time,
        tr_duration: 0,
        tr_base: origin,
        tr_delta: v,
    };
    // It spins about the axis across its path, once per turn of its spin radius.
    let yaw = v[1].atan2(v[0]).to_degrees();
    projectile.apos = Trajectory {
        tr_type: entity_t6::trajectory::tr_type::LINEAR,
        tr_time: time,
        tr_duration: 0,
        tr_base: [angles[0], yaw, 90.0],
        tr_delta: [speed / (ROLL_SPIN_RADIUS * core::f32::consts::TAU) * 360.0, 0.0, 0.0],
    };
    false
}
