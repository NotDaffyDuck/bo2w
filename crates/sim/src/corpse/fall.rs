//! A body in the air: follow its arc each tick, stop where it hits something, and settle there.
//!
//! [`fall_step`] is the whole rule for one body and knows nothing of the match. It is handed a
//! tracer (a closure that sweeps the player-sized box from one point to another) so a test can
//! stand in for the map.

use super::PlayerCorpseSlot;
use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
use crate::frame::FrameWorld;
use entity_t6::evaluate_trajectory;
use entity_t6::tr_type::{FIRST_RAGDOLL, INTERPOLATE, LAST_RAGDOLL, RAGDOLL_INTERPOLATE};
use trace_t6::Trace;

/// How far a body buried in the floor is lifted before it is traced out again.
const UNSTICK_LIFT: f32 = 32.0;

/// Clip-mask bit left out of the second, unsticking sweep.
const UNSTICK_MASK_DROP: u32 = 0x0001_0000;

/// Moves every body that is still in the air to where level time `time_ms` puts it.
pub(crate) fn phase_run_corpse_move(world: &mut FrameWorld, time_ms: i32) {
    for index in 0..world.corpses().slots.len() {
        let mut body = world.corpses().slots[index];
        if !body.occupied || !body.falling {
            continue;
        }
        fall_step(&mut body, time_ms, |from, to, mask| {
            world.trace_clip(from, to, PLAYER_MINS, PLAYER_MAXS, mask)
        });
        world.corpses_mut().slots[index] = body;
    }
}

/// One tick of a falling body. `sweep(from, to, mask)` is the player-box trace.
pub(super) fn fall_step(
    body: &mut PlayerCorpseSlot,
    time_ms: i32,
    mut sweep: impl FnMut([f32; 3], [f32; 3], u32) -> Trace,
) {
    let target = evaluate_trajectory(&body.trajectory(), time_ms);
    let hit = sweep(body.origin, target, MASK_PLAYER_SOLID);
    body.origin = if hit.fraction >= 1.0 { target } else { hit.end_pos(body.origin, target) };

    let reached_target = hit.fraction >= 1.0 && !hit.startsolid;
    if reached_target {
        return;
    }

    if hit.allsolid {
        // Buried in something solid: try once from higher up, ignoring one kind of clip.
        let mut lifted = body.origin;
        lifted[2] += UNSTICK_LIFT;
        let retry = sweep(lifted, target, MASK_PLAYER_SOLID & !UNSTICK_MASK_DROP);
        if !retry.allsolid {
            body.origin = retry.end_pos(lifted, target);
        }
    }
    touch_down(body, &hit, time_ms);
}

/// The body struck `hit`: either slide off an overhang or wall and keep falling, or come to rest.
fn touch_down(body: &mut PlayerCorpseSlot, hit: &Trace, time_ms: i32) {
    let was_ragdoll = (FIRST_RAGDOLL..=LAST_RAGDOLL).contains(&body.tr_type);
    body.tr_delta = [0.0; 3];

    let facing_down = hit.normal[2] <= 0.0;
    if !hit.allsolid && facing_down {
        // Under a ceiling or against a wall: ease off the surface and drop straight down.
        for axis in 0..3 {
            body.origin[axis] += hit.normal[axis];
        }
        body.tr_base = body.origin;
        body.tr_time = time_ms;
        return;
    }

    body.falling = false;
    body.tr_type = if was_ragdoll { RAGDOLL_INTERPOLATE } else { INTERPOLATE };
    body.tr_base = body.origin;
    body.tr_time = 0;
    body.tr_duration = 0;
    body.ground_entity_num = i32::from(trace_t6::trace_get_entity_hit_id(hit));
    if !hit.allsolid {
        body.viewangles = lying_angles(body.viewangles[1], hit.normal);
    }
}

/// View angles for a body lying on a surface with `normal`, still facing `yaw`: the surface
/// normal is its up axis and the facing is flattened onto the surface.
pub(super) fn lying_angles(yaw: f32, normal: [f32; 3]) -> [f32; 3] {
    let (facing, _, _) = game_math::angle_vectors([0.0, yaw, 0.0]);
    let up = normal;
    let left = cross(up, facing);
    let forward = cross(left, up);
    game_math::axis_to_angles([forward, left, up])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
