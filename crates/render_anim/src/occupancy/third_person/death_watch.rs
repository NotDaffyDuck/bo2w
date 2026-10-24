use super::*;

/// The camera that orbits a dead player's body.
///
/// Nothing is returned without a world to trace against, nor on the frame
/// the player's own state already says dead while the snapshot's copy does
/// not: the direction to the killer only arrives with the dead snapshot, and
/// guessing it swung the camera to the wrong side for a frame.
pub fn death_watch_camera(
    presented: &PresentedSnapshot,
    local: ClientId,
    clip: Option<&ClipCollision>,
    corpse_j_mainroot: Option<[f32; 3]>,
) -> Option<WorldCameraPose> {
    let ps = presented.player(local)?;
    let clip = clip?;
    let eye_shift = presented.view_offset();
    let snapshot = presented.snapshot()?;
    let snapshot_still_alive = snapshot
        .players
        .iter()
        .find(|(client, _)| *client == local)
        .is_some_and(|(_, state)| state.pm_type < PM_TYPE_DEAD);
    if ps.pm_type >= PM_TYPE_DEAD && snapshot_still_alive {
        return None;
    }
    let killer_yaw = snapshot
        .meta
        .for_client(local)
        .map_or(ps.viewangles[1], |meta| meta.look_at_killer_yaw as f32);

    let half = CG_CAMERA_PULLBACK_BOX_HALF;
    let fraction_of = |from: [f32; 3], to: [f32; 3]| {
        clip.sweep_box(from, to, [-half; 3], [half; 3], CG_CAMERA_PULLBACK_CLIPMASK)
            .fraction
    };
    let placed = offset_third_person_view(
        OffsetThirdPersonViewInputs {
            origin: std::array::from_fn(|axis| ps.origin[axis] + eye_shift[axis]),
            view_height_current: ps.view_height_current,
            viewangles: ps.viewangles,
            pm_type: ps.pm_type,
            look_at_killer_yaw: killer_yaw,
            corpse_j_mainroot,
            other_flags: ps.other_flags,
            delta_time: ps.delta_time,
            cg_third_person_angle: CG_THIRD_PERSON_ANGLE_MP,
            cg_third_person_range: CG_THIRD_PERSON_RANGE_DEFAULT,
        },
        fraction_of,
    );
    Some(WorldCameraPose {
        origin: placed.origin,
        angles: placed.angles,
    })
}
