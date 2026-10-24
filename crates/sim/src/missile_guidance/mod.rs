//! Guided missiles: the state a script's homing calls leave on a projectile, and the per-frame steering that turns
//! it into a new velocity and flight path.
//!
//! Three steering modes share this entry point: the plain homing missile, the javelin (climb, then dive), and the
//! hunter-killer drone. Black Ops II's own guidance types are handed to `t6_steer`.

mod drone;
mod javelin;
mod target;
mod turn;

use crate::frame::FrameWorld;
use glam::Vec3;

pub use target::MissileTarget;
pub(crate) use turn::turn_toward;

/// The guidance a projectile carries: who it hunts and where its steering has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissileGuide {
    pub target: Option<MissileTarget>,
    /// Attack from above (javelin top-attack mode).
    pub top: bool,
    /// How far through its flight plan the missile is: 0 launching, 1 climbing or hunting, 2 homing.
    pub stage: u8,
    /// The missile has overshot its target and no longer steers.
    pub passed: bool,
    /// The spot on its target the scripts asked for (`missile_settarget(ent, offset)`). It is already folded into
    /// `target`'s point; a drone's climb measures its ceiling without it.
    pub offset: [f32; 3],
    /// A drone's bank in degrees, carried from frame to frame.
    pub roll: f32,
    /// The goal a drone's script last sent it toward. While there is no lock `target` is this; it is kept through a
    /// lock so the drone can hunt on if it loses sight of what it locked.
    pub home: Option<MissileTarget>,
    /// Where a drone was launched.
    pub from: [f32; 3],
}

/// `guidedMissileType` of the javelin.
const JAVELIN: i32 = 3;

/// Where a projectile is and which way it travels, worked out once per steering step.
#[derive(Clone, Copy)]
struct Heading {
    origin: Vec3,
    velocity: Vec3,
    forward: Vec3,
}

/// Seconds in one match tick.
fn tick_seconds() -> f32 {
    crate::MATCH_TICK_MS as f32 * 0.001
}

/// Points a projectile along `direction` at `speed`: velocity, and a straight-line path that starts one tick back.
fn set_course(projectile: &mut crate::ProjectileState, now: i32, direction: Vec3, speed: f32) {
    projectile.velocity = (direction * speed).to_array();
    projectile.pos = entity_t6::Trajectory {
        tr_time: now
            .saturating_sub(crate::MATCH_TICK_MS as i32)
            .max(projectile.spawn_time_ms),
        tr_type: entity_t6::trajectory::tr_type::LINEAR,
        tr_duration: 0,
        tr_delta: projectile.velocity,
        tr_base: projectile.origin,
    };
}

/// One steering step for a projectile, at match time `now`.
pub(crate) fn steer(
    world: &FrameWorld,
    projectile: &mut crate::ProjectileState,
    now: i32,
    facts: crate::EquipmentRuntimeFacts,
) {
    if !projectile.live || projectile.guide.passed {
        return;
    }
    let Some(target) = projectile.guide.target else {
        return;
    };
    let Some(point) = target.resolve(world) else {
        projectile.guide.target = None;
        return;
    };
    let velocity = Vec3::from_array(projectile.velocity);
    let Some(forward) = velocity.try_normalize() else {
        return;
    };
    let heading = Heading {
        origin: Vec3::from_array(projectile.origin),
        velocity,
        forward,
    };
    let mode = facts.missile_guidance;
    if mode == drone::KIND {
        drone::steer(world, projectile, now, facts, Vec3::from_array(point));
    } else if facts.projectile_parent_velocity.is_some() && crate::t6_steer::steers(mode) {
        crate::t6_steer::fly(projectile, now, point, facts);
    } else if mode == JAVELIN {
        javelin::steer(world, projectile, now, facts, heading, Vec3::from_array(point));
    } else {
        home_in(projectile, now, heading, Vec3::from_array(point));
    }
}

/// The plain homing missile: keeps its speed and turns toward the target at a fixed rate.
fn home_in(projectile: &mut crate::ProjectileState, now: i32, heading: Heading, point: Vec3) {
    /// Degrees a second.
    const TURN_RATE: f32 = 240.0;
    let Some(wanted) = (point - heading.origin).try_normalize() else {
        return;
    };
    let direction = turn_toward(heading.forward, wanted, TURN_RATE.to_radians() * tick_seconds());
    set_course(projectile, now, direction, heading.velocity.length());
    projectile.apos = crate::entity_local::fire_missile_apos(direction.to_array());
}
