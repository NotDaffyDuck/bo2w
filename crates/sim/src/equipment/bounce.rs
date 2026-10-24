//! What a missile does when it hits something that does not stop it: the new velocity off a surface, whether the hit is
//! a dud, and whether the item comes to rest.

use super::events::push_bounce_event;
use super::facts::EquipmentRuntimeFacts;
use super::roll::bo2_rolling_bounce;
use super::scene::required_facts;
use super::state::{ProjectileState, detonation_ordered};
use super::stick::{impact_time_ms, stick_missile, tumble_to_land};
use super::vec3::{dot, norm, unit};
use crate::frame::FrameWorld;
use crate::{Tick, level_time_ms};

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
fn is_projectile_weapon(facts: &EquipmentRuntimeFacts) -> bool {
    facts.weap_type == i32::from(weapon_t6::facts::weap_type::PROJECTILE)
}

/// Velocity after meeting a surface facing `normal` whose material is `surface`, by the bounce rule of the weapon's
/// file: a rolling grenade rolls on, a game-native row mirrors with separate along/off factors, anything else mirrors
/// with one factor blended by the angle of incidence.
pub(super) fn bounce_velocity(
    missile: &mut ProjectileState,
    normal: [f32; 3],
    facts: &EquipmentRuntimeFacts,
    surface: u8,
) {
    let along_off = facts
        .parallel_bounce
        .as_ref()
        .zip(facts.perpendicular_bounce.as_ref())
        .and_then(|(parallel, perpendicular)| {
            parallel.get(surface as usize).zip(perpendicular.get(surface as usize))
        })
        .unwrap_or_else(|| panic!("missing projectile surface bounce coefficients"));
    let (parallel, perpendicular) = (*along_off.0, *along_off.1);
    let native = facts.projectile_parent_velocity.is_some();
    let incoming = missile.velocity;
    missile.velocity = if native && facts.rolling_radius.is_some() {
        bo2_rolling_bounce(incoming, normal, perpendicular)
    } else if native {
        mirror_split(incoming, normal, parallel, perpendicular)
    } else {
        mirror_blended(incoming, normal, parallel, perpendicular)
    };
}

/// Mirror off the surface, then scale the part along it by `parallel` and the part off it by `perpendicular`, each on
/// its own.
fn mirror_split(incoming: [f32; 3], normal: [f32; 3], parallel: f32, perpendicular: f32) -> [f32; 3] {
    let Some(n) = unit(normal) else {
        return [0.0; 3];
    };
    let d = dot(incoming, n);
    let mirrored: [f32; 3] = core::array::from_fn(|axis| incoming[axis] - 2.0 * d * n[axis]);
    let off = dot(mirrored, n);
    core::array::from_fn(|axis| {
        let off_part = n[axis] * off;
        (mirrored[axis] - off_part) * parallel + off_part * perpendicular
    })
}

/// Mirror, then one factor between `parallel` (grazing) and `perpendicular` (head on).
fn mirror_blended(incoming: [f32; 3], normal: [f32; 3], parallel: f32, perpendicular: f32) -> [f32; 3] {
    let speed = norm(incoming);
    let n = match unit(normal) {
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
    facts: &EquipmentRuntimeFacts,
    missile: &ProjectileState,
    tick: Tick,
    fraction: f32,
    normal: [f32; 3],
    flesh: bool,
) -> bool {
    if !is_projectile_weapon(facts) {
        return false;
    }
    if flesh {
        return true;
    }
    let velocity = missile.velocity_at(impact_time_ms(tick, fraction));
    let Some(heading) = unit(velocity) else {
        return false;
    };
    norm(velocity) >= CRUMPLE_SPEED && -dot(heading, normal) > CRUMPLE_COS
}

/// An unarmed missile that does not crumple still bounces, unless it sticks to everything or to flesh.
pub(super) fn unarmed_dud(
    facts: &EquipmentRuntimeFacts,
    missile: &ProjectileState,
    tick: Tick,
    fraction: f32,
    normal: [f32; 3],
    flesh: bool,
) -> bool {
    crumples(facts, missile, tick, fraction, normal, flesh) || matches!(facts.stickiness, 1 | 5)
}

/// Whether an item leaving a bounce at `speed` off a surface facing `normal` comes to rest there. A rolling grenade
/// rests in its roll instead.
pub(super) fn comes_to_rest(facts: &EquipmentRuntimeFacts, speed: f32, normal: [f32; 3]) -> bool {
    facts.rolling_radius.is_none() && (speed < 1.0 || (normal[2] > 0.7 && speed < REST_SPEED))
}

/// Bounces `missile` off the surface it hit at `origin`; true when that counts as a bounce (a hard one, or the one it
/// rests on). `step_off(from, to)` is how far along that short move nothing blocks it.
#[allow(clippy::too_many_arguments)]
pub(super) fn bounce_off(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    origin: [f32; 3],
    normal: [f32; 3],
    fraction: f32,
    surface: u8,
    step_off: &dyn Fn([f32; 3], [f32; 3]) -> f32,
) -> bool {
    let now = level_time_ms(tick);
    missile.grounded |= normal[2] > 0.7;
    missile.velocity = missile.velocity_at(impact_time_ms(tick, fraction));
    let incoming = missile.velocity;
    let facts = required_facts(world, missile.weapon);
    bounce_velocity(missile, normal, &facts, surface);
    let outgoing = missile.velocity;

    // A bounced grenade then moves a tenth of a unit along the normal (never up) as far as nothing blocks it, so its
    // next move starts clear of what it hit.
    let target = [
        origin[0] + normal[0] * 0.1,
        origin[1] + normal[1] * 0.1,
        origin[2] + (normal[2] * 0.1).min(0.0),
    ];
    let clear = step_off(origin, target).clamp(0.0, 1.0);
    let origin: [f32; 3] = core::array::from_fn(|axis| origin[axis] + (target[axis] - origin[axis]) * clear);
    missile.origin = origin;
    missile.pos.tr_base = origin;
    missile.pos.tr_time = now;
    missile.pos.tr_delta = missile.velocity;
    tumble_to_land(world, tick, missile, normal, fraction);

    let speed_change = norm([outgoing[0] - incoming[0], outgoing[1] - incoming[1], outgoing[2] - incoming[2]]);
    let rests = comes_to_rest(&facts, norm(outgoing), normal);
    let bounced = speed_change > BOUNCE_EVENT_SPEED_DELTA || rests;
    if bounced {
        push_bounce_event(world, tick, missile, missile.origin, normal, surface);
    }
    if rests {
        stick_missile(tick, missile, origin);
        // Without a timed fuse (care package and similar markers) it then never goes off by itself.
        if !facts.timed_detonation && !detonation_ordered(world, missile.id) {
            missile.detonate_at_ms = None;
            missile.cleanup_at_ms = i32::MAX;
        }
    }
    bounced
}
