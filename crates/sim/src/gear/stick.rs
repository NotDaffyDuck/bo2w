//! Missiles coming to rest: sticking to a surface or a player, being freed again, spinning to a landing, and the knife
//! that either bounces or sticks by its own rules.

use entity_t6::flight::path_kind;
use entity_t6::{Trajectory, evaluate_trajectory};

use super::bounce::bounce_velocity;
use super::events::{PROJ_EXPLOSION_BOLT, StuckTo, push_bounce_event, push_stick_event};
use super::facts::GadgetLiveStats;
use super::launch::{GRENADE_DEFAULT_FUSE_MS, GRENADE_FUSE_CAP_MS, draw_between};
use super::scene::{SURF_FLESH, required_facts};
use super::flying::{FlyingProjectile, detonation_ordered};
use super::vec3::{dot, look_dir, norm, unit};
use crate::ent_links::{LandingSpin, landing_spin_of};
use crate::step_view::StepWorldView;
use crate::{SeatNo, StepNo, level_time_ms};

/// Point in this tick's move, in milliseconds, at which a hit `fraction` of the way along happened.
pub(super) fn impact_time_ms(step: StepNo, fraction: f32) -> i32 {
    let now = level_time_ms(step);
    let tick_start = now.saturating_sub(crate::SIM_STEP_MILLIS as i32);
    tick_start.saturating_add(((now - tick_start) as f32 * fraction) as i32)
}

/// Whether this item sticks to a surface facing `normal`: always for the sticky kinds, to floors only for the others.
pub(super) fn sticks_to_surface(facts: GadgetLiveStats, face: [f32; 3]) -> bool {
    matches!(facts.stickiness, 1 | 2) || (matches!(facts.stickiness, 3 | 4) && face[2] > 0.7)
}

/// Stops `missile` where it is, keeping its current angles as a fixed pose.
pub(super) fn stick_missile(step: StepNo, missile: &mut FlyingProjectile, spot: [f32; 3]) {
    let now = level_time_ms(step);
    missile.origin = spot;
    missile.motion = [0.0; 3];
    let held_angles = evaluate_trajectory(&missile.turn_path, now);
    missile.at = frozen_at(now, spot);
    missile.turn_path = frozen_at(now, held_angles);
}

/// A trajectory that stays on `tr_base` from `tr_time` on.
fn frozen_at(tr_time: i32, tr_base: [f32; 3]) -> Trajectory {
    Trajectory { tr_type: path_kind::AT_REST, began_at: tr_time, span_ms: 0, base_point: tr_base, rate: [0.0; 3] }
}

/// Lets a stuck missile fall again from where it hangs.
pub(super) fn unstick_missile(tick: StepNo, missile: &mut FlyingProjectile) {
    missile.motion = [0.0; 3];
    let (tr_time, tr_base) = (level_time_ms(tick), missile.origin);
    missile.at = Trajectory { tr_type: path_kind::FALLING, began_at: tr_time, span_ms: 0, base_point: tr_base, rate: [0.0; 3] };
}

/// Puts `missile` at `origin` moving at `velocity`, its path starting now.
pub(super) fn park_missile_at(tick: StepNo, missile: &mut FlyingProjectile, spot: [f32; 3], speed_vec: [f32; 3]) {
    missile.origin = spot;
    missile.motion = speed_vec;
    missile.at.base_point = spot;
    missile.at.began_at = level_time_ms(tick);
    missile.at.rate = speed_vec;
}

/// Turns the missile's angles toward the surface it just hit (two draws from the combat stream: the spin, then the
/// wall-spin offset).
pub(super) fn tumble_to_land(world: &mut StepWorldView, tick: StepNo, missile: &mut FlyingProjectile, face: [f32; 3], fraction: f32) {
    let (spin_random, wall_offset) = {
        let stream = world.combat_dice_mut();
        let spin = stream.draw_word() as f32 * (1.0 / 4_294_967_296.0);
        let wall = ((stream.draw_word() & 0x7f) as i32 - 63) as f32;
        (spin, wall)
    };
    let lay_flat = false;
    let landing = LandingSpin {
        turn_path: missile.turn_path,
        face_dir: face,
        impact_ms: impact_time_ms(tick, fraction),
        lay_flat,
        spin_random,
        wall_spin_extra: wall_offset,
    };
    missile.turn_path = landing_spin_of(landing).turn_path;
}

/// Roll of `angles` about the facing that lays the item flat against a surface facing `normal`, plus the facing itself
/// slid into the surface plane.
fn angles_along_surface(facing: [f32; 3], face: [f32; 3]) -> [f32; 3] {
    let into = dot(facing, face);
    let flat = core::array::from_fn(|axis| facing[axis] - into * face[axis]);
    gsc_t6::math::vector_to_angles(flat)
}

/// Roll that puts the item's up axis against `normal` for an item with the given `angles`.
fn roll_against(angles: [f32; 3], face: [f32; 3]) -> f32 {
    let (_, right, up) = game_math::euler_basis(angles);
    dot(face, right).atan2(dot(face, up)).to_degrees()
}

/// An item that comes to rest on a surface it sticks to: lands on its angles, moves a quarter unit off the surface, and
/// lets the clients and scripts know.
pub(super) fn settle_on_surface(
    world: &mut StepWorldView,
    step: StepNo,
    missile: &mut FlyingProjectile,
    spot: [f32; 3],
    face: [f32; 3],
    hit_frac: f32,
    surface: u8,
) {
    let facts = required_facts(world, missile.gun);
    let yaw = evaluate_trajectory(&missile.turn_path, level_time_ms(step))[1];
    if facts.stickiness == 3 {
        tumble_to_land(world, step, missile, face, hit_frac);
    }
    let off_surface = core::array::from_fn(|axis| spot[axis] + face[axis] * 0.25);
    stick_missile(step, missile, off_surface);
    if matches!(facts.stickiness, 2 | 4) {
        let mut angles = missile.turn_path.base_point;
        if facts.stickiness == 4 {
            angles = [game_math::slope_pitch(yaw, face), yaw, 0.0];
        } else {
            angles = angles_along_surface(look_dir(angles), face);
        }
        angles[2] = roll_against(angles, face);
        missile.turn_path.base_point = angles;
    }
    if !facts.timed_detonation && !detonation_ordered(world, missile.id) {
        missile.detonate_at_ms = None;
        missile.cleanup_at_ms = i32::MAX;
    }
    push_stick_event(world, step, missile, StuckTo::default(), face, surface);
    attach_bolt_grenade(world, step, missile, &facts);
}

/// A bolt that sticks leaves its spawned grenade (the crossbow's explosive bolt) where it struck and on what it
/// struck, on that grenade's own fuse; the scripts hear it as a thrown grenade. The stuck bolt goes when it blows.
pub(super) fn attach_bolt_grenade(
    world: &mut StepWorldView,
    step: StepNo,
    bolt: &mut FlyingProjectile,
    bolt_facts: &GadgetLiveStats,
) {
    if bolt_facts.shell_blast_kind != PROJ_EXPLOSION_BOLT || bolt_facts.spawned_grenade_weapon == 0 {
        return;
    }
    let weapon = bolt_facts.spawned_grenade_weapon;
    let Some(grenade) = world.gadget_stats_lookup(weapon) else {
        diag::warn!(Sim, "bolt grenade not spawned: weapon {weapon} has no usable equipment row");
        return;
    };
    let now = level_time_ms(step);
    let fuse = if grenade.detonation_delay_millis > 0 {
        grenade.detonation_delay_millis.min(GRENADE_FUSE_CAP_MS)
    } else {
        GRENADE_DEFAULT_FUSE_MS
    };
    let detonate_at = now.saturating_add(fuse);
    let entnum = match world.allocate_dynamic_entity(crate::ent_slots::EntityBehaviorClass::Missile) {
        Ok(entity) => entity.slot(),
        Err(error) => {
            diag::warn!(Sim, "bolt grenade not spawned: {error:?}");
            return;
        }
    };
    let grenade_id = world.reserve_projectile_slot();
    if crate::gunfire::shot_log() {
        diag::info!(
            Sim,
            "bolt grenade: weapon {weapon} from bolt {} at {:?} fuse {fuse}ms on player {:?}",
            bolt.id.0,
            bolt.origin.map(f32::round),
            bolt.rider.map(|r| r.client)
        );
    }
    let spawned = FlyingProjectile {
        id: grenade_id,
        owner: bolt.owner,
        owner_respawn_count: bolt.owner_respawn_count,
        gun: weapon,
        origin: bolt.origin,
        motion: [0.0, 0.0, 0.0],
        at: bolt.at,
        turn_path: bolt.turn_path,
        ent_no: entnum,
        spawn_moment: now,
        spawn_time_ms: now,
        detonate_at_ms: Some(detonate_at),
        cleanup_at_ms: now.saturating_add(GRENADE_FUSE_CAP_MS),
        travel_distance: 0.0,
        in_flight: true,
        stuck_pane: bolt.stuck_pane,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: bolt.impact_target,
        rider: bolt.rider,
        coasting: false,
    };
    world.add_shell(spawned);
    bolt.detonate_at_ms = None;
    bolt.cleanup_at_ms = detonate_at;
}

/// A thrown knife meets a surface or a player. A ballistic blade sticks at once. Any other knife bounces off, unless it
/// hits a player that things stick to, comes to rest on a floor, or hits a surface nearly head-on; then it sticks,
/// flat on a floor or point-in on a wall.
pub(super) fn knife_impact(
    world: &mut StepWorldView,
    step_no: StepNo,
    missile: &mut FlyingProjectile,
    face: [f32; 3],
    hit_frac: f32,
    surface: u8,
    struck: Option<(SeatNo, u8)>,
) {
    let on_player = struck.is_some();
    // A player's flesh is the surface its landing event names.
    let event_surface = if on_player { SURF_FLESH } else { surface };
    let now = level_time_ms(step_no);
    let hit_time = now - crate::SIM_STEP_MILLIS as i32 + (crate::SIM_STEP_MILLIS as f32 * hit_frac) as i32;
    let facts = required_facts(world, missile.gun);
    let stuck = StuckTo { struck, ..StuckTo::default() };
    if facts.ballistic_blade {
        let origin = core::array::from_fn(|axis| missile.origin[axis] + face[axis] * 0.25);
        stick_missile(step_no, missile, origin);
        push_stick_event(world, step_no, missile, stuck, face, event_surface);
        return;
    }
    missile.motion = missile.speed_vector_at(hit_time);
    bounce_velocity(missile, face, &facts, surface);
    let speed = norm(missile.motion);
    let heading = unit(missile.motion).unwrap_or([0.0; 3]);
    let head_on = dot(heading, face);
    let floor = face[2] > 0.7;
    let sticks = if on_player {
        facts.adheres_to_bodies
    } else {
        (floor && speed < 20.0) || head_on > 0.7
    };
    if !sticks {
        missile.origin = core::array::from_fn(|axis| {
            let step = face[axis] * 0.1;
            missile.origin[axis] + if axis == 2 { step.min(0.0) } else { step }
        });
        missile.at.base_point = missile.origin;
        missile.at.began_at = now;
        missile.at.rate = missile.motion;
        tumble_to_land(world, step_no, missile, face, hit_frac);
        push_bounce_event(world, step_no, missile, missile.origin, face, event_surface);
        return;
    }
    let mut angles = evaluate_trajectory(&missile.turn_path, hit_time);
    let mut origin = missile.origin;
    if !on_player && floor && (speed < 20.0 || head_on < 0.7) {
        angles = angles_along_surface(look_dir(angles), face);
        angles[2] = roll_against(angles, face) + 90.0;
        origin[2] -= 1.0;
    } else {
        angles[0] = face[2].atan2(face[0].hypot(face[1])).to_degrees()
            + draw_between(world.combat_dice_mut(), -15.0, 15.0);
        let facing = look_dir(angles);
        origin = core::array::from_fn(|axis| origin[axis] - face[axis] * 1.5 - facing[axis] * 4.5);
    }
    stick_missile(step_no, missile, origin);
    missile.turn_path.base_point = angles;
    push_stick_event(world, step_no, missile, stuck, face, event_surface);
}
