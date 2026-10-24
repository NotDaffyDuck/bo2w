//! The hunter-killer drone. Launched softly, it hunts toward the goal its script keeps moving (stage 1) until the
//! script gives it a target it can see; then it flies straight at that target (stage 2), banking into its turns. It
//! never steers round walls: it either has a clear line to its target or it carries on hunting.

use super::{Heading, set_course, turn::bend};
use crate::bullet_collision::EntityCollisionTraceGeom;
use crate::frame::FrameWorld;
use glam::Vec3;

/// `guidedMissileType` of a drone.
pub(super) const KIND: i32 = 7;

const HUNTING: u8 = 1;
const LOCKED: u8 = 2;

/// Numbers that shape the flight. Most come from the game's own drone settings; the ones marked "play" were tuned to
/// how the drone should feel in play.
mod flight {
    /// Lifetime of the projectile, ms; it blows up when it runs out.
    pub const LIFETIME_MS: i32 = 25_000;
    /// Acceleration when flying level or diving, units/s², and the speed it stops gaining at.
    pub const ACCEL: f32 = 900.0;
    pub const TOP_SPEED: f32 = 2000.0;
    /// Share of speed lost per 180 degrees of turn.
    pub const TURN_DRAG: f32 = 0.15;
    /// The drone never slows below this.
    pub const MIN_SPEED: f32 = 400.0;
    /// A turn under this many degrees still lets it speed up.
    pub const STRAIGHT_ENOUGH: f32 = 30.0;
    /// Play: turn rate (degrees a second) of a drone chasing a locked target.
    pub const LOCK_TURN_RATE: f32 = 360.0;
    /// Play: how far above its launch point a hunting drone cruises.
    pub const HUNT_HEIGHT: f32 = 350.0;
    /// Play: steepest pitch (degrees) a hunting drone climbs or drops at.
    pub const HUNT_MAX_PITCH: f32 = 8.0;
    /// Play: a hunting drone's turn rate, degrees a second.
    pub const HUNT_TURN_RATE: f32 = 120.0;
}

/// One steering step (a server frame; BO2's equals our 50 ms tick) for a drone heading for `target`.
pub(super) fn steer(
    world: &FrameWorld,
    projectile: &mut crate::ProjectileState,
    now: i32,
    facts: crate::EquipmentRuntimeFacts,
    target: Vec3,
) {
    let age = now.saturating_sub(projectile.spawn_time_ms);
    if projectile.guide.stage == 0 {
        projectile.guide.from = projectile.origin;
        if age < facts.ignition_delay_ms {
            return;
        }
        projectile.guide.stage = HUNTING;
    }
    if age >= flight::LIFETIME_MS {
        projectile.detonate_at_ms = Some(now);
        return;
    }
    let origin = Vec3::from_array(projectile.origin);
    let velocity = Vec3::from_array(projectile.velocity);
    let heading = Heading {
        origin,
        velocity,
        forward: velocity.try_normalize().unwrap_or(Vec3::X),
    };
    // No lock: the target is just the script's moving goal.
    if projectile.guide.home.is_some() && projectile.guide.target == projectile.guide.home {
        hunt(projectile, now, heading, target);
        return;
    }
    // Locked: straight at the target, but only while nothing solid is in the way (the lock sees through cars; the
    // flight does not). Without a clear line it hunts on as if it had no lock.
    let to_aim = target - origin;
    let blockers = solid_entities(world, projectile);
    if clear_run(world, &blockers, origin, target) < to_aim.length() - 64.0 {
        let goal = match projectile.guide.home {
            Some(crate::MissileTarget::Point(goal)) => Vec3::from_array(goal),
            _ => origin + heading.forward * 1024.0,
        };
        hunt(projectile, now, heading, goal);
        return;
    }
    projectile.guide.stage = LOCKED;
    let wanted = to_aim.try_normalize().unwrap_or(Vec3::X);
    let forward = velocity.try_normalize().unwrap_or(wanted);
    let bent = bend(forward, wanted, flight::LOCK_TURN_RATE, false, projectile.guide.roll);
    let speed = cruise_speed(velocity.length(), bent.wanted_turn);
    if trace_on() {
        diag::info!(
            Sim,
            "drone trace t {now} lock origin {origin:?} target {target:?} vel {velocity:?} dir {:?} speed {speed} turn {}",
            bent.heading,
            bent.wanted_turn
        );
    }
    fly(projectile, now, bent.heading, speed, bent.bank);
}

/// The direction a hunting drone wants to fly: toward `goal` on the ground plane, pitched to hold its cruise height
/// above where it was launched.
fn hunt_direction(heading: Heading, goal: Vec3, launch_z: f32) -> Vec3 {
    use flight::*;
    let flat = (goal - heading.origin)
        .truncate()
        .try_normalize()
        .or_else(|| heading.forward.truncate().try_normalize())
        .unwrap_or(glam::Vec2::X);
    let hold = launch_z + HUNT_HEIGHT;
    let lookahead = (heading.velocity.truncate().length() * 0.5).max(200.0);
    let limit = HUNT_MAX_PITCH.to_radians();
    let pitch = ((hold - heading.origin.z) / lookahead).atan().clamp(-limit, limit);
    Vec3::new(flat.x * pitch.cos(), flat.y * pitch.cos(), pitch.sin())
}

/// With no lock the drone flies fast and low toward `goal`, the point its script keeps moving (down the throw line,
/// then round the map's middle), and holds a cruise height over where it was thrown instead of the script's own.
fn hunt(projectile: &mut crate::ProjectileState, now: i32, heading: Heading, goal: Vec3) {
    let wanted = hunt_direction(heading, goal, projectile.guide.from[2]);
    let bent = bend(
        heading.forward,
        wanted,
        flight::HUNT_TURN_RATE,
        false,
        projectile.guide.roll,
    );
    let speed = cruise_speed(heading.velocity.length(), bent.wanted_turn);
    if trace_on() {
        diag::info!(
            Sim,
            "drone trace t {now} hunt origin {:?} goal {goal:?} vel {:?} dir {:?} speed {speed} turn {}",
            heading.origin,
            heading.velocity,
            bent.heading,
            bent.wanted_turn
        );
    }
    fly(projectile, now, bent.heading, speed, bent.bank);
}

/// The speed after a turn of `turn` degrees: it gains on a straight run, loses some in a hard turn, and never drops
/// under the minimum.
fn cruise_speed(speed: f32, turn: f32) -> f32 {
    use flight::*;
    let mut speed = speed;
    if turn < STRAIGHT_ENOUGH {
        speed = (speed + ACCEL * 0.05).min(TOP_SPEED);
    }
    (speed * (1.0 - turn / 180.0 * TURN_DRAG)).max(MIN_SPEED)
}

/// Commits a step: the new bank, velocity and path, and an orientation that leans into the bank.
fn fly(projectile: &mut crate::ProjectileState, now: i32, direction: Vec3, speed: f32, bank: f32) {
    projectile.guide.roll = bank;
    set_course(projectile, now, direction, speed);
    let mut apos = crate::entity_local::fire_missile_apos(direction.to_array());
    apos.tr_base[2] = bank;
    projectile.apos = apos;
}

/// The solid things in the drone's way: cars, props and other entities, but not the models it carries itself.
fn solid_entities(world: &FrameWorld, projectile: &crate::ProjectileState) -> Vec<EntityCollisionTraceGeom> {
    let own = world.missile_collision_models(projectile.id);
    world
        .entity_collision_capabilities()
        .iter()
        .filter(|capabilities| !capabilities.owner.script_model().is_some_and(|id| own.contains(&id)))
        .map(|capabilities| capabilities.trace_geom())
        .collect()
}

/// How far a drone gets from `from` toward `to` before the world, glass or one of `blockers` stops it. Players do
/// not stop it: it flies into its target.
fn clear_run(world: &FrameWorld, blockers: &[EntityCollisionTraceGeom], from: Vec3, to: Vec3) -> f32 {
    use crate::bullet_collision::{BulletTraceQuery, MASK_BULLET_WORLD, TraceOutcome};
    let content = world.content();
    let objects = world.world_objects();
    let outcome = crate::bullet_collision::bullet_trace_with_entity_models(
        content.clip_brushes(),
        content.clip_bsp(),
        content.clip_cmodels(),
        content.clip_mesh(),
        &[],
        blockers,
        &BulletTraceQuery {
            start: from.to_array(),
            end: to.to_array(),
            mask: MASK_BULLET_WORLD,
            ignore: None,
            ignore_hit: None,
            ignore_model: None,
        },
        &|piece| objects.glass_is_solid(piece as u32),
    );
    let length = (to - from).length();
    match outcome {
        TraceOutcome::Hit { fraction, .. } => fraction * length,
        TraceOutcome::StartSolid { .. } => 0.0,
        _ => length,
    }
}

/// `BO2MP_DRONE_TRACE` set in the environment logs every steering step.
fn trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_DRONE_TRACE").is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(origin: Vec3, velocity: Vec3) -> Heading {
        Heading {
            origin,
            velocity,
            forward: velocity.normalize(),
        }
    }

    #[test]
    fn a_straight_run_speeds_up_by_the_dive_numbers() {
        assert_eq!(cruise_speed(1000.0, 0.0), 1045.0);
    }

    #[test]
    fn speed_stops_gaining_at_the_top() {
        assert_eq!(cruise_speed(1990.0, 0.0), 2000.0);
    }

    #[test]
    fn a_slow_drone_is_lifted_to_the_minimum() {
        assert_eq!(cruise_speed(100.0, 0.0), 400.0);
        assert_eq!(cruise_speed(10.0, 170.0), 400.0);
    }

    #[test]
    fn a_hard_turn_drags_speed_down_without_adding_any() {
        let out = cruise_speed(1000.0, 90.0);
        assert!((out - 1000.0 * (1.0 - 0.5 * 0.15)).abs() < 1e-3);
    }

    #[test]
    fn a_hunter_at_cruise_height_flies_level_toward_the_goal() {
        let here = at(Vec3::new(0.0, 0.0, 350.0), Vec3::new(800.0, 0.0, 0.0));
        let dir = hunt_direction(here, Vec3::new(5000.0, 0.0, 0.0), 0.0);
        assert!((dir - Vec3::X).length() < 1e-4, "{dir:?}");
    }

    #[test]
    fn a_hunter_far_below_cruise_height_climbs_at_the_pitch_limit() {
        let here = at(Vec3::ZERO, Vec3::new(800.0, 0.0, 0.0));
        let dir = hunt_direction(here, Vec3::new(0.0, 5000.0, 0.0), 0.0);
        assert!((dir.z - flight::HUNT_MAX_PITCH.to_radians().sin()).abs() < 1e-5);
        assert!(dir.y > 0.9 && dir.x.abs() < 1e-5);
    }

    #[test]
    fn a_hunter_far_above_drops_at_the_pitch_limit() {
        let here = at(Vec3::new(0.0, 0.0, 3000.0), Vec3::new(800.0, 0.0, 0.0));
        let dir = hunt_direction(here, Vec3::new(5000.0, 0.0, 0.0), 0.0);
        assert!((dir.z + flight::HUNT_MAX_PITCH.to_radians().sin()).abs() < 1e-5);
    }

    #[test]
    fn a_hunter_sitting_on_its_goal_keeps_its_own_heading() {
        let here = at(Vec3::new(0.0, 0.0, 350.0), Vec3::new(0.0, 600.0, 0.0));
        let dir = hunt_direction(here, here.origin, 0.0);
        assert!((dir - Vec3::Y).length() < 1e-4, "{dir:?}");
    }
}
