//! A body in the air: follow its arc each tick, stop where it hits something, and settle there.
//!
//! [`fall_step`] is the whole rule for one body and knows nothing of the match. It is handed a
//! tracer (a closure that sweeps the player-sized box from one point to another) so a test can
//! stand in for the map.

use super::CorpseBay;
use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::step_view::StepWorldView;
use entity_t6::evaluate_trajectory;
use entity_t6::path_kind::{FIRST_RAGDOLL, BLEND, LIMP_LAST, RAGDOLL_INTERPOLATE};
use trace_t6::Trace;

/// How far a body buried in the floor is lifted before it is traced out again.
const UNSTICK_LIFT: f32 = 32.0;

/// Clip-mask bit left out of the second, unsticking sweep.
const UNSTICK_MASK_DROP: u32 = 0x0001_0000;

/// Moves every body that is still in the air to where level time `clock_stamp_ms` puts it.
pub(crate) fn settle_corpses(world: &mut StepWorldView, clock_stamp_ms: i32) {
    for index in 0..world.body_pool().slots.len() {
        let mut body = world.body_pool().slots[index];
        if !body.occupied || !body.falling {
            continue;
        }
        fall_step(&mut body, clock_stamp_ms, |from, to, mask| {
            world.trace_clip(from, to, BODY_BOX_LOWER, BODY_BOX_UPPER, mask)
        });
        world.corpses_mut().slots[index] = body;
    }
}

/// One tick of a falling body. `sweep(from, to, mask)` is the player-box trace.
pub(super) fn fall_step(
    body: &mut CorpseBay,
    clock_stamp_ms: i32,
    mut sweep: impl FnMut([f32; 3], [f32; 3], u32) -> Trace,
) {
    let target = evaluate_trajectory(&body.trajectory(), clock_stamp_ms);
    let hit = sweep(body.origin, target, PLAYER_BODY_MASK);
    body.origin = if hit.fraction >= 1.0 { target } else { hit.finish_location(body.origin, target) };

    let reached_target = hit.fraction >= 1.0 && !hit.starts_inside;
    if reached_target {
        return;
    }

    if hit.fully_embedded {
        // Buried in something solid: try once from higher up, ignoring one kind of clip.
        let mut lifted = body.origin;
        lifted[2] += UNSTICK_LIFT;
        let retry = sweep(lifted, target, PLAYER_BODY_MASK & !UNSTICK_MASK_DROP);
        if !retry.fully_embedded {
            body.origin = retry.finish_location(lifted, target);
        }
    }
    touch_down(body, &hit, clock_stamp_ms);
}

/// The body struck `hit`: either slide off an overhang or wall and keep falling, or come to rest.
fn touch_down(body: &mut CorpseBay, struck: &Trace, clock_stamp_ms: i32) {
    let was_ragdoll = (FIRST_RAGDOLL..=LIMP_LAST).contains(&body.tr_type);
    body.tr_delta = [0.0; 3];

    let facing_down = struck.normal[2] <= 0.0;
    if !struck.fully_embedded && facing_down {
        // Under a ceiling or against a wall: ease off the surface and drop straight down.
        for (spot, push) in body.origin.iter_mut().zip(struck.normal) {
            *spot += push;
        }
        body.tr_base = body.origin;
        body.tr_time = clock_stamp_ms;
        return;
    }

    body.falling = false;
    body.tr_type = if was_ragdoll { RAGDOLL_INTERPOLATE } else { BLEND };
    body.tr_base = body.origin;
    body.tr_time = 0;
    body.tr_duration = 0;
    body.ground_entity_num = i32::from(trace_t6::entity_hit_slot(struck));
    if !struck.fully_embedded {
        body.viewangles = lying_angles(body.viewangles[1], struck.normal);
    }
}

/// View angles for a body lying on a surface with `normal`, still facing `yaw`: the surface
/// normal is its up axis and the facing is flattened onto the surface.
pub(super) fn lying_angles(yaw: f32, face: [f32; 3]) -> [f32; 3] {
    let (facing, _, _) = game_math::euler_basis([0.0, yaw, 0.0]);
    let up = face;
    let left = cross_product(up, facing);
    let forward = cross_product(left, up);
    game_math::rows_to_euler([forward, left, up])
}

fn cross_product(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    [
        lhs[1] * rhs[2] - lhs[2] * rhs[1],
        lhs[2] * rhs[0] - lhs[0] * rhs[2],
        lhs[0] * rhs[1] - lhs[1] * rhs[0],
    ]
}
