//! Getting a grenade or launched round into the air: its first velocity, its spin, its deadlines, and the three ways one
//! comes to exist (thrown from the hand, fired from a launcher, let go by a dying player).

use entity_t6::missile::{
    LOB_FLIP_CAP, LOB_SPIN_FLOOR, LOB_ROLL_HIGH, LOB_ROLL_LOW,
    fire_grenade_no_draw_ms,
};

use super::facts::{GadgetLiveStats, OFFHAND_SMOKE};
use super::flying::{GrenadeLaunchKind, FlyingProjectile};
use super::vec3::{dot, look_dir, norm, unit};
use crate::ent_links::{fire_missile_apos, init_grenade_apos, init_grenade_pos};
use crate::step_view::StepWorldView;
use crate::{SeatNo, StepNo, level_time_ms};

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
pub(super) fn draw_between(rng: &mut crate::MatchRandom, lo: f32, hi: f32) -> f32 {
    let unit_draw = rng.draw_word() as f32 * (1.0 / 4_294_967_296.0);
    lo + (hi - lo) * unit_draw
}

/// Where a throw or launcher shot starts out: its speed along the view, its lift, then (throws only) a flat push and a
/// rise along the view's up axis; finally the thrower's own velocity along the line of flight.
pub(super) fn throw_velocity(
    aim: [f32; 3],
    facts: &GadgetLiveStats,
    owner_vel: [f32; 3],
    launcher: bool,
) -> [f32; 3] {
    let view = look_dir(aim);
    let speed = facts.missile_velocity as f32;
    let mut v = [view[0] * speed, view[1] * speed, view[2] * speed + facts.missile_upward_velocity as f32];
    if !launcher {
        if facts.projectile_speed_forward != 0
            && let Some(flat) = unit([view[0], view[1], 0.0])
        {
            let push = facts.projectile_speed_forward as f32;
            for (vk, fk) in v.iter_mut().zip(flat) {
                *vk += fk * push;
            }
        }
        if facts.projectile_speed_relative_up != 0 {
            let (_, _, up) = game_math::euler_basis(aim);
            let lift = facts.projectile_speed_relative_up as f32;
            for (vk, uk) in v.iter_mut().zip(up) {
                *vk += uk * lift;
            }
        }
    }
    // The weapon file's parent-velocity share is only read for vehicles; a thrower's speed carries along the throw.
    match unit(v) {
        None => v,
        Some(heading) => {
            let along = dot(owner_vel, heading);
            [v[0] + heading[0] * along, v[1] + heading[1] * along, v[2] + heading[2] * along]
        }
    }
}

fn clamped_fuse(ms: i32) -> i32 {
    if ms <= 0 { GRENADE_DEFAULT_FUSE_MS } else { ms.min(GRENADE_FUSE_CAP_MS) }
}

/// When the fuse ends (None: it has none). The item is swept away at the fuse cap regardless.
pub(super) fn fuse_end(facts: &GadgetLiveStats, kind: GrenadeLaunchKind, clock_millis: i32) -> Option<i32> {
    let authored = facts.detonation_delay_millis;
    let fuse_ms = match kind {
        _ if facts.is_throwing_knife() => return None,
        GrenadeLaunchKind::InHand => return Some(clock_millis.saturating_add(1)),
        // A fuse past the cap (the buildables' 30000 s) never goes off.
        GrenadeLaunchKind::Magic { fuse_ms } if fuse_ms >= GRENADE_FUSE_CAP_MS => return None,
        GrenadeLaunchKind::Magic { fuse_ms } => return Some(clock_millis.saturating_add(fuse_ms.max(1))),
        GrenadeLaunchKind::Launcher => {
            if authored <= 0 && facts.shell_arming_range != 0 {
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
    fuse_ms.map(|ms| clock_millis.saturating_add(ms))
}

/// True when a fuse that ends in mid-air is retried every frame until the item lies on a floor.
pub(super) fn fuse_waits_for_floor(world: &StepWorldView, facts: &GadgetLiveStats, gun: u32) -> bool {
    facts.explode_on_ground
        || facts.secondary_hand_category == OFFHAND_SMOKE
        || world.weapon_script_label(gun) == AIRDROP_MARKER_WEAPON
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Spin {
    None,
    Fixed { pitch: f32, roll: f32 },
    /// A random rate in each range, each with a random sign; `whole`: cut to whole degrees.
    Tumble { pitch: (f32, f32), roll: (f32, f32), whole: bool },
}

pub(super) fn spin_of(facts: &GadgetLiveStats, kind: GrenadeLaunchKind) -> Spin {
    if matches!(kind, GrenadeLaunchKind::Launcher | GrenadeLaunchKind::InHand | GrenadeLaunchKind::Magic { .. }) {
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
            pitch: (LOB_SPIN_FLOOR, LOB_FLIP_CAP),
            roll: (LOB_ROLL_LOW, LOB_ROLL_HIGH),
            whole: false,
        }
    } else {
        Spin::None
    }
}

/// Pitch and roll rates (degrees a second) a new item turns at; random draws come off the match stream in the host's
/// order: sign, size, sign, size.
fn spin_rates(world: &mut StepWorldView, spin: Spin) -> (f32, f32) {
    match spin {
        Spin::None => (0.0, 0.0),
        Spin::Fixed { pitch, roll } => (pitch, roll),
        Spin::Tumble { pitch, roll, whole } => {
            let rng = world.combat_dice_mut();
            let tilt_direction = if rng.below(2) == 0 { 1.0 } else { -1.0 };
            let pitch_size = draw_between(rng, pitch.0, pitch.1);
            let twist_direction = if rng.below(2) == 0 { 1.0 } else { -1.0 };
            let roll_size = draw_between(rng, roll.0, roll.1);
            let (p, r) = (tilt_direction * pitch_size, twist_direction * roll_size);
            if whole { (p.trunc(), r.trunc()) } else { (p, r) }
        }
    }
}

/// Starts an item flying. `carry` is the velocity of the platform its thrower stands on, added as it is; a dropped
/// item sets its own velocity instead. False when nothing was spawned.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_grenade_projectile(
    world: &mut StepWorldView,
    owner: SeatNo,
    gun: u32,
    tick: StepNo,
    spot: [f32; 3],
    aim: [f32; 3],
    owner_vel: [f32; 3],
    carry: [f32; 3],
    kind: GrenadeLaunchKind,
) -> bool {
    let Some(facts) = world.gadget_stats_lookup(gun) else {
        let why = match world.equipment_row_raw(gun) {
            None => "none".to_owned(),
            Some(f) => format!(
                "speed {} sticky {} fuse {}ms impact {} inner {}",
                f.missile_velocity, f.stickiness, f.detonation_delay_millis, f.collision_harm, f.blast_core_damage
            ),
        };
        diag::warn!(Sim, "grenade not spawned: weapon {gun} has no usable equipment row ({why})");
        return false;
    };
    if !facts.ready_for_use() {
        return false;
    }
    let owner_respawn_count = world.client_meta(owner).unwrap().life_sequence;

    let speed_vec = match kind {
        GrenadeLaunchKind::Dropped { motion: speed_vec, .. } => speed_vec,
        GrenadeLaunchKind::InHand | GrenadeLaunchKind::Magic { .. } => owner_vel,
        _ => {
            let thrown = throw_velocity(aim, &facts, owner_vel, matches!(kind, GrenadeLaunchKind::Launcher));
            core::array::from_fn(|axis| thrown[axis] + carry[axis])
        }
    };
    // This game's items face along their velocity, not along the view; a dropped one too.
    let facing = match kind {
        GrenadeLaunchKind::Dropped { .. } => unit(speed_vec).unwrap_or([0.0, 0.0, 1.0]),
        _ if facts.projectile_parent_velocity.is_some() => unit(speed_vec).unwrap_or_else(|| look_dir(aim)),
        _ => look_dir(aim),
    };

    let clock_millis = level_time_ms(tick);
    let (pitch_spin, roll_turn) = spin_rates(world, spin_of(&facts, kind));
    let apos = if pitch_spin == 0.0 && roll_turn == 0.0 {
        fire_missile_apos(facing)
    } else {
        init_grenade_apos(facing, clock_millis, pitch_spin, roll_turn)
    };
    if crate::gunfire::shot_log() {
        diag::info!(
            Sim,
            "throw: weapon {gun} from {:?} angles [{:.1}, {:.1}] velocity {:?} owner {:?}",
            spot.map(f32::round),
            aim[0],
            aim[1],
            speed_vec.map(f32::round),
            owner_vel.map(f32::round)
        );
    }
    let path = init_grenade_pos(spot, speed_vec, clock_millis);
    let spawn_moment = clock_millis + fire_grenade_no_draw_ms(norm(speed_vec));
    let detonate_at_ms = fuse_end(&facts, kind, clock_millis);
    let cleanup_at_ms = clock_millis.saturating_add(GRENADE_FUSE_CAP_MS);

    let id = world.reserve_projectile_slot();
    let entnum = match world.allocate_dynamic_entity(crate::ent_slots::EntityBehaviorClass::Missile) {
        Ok(entity) => entity.slot(),
        Err(error) => {
            diag::warn!(Sim, "grenade not spawned: {error:?}");
            return false;
        }
    };
    world.add_shell(FlyingProjectile {
        id,
        owner,
        owner_respawn_count,
        gun,
        ent_no: entnum,
        origin: spot,
        motion: path.rate,
        at: path,
        turn_path: apos,
        spawn_moment,
        spawn_time_ms: clock_millis,
        detonate_at_ms,
        cleanup_at_ms,
        travel_distance: 0.0,
        in_flight: true,
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
pub(crate) fn launch_gear_projectile(
    world: &mut StepWorldView,
    id: SeatNo,
    gun: u32,
    step: StepNo,
    remaining_fuse_ms: Option<i32>,
) -> bool {
    let Some((eyes, angles, owner_vel)) = world.avatar(id).map(|pstate| {
        let eyes = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
        (eyes, pstate.look_angles, pstate.velocity)
    }) else {
        return false;
    };
    let carry = crate::rocketry::platform_velocity(world, id, level_time_ms(step));
    let kind = GrenadeLaunchKind::Thrown { remaining_fuse_ms };
    spawn_grenade_projectile(world, id, gun, step, eyes, angles, owner_vel, carry, kind)
}

/// bo2zm: a grenade cooked too long goes off in his hand as a real grenade (its explosion effect and sound, its damage
/// through the game's rules); false when none could spawn.
pub(crate) fn spawn_offhand_in_hand(level: &mut StepWorldView, seat: SeatNo, gun: u32, tick: StepNo) -> bool {
    let Some(pstate) = level.avatar(seat).copied() else {
        return false;
    };
    let spot = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now - 8.0];
    spawn_grenade_projectile(level, seat, gun, tick, spot, pstate.look_angles, pstate.velocity, [0.0; 3], GrenadeLaunchKind::InHand)
}

/// A player who dies holding a live grenade lets it go where he stands (40 up, unless it sticks to things), tossed up to
/// 160 a second each way with the fuse still running. A cook-off's spent fuse sets it off at once, credited to him.
pub(crate) fn drop_grenade_on_death(
    level: &mut StepWorldView,
    seat: SeatNo,
    gun: u32,
    tick: StepNo,
    remaining_fuse_ms: i32,
) -> bool {
    let Some(facts) = level.gadget_stats_lookup(gun) else {
        return false;
    };
    let Some(pstate) = level.avatar(seat).copied() else {
        return false;
    };
    let mut origin = pstate.origin;
    if facts.stickiness != 1 {
        origin[2] += 40.0;
    }
    let rng = level.combat_dice_mut();
    let velocity: [f32; 3] = core::array::from_fn(|_| draw_between(rng, -160.0, 160.0));
    if crate::gunfire::shot_log() {
        diag::info!(
            Sim,
            "death drop: weapon {gun} fuse {remaining_fuse_ms}ms from {:?} velocity {:?}",
            origin.map(f32::round),
            velocity.map(f32::round)
        );
    }
    let kind = GrenadeLaunchKind::Dropped { remaining_fuse_ms, motion: velocity };
    spawn_grenade_projectile(level, seat, gun, tick, origin, pstate.look_angles, [0.0; 3], [0.0; 3], kind)
}
