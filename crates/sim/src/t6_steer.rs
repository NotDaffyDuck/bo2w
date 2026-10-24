//! Black Ops II's own steering for its lock-on rockets (`guidedMissileType`
//! 1: the SMAW, the FHJ-18 and the strafe run's rockets) and the Hellstorm's
//! bomblets (2). Each tick the rocket pushes sideways toward its target, no
//! harder than the weapon's `maxSteeringAccel`, and matches its climb to the
//! target's height; a bomblet first climbs gently, then dives once it is
//! high enough. Its speed is never set back: the pushes add to it.

use glam::{Vec2, Vec3};

const SIDEWINDER: i32 = 1;
const HELLFIRE: i32 = 2;
/// BO2's `missileHellfireMaxSlope`: the steepest climb, a share of the flat
/// speed.
const HELLFIRE_MAX_SLOPE: f32 = 0.3;
/// BO2's `missileHellfireUpAccel`: the hardest upward push while climbing.
const HELLFIRE_UP_ACCEL: f32 = 20.0;
/// A target behind still gets the full turn when the circle to it is this
/// much wider than the tightest turn.
const BEHIND_SLACK: f32 = 60.0;
/// How fast the climb matches the one it wants (BO2's own factor).
const MATCH: f32 = 20.0;

/// This guidance steers BO2's way.
pub(crate) fn steers(kind: i32) -> bool {
    matches!(kind, SIDEWINDER | HELLFIRE)
}

/// The rocket's velocity after one tick toward `goal`. `max` is its
/// `maxSteeringAccel`, `speed` its launch speed; `diving` is a bomblet's
/// switch from climbing to diving.
pub(crate) fn steer(
    origin: Vec3,
    velocity: Vec3,
    goal: Vec3,
    kind: i32,
    max: f32,
    speed: f32,
    diving: &mut bool,
) -> Vec3 {
    if max <= 0.0 || speed <= 0.0 {
        return velocity;
    }
    let flat = velocity.truncate();
    let v = flat.length();
    let ahead_dir = flat.normalize_or_zero();
    let right = Vec2::new(ahead_dir.y, -ahead_dir.x);
    let to = goal - origin;
    let fwd = to.truncate().dot(ahead_dir);
    let side = to.truncate().dot(right);
    let push = sideways(fwd, side, v, max, speed);
    let up = climb(kind, fwd, side, to.z, v, velocity.z, max, speed, diving);
    let seconds = crate::MATCH_TICK_MS as f32 * 0.001;
    velocity + (right * push).extend(up) * seconds
}

/// The push to its right (negative: to its left): the turn on a circle
/// through the target, never tighter than the rocket can fly. A target
/// behind gets the full push, unless it sits inside that tightest circle.
fn sideways(fwd: f32, side: f32, v: f32, max: f32, speed: f32) -> f32 {
    let radius = if side == 0.0 {
        f32::MAX
    } else {
        (fwd * fwd + side * side) / (2.0 * side)
    };
    let tightest = speed * speed / max;
    if fwd <= 0.0 {
        if radius.abs() >= tightest + BEHIND_SLACK {
            if side <= 0.0 { -max } else { max }
        } else {
            0.0
        }
    } else if side != 0.0 {
        let r = if radius.abs() < tightest {
            if radius <= 0.0 { -tightest } else { tightest }
        } else {
            radius
        };
        (2.0 * v * v / r).clamp(-max, max)
    } else {
        0.0
    }
}

/// The push up (negative: down). A rocket ahead of its target matches the
/// slope to it; a bomblet climbing wants just enough height to drop onto the
/// target, and dives once it has it.
#[allow(clippy::too_many_arguments)]
fn climb(
    kind: i32,
    fwd: f32,
    side: f32,
    dz: f32,
    v: f32,
    vz: f32,
    max: f32,
    speed: f32,
    diving: &mut bool,
) -> f32 {
    let dist = (fwd * fwd + side * side).sqrt();
    if dist == 0.0 {
        return 0.0;
    }
    if kind == HELLFIRE && !*diving {
        let time = if fwd <= 0.0 || v == 0.0 {
            dist / speed
        } else {
            fwd / speed
        };
        let want = ((dz / time + max * 0.5 * time) * 0.9).min(HELLFIRE_MAX_SLOPE * v);
        if vz < want {
            return ((want - vz) * MATCH).clamp(-max, HELLFIRE_UP_ACCEL);
        }
        *diving = true;
        return 0.0;
    }
    let ahead = fwd / dist;
    if ahead >= 0.0 {
        (((ahead * v).abs() * dz / dist - vz) * MATCH).clamp(-max, max)
    } else {
        0.0
    }
}

/// One tick of a BO2 rocket on its target `point`: steer, then fly on.
pub(crate) fn fly(
    projectile: &mut crate::ProjectileState,
    now: i32,
    point: [f32; 3],
    facts: crate::EquipmentRuntimeFacts,
) {
    let mut diving = projectile.guide.stage != 0;
    let velocity = steer(
        Vec3::from_array(projectile.origin),
        Vec3::from_array(projectile.velocity),
        Vec3::from_array(point),
        facts.missile_guidance,
        facts.max_steering_accel,
        facts.projectile_speed as f32,
        &mut diving,
    );
    projectile.guide.stage = u8::from(diving);
    projectile.velocity = velocity.to_array();
    projectile.pos = entity_t6::Trajectory {
        tr_time: now
            .saturating_sub(crate::MATCH_TICK_MS as i32)
            .max(projectile.spawn_time_ms),
        tr_type: entity_t6::trajectory::tr_type::LINEAR,
        tr_duration: 0,
        tr_delta: projectile.velocity,
        tr_base: projectile.origin,
    };
    if let Some(direction) = velocity.try_normalize() {
        projectile.apos = crate::entity_local::fire_missile_apos(direction.to_array());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMAW_MAX: f32 = 3000.0;
    const SMAW_SPEED: f32 = 2500.0;

    fn tick(velocity: Vec3, goal: Vec3, kind: i32, max: f32, speed: f32) -> Vec3 {
        steer(Vec3::ZERO, velocity, goal, kind, max, speed, &mut false)
    }

    #[test]
    fn only_sidewinders_and_hellfires_steer_this_way() {
        assert!(steers(1) && steers(2));
        assert!(!steers(0) && !steers(3) && !steers(7));
    }

    #[test]
    fn a_target_on_its_right_pulls_it_right_and_no_faster_ahead() {
        let v = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(1000.0, -500.0, 0.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert!(v.y < 0.0, "{v}");
        assert_eq!(v.x, 1000.0);
    }

    #[test]
    fn a_tight_turn_is_held_to_its_steering_limit() {
        // Fast and close: the circle to it is tighter than it can fly.
        let v = tick(
            Vec3::new(2500.0, 0.0, 0.0),
            Vec3::new(10.0, -10.0, 0.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert!((v.y + SMAW_MAX * 0.05).abs() < 1e-3, "{v}");
    }

    #[test]
    fn a_target_behind_gets_the_full_turn_unless_it_is_too_close() {
        let far = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(-1000.0, -10.0, 0.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert!((far.y + SMAW_MAX * 0.05).abs() < 1e-3, "{far}");
        let near = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(-10.0, -100.0, 0.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert_eq!(near, Vec3::new(1000.0, 0.0, 0.0));
    }

    #[test]
    fn a_sidewinder_matches_the_slope_to_its_target() {
        // The FHJ-18 pulls hard enough to match it in one tick.
        let v = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(1000.0, 0.0, 500.0),
            SIDEWINDER,
            10000.0,
            3000.0,
        );
        assert!((v.z - 500.0).abs() < 1e-3, "{v}");
        // The SMAW gets there over a few ticks.
        let v = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(1000.0, 0.0, 500.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert!((v.z - SMAW_MAX * 0.05).abs() < 1e-3, "{v}");
        // Nothing up or down for a target behind.
        let v = tick(
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(-1000.0, 0.0, 500.0),
            SIDEWINDER,
            SMAW_MAX,
            SMAW_SPEED,
        );
        assert_eq!(v.z, 0.0);
    }

    #[test]
    fn a_bomblet_climbs_gently_then_dives() {
        let (max, speed) = (4000.0, 5000.0);
        let goal = Vec3::new(2000.0, 0.0, 0.0);
        let mut diving = false;
        let v = steer(
            Vec3::ZERO,
            Vec3::new(1000.0, 0.0, 0.0),
            goal,
            HELLFIRE,
            max,
            speed,
            &mut diving,
        );
        assert!(!diving);
        assert!((v.z - HELLFIRE_UP_ACCEL * 0.05).abs() < 1e-4, "{v}");
        // High enough: it dives, and that tick pushes nothing up or down.
        let v = steer(
            Vec3::ZERO,
            Vec3::new(1000.0, 0.0, 400.0),
            goal,
            HELLFIRE,
            max,
            speed,
            &mut diving,
        );
        assert!(diving);
        assert_eq!(v.z, 400.0);
        // Diving, it steers like a sidewinder: down onto the target.
        let v = steer(
            Vec3::ZERO,
            Vec3::new(1000.0, 0.0, 400.0),
            goal,
            HELLFIRE,
            max,
            speed,
            &mut diving,
        );
        assert!((v.z - (400.0 - max * 0.05)).abs() < 1e-3, "{v}");
    }

    #[test]
    fn no_steering_limit_steers_nothing() {
        let v = Vec3::new(1000.0, 0.0, 0.0);
        assert_eq!(tick(v, Vec3::new(0.0, -500.0, 300.0), SIDEWINDER, 0.0, SMAW_SPEED), v);
    }
}
