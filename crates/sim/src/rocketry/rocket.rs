//! Firing a rocket-class projectile (rockets, javelins, bolts, TV-guided missiles) from a shot.

use crate::gunfire::{ValidatedShot, spread_direction_on_plane};
use crate::ent_links::{fire_missile_apos, clipped_shift};
use crate::gear::{FlyingProjectile, ROCKET_CLEANUP_MS};
use crate::step_view::StepWorldView;
use crate::match_ids::MatchRandom;
use entity_t6::{Trajectory, missile::fire_grenade_no_draw_ms, flight::path_kind};
use game_math::vec3_norm;
use glam::Vec3;
use weapon_t6::damage::ROCKET_SPREAD_PLANE;

/// Guidance kinds that change how the projectile leaves the gun.
const JAVELIN: i32 = 3;
const BALLISTIC: i32 = 4;
const TV_GUIDED: i32 = 6;

/// How hard (units a second) a launcher that is not full-auto shoves its holder back.
const KICKBACK: f32 = 64.0;
/// A javelin leaves tilted up by this much of its length, and from this far to the right of the eye.
const JAVELIN_TILT: f32 = 0.3;
const JAVELIN_SIDE_OFFSET: f32 = 2.5;
/// A TV-guided missile starts this far along its heading.
const TV_LEAD: f32 = 13.0;
/// The weapon-lock flag that asks for a top attack.
const LOCK_TOP_ATTACK: u8 = 4;

/// Fires the projectile of a rocket-class weapon. `pulled` is true for a real trigger pull, which adds the launcher's
/// own rules: the rocket inherits the speed of the platform its holder stands on (a TV-guided missile does not), and
/// the holder is shoved back. A script's `MagicBullet` passes false.
pub(super) fn fire(world: &mut StepWorldView, step: crate::StepNo, shot: &ValidatedShot, pulled: bool) -> Option<FlyingProjectile> {
    let weapon = shot.gun;
    let Some(spec) = world.rocket_launch_facts(weapon) else {
        diag::warn!(Sim, "weapon {} fires no missile: no projectile speed", weapon);
        return None;
    };
    let seed = shot.combat_random_seed as u64;
    let mut dice = MatchRandom::new(seed);
    let aim = spread_direction_on_plane(shot.turn, shot.cone_angle_deg, &mut dice, ROCKET_SPREAD_PLANE);
    let LaunchStart { heading: dir, from: muzzle_point } = muzzle(spec.missile_guidance, shot, aim);
    let guide = guide_for(world, shot);
    let mut carried = shot.owner_velocity;
    if pulled && spec.missile_guidance != TV_GUIDED {
        let platform = super::platform_velocity(world, shot.attacker, crate::level_time_ms(step));
        carried = core::array::from_fn(|i| carried[i] + platform[i]);
    }
    let projectile_id = world.reserve_projectile_slot();
    let entnum = match world.allocate_dynamic_entity(crate::ent_slots::EntityBehaviorClass::Missile) {
        Ok(entity) => entity.slot(),
        Err(error) => {
            diag::warn!(Sim, "missile not spawned: {error:?}");
            return None;
        }
    };
    let born = crate::level_time_ms(step);
    let launch = launch_velocity(dir, spec.missile_velocity as f32, spec.missile_upward_velocity, carried);
    let muzzle_velocity = clipped_shift(launch);
    // A javelin climbs and a ballistic shot drops: both fall under gravity.
    let falls = matches!(spec.missile_guidance, JAVELIN | BALLISTIC);
    perf::projectile(weapon);
    let (owner, owner_respawn_count) = (shot.attacker, shot.killer_life);
    let shell = FlyingProjectile {
        id: projectile_id,
        owner,
        owner_respawn_count,
        gun: weapon,
        origin: muzzle_point,
        motion: muzzle_velocity,
        at: Trajectory {
            began_at: born,
            tr_type: if falls { path_kind::FALLING } else { path_kind::STRAIGHT },
            span_ms: 0,
            rate: muzzle_velocity,
            base_point: muzzle_point,
        },
        turn_path: fire_missile_apos(dir),
        ent_no: entnum,
        spawn_moment: born + fire_grenade_no_draw_ms(vec3_norm(launch)),
        spawn_time_ms: born,
        detonate_at_ms: None,
        cleanup_at_ms: born.saturating_add(ROCKET_CLEANUP_MS),
        travel_distance: 0.0,
        in_flight: true,
        stuck_pane: None,
        grounded: false,
        guide,
        impact_target: None,
        rider: None,
        coasting: false,
    };
    world.add_shell(shell);
    if pulled && !is_full_auto(world, weapon) {
        let (ahead, _, _) = game_math::euler_basis(shot.turn);
        if let Some(ps) = world.occupant_mut(shot.attacker) {
            kick_back(&mut ps.velocity, ahead);
        }
    }
    Some(shell)
}

/// The launch direction and start point: the spread-adjusted aim, adjusted again for the weapons that leave tilted
/// (javelin) or start ahead of the eye (TV-guided).
fn muzzle(guidance: i32, shot: &ValidatedShot, aim: [f32; 3]) -> LaunchStart {
    let mut dir = aim;
    let mut origin = shot.spot;
    if guidance == JAVELIN {
        dir = Vec3::new(dir[0], dir[1], dir[2] + JAVELIN_TILT).normalize().to_array();
        let (_, right, _) = game_math::euler_basis(shot.turn);
        origin = core::array::from_fn(|i| origin[i] + right[i] * JAVELIN_SIDE_OFFSET);
    }
    if guidance == TV_GUIDED {
        origin = core::array::from_fn(|i| origin[i] + dir[i] * TV_LEAD);
    }
    LaunchStart { heading: dir, from: origin }
}

/// Where a missile leaves from and the way it heads.
struct LaunchStart {
    heading: [f32; 3],
    from: [f32; 3],
}

/// The guidance the missile starts with: the shooter's weapon lock, if they have one that is good for this weapon
/// and this life.
fn guide_for(world: &StepWorldView, shot: &ValidatedShot) -> crate::MissileGuide {
    let lock = world
        .client_meta(shot.attacker)
        .map(|meta| meta.weapon_lock)
        .filter(|lock| lock.can_fire(shot.gun, shot.killer_life.0));
    crate::MissileGuide {
        target: lock.and_then(|lock| lock.aim),
        top: lock.is_some_and(|lock| lock.bits & LOCK_TOP_ATTACK != 0),
        ..Default::default()
    }
}

/// Whether the weapon fires continuously. With no gun table loaded, nothing is known and it counts as full-auto.
fn is_full_auto(world: &StepWorldView, gun: u32) -> bool {
    let guns = world.gun_facts();
    guns.is_empty() || guns.get(gun).core.fire_type == weapon_t6::facts::fire_type::FULLAUTO
}

/// Muzzle velocity: `speed` along `dir`, plus the weapon's upward kick (90 for the crossbow and the ballistic knife,
/// none for a rocket), plus the velocity the holder carries.
fn launch_velocity(dir: [f32; 3], speed: f32, kick_up: i32, carried: [f32; 3]) -> [f32; 3] {
    let mut along: [f32; 3] = core::array::from_fn(|i| dir[i] * speed);
    along[2] += kick_up as f32;
    core::array::from_fn(|i| along[i] + carried[i])
}

/// Shoves the holder back along their aim, but never up: a launcher aimed down only pushes them level.
fn kick_back(velocity: &mut [f32; 3], ahead: [f32; 3]) {
    velocity[0] -= ahead[0] * KICKBACK;
    velocity[1] -= ahead[1] * KICKBACK;
    velocity[2] += (-ahead[2]).min(0.0) * KICKBACK;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot_at(angles: [f32; 3]) -> ValidatedShot {
        ValidatedShot {
            shot_serial: crate::BulletNo(0),
            attacker: crate::SeatNo(0),
            killer_life: crate::LifeCount(1),
            fist: 0,
            gun: 1,
            rounds_spent: 0,
            spot: [100.0, 200.0, 300.0],
            turn: angles,
            scope_frac: 0.0,
            eye_rise: Default::default(),
            sight_spread_factor: Default::default(),
            combat_random_seed: 0,
            owner_velocity: [0.0; 3],
            cone_angle_deg: 0.0,
            pen_scale: None,
        }
    }

    #[test]
    fn a_bolt_leaves_with_its_kick_up_and_the_holders_speed() {
        // The ballistic knife: level along x at 2180, kick up 90, holder running along y.
        let out = launch_velocity([1.0, 0.0, 0.0], 2180.0, 90, [0.0, 190.0, 0.0]);
        assert_eq!(out, [2180.0, 190.0, 90.0]);
    }

    #[test]
    fn a_rocket_has_no_kick_up() {
        assert_eq!(launch_velocity([0.0, 0.0, 1.0], 1500.0, 0, [0.0; 3]), [0.0, 0.0, 1500.0]);
    }

    #[test]
    fn a_launcher_shoves_the_holder_back_along_the_aim() {
        let mut v = [0.0; 3];
        kick_back(&mut v, [1.0, 0.0, 0.0]);
        assert_eq!(v, [-64.0, 0.0, 0.0]);
    }

    #[test]
    fn aimed_straight_up_the_holder_is_pushed_down() {
        let mut v = [0.0; 3];
        kick_back(&mut v, [0.0, 0.0, 1.0]);
        assert_eq!(v, [0.0, 0.0, -64.0]);
    }

    #[test]
    fn aimed_straight_down_the_holder_is_not_lifted() {
        let mut v = [0.0, 0.0, 10.0];
        kick_back(&mut v, [0.0, 0.0, -1.0]);
        assert_eq!(v, [0.0, 0.0, 10.0]);
    }

    #[test]
    fn an_ordinary_rocket_leaves_where_it_was_aimed() {
        let shot = shot_at([0.0; 3]);
        let LaunchStart { heading: dir, from: origin } = muzzle(0, &shot, [0.0, 1.0, 0.0]);
        assert_eq!((dir, origin), ([0.0, 1.0, 0.0], [100.0, 200.0, 300.0]));
    }

    #[test]
    fn a_javelin_tilts_up_and_starts_off_to_the_right() {
        let shot = shot_at([0.0; 3]);
        let LaunchStart { heading: dir, from: origin } = muzzle(JAVELIN, &shot, [1.0, 0.0, 0.0]);
        let tilted = Vec3::new(1.0, 0.0, JAVELIN_TILT).normalize();
        assert!((Vec3::from_array(dir) - tilted).length() < 1e-6);
        let (_, right, _) = game_math::euler_basis(shot.turn);
        for i in 0..3 {
            assert!((origin[i] - (shot.spot[i] + right[i] * 2.5)).abs() < 1e-5);
        }
    }

    #[test]
    fn a_tv_missile_starts_thirteen_units_along_its_way() {
        let shot = shot_at([0.0; 3]);
        let LaunchStart { heading: dir, from: origin } = muzzle(TV_GUIDED, &shot, [0.0, 0.0, 1.0]);
        assert_eq!(dir, [0.0, 0.0, 1.0]);
        assert_eq!(origin, [100.0, 200.0, 313.0]);
    }
}
