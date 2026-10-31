use super::*;

/// The camera that orbits a dead player's body.
///

/// Nothing is returned without a world to trace against, nor on the frame
/// the player's own state already says dead while the snapshot's copy does
/// not: the direction to the killer only arrives with the dead snapshot, and
/// guessing it swung the camera to the wrong side for a frame.
pub fn victim_follow_view(
    presented: &ShownWorldFrame,
    own_client: ClientId,
    solids: Option<&CollisionClipData>,
    body_root_joint_pos: Option<[f32; 3]>,
) -> Option<WorldEyePlacement> {
    let (pstate, solids) = (presented.player(own_client)?, solids?);
    let snapshot = presented.snapshot()?;
    let snapshot_alive = |client_num: &ClientId| {
        let mine = snapshot.players.iter().find(|(id, _)| id == client_num);
        mine.is_some_and(|(_, state)| state.movement_mode < MOVE_KIND_DEAD)
    };
    if pstate.movement_mode >= MOVE_KIND_DEAD && snapshot_alive(&own_client) {
        return None;
    }
    let look_pos_killer_yaw = match snapshot.meta.for_client(own_client) {
        Some(meta) => meta.look_at_killer_yaw as f32,
        None => pstate.look_angles[1],
    };

    let eye_shift = presented.eye_displacement();
    let origin = [0, 1, 2].map(|axis| eye_shift[axis] + pstate.origin[axis]);
    let inputs = ChaseCamPlacementInputs {
        chase_cam_back_distance: CHASE_CAM_DISTANCE_FALLBACK,
        chase_cam_yaw_offset: TP_CAMERA_ANGLE_MP,
        origin,
        eye_height_now: pstate.eye_height_now,
        look_angles: pstate.look_angles,
        move_mode: pstate.movement_mode,
        killer_facing_yaw: look_pos_killer_yaw,
        body_root_joint_pos,
        secondary_state_bits: pstate.secondary_state_bits,
        time_slice: pstate.time_slice,
    };
    let pullback = [CHASE_CAM_PROBE_HALF_EXTENT; 3];
    let clear_fraction = |start: [f32; 3], stop: [f32; 3]| {
        let hull_min = pullback.map(|half| -half);
        let swept = solids.sweep_box(start, stop, hull_min, pullback, CHASE_CAM_RETREAT_MASK);
        swept.fraction
    };
    let vw = place_chase_camera(inputs, clear_fraction);
    Some(WorldEyePlacement {
        origin: vw.origin,
        turn: vw.angles,
    })
}
