use crate::frame::FrameWorld;
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MissileTarget {
    Point([f32; 3]),
    Entity {
        entity: crate::EntityRef,
        offset: [f32; 3],
    },
    Player {
        client: crate::ClientId,
        life: crate::LifeSequence,
        offset: [f32; 3],
    },
}

impl MissileTarget {
    pub fn is_point(self) -> bool {
        matches!(self, Self::Point(_))
    }

    pub(crate) fn resolve(self, world: &FrameWorld) -> Option<[f32; 3]> {
        let (origin, angles, offset) = match self {
            Self::Point(point) => return Some(point),
            Self::Entity { entity, offset } => {
                world.entity_kernel().resolve(entity).ok()?;
                let mover = world.script_mover_by_number(entity.number())?;
                (mover.state.lerp.pos.tr_base, mover.state.lerp.apos.tr_base, offset)
            }
            Self::Player {
                client,
                life,
                offset,
            } => {
                let meta = world.client_meta(client)?;
                if meta.life_sequence != life || meta.lifecycle != crate::ClientLifecycle::Alive {
                    return None;
                }
                let ps = world.player(client)?;
                (ps.origin, ps.viewangles, offset)
            }
        };
        let axis = game_math::angles_to_axis(angles);
        Some(core::array::from_fn(|i| {
            origin[i] + offset[0] * axis[0][i] + offset[1] * axis[1][i] + offset[2] * axis[2][i]
        }))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissileGuide {
    pub target: Option<MissileTarget>,
    pub top: bool,
    pub stage: u8,
    pub passed: bool,
    /// The offset the scripts aimed at on their target
    /// (`missile_settarget(ent, offset)`), already in `target`'s point: a
    /// drone's climb measures its ceiling without it.
    pub offset: [f32; 3],
    /// A drone's bank (degrees), kept frame to frame.
    pub roll: f32,
    /// The goal a drone's script last had it hunt toward (`target` is this
    /// while it has no lock); kept through a lock, so it can hunt on if it
    /// loses sight of its target.
    pub home: Option<MissileTarget>,
    /// Where a drone started.
    pub from: [f32; 3],
}

pub(crate) fn turn_toward(current: Vec3, wanted: Vec3, max_radians: f32) -> Vec3 {
    let angle = current.angle_between(wanted);
    if angle <= max_radians || angle.is_nan() {
        return wanted;
    }
    let axis = current.cross(wanted);
    let axis = if axis.length_squared() < 1e-8 {
        current.any_orthonormal_vector()
    } else {
        axis.normalize()
    };
    glam::Quat::from_axis_angle(axis, max_radians) * current
}

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
    let origin = Vec3::from_array(projectile.origin);
    let velocity = Vec3::from_array(projectile.velocity);
    let Some(forward) = velocity.try_normalize() else {
        return;
    };
    if facts.missile_guidance == DRONE {
        drone(world, projectile, now, facts, Vec3::from_array(point));
        return;
    }
    // Black Ops II's sidewinders and hellfires steer their own way.
    if facts.projectile_parent_velocity.is_some() && crate::t6_steer::steers(facts.missile_guidance) {
        crate::t6_steer::fly(projectile, now, point, facts);
        return;
    }
    let delta = Vec3::from_array(point) - origin;
    let seconds = crate::MATCH_TICK_MS as f32 * 0.001;
    let mut speed = velocity.length();
    let mut goal = Vec3::from_array(point);
    let javelin = facts.missile_guidance == 3;
    if javelin {
        if now.saturating_sub(projectile.spawn_time_ms) < facts.ignition_delay_ms {
            return;
        }
        projectile.guide.stage = projectile.guide.stage.max(1);
        if projectile.guide.stage == 1 {
            let flat_forward = forward.truncate().normalize_or_zero();
            let climb_angle = (flat_forward.dot(delta.truncate()) / delta.z)
                .atan()
                .abs()
                .to_degrees();
            let ceiling = if projectile.guide.top { 3000.0 } else { 0.0 };
            let limit = if projectile.guide.top { 50.0 } else { 85.0 };
            if origin.z - point[2] > ceiling
                && (climb_angle < limit || delta.truncate().length() < 400.0)
            {
                projectile.guide.stage = 2;
            } else {
                goal.z += if projectile.guide.top {
                    15000.0
                } else {
                    10000.0
                };
                if !projectile.guide.top
                    && let Some(owner) = world.player(projectile.owner)
                {
                    let toward_owner = (Vec3::from_array(owner.origin) - goal)
                        .truncate()
                        .normalize_or_zero();
                    goal.x += toward_owner.x * 700.0;
                    goal.y += toward_owner.y * 700.0;
                }
            }
        }
        if projectile.guide.stage == 2
            && delta.length_squared() <= 90000.0
            && forward.dot(delta.normalize_or_zero()) < -0.2
        {
            projectile.guide.passed = true;
            return;
        }
    }
    let Some(wanted) = (goal - origin).try_normalize() else {
        return;
    };
    let rate: f32 = if javelin {
        if projectile.guide.top { 100.0 } else { 60.0 }
    } else {
        240.0
    };
    let (direction, turn) = if javelin {
        let turn = (90.0 * (1.0 - forward.dot(wanted))).clamp(0.0, 180.0);
        if turn <= 0.1 || turn <= rate * seconds {
            (wanted, 0.0)
        } else {
            let rotation = |dir: Vec3| {
                let angles = gsc_t6::math::vector_to_angles(dir.to_array());
                glam::Quat::from_euler(
                    glam::EulerRot::ZYX,
                    angles[1].to_radians(),
                    angles[0].to_radians(),
                    0.0,
                )
            };
            (
                rotation(forward).slerp(rotation(wanted), rate * seconds / turn) * Vec3::X,
                turn,
            )
        }
    } else {
        (
            turn_toward(forward, wanted, rate.to_radians() * seconds),
            0.0,
        )
    };
    if javelin {
        let climbing = projectile.guide.stage == 1 || velocity.z > 0.0;
        if turn < 30.0 {
            speed = (speed + if climbing { 300.0 } else { 4500.0 } * seconds).min(if climbing {
                1000.0
            } else {
                2500.0
            });
        }
        speed *= 1.0 - turn / 180.0 * 0.05;
    }
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
    projectile.apos = crate::entity_local::fire_missile_apos(direction.to_array());
}

/// BO2's `guidedMissileType` for a drone (the Hunter Killer).
const DRONE: i32 = 7;

/// BO2's drone dvars (g_missile.cpp, their defaults in the exe). A script's
/// goal is never a "top attack" target, so a drone always flies DIRECT. The
/// climb and slow-turn ones are kept for the record: his 10-15 play has a
/// locked drone fly straight and fast at its target instead.
#[allow(dead_code)]
mod drone_dvars {
    pub const CLIMB_HEIGHT_DIRECT: f32 = 0.0;
    pub const CLIMB_ANGLE_DIRECT: f32 = 90.0;
    pub const CLIMB_CEILING_DIRECT: f32 = 0.0;
    pub const CLIMB_MAX_DIST: f32 = 400.0;
    pub const CLIMB_TO_OWNER: f32 = 700.0;
    pub const TURN_RATE_DESCENT: f32 = 45.0;
    pub const TURN_RATE_ASCENT: f32 = 25.0;
    pub const TURN_RATE_AVOIDANCE: f32 = 100.0;
    pub const TURN_RATE_SMALL_ADJUSTMENTS: f32 = 0.9;
    pub const FLATTEN_ROLL_RATIO: f32 = 0.85;
    pub const DISTANCE_AVOIDANCE: f32 = 800.0;
    pub const DISTANCE_AVOIDANCE_SEC: f32 = 2.0;
    pub const DIST_SQ_TARGET_AVOIDANCE_DISABLED: f32 = 640_000.0;
    pub const TURN_MAX_ROLL: f32 = 40.0;
    pub const TURN_MAX_ROLL_CHANGE: f32 = 3.0;
    pub const TURN_MAX_RATE: f32 = 300.0;
    pub const ACCEL_CLIMB: f32 = 1.0;
    pub const ACCEL_DESCEND: f32 = 900.0;
    pub const SPEED_LIMIT_CLIMB: f32 = 300.0;
    pub const SPEED_LIMIT_DESCEND: f32 = 2000.0;
    pub const TURN_DECEL: f32 = 0.02;
    /// missile_drone_projectile_mp's projLifetime (t6wdef): 25 s, then it
    /// blows up.
    pub const LIFETIME_MS: i32 = 25_000;
    /// bo2mp, his play (10-15: "Needs to be fast and lock on fast", "tries
    /// to go straight at a target, but not if it's going to hit a wall
    /// first"): a locked drone turns this fast (degrees a second) and slows
    /// this much in a hard turn, so it flies the clear line to its target
    /// instead of a wide arc into a wall.
    pub const LOCK_TURN_RATE: f32 = 360.0;
    pub const HARD_TURN_DECEL: f32 = 0.15;
    pub const MIN_SPEED: f32 = 400.0;
    /// bo2mp, his play (10-14 "doesn't go as high", 10-15 "it won't ever
    /// try to circle back"): a hunting drone follows the script's goal round
    /// the map's middle, but stays this far over where it was thrown,
    /// climbing or dropping no steeper than HUNT_MAX_PITCH, turning this fast.
    pub const HUNT_HEIGHT: f32 = 350.0;
    pub const HUNT_MAX_PITCH: f32 = 8.0;
    pub const HUNT_TURN_RATE: f32 = 120.0;
}

/// `DroneSteering`, once a frame (BO2's server frame is our 50 ms tick):
/// a soft launch, then it hunts (stage 1) until it has a target it can see,
/// then flies straight at it (stage 2), banking into its turns. It never
/// steers round walls (bo2mp, his play 10-15: "it shouldn't dodge walls").
fn drone(
    world: &FrameWorld,
    projectile: &mut crate::ProjectileState,
    now: i32,
    facts: crate::EquipmentRuntimeFacts,
    target: Vec3,
) {
    use drone_dvars::*;
    if projectile.guide.stage == 0 {
        projectile.guide.from = projectile.origin;
        if now.saturating_sub(projectile.spawn_time_ms) < facts.ignition_delay_ms {
            return;
        }
        projectile.guide.stage = 1;
    }
    if now.saturating_sub(projectile.spawn_time_ms) >= LIFETIME_MS {
        projectile.detonate_at_ms = Some(now);
        return;
    }
    let origin = Vec3::from_array(projectile.origin);
    let velocity = Vec3::from_array(projectile.velocity);
    if projectile.guide.home.is_some() && projectile.guide.target == projectile.guide.home {
        hunt(projectile, now, origin, velocity, target);
        return;
    }
    // Locked: straight at the target, fast, turning hard, but only while
    // it has a clear line to it (bo2mp, his play 10-15: "It should have a
    // clear LOS ... and then target them. They can react by trying to put a
    // wall between the drone and them"; BO2's own lock test passes through
    // cars). No clear line: it hunts on as if it had no lock.
    let to_aim = target - origin;
    let obstacles = obstacles(world, projectile);
    let seen = clear_run(world, &obstacles, origin, target) >= to_aim.length() - 64.0;
    if !seen {
        let goal = match projectile.guide.home {
            Some(crate::MissileTarget::Point(goal)) => Vec3::from_array(goal),
            _ => origin + velocity.try_normalize().unwrap_or(Vec3::X) * 1024.0,
        };
        hunt(projectile, now, origin, velocity, goal);
        return;
    }
    projectile.guide.stage = 2;
    let wanted = to_aim.try_normalize().unwrap_or(Vec3::X);
    let speed = velocity.length();
    let forward = velocity.try_normalize().unwrap_or(wanted);
    let (direction, turn, roll) =
        rotate_dir(forward, wanted, LOCK_TURN_RATE, false, projectile.guide.roll);
    let speed = drone_speed(speed, turn);
    if drone_trace() {
        diag::info!(
            Sim,
            "drone trace t {now} lock origin {origin:?} target {target:?} vel {velocity:?} dir {direction:?} speed {speed} turn {turn}"
        );
    }
    fly(projectile, now, direction, speed, roll);
}

/// What stops the drone: cars, props and other solid entities, but not the
/// drone's own models.
fn obstacles(
    world: &FrameWorld,
    projectile: &crate::ProjectileState,
) -> Vec<crate::bullet_collision::EntityCollisionTraceGeom> {
    let attached = world.missile_collision_models(projectile.id);
    world
        .entity_collision_capabilities()
        .iter()
        .filter(|capabilities| {
            !capabilities
                .owner
                .script_model()
                .is_some_and(|id| attached.contains(&id))
        })
        .map(|capabilities| capabilities.trace_geom())
        .collect()
}

/// How far the drone gets from `from` toward `to` before the world, glass
/// or one of `obstacles` stops it (players don't: it flies into its target).
fn clear_run(
    world: &FrameWorld,
    obstacles: &[crate::bullet_collision::EntityCollisionTraceGeom],
    from: Vec3,
    to: Vec3,
) -> f32 {
    use crate::bullet_collision::{BulletTraceQuery, MASK_BULLET_WORLD, TraceOutcome};
    let content = world.content();
    let objects = world.world_objects();
    let outcome = crate::bullet_collision::bullet_trace_with_entity_models(
        content.clip_brushes(),
        content.clip_bsp(),
        content.clip_cmodels(),
        content.clip_mesh(),
        &[],
        obstacles,
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

/// Speeds up by BO2's dive numbers on a straight line, slows in a hard turn.
fn drone_speed(speed: f32, turn: f32) -> f32 {
    use drone_dvars::*;
    let mut speed = speed;
    if turn < 30.0 {
        speed = (speed + ACCEL_DESCEND * 0.05).min(SPEED_LIMIT_DESCEND);
    }
    (speed * (1.0 - turn / 180.0 * HARD_TURN_DECEL)).max(MIN_SPEED)
}

/// bo2mp, from his play of real BO2 (10-14: "It is a jet. It flies
/// straight... It's literally hunting for a target. If it doesn't find it,
/// it explodes"; 10-15: "it won't ever try to circle back to find a
/// target"): with no lock the Hunter Killer flies fast and low toward the
/// goal BO2's script keeps moving (down the throw line, then round the map's
/// middle) while the script looks for a target. It holds HUNT_HEIGHT over
/// where it was thrown rather than the script's 2400; BO2's projLifetime
/// blows it up.
fn hunt(
    projectile: &mut crate::ProjectileState,
    now: i32,
    origin: Vec3,
    velocity: Vec3,
    goal: Vec3,
) {
    use drone_dvars::*;
    let forward = velocity.try_normalize().unwrap_or(Vec3::X);
    let flat = (goal - origin)
        .truncate()
        .try_normalize()
        .or_else(|| forward.truncate().try_normalize())
        .unwrap_or(glam::Vec2::X);
    let height = projectile.guide.from[2] + HUNT_HEIGHT;
    let ahead = (velocity.truncate().length() * 0.5).max(200.0);
    let pitch = ((height - origin.z) / ahead)
        .atan()
        .clamp(-HUNT_MAX_PITCH.to_radians(), HUNT_MAX_PITCH.to_radians());
    let wanted = Vec3::new(flat.x * pitch.cos(), flat.y * pitch.cos(), pitch.sin());
    let speed = velocity.length();
    let (direction, turn, roll) =
        rotate_dir(forward, wanted, HUNT_TURN_RATE, false, projectile.guide.roll);
    let speed = drone_speed(speed, turn);
    if drone_trace() {
        diag::info!(
            Sim,
            "drone trace t {now} hunt origin {origin:?} goal {goal:?} vel {velocity:?} dir {direction:?} speed {speed} turn {turn}"
        );
    }
    fly(projectile, now, direction, speed, roll);
}

fn fly(projectile: &mut crate::ProjectileState, now: i32, direction: Vec3, speed: f32, roll: f32) {
    projectile.guide.roll = roll;
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
    let mut apos = crate::entity_local::fire_missile_apos(direction.to_array());
    apos.tr_base[2] = roll;
    projectile.apos = apos;
}

/// BO2MP_DRONE_TRACE=1: every drone steering step.
fn drone_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("BO2MP_DRONE_TRACE").is_ok())
}

/// `DroneRotateDir`: the new heading (at most `max_dps` a second, small
/// turns 90% of the way at once, a wall turn all at once), how far it
/// wanted to turn (0..180) and its bank.
fn rotate_dir(current: Vec3, wanted: Vec3, max_dps: f32, avoid: bool, roll: f32) -> (Vec3, f32, f32) {
    use drone_dvars::*;
    let turn = (1.0 - (wanted.dot(current) + 1.0) * 0.5) * 180.0;
    if turn <= 0.1 {
        return (wanted, turn, roll * FLATTEN_ROLL_RATIO);
    }
    let need = turn * 20.0;
    let mut fraction = max_dps / need;
    if need < max_dps {
        if avoid {
            return (wanted, turn, roll);
        }
        fraction = TURN_RATE_SMALL_ADJUSTMENTS;
    }
    let rotation = |dir: Vec3| {
        let angles = gsc_t6::math::vector_to_angles(dir.to_array());
        glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            angles[1].to_radians(),
            angles[0].to_radians(),
            0.0,
        )
    };
    let out = (rotation(current).slerp(rotation(wanted), fraction).normalize() * Vec3::X)
        .normalize_or_zero();
    let yaw = |v: Vec3| v.y.atan2(v.x).to_degrees();
    let mut diff = yaw(current) - yaw(out);
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    let max_roll = if diff < 0.0 { -TURN_MAX_ROLL } else { TURN_MAX_ROLL };
    let mut next = turn / TURN_MAX_RATE * max_roll;
    if (next - roll).abs() > TURN_MAX_ROLL_CHANGE {
        // BO2 steps by the sign of the bank it wants, not of the change.
        next = roll + if next < 0.0 { -TURN_MAX_ROLL_CHANGE } else { TURN_MAX_ROLL_CHANGE };
    }
    (out, turn, next)
}
