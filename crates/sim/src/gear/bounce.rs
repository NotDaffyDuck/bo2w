//! What a missile does when it hits something that does not stop it: the new velocity off a surface, whether the hit is
//! a dud, and whether the item comes to rest.

use super::events::push_bounce_event;
use super::facts::GadgetLiveStats;
use super::roll::bo2_rolling_bounce;
use super::scene::required_facts;
use super::flying::{FlyingProjectile, detonation_ordered};
use super::stick::{impact_time_ms, stick_missile, tumble_to_land};
use super::vec3::{dot, norm, unit};
use crate::step_view::StepWorldView;
use crate::{StepNo, level_time_ms};

/// A bounce that changes the speed by more than this raises a bounce event.
const BOUNCE_EVENT_SPEED_DELTA: f32 = 100.0;
/// Speed below which an item landing on a floor stops there.
const REST_SPEED: f32 = 20.0;
/// A projectile hitting head-on at this speed or faster crumples (a dud) ...
const CRUMPLE_SPEED: f32 = 500.0;
/// ... when its path is within about 45 degrees of the surface normal.
const CRUMPLE_COS: f32 = 0.707;
/// Surface flags of flesh.
pub(super) const FLESH_SURFACE_FLAGS: u32 = 0x0070_0000;
/// Weapon type of projectile launchers.
fn is_projectile_weapon(facts: &GadgetLiveStats) -> bool {
    facts.gun_family_code == i32::from(weapon_t6::facts::gun_family_code::PROJECTILE)
}

/// Velocity after meeting a surface facing `normal` whose material is `surface`, by the bounce rule of the weapon's
/// file: a rolling grenade rolls on, a game-native row mirrors with separate along/off factors, anything else mirrors
/// with one factor blended by the angle of incidence.
pub(super) fn bounce_velocity(
    missile: &mut FlyingProjectile,
    face: [f32; 3],
    rules: &GadgetLiveStats,
    surface: u8,
) {
    let slot = surface as usize;
    let (parallel, sideways) = match (rules.along_rebound.as_ref(), rules.across_rebound.as_ref()) {
        (Some(along), Some(across)) => along.get(slot).copied().zip(across.get(slot).copied()),
        _ => None,
    }
    .expect("missing projectile surface bounce coefficients");
    let native = rules.projectile_parent_velocity.is_some();
    let incoming = missile.motion;
    missile.motion = if native && rules.rolling_radius.is_some() {
        bo2_rolling_bounce(incoming, face, sideways)
    } else if native {
        mirror_split(incoming, face, parallel, sideways)
    } else {
        mirror_blended(incoming, face, parallel, sideways)
    };
}

/// Mirror off the surface, then scale the part along it by `parallel` and the part off it by `perpendicular`, each on
/// its own.
fn mirror_split(incoming: [f32; 3], face: [f32; 3], parallel: f32, perpendicular: f32) -> [f32; 3] {
    let Some(n) = unit(face) else {
        return [0.0; 3];
    };
    let d = dot(incoming, n);
    let mirrored: [f32; 3] = core::array::from_fn(|axis| incoming[axis] - 2.0 * d * n[axis]);
    let off = dot(mirrored, n);
    [0usize, 1, 2].map(|axis| {
        let off_part = n[axis] * off;
        (mirrored[axis] - off_part) * parallel + off_part * perpendicular
    })
}

/// Mirror, then one factor between `parallel` (grazing) and `perpendicular` (head on).
fn mirror_blended(incoming: [f32; 3], face: [f32; 3], parallel: f32, perpendicular: f32) -> [f32; 3] {
    let speed = norm(incoming);
    let n = match unit(face) {
        Some(n) if speed >= 1e-6 => n,
        _ => return [0.0; 3],
    };
    let d = dot(incoming, n);
    let incidence = (-d / speed).clamp(0.0, 1.0);
    let factor = parallel + (perpendicular - parallel) * incidence;
    core::array::from_fn(|axis| (incoming[axis] - 2.0 * d * n[axis]) * factor)
}

/// An unarmed projectile (a launcher round inside its arming distance) crumples into a dud on flesh, or head-on at
/// 500 or faster; slower or at a glancing angle it bounces off.
fn crumples(
    weapon_facts: &GadgetLiveStats,
    missile: &FlyingProjectile,
    tick: StepNo,
    hit_frac: f32,
    face: [f32; 3],
    flesh: bool,
) -> bool {
    if !is_projectile_weapon(weapon_facts) {
        return false;
    }
    if flesh {
        return true;
    }
    let velocity = missile.speed_vector_at(impact_time_ms(tick, hit_frac));
    let Some(heading) = unit(velocity) else {
        return false;
    };
    norm(velocity) >= CRUMPLE_SPEED && -dot(heading, face) > CRUMPLE_COS
}

/// An unarmed missile that does not crumple still bounces, unless it sticks to everything or to flesh.
pub(super) fn unarmed_dud(
    shot_facts: &GadgetLiveStats,
    missile: &FlyingProjectile,
    tick: StepNo,
    hit_frac: f32,
    face: [f32; 3],
    flesh: bool,
) -> bool {
    crumples(shot_facts, missile, tick, hit_frac, face, flesh) || matches!(shot_facts.stickiness, 1 | 5)
}

/// Whether an item leaving a bounce at `speed` off a surface facing `normal` comes to rest there. A rolling grenade
/// rests in its roll instead.
pub(super) fn comes_to_rest(facts: &GadgetLiveStats, speed: f32, face: [f32; 3]) -> bool {
    facts.rolling_radius.is_none() && (speed < 1.0 || (face[2] > 0.7 && speed < REST_SPEED))
}

/// Bounces `missile` off the surface it hit at `origin`; true when that counts as a bounce (a hard one, or the one it
/// rests on). `step_off(from, to)` is how far along that short move nothing blocks it.
#[allow(clippy::too_many_arguments)]
pub(super) fn bounce_off(
    world: &mut StepWorldView,
    step: StepNo,
    missile: &mut FlyingProjectile,
    spot: [f32; 3],
    face: [f32; 3],
    hit_frac: f32,
    surface: u8,
    step_off: &dyn Fn([f32; 3], [f32; 3]) -> f32,
) -> bool {
    let now = level_time_ms(step);
    missile.grounded |= face[2] > 0.7;
    missile.motion = missile.speed_vector_at(impact_time_ms(step, hit_frac));
    let incoming = missile.motion;
    let facts = required_facts(world, missile.gun);
    bounce_velocity(missile, face, &facts, surface);
    let outgoing = missile.motion;

    // A bounced grenade then moves a tenth of a unit along the normal (never up) as far as nothing blocks it, so its
    // next move starts clear of what it hit.
    let target = [
        spot[0] + face[0] * 0.1,
        spot[1] + face[1] * 0.1,
        spot[2] + (face[2] * 0.1).min(0.0),
    ];
    let clear = step_off(spot, target).clamp(0.0, 1.0);
    let spot: [f32; 3] = core::array::from_fn(|axis| spot[axis] + (target[axis] - spot[axis]) * clear);
    missile.origin = spot;
    missile.at.base_point = spot;
    missile.at.began_at = now;
    missile.at.rate = missile.motion;
    tumble_to_land(world, step, missile, face, hit_frac);

    let speed_change = norm([outgoing[0] - incoming[0], outgoing[1] - incoming[1], outgoing[2] - incoming[2]]);
    let rests = comes_to_rest(&facts, norm(outgoing), face);
    let bounced = speed_change > BOUNCE_EVENT_SPEED_DELTA || rests;
    if bounced {
        push_bounce_event(world, step, missile, missile.origin, face, surface);
    }
    if rests {
        stick_missile(step, missile, spot);
        // Without a timed fuse (care package and similar markers) it then never goes off by itself.
        if !facts.timed_detonation && !detonation_ordered(world, missile.id) {
            missile.detonate_at_ms = None;
            missile.cleanup_at_ms = i32::MAX;
        }
    }
    bounced
}
