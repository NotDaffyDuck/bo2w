//! How one part moves over a time slice: free flight along its velocity
//! graphs and gravity, or the same with collision against the world.

use fx_t6::runtime::{
    FX_ELEM_AT_REST_NONE, FxElemVec3Range, collide_marks_at_rest, collide_substep_schedule,
    collision_reflect_base_vel_delta, elem_dies_on_touch, elem_gravity_accel_z_sampled,
    elem_update_has_velocity_graph, elem_uses_collision, elem_uses_vel_local, elem_uses_vel_world,
    get_at_rest_fraction, get_velocity_at_time, impact_child_speed_allows,
    integrate_velocity_graph, orientation_pos_from_world, orientation_pos_to_world,
    sample_reflection_factor, trace_mask, vec3_length_sq,
};

use crate::update::{FxElemMotionQuery, FxElemMotionResult, FxElemTraceHit, FxImpactSpawn};

/// A collision sweep splits one time slice into at most this many pieces
/// per sub-step (one for each bounce that stays in the slice).
const MAX_BOUNCES_PER_STEP: usize = 8;

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add3(a, scale3(sub3(b, a), t))
}

/// Milliseconds from `from` to `to`, never negative.
fn span_msec(from: i32, to: i32) -> f32 {
    to.saturating_sub(from).max(0) as f32
}

/// Motion for a part that does not collide; `None` when its flags say it
/// does (use [`evaluate_elem_collide_motion`] for those).
pub fn evaluate_elem_motion(
    flags: i32,
    gravity_base: f32,
    gravity_amp: f32,
    elem_random_seed: u32,
    vel_graph_local: &[FxElemVec3Range],
    vel_graph_world: &[FxElemVec3Range],
    q: FxElemMotionQuery<'_>,
) -> Option<FxElemMotionResult> {
    if elem_uses_collision(flags) {
        return None;
    }
    Some(integrate_free_flight(
        flags,
        gravity_base,
        gravity_amp,
        elem_random_seed,
        vel_graph_local,
        vel_graph_world,
        q,
    ))
}

/// Motion for any part. A part that collides is swept in sub-steps; `trace`
/// is asked to move a box from one world point to another and report what it
/// hit. A part already at rest stays put.
pub fn evaluate_elem_collide_motion(
    flags: i32,
    gravity_base: f32,
    gravity_amp: f32,
    reflection_base: f32,
    reflection_amp: f32,
    coll_mins: [f32; 3],
    coll_maxs: [f32; 3],
    use_item_clip: bool,
    has_effect_on_impact: bool,
    elem_random_seed: u32,
    vel_graph_local: &[FxElemVec3Range],
    vel_graph_world: &[FxElemVec3Range],
    q: FxElemMotionQuery<'_>,
    mut trace: impl FnMut([f32; 3], [f32; 3], [f32; 3], [f32; 3], u32) -> FxElemTraceHit,
) -> Option<FxElemMotionResult> {
    if !elem_uses_collision(flags) {
        return evaluate_elem_motion(
            flags,
            gravity_base,
            gravity_amp,
            elem_random_seed,
            vel_graph_local,
            vel_graph_world,
            q,
        );
    }

    if q.at_rest_fraction != FX_ELEM_AT_REST_NONE {
        return Some(FxElemMotionResult {
            origin_delta: [0.0; 3],
            base_vel: q.base_vel,
            remove: false,
            at_rest_fraction: Some(q.at_rest_fraction),
            spawn_impact: None,
        });
    }

    let mask = trace_mask(use_item_clip);
    let life_ms = q.life_ms.max(1.0);
    let mut origin = q.origin;
    let mut base_vel = q.base_vel;
    let mut total_delta = [0.0f32; 3];
    let mut remove = false;
    let mut spawn_impact = None;
    let mut at_rest_fraction = None;

    for step in collide_substep_schedule(q.prev_msec, q.msec_now) {
        let mut t0 = step.msec_start;
        let t_end = step.msec_end;

        for _ in 0..MAX_BOUNCES_PER_STEP {
            if t0 >= t_end || remove || at_rest_fraction.is_some() {
                break;
            }
            // Fly freely from t0 to the end of the step, then see how far
            // the world lets it get.
            let free = integrate_free_flight(
                flags,
                gravity_base,
                gravity_amp,
                elem_random_seed,
                vel_graph_local,
                vel_graph_world,
                FxElemMotionQuery {
                    age0: span_msec(q.msec_begin, t0) / life_ms,
                    age1: span_msec(q.msec_begin, t_end) / life_ms,
                    life_ms,
                    dt_sec: span_msec(t0, t_end) * 0.001,
                    base_vel,
                    elem_random_seed,
                    origin,
                    prev_msec: t0,
                    msec_now: t_end,
                    ..q
                },
            );
            let start_local = origin;
            let end_local = add3(start_local, free.origin_delta);
            let start = orientation_pos_to_world(q.orient.origin, q.orient.axis, start_local);
            let end = orientation_pos_to_world(q.orient.origin, q.orient.axis, end_local);
            let hit = trace(start, end, coll_mins, coll_maxs, mask);

            if hit.startsolid || hit.allsolid {
                base_vel = [0.0; 3];
                break;
            }
            if hit.fraction >= 1.0 {
                total_delta = add3(total_delta, free.origin_delta);
                origin = end_local;
                base_vel = free.base_vel;
                break;
            }

            // Stopped short: stand where it hit.
            let hit_world = lerp3(start, end, hit.fraction);
            let hit_local = orientation_pos_from_world(q.orient.origin, q.orient.axis, hit_world);
            total_delta = add3(total_delta, sub3(hit_local, start_local));
            origin = hit_local;

            let msec_at_hit = span_msec(q.msec_begin, t0) + span_msec(t0, t_end) * hit.fraction;
            let pre_vel = get_velocity_at_time(
                flags,
                base_vel,
                msec_at_hit,
                life_ms,
                vel_graph_local,
                vel_graph_world,
                q.effect_axis,
                elem_random_seed,
            );
            if has_effect_on_impact && impact_child_speed_allows(vec3_length_sq(pre_vel)) {
                spawn_impact = Some(FxImpactSpawn {
                    origin: hit_world,
                    pre_vel,
                });
            }

            if elem_dies_on_touch(flags) {
                remove = true;
                break;
            }

            let reflection =
                sample_reflection_factor(reflection_base, reflection_amp, elem_random_seed);
            let rebound = scale3(pre_vel, reflection);
            // Touching the surface at the very start of the move with too
            // little rebound left to leave it means it has come to rest.
            if hit.fraction <= 0.0 && collide_marks_at_rest(rebound, hit.normal[2]) {
                let fraction = get_at_rest_fraction(t0 as f32, q.msec_begin as f32, 1.0 / life_ms);
                at_rest_fraction = Some(fraction as u8);
                base_vel = [0.0; 3];
                break;
            }

            base_vel = add3(
                base_vel,
                collision_reflect_base_vel_delta(pre_vel, hit.normal, reflection),
            );

            // Carry on from the hit point; always make some progress.
            t0 += (span_msec(t0, t_end) * hit.fraction) as i32;
            if t0 <= step.msec_start {
                t0 = step.msec_start.saturating_add(1);
            }
        }
        if remove || at_rest_fraction.is_some() {
            break;
        }
    }

    Some(FxElemMotionResult {
        origin_delta: total_delta,
        base_vel,
        remove,
        at_rest_fraction,
        spawn_impact,
    })
}

/// Moves a part through the time slice with nothing in its way: the local
/// and world velocity graphs, then base velocity and gravity.
fn integrate_free_flight(
    flags: i32,
    gravity_base: f32,
    gravity_amp: f32,
    elem_random_seed: u32,
    local_graph: &[FxElemVec3Range],
    world_graph: &[FxElemVec3Range],
    q: FxElemMotionQuery<'_>,
) -> FxElemMotionResult {
    let graph_move = |graph: &[FxElemVec3Range]| {
        integrate_velocity_graph(graph, q.age0, q.age1, q.life_ms, elem_random_seed)
    };

    let mut stored = q.origin;
    if elem_uses_vel_local(flags) && local_graph.len() >= 2 {
        stored = add3(stored, graph_move(local_graph));
    }
    let mut world = orientation_pos_to_world(q.orient.origin, q.orient.axis, stored);
    if elem_uses_vel_world(flags) && world_graph.len() >= 2 {
        world = add3(world, graph_move(world_graph));
    }

    let mut base_vel = q.base_vel;
    if elem_update_has_velocity_graph(flags) {
        let dt = q.dt_sec;
        let gravity = elem_gravity_accel_z_sampled(gravity_base, gravity_amp, elem_random_seed);
        world = add3(world, scale3(base_vel, dt));
        world[2] -= gravity * dt * dt * 0.5;
        base_vel[2] -= gravity * dt;
        stored = orientation_pos_from_world(q.orient.origin, q.orient.axis, world);
    }

    FxElemMotionResult {
        origin_delta: sub3(stored, q.origin),
        base_vel,
        remove: false,
        at_rest_fraction: None,
        spawn_impact: None,
    }
}
