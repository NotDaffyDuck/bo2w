use super::*;

/// Catches the physics clock up to `now` in at most `MAX_STEPS` steps and
/// returns how many milliseconds it advanced. The first call only starts the
/// clock.
pub(super) fn run_steps(
    world: &mut DebrisPhysLevel,
    solids: Option<&CollisionClipData>,
    now: i32,
) -> i32 {
    let Some(last) = world.clock_ms else {
        world.clock_ms = Some(now);
        return 0;
    };
    if now <= last {
        return 0;
    }
    let mut remaining_in = now.saturating_sub(last);
    let mut steps_left = MAX_STEPS;
    let mut advanced = 0;
    while remaining_in > 0 && steps_left > 0 {
        // Long frames are split evenly over the remaining steps.
        let share = remaining_in / steps_left;
        let step_ms = if STEP_MS < share { share } else { STEP_MS }.clamp(1, remaining_in);
        let dt = step_ms as f32 / 1000.0;
        for (ent, body) in world.awake.iter_mut() {
            match solids {
                Some(solids) => advance_in_world(*ent, body, solids, dt),
                None => advance_in_air(body, dt),
            }
        }
        remaining_in -= step_ms;
        steps_left -= 1;
        advanced += step_ms;
    }
    world.clock_ms = Some(last + advanced);
    advanced
}

/// Counts how long each grounded, slow piece has been still; returns those
/// that have been still long enough to sleep.
pub(super) fn settle(world: &mut DebrisPhysLevel, advanced_ms: i32) -> Vec<Entity> {
    let mut ready = Vec::new();
    for (id, body) in world.awake.iter_mut() {
        if body.grounded && body.motion_vec.length() < SETTLE_SPEED {
            body.still_ms = body.still_ms.saturating_add(advanced_ms);
            if body.still_ms >= SLEEP_AFTER_MS {
                ready.push(*id);
            }
        } else {
            body.still_ms = 0;
        }
    }
    ready
}

/// Pure ballistic motion, used while no world geometry is loaded.
pub(super) fn advance_in_air(body: &mut PhysBody, dt: f32) {
    body.motion_vec.z += GRAVITY * dt;
    body.pos += body.motion_vec * dt;
    body.grounded = false;
}

/// Velocity after striking a surface with normal `n` (unit length): the
/// normal component bounces (not at all for a resting contact), and friction
/// removes tangential speed in proportion to how hard the surface pushed
/// back, so the slide is the same at any frame rate.
pub(super) fn contact_response(motion_vec: Vec3, n: Vec3, rebound: f32, drag: f32, dt: f32) -> Vec3 {
    let approach = motion_vec.dot(n);
    if approach >= 0.0 {
        return motion_vec;
    }
    let restitution = if -approach < RESTING_STEPS * -GRAVITY * dt {
        0.0
    } else {
        rebound.max(0.0)
    };
    let push = -(1.0 + restitution) * approach;
    let mut out = motion_vec + n * push;
    let sliding = out - n * out.dot(n);
    let slide_speed = sliding.length();
    if slide_speed > 0.0 {
        let lost = (drag.clamp(0.0, 1.0) * push).min(slide_speed);
        out -= sliding * (lost / slide_speed);
    }
    out
}
