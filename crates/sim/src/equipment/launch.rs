//! Getting a grenade or launched round into the air: its first velocity, its spin, its deadlines, and the three ways one
//! comes to exist (thrown from the hand, fired from a launcher, let go by a dying player).

use entity_t6::missile::{
    GRENADE_SPIN_PITCH_MAX, GRENADE_SPIN_PITCH_MIN, GRENADE_SPIN_ROLL_MAX, GRENADE_SPIN_ROLL_MIN,
    fire_grenade_no_draw_ms,
};

use super::facts::{EquipmentRuntimeFacts, OFFHAND_SMOKE};
use super::state::{GrenadeLaunchKind, ProjectileState};
use super::vec3::{dot, look_dir, norm, unit, unit_flat};
use crate::entity_local::{fire_missile_apos, init_grenade_apos, init_grenade_pos};
use crate::frame::FrameWorld;
use crate::{ClientId, Tick, level_time_ms};

/// Longest any item may live, in milliseconds.
pub(super) const GRENADE_FUSE_CAP_MS: i32 = 60_000;
/// Fuse of an item whose file names none.
pub(super) const GRENADE_DEFAULT_FUSE_MS: i32 = 30_000;

/// Weapon file `rotateType` of a blade.
const ROTATE_BLADE: i32 = 1;
/// A blade turns end over end this fast (degrees a second), with no yaw or roll.
const BLADE_PITCH_RATE: f32 = 1500.0;
/// Any other spinning item tumbles at a whole-degree rate drawn from these ranges.
const TUMBLE_PITCH: (f32, f32) = (340.0, 800.0);
const TUMBLE_ROLL: (f32, f32) = (180.0, 540.0);

/// Weapon whose fuse never ends in mid-air for want of a floor.
pub(super) const AIRDROP_MARKER_WEAPON: &str = "airdrop_marker_mp";

/// A uniform draw in `[min, max)` the way the host does it: the next 32 bits scaled by 2^-32.
pub(super) fn draw_between(rng: &mut crate::MatchRng, min: f32, max: f32) -> f32 {
    let unit_draw = rng.next_u32() as f32 * (1.0 / 4_294_967_296.0);
    min + (max - min) * unit_draw
}

/// Where a throw or launcher shot starts out: its speed along the view, its lift, then (throws only) a flat push and a
/// rise along the view's up axis; finally the thrower's own velocity along the line of flight.
pub(super) fn throw_velocity(
    angles: [f32; 3],
    facts: &EquipmentRuntimeFacts,
    owner_vel: [f32; 3],
    launcher: bool,
) -> [f32; 3] {
    let view = look_dir(angles);
    let speed = facts.projectile_speed as f32;
    let mut v = [view[0] * speed, view[1] * speed, view[2] * speed + facts.projectile_speed_up as f32];
    if !launcher {
        if facts.projectile_speed_forward != 0
            && let Some(flat) = unit_flat(view)
        {
            let push = facts.projectile_speed_forward as f32;
            for axis in 0..3 {
                v[axis] += flat[axis] * push;
            }
        }
        if facts.projectile_speed_relative_up != 0 {
            let (_, _, up) = game_math::angle_vectors(angles);
            let lift = facts.projectile_speed_relative_up as f32;
            for axis in 0..3 {
                v[axis] += up[axis] * lift;
            }
        }
    }
    // The weapon file's parent-velocity share is only read for vehicles; a thrower's speed carries along the throw.
    match unit(v) {
        None => v,
        Some(dir) => {
            let along = dot(owner_vel, dir);
            [v[0] + dir[0] * along, v[1] + dir[1] * along, v[2] + dir[2] * along]
        }
    }
}

fn clamped_fuse(ms: i32) -> i32 {
    if ms <= 0 { GRENADE_DEFAULT_FUSE_MS } else { ms.min(GRENADE_FUSE_CAP_MS) }
}

/// When the fuse ends (None: it has none) and when the item is swept away regardless.
pub(super) fn deadlines(facts: &EquipmentRuntimeFacts, kind: GrenadeLaunchKind, now_ms: i32) -> (Option<i32>, i32) {
    let sweep_at = now_ms.saturating_add(GRENADE_FUSE_CAP_MS);
    if facts.is_throwing_knife() {
        return (None, sweep_at);
    }
    let authored = facts.fuse_time_ms;
    let fuse_ms = match kind {
        GrenadeLaunchKind::Launcher => {
            if authored <= 0 && facts.projectile_activate_dist != 0 {
                None
            } else {
                Some(clamped_fuse(authored))
            }
        }
        GrenadeLaunchKind::Thrown { remaining_fuse_ms } => {
            let picked = match remaining_fuse_ms {
                Some(left) if facts.timed_detonation && left > 0 => left,
                _ => authored,
            };
            Some(clamped_fuse(picked))
        }
        // A dropped item runs on the fuse it was handed, if its weapon times detonation at all.
        GrenadeLaunchKind::Dropped { remaining_fuse_ms, .. } if facts.timed_detonation => {
            Some(remaining_fuse_ms.min(GRENADE_FUSE_CAP_MS))
        }
        GrenadeLaunchKind::Dropped { .. } => Some(clamped_fuse(authored)),
    };
    (fuse_ms.map(|ms| now_ms.saturating_add(ms)), sweep_at)
}

/// True when a fuse that ends in mid-air is retried every frame until the item lies on a floor.
pub(super) fn fuse_waits_for_floor(world: &FrameWorld, facts: &EquipmentRuntimeFacts, weapon: u32) -> bool {
    facts.explode_on_ground
        || facts.offhand_class == OFFHAND_SMOKE
        || world.weapon_script_name(weapon) == AIRDROP_MARKER_WEAPON
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Spin {
    None,
    Fixed { pitch: f32, roll: f32 },
    /// A random rate in each range, each with a random sign; `whole`: cut to whole degrees.
    Tumble { pitch: (f32, f32), roll: (f32, f32), whole: bool },
}

pub(super) fn spin_of(facts: &EquipmentRuntimeFacts, kind: GrenadeLaunchKind) -> Spin {
    if matches!(kind, GrenadeLaunchKind::Launcher) {
        return Spin::None;
    }
    if facts.projectile_parent_velocity.is_some() {
        // Only what the weapon file's `rotate` asks for turns.
        return if !facts.projectile_rotates {
            Spin::None
        } else if facts.projectile_rotate_type == ROTATE_BLADE {
            Spin::Fixed { pitch: BLADE_PITCH_RATE, roll: 0.0 }
        } else {
            Spin::Tumble { pitch: TUMBLE_PITCH, roll: TUMBLE_ROLL, whole: true }
        };
    }
    if facts.is_throwing_knife() {
        Spin::Fixed { pitch: entity_t6::missile::BLADE_SPIN_PITCH, roll: 0.0 }
    } else if facts.projectile_rotates {
        Spin::Tumble {
            pitch: (GRENADE_SPIN_PITCH_MIN, GRENADE_SPIN_PITCH_MAX),
            roll: (GRENADE_SPIN_ROLL_MIN, GRENADE_SPIN_ROLL_MAX),
            whole: false,
        }
    } else {
        Spin::None
    }
}

/// Pitch and roll rates (degrees a second) a new item turns at; random draws come off the match stream in the host's
/// order: sign, size, sign, size.
fn spin_rates(world: &mut FrameWorld, spin: Spin) -> (f32, f32) {
    match spin {
        Spin::None => (0.0, 0.0),
        Spin::Fixed { pitch, roll } => (pitch, roll),
        Spin::Tumble { pitch, roll, whole } => {
            let rng = world.combat_rng_mut();
            let pitch_sign = if rng.next_index(2) == 0 { 1.0 } else { -1.0 };
            let pitch_size = draw_between(rng, pitch.0, pitch.1);
            let roll_sign = if rng.next_index(2) == 0 { 1.0 } else { -1.0 };
            let roll_size = draw_between(rng, roll.0, roll.1);
            let (p, r) = (pitch_sign * pitch_size, roll_sign * roll_size);
            if whole { (p.trunc(), r.trunc()) } else { (p, r) }
        }
    }
}

/// Starts an item flying. `carry` is the velocity of the platform its thrower stands on, added as it is; a dropped
/// item sets its own velocity instead. False when nothing was spawned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_grenade_projectile(
    world: &mut FrameWorld,
    owner: ClientId,
    weapon: u32,
    tick: Tick,
    origin: [f32; 3],
    angles: [f32; 3],
    owner_vel: [f32; 3],
    carry: [f32; 3],
    kind: GrenadeLaunchKind,
) -> bool {
    let Some(facts) = world.equipment_facts_for(weapon) else {
        let why = match world.equipment_row_raw(weapon) {
            None => "none".to_owned(),
            Some(f) => format!(
                "speed {} sticky {} fuse {}ms impact {} inner {}",
                f.projectile_speed, f.stickiness, f.fuse_time_ms, f.impact_damage, f.explosion_inner_damage
            ),
        };
        diag::warn!(Sim, "grenade not spawned: weapon {weapon} has no usable equipment row ({why})");
        return false;
    };
    if !facts.is_usable() {
        return false;
    }
    let owner_life = world.client_meta(owner).unwrap().life_sequence;

    let velocity = match kind {
        GrenadeLaunchKind::Dropped { velocity, .. } => velocity,
        _ => {
            let thrown = throw_velocity(angles, &facts, owner_vel, matches!(kind, GrenadeLaunchKind::Launcher));
            core::array::from_fn(|axis| thrown[axis] + carry[axis])
        }
    };
    // This game's items face along their velocity, not along the view; a dropped one too.
    let facing = match kind {
        GrenadeLaunchKind::Dropped { .. } => unit(velocity).unwrap_or([0.0, 0.0, 1.0]),
        _ if facts.projectile_parent_velocity.is_some() => unit(velocity).unwrap_or_else(|| look_dir(angles)),
        _ => look_dir(angles),
    };

    let now_ms = level_time_ms(tick);
    let (pitch_rate, roll_rate) = spin_rates(world, spin_of(&facts, kind));
    let apos = if pitch_rate == 0.0 && roll_rate == 0.0 {
        fire_missile_apos(facing)
    } else {
        init_grenade_apos(facing, now_ms, pitch_rate, roll_rate)
    };
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "throw: weapon {weapon} from {:?} angles [{:.1}, {:.1}] velocity {:?} owner {:?}",
            origin.map(f32::round),
            angles[0],
            angles[1],
            velocity.map(f32::round),
            owner_vel.map(f32::round)
        );
    }
    let pos = init_grenade_pos(origin, velocity, now_ms);
    let launch_time = now_ms + fire_grenade_no_draw_ms(norm(velocity));
    let (detonate_at_ms, cleanup_at_ms) = deadlines(&facts, kind, now_ms);

    let id = world.allocate_projectile_id();
    let entnum = match world.allocate_dynamic_entity(crate::gentity::EntityRunKind::Missile) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "grenade not spawned: {error:?}");
            return false;
        }
    };
    world.push_projectile(ProjectileState {
        id,
        owner,
        owner_life,
        weapon,
        entnum,
        origin,
        velocity: pos.tr_delta,
        pos,
        apos,
        launch_time,
        spawn_time_ms: now_ms,
        detonate_at_ms,
        cleanup_at_ms,
        travel_distance: 0.0,
        live: true,
        stuck_pane: None,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    });
    true
}

/// A player throws the offhand item of `weapon` from his eyes along his view.
pub(crate) fn spawn_offhand_projectile(
    world: &mut FrameWorld,
    id: ClientId,
    weapon: u32,
    tick: Tick,
    remaining_fuse_ms: Option<i32>,
) -> bool {
    let Some((eyes, angles, owner_vel)) = world.player(id).map(|ps| {
        let eyes = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
        (eyes, ps.viewangles, ps.velocity)
    }) else {
        return false;
    };
    let carry = crate::missile::platform_velocity(world, id, level_time_ms(tick));
    let kind = GrenadeLaunchKind::Thrown { remaining_fuse_ms };
    spawn_grenade_projectile(world, id, weapon, tick, eyes, angles, owner_vel, carry, kind)
}

/// A player who dies holding a live grenade lets it go where he stands (40 up, unless it sticks to things), tossed up to
/// 160 a second each way with the fuse still running. A cook-off's spent fuse sets it off at once, credited to him.
pub(crate) fn drop_grenade_on_death(
    world: &mut FrameWorld,
    id: ClientId,
    weapon: u32,
    tick: Tick,
    remaining_fuse_ms: i32,
) -> bool {
    let Some(facts) = world.equipment_facts_for(weapon) else {
        return false;
    };
    let Some(ps) = world.player(id).copied() else {
        return false;
    };
    let mut origin = ps.origin;
    if facts.stickiness != 1 {
        origin[2] += 40.0;
    }
    let rng = world.combat_rng_mut();
    let velocity: [f32; 3] = core::array::from_fn(|_| draw_between(rng, -160.0, 160.0));
    if crate::combat::shot_log() {
        diag::info!(
            Sim,
            "death drop: weapon {weapon} fuse {remaining_fuse_ms}ms from {:?} velocity {:?}",
            origin.map(f32::round),
            velocity.map(f32::round)
        );
    }
    let kind = GrenadeLaunchKind::Dropped { remaining_fuse_ms, velocity };
    spawn_grenade_projectile(world, id, weapon, tick, origin, ps.viewangles, [0.0; 3], [0.0; 3], kind)
}
