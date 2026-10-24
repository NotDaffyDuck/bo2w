//! Missiles coming to rest: sticking to a surface or a player, being freed again, spinning to a landing, and the knife
//! that either bounces or sticks by its own rules.

use entity_t6::trajectory::tr_type::{GRAVITY as TR_GRAVITY, STATIONARY as TR_STATIONARY};
use entity_t6::{Trajectory, evaluate_trajectory};

use super::bounce::bounce_velocity;
use super::events::{PROJ_EXPLOSION_BOLT, StuckTo, push_bounce_event, push_stick_event};
use super::facts::EquipmentRuntimeFacts;
use super::launch::{GRENADE_DEFAULT_FUSE_MS, GRENADE_FUSE_CAP_MS, draw_between};
use super::scene::{SURF_FLESH, required_facts};
use super::state::{ProjectileState, detonation_ordered};
use super::vec3::{dot, look_dir, norm, unit};
use crate::entity_local::{MissileLandAnglesIn, missile_land_angles};
use crate::frame::FrameWorld;
use crate::{ClientId, Tick, level_time_ms};

/// Point in this tick's move, in milliseconds, at which a hit `fraction` of the way along happened.
pub(super) fn impact_time_ms(tick: Tick, fraction: f32) -> i32 {
    let now = level_time_ms(tick);
    let tick_start = now.saturating_sub(crate::MATCH_TICK_MS as i32);
    tick_start.saturating_add(((now - tick_start) as f32 * fraction) as i32)
}

/// Whether this item sticks to a surface facing `normal`: always for the sticky kinds, to floors only for the others.
pub(super) fn sticks_to_surface(facts: EquipmentRuntimeFacts, normal: [f32; 3]) -> bool {
    matches!(facts.stickiness, 1 | 2) || (matches!(facts.stickiness, 3 | 4) && normal[2] > 0.7)
}

/// Stops `missile` where it is, keeping its current angles as a fixed pose.
pub(super) fn stick_missile(tick: Tick, missile: &mut ProjectileState, origin: [f32; 3]) {
    let now = level_time_ms(tick);
    missile.origin = origin;
    missile.velocity = [0.0; 3];
    missile.pos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: now,
        tr_duration: 0,
        tr_base: origin,
        tr_delta: [0.0; 3],
    };
    missile.apos = Trajectory {
        tr_type: TR_STATIONARY,
        tr_time: now,
        tr_duration: 0,
        tr_base: evaluate_trajectory(&missile.apos, now),
        tr_delta: [0.0; 3],
    };
}

/// Lets a stuck missile fall again from where it hangs.
pub(super) fn unstick_missile(tick: Tick, missile: &mut ProjectileState) {
    missile.velocity = [0.0; 3];
    missile.pos = Trajectory {
        tr_type: TR_GRAVITY,
        tr_time: level_time_ms(tick),
        tr_duration: 0,
        tr_base: missile.origin,
        tr_delta: [0.0; 3],
    };
}

/// Puts `missile` at `origin` moving at `velocity`, its path starting now.
pub(super) fn park_missile_at(tick: Tick, missile: &mut ProjectileState, origin: [f32; 3], velocity: [f32; 3]) {
    missile.origin = origin;
    missile.velocity = velocity;
    missile.pos.tr_base = origin;
    missile.pos.tr_time = level_time_ms(tick);
    missile.pos.tr_delta = velocity;
}

/// Turns the missile's angles toward the surface it just hit (two draws from the combat stream: the spin, then the
/// wall-spin offset).
pub(super) fn tumble_to_land(world: &mut FrameWorld, tick: Tick, missile: &mut ProjectileState, normal: [f32; 3], fraction: f32) {
    let (spin_random, wall_spin_addend) = {
        let rng = world.combat_rng_mut();
        let spin = rng.next_u32() as f32 * (1.0 / 4_294_967_296.0);
        let wall = ((rng.next_u32() & 0x7f) as i32 - 63) as f32;
        (spin, wall)
    };
    missile.apos = missile_land_angles(MissileLandAnglesIn {
        apos: missile.apos,
        normal,
        hit_time_ms: impact_time_ms(tick, fraction),
        force_align: false,
        spin_random,
        wall_spin_addend,
    })
    .apos;
}

/// Roll of `angles` about the facing that lays the item flat against a surface facing `normal`, plus the facing itself
/// slid into the surface plane.
fn angles_along_surface(facing: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let into = dot(facing, normal);
    let flat = core::array::from_fn(|axis| facing[axis] - into * normal[axis]);
    gsc_t6::math::vector_to_angles(flat)
}

/// Roll that puts the item's up axis against `normal` for an item with the given `angles`.
fn roll_against(angles: [f32; 3], normal: [f32; 3]) -> f32 {
    let (_, right, up) = game_math::angle_vectors(angles);
    dot(normal, right).atan2(dot(normal, up)).to_degrees()
}

/// An item that comes to rest on a surface it sticks to: lands on its angles, moves a quarter unit off the surface, and
/// lets the clients and scripts know.
pub(super) fn settle_on_surface(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    origin: [f32; 3],
    normal: [f32; 3],
    fraction: f32,
    surface: u8,
) {
    let facts = required_facts(world, missile.weapon);
    let yaw = evaluate_trajectory(&missile.apos, level_time_ms(tick))[1];
    if facts.stickiness == 3 {
        tumble_to_land(world, tick, missile, normal, fraction);
    }
    let off_surface = core::array::from_fn(|axis| origin[axis] + normal[axis] * 0.25);
    stick_missile(tick, missile, off_surface);
    if matches!(facts.stickiness, 2 | 4) {
        let mut angles = missile.apos.tr_base;
        if facts.stickiness == 4 {
            angles = [game_math::pitch_for_yaw_on_normal(yaw, normal), yaw, 0.0];
        } else {
            angles = angles_along_surface(look_dir(angles), normal);
        }
        angles[2] = roll_against(angles, normal);
        missile.apos.tr_base = angles;
    }
    if !facts.timed_detonation && !detonation_ordered(world, missile.id) {
        missile.detonate_at_ms = None;
        missile.cleanup_at_ms = i32::MAX;
    }
    push_stick_event(world, tick, missile, StuckTo::default(), normal, surface);
    attach_bolt_grenade(world, tick, missile, &facts);
}

/// A bolt that sticks leaves its spawned grenade (the crossbow's explosive bolt) where it struck and on what it
/// struck, on that grenade's own fuse; the scripts hear it as a thrown grenade. The stuck bolt goes when it blows.
pub(super) fn attach_bolt_grenade(
    world: &mut FrameWorld,
    tick: Tick,
    bolt: &mut ProjectileState,
    facts: &EquipmentRuntimeFacts,
) {
    if facts.projectile_explosion_type != PROJ_EXPLOSION_BOLT || facts.spawned_grenade_weapon == 0 {
        return;
    }
    let weapon = facts.spawned_grenade_weapon;
    let Some(grenade) = world.equipment_facts_for(weapon) else {
        diag::warn!(Sim, "bolt grenade not spawned: weapon {weapon} has no usable equipment row");
        return;
    };
    let now = level_time_ms(tick);
    let fuse = if grenade.fuse_time_ms > 0 {
        grenade.fuse_time_ms.min(GRENADE_FUSE_CAP_MS)
    } else {
        GRENADE_DEFAULT_FUSE_MS
    };
    let detonate_at = now.saturating_add(fuse);
    let entnum = match world.allocate_dynamic_entity(crate::gentity::EntityRunKind::Missile) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "bolt grenade not spawned: {error:?}");
            return;
        }
    };
    let id = world.allocate_projectile_id();
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "bolt grenade: weapon {weapon} from bolt {} at {:?} fuse {fuse}ms on player {:?}",
            bolt.id.0,
            bolt.origin.map(f32::round),
            bolt.rider.map(|r| r.client)
        );
    }
    world.push_projectile(ProjectileState {
        id,
        owner: bolt.owner,
        owner_life: bolt.owner_life,
        weapon,
        origin: bolt.origin,
        velocity: [0.0; 3],
        pos: bolt.pos,
        apos: bolt.apos,
        entnum,
        launch_time: now,
        spawn_time_ms: now,
        detonate_at_ms: Some(detonate_at),
        cleanup_at_ms: now.saturating_add(GRENADE_FUSE_CAP_MS),
        travel_distance: 0.0,
        live: true,
        stuck_pane: bolt.stuck_pane,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: bolt.impact_target,
        rider: bolt.rider,
        coasting: false,
    });
    bolt.detonate_at_ms = None;
    bolt.cleanup_at_ms = detonate_at;
}

/// A thrown knife meets a surface or a player. A ballistic blade sticks at once. Any other knife bounces off, unless it
/// hits a player that things stick to, comes to rest on a floor, or hits a surface nearly head-on; then it sticks,
/// flat on a floor or point-in on a wall.
pub(super) fn knife_impact(
    world: &mut FrameWorld,
    tick: Tick,
    missile: &mut ProjectileState,
    normal: [f32; 3],
    fraction: f32,
    surface: u8,
    struck: Option<(ClientId, u8)>,
) {
    let on_player = struck.is_some();
    // A player's flesh is the surface its landing event names.
    let event_surface = if on_player { SURF_FLESH } else { surface };
    let now = level_time_ms(tick);
    let hit_time = now - crate::MATCH_TICK_MS as i32 + (crate::MATCH_TICK_MS as f32 * fraction) as i32;
    let facts = required_facts(world, missile.weapon);
    let stuck = StuckTo { struck, ..StuckTo::default() };
    if facts.ballistic_blade {
        let origin = core::array::from_fn(|axis| missile.origin[axis] + normal[axis] * 0.25);
        stick_missile(tick, missile, origin);
        push_stick_event(world, tick, missile, stuck, normal, event_surface);
        return;
    }
    missile.velocity = missile.velocity_at(hit_time);
    bounce_velocity(missile, normal, &facts, surface);
    let speed = norm(missile.velocity);
    let heading = unit(missile.velocity).unwrap_or([0.0; 3]);
    let head_on = dot(heading, normal);
    let floor = normal[2] > 0.7;
    let sticks = if on_player {
        facts.stick_to_players
    } else {
        (floor && speed < 20.0) || head_on > 0.7
    };
    if !sticks {
        missile.origin = core::array::from_fn(|axis| {
            let step = normal[axis] * 0.1;
            missile.origin[axis] + if axis == 2 { step.min(0.0) } else { step }
        });
        missile.pos.tr_base = missile.origin;
        missile.pos.tr_time = now;
        missile.pos.tr_delta = missile.velocity;
        tumble_to_land(world, tick, missile, normal, fraction);
        push_bounce_event(world, tick, missile, missile.origin, normal, event_surface);
        return;
    }
    let mut angles = evaluate_trajectory(&missile.apos, hit_time);
    let mut origin = missile.origin;
    if !on_player && floor && (speed < 20.0 || head_on < 0.7) {
        angles = angles_along_surface(look_dir(angles), normal);
        angles[2] = roll_against(angles, normal) + 90.0;
        origin[2] -= 1.0;
    } else {
        angles[0] = normal[2].atan2(normal[0].hypot(normal[1])).to_degrees()
            + draw_between(world.combat_rng_mut(), -15.0, 15.0);
        let facing = look_dir(angles);
        origin = core::array::from_fn(|axis| origin[axis] - normal[axis] * 1.5 - facing[axis] * 4.5);
    }
    stick_missile(tick, missile, origin);
    missile.apos.tr_base = angles;
    push_stick_event(world, tick, missile, stuck, normal, event_surface);
}
