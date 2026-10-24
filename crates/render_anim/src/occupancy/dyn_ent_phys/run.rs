use super::*;

/// Catches the physics clock up to `now` in at most `MAX_STEPS` steps and
/// returns how many milliseconds it advanced. The first call only starts the
/// clock.
pub(super) fn run_steps(
    world: &mut DynEntPhysWorld,
    clip: Option<&ClipCollision>,
    now: i32,
) -> i32 {
    let Some(last) = world.clock_ms else {
        world.clock_ms = Some(now);
        return 0;
    };
    if now <= last {
        return 0;
    }
    let mut remaining = now.saturating_sub(last);
    let mut steps_left = MAX_STEPS;
    let mut advanced = 0;
    while remaining > 0 && steps_left > 0 {
        // Long frames are split evenly over the remaining steps.
        let share = remaining / steps_left;
        let step_ms = if STEP_MS < share { share } else { STEP_MS }.clamp(1, remaining);
        let dt = step_ms as f32 / 1000.0;
        for (entity, body) in world.awake.iter_mut() {
            match clip {
                Some(clip) => advance_in_world(*entity, body, clip, dt),
                None => advance_in_air(body, dt),
            }
        }
        remaining -= step_ms;
        steps_left -= 1;
        advanced += step_ms;
    }
    world.clock_ms = Some(last + advanced);
    advanced
}

/// Counts how long each grounded, slow piece has been still; returns those
/// that have been still long enough to sleep.
pub(super) fn settle(world: &mut DynEntPhysWorld, advanced_ms: i32) -> Vec<Entity> {
    let mut ready = Vec::new();
    for (entity, body) in world.awake.iter_mut() {
        if body.grounded && body.vel.length() < SETTLE_SPEED {
            body.still_ms = body.still_ms.saturating_add(advanced_ms);
            if body.still_ms >= SLEEP_AFTER_MS {
                ready.push(*entity);
            }
        } else {
            body.still_ms = 0;
        }
    }
    ready
}

/// Pure ballistic motion, used while no world geometry is loaded.
pub(super) fn advance_in_air(body: &mut PhysBody, dt: f32) {
    body.vel.z += GRAVITY * dt;
    body.origin += body.vel * dt;
    body.grounded = false;
}

/// Velocity after striking a surface with normal `n` (unit length): the
/// normal component bounces (not at all for a resting contact), and friction
/// removes tangential speed in proportion to how hard the surface pushed
/// back, so the slide is the same at any frame rate.
pub(super) fn contact_response(vel: Vec3, n: Vec3, bounce: f32, friction: f32, dt: f32) -> Vec3 {
    let approach = vel.dot(n);
    if approach >= 0.0 {
        return vel;
    }
    let restitution = if -approach < RESTING_STEPS * -GRAVITY * dt {
        0.0
    } else {
        bounce.max(0.0)
    };
    let push = -(1.0 + restitution) * approach;
    let mut out = vel + n * push;
    let sliding = out - n * out.dot(n);
    let slide_speed = sliding.length();
    if slide_speed > 0.0 {
        let lost = (friction.clamp(0.0, 1.0) * push).min(slide_speed);
        out -= sliding * (lost / slide_speed);
    }
    out
}
