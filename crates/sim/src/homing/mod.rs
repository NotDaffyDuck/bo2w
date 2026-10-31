//! Guided missiles: the state a script's homing calls leave on a rocket, and the per-frame steering that turns
//! it into a new velocity and flight path.
//!
//! Three steering modes share this entry point: the plain homing missile, the javelin (climb, then dive), and the
//! hunter-killer drone. Black Ops II's own guidance types are handed to `t6_steer`.

mod drone;
mod javelin;
mod target;
mod turn;

use crate::step_view::StepWorldView;
use glam::Vec3;

pub use target::MissileTarget;
pub(crate) use turn::turn_toward;

/// The guidance a rocket carries: who it hunts and where its steering has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissileGuide {
    pub target: Option<MissileTarget>,
    /// Attack from above (javelin top-attack mode).
    pub top: bool,
    /// How far through its flight plan the missile is: 0 launching, 1 climbing or hunting, 2 homing.
    pub stage: u8,
    /// The missile has overshot its target and no longer steers.
    pub went_by: bool,
    /// The spot on its target the scripts asked for (`missile_settarget(ent, offset)`). It is already folded into
    /// `target`'s point; a drone's climb measures its ceiling without it.
    pub offset: [f32; 3],
    /// A drone's bank in degrees, carried from frame to frame.
    pub bank: f32,
    /// The goal a drone's script last sent it toward. While there is no lock `target` is this; it is kept through a
    /// lock so the drone can hunt on if it loses sight of what it locked.
    pub home: Option<MissileTarget>,
    /// Where a drone was launched.
    pub from: [f32; 3],
}

/// `guidedMissileType` of the javelin.
const JAVELIN: i32 = 3;

/// Where a rocket is and which way it travels, worked out once per steering step.
#[derive(Clone, Copy)]
struct Heading {
    location: Vec3,
    velocity: Vec3,
    forward: Vec3,
}

/// Seconds in one match tick.
fn tick_seconds() -> f32 {
    crate::SIM_STEP_MILLIS as f32 * 0.001
}

/// Points a rocket along `direction` at `speed`: velocity, and a straight-line path that starts one tick back.
fn set_course(rocket: &mut crate::FlyingProjectile, now: i32, direction: Vec3, speed: f32) {
    rocket.motion = (direction * speed).to_array();
    rocket.at = entity_t6::Trajectory {
        began_at: now
            .saturating_sub(crate::SIM_STEP_MILLIS as i32)
            .max(rocket.spawn_time_ms),
        tr_type: entity_t6::flight::path_kind::STRAIGHT,
        span_ms: Default::default(),
        rate: rocket.motion,
        base_point: rocket.origin,
    };
}

/// One steering step for a rocket, at match time `now`.
pub(crate) fn steer(
    world: &StepWorldView,
    rocket: &mut crate::FlyingProjectile,
    now: i32,
    facts: crate::GadgetLiveStats,
) {
    if !rocket.in_flight || rocket.guide.went_by {
        return;
    }
    let Some(target) = rocket.guide.target else {
        return;
    };
    let Some(point) = target.resolve(world) else {
        rocket.guide.target = None;
        return;
    };
    let velocity = Vec3::from_array(rocket.motion);
    let Some(forward) = velocity.try_normalize() else {
        return;
    };
    let heading = Heading {
        location: Vec3::from_array(rocket.origin),
        velocity,
        forward,
    };
    let mode = facts.missile_guidance;
    if mode == drone::KIND {
        drone::steer(world, rocket, now, facts, Vec3::from_array(point));
    } else if facts.projectile_parent_velocity.is_some() && crate::lock_steer::steers(mode) {
        crate::lock_steer::fly(rocket, now, point, facts);
    } else if mode == JAVELIN {
        javelin::steer(world, rocket, now, facts, heading, Vec3::from_array(point));
    } else {
        home_in(rocket, now, heading, Vec3::from_array(point));
    }
}

/// The plain homing missile: keeps its speed and turns toward the target at a fixed rate.
fn home_in(rocket: &mut crate::FlyingProjectile, now: i32, heading: Heading, point: Vec3) {
    /// Degrees a second.
    const TURN_RATE: f32 = 240.0;
    let Some(wanted) = (point - heading.location).try_normalize() else {
        return;
    };
    let direction = turn_toward(heading.forward, wanted, TURN_RATE.to_radians() * tick_seconds());
    set_course(rocket, now, direction, heading.velocity.length());
    rocket.turn_path = crate::ent_links::fire_missile_apos(direction.to_array());
}
