//! The javelin: a short soft launch, a climb well above the target (higher when it attacks from the top), then a dive
//! onto it that turns slowly and speeds up when it points the right way.

use super::{Heading, set_course, tick_seconds, turn::facing};
use crate::frame::FrameWorld;
use glam::Vec3;

const CLIMBING: u8 = 1;
const DIVING: u8 = 2;

/// How far the aim point is lifted while climbing, by attack mode.
fn climb_lift(top: bool) -> f32 {
    if top { 15000.0 } else { 10000.0 }
}

/// Turn rate in degrees a second, by attack mode.
fn turn_rate(top: bool) -> f32 {
    if top { 100.0 } else { 60.0 }
}

pub(super) fn steer(
    world: &FrameWorld,
    projectile: &mut crate::ProjectileState,
    now: i32,
    facts: crate::EquipmentRuntimeFacts,
    heading: Heading,
    point: Vec3,
) {
    if now.saturating_sub(projectile.spawn_time_ms) < facts.ignition_delay_ms {
        return;
    }
    projectile.guide.stage = projectile.guide.stage.max(CLIMBING);
    let goal = aim_point(world, projectile, heading, point);
    if projectile.guide.stage == DIVING && has_passed(heading, point - heading.origin) {
        projectile.guide.passed = true;
        return;
    }
    let Some(wanted) = (goal - heading.origin).try_normalize() else {
        return;
    };
    let (direction, turn) = bend(heading.forward, wanted, turn_rate(projectile.guide.top), tick_seconds());
    let climbing = projectile.guide.stage == CLIMBING || heading.velocity.z > 0.0;
    let speed = next_speed(heading.velocity.length(), turn, climbing, tick_seconds());
    set_course(projectile, now, direction, speed);
    projectile.apos = crate::entity_local::fire_missile_apos(direction.to_array());
}

/// Where to aim this frame. While climbing that is a point far above (and, in the direct mode, back toward the
/// owner); once the missile is high enough, or close enough, it switches to diving and aims at the target itself.
fn aim_point(world: &FrameWorld, projectile: &mut crate::ProjectileState, heading: Heading, point: Vec3) -> Vec3 {
    if projectile.guide.stage != CLIMBING {
        return point;
    }
    let top = projectile.guide.top;
    let delta = point - heading.origin;
    let flat_forward = heading.forward.truncate().normalize_or_zero();
    let slope = (flat_forward.dot(delta.truncate()) / delta.z).atan().abs().to_degrees();
    let (ceiling, slope_limit) = if top { (3000.0, 50.0) } else { (0.0, 85.0) };
    let high_enough = heading.origin.z - point.z > ceiling;
    if high_enough && (slope < slope_limit || delta.truncate().length() < 400.0) {
        projectile.guide.stage = DIVING;
        return point;
    }
    let mut goal = point;
    goal.z += climb_lift(top);
    if !top && let Some(owner) = world.player(projectile.owner) {
        let toward_owner = (Vec3::from_array(owner.origin) - goal).truncate().normalize_or_zero();
        goal.x += toward_owner.x * 700.0;
        goal.y += toward_owner.y * 700.0;
    }
    goal
}

/// Whether a diving missile is within 300 units of its target and already moving away from it.
fn has_passed(heading: Heading, delta: Vec3) -> bool {
    delta.length_squared() <= 90000.0 && heading.forward.dot(delta.normalize_or_zero()) < -0.2
}

/// A heading one tick closer to `wanted`, and the size of the turn still asked for (degrees, 0 when it arrives).
fn bend(forward: Vec3, wanted: Vec3, rate_dps: f32, seconds: f32) -> (Vec3, f32) {
    let turn = (90.0 * (1.0 - forward.dot(wanted))).clamp(0.0, 180.0);
    if turn <= 0.1 || turn <= rate_dps * seconds {
        return (wanted, 0.0);
    }
    let heading = facing(forward).slerp(facing(wanted), rate_dps * seconds / turn) * Vec3::X;
    (heading, turn)
}

/// Speed after one tick: it accelerates while pointed close to where it wants to go (gently while climbing, hard when
/// diving, up to a cap either way), and every degree of turn costs a little.
fn next_speed(speed: f32, turn: f32, climbing: bool, seconds: f32) -> f32 {
    let mut speed = speed;
    if turn < 30.0 {
        let (accel, cap) = if climbing { (300.0, 1000.0) } else { (4500.0, 2500.0) };
        speed = (speed + accel * seconds).min(cap);
    }
    speed * (1.0 - turn / 180.0 * 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: f32 = 0.05;

    fn heading(origin: Vec3, forward: Vec3) -> Heading {
        Heading {
            origin,
            velocity: forward * 1000.0,
            forward,
        }
    }

    #[test]
    fn the_top_mode_climbs_higher_and_turns_faster() {
        assert!(climb_lift(true) > climb_lift(false));
        assert!(turn_rate(true) > turn_rate(false));
    }

    #[test]
    fn a_missile_pointing_at_its_goal_turns_nothing() {
        let (dir, turn) = bend(Vec3::X, Vec3::X, 60.0, TICK);
        assert_eq!((dir, turn), (Vec3::X, 0.0));
    }

    #[test]
    fn a_turn_inside_one_ticks_allowance_arrives_at_once() {
        // 60 deg/s over 0.05 s is 3 degrees of allowance; this wants about 1.
        let wanted = Vec3::new(1.0, 0.1, 0.0).normalize();
        let (dir, turn) = bend(Vec3::X, wanted, 60.0, TICK);
        assert_eq!(dir, wanted);
        assert_eq!(turn, 0.0);
    }

    #[test]
    fn a_big_turn_moves_by_the_rate_and_reports_its_size() {
        let (dir, turn) = bend(Vec3::X, Vec3::Y, 60.0, TICK);
        assert!((turn - 90.0).abs() < 1e-3);
        // The turn is 90 (a quarter of a turn on its measure); the step is rate*seconds/turn = 1/30 of the way.
        let moved = dir.angle_between(Vec3::X).to_degrees();
        assert!((moved - 90.0 / 30.0).abs() < 0.1, "{moved}");
    }

    #[test]
    fn climbing_speeds_up_slowly_and_stops_at_its_cap() {
        assert_eq!(next_speed(500.0, 0.0, true, TICK), 515.0);
        assert_eq!(next_speed(995.0, 0.0, true, TICK), 1000.0);
    }

    #[test]
    fn diving_speeds_up_hard_and_stops_at_its_cap() {
        assert_eq!(next_speed(1000.0, 0.0, false, TICK), 1225.0);
        assert_eq!(next_speed(2400.0, 0.0, false, TICK), 2500.0);
    }

    #[test]
    fn a_sharp_turn_costs_speed_and_gains_none() {
        let out = next_speed(1000.0, 90.0, false, TICK);
        assert!((out - 1000.0 * (1.0 - 0.5 * 0.05)).abs() < 1e-3);
    }

    #[test]
    fn it_has_passed_only_when_close_and_moving_away() {
        let here = heading(Vec3::ZERO, Vec3::X);
        assert!(has_passed(here, Vec3::new(-100.0, 0.0, 0.0)));
        assert!(!has_passed(here, Vec3::new(100.0, 0.0, 0.0)), "target still ahead");
        assert!(!has_passed(here, Vec3::new(-1000.0, 0.0, 0.0)), "too far to count");
    }
}
