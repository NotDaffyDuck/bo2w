//! A missile stuck into a player keeps its place on his body: it is stored against the hit box it struck, in that
//! box's own axes, and carried along as the box moves.

use crate::shot_trace::PlayerHitboxPose;
use crate::match_ids::LifeCount;
use crate::SeatNo;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckRider {
    pub client: SeatNo,
    pub life: LifeCount,
    /// Box bone it hangs on; None: it hangs off his origin.
    pub bone: Option<u16>,
    /// Offset in the box's axes (or from the origin when `bone` is None).
    pub local: [f32; 3],
    /// It stuck to his riot shield.
    pub shield: bool,
}

fn pose_of(poses: &[PlayerHitboxPose], client: SeatNo, life: LifeCount) -> Option<&PlayerHitboxPose> {
    poses
        .iter()
        .find(|pose| pose.client == client && pose.life_sequence == life)
}

fn squared_gap(bone: &xmodel_runtime::HitShapeBone, at: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    for k in 0..3 {
        let gap = bone.center[k] - at[k];
        sum += gap * gap;
    }
    sum
}

/// Pins a missile that landed at `at` on the box nearest to it, preferring boxes of the body part that was hit.
pub(super) fn ride_on(
    poses: &[PlayerHitboxPose],
    client: SeatNo,
    life: LifeCount,
    hitloc: u8,
    at: [f32; 3],
) -> Option<StuckRider> {
    let pose = pose_of(poses, client, life)?;
    let closest = |only_part: Option<u8>| {
        pose.joints
            .iter()
            .filter(|bone| only_part.is_none_or(|part| bone.body_part_category == part))
            .min_by(|p, q| squared_gap(p, at).total_cmp(&squared_gap(q, at)))
    };
    let Some(bone) = closest(Some(hitloc)).or_else(|| closest(None)) else {
        return Some(StuckRider {
            client,
            life,
            bone: None,
            local: core::array::from_fn(|axis| at[axis] - pose.origin[axis]),
            shield: false,
        });
    };
    let offset: [f32; 3] = core::array::from_fn(|axis| at[axis] - bone.center[axis]);
    let local = bone.axes.map(|axis_dir| {
        let mut dot = 0.0;
        for k in 0..3 {
            dot += axis_dir[k] * offset[k];
        }
        dot
    });
    Some(StuckRider { client, life, bone: Some(bone.bone), local, shield: false })
}

/// Current world position of a riding missile; None once its player is dead or gone.
pub(super) fn rider_origin(poses: &[PlayerHitboxPose], rider: StuckRider) -> Option<[f32; 3]> {
    let pose = pose_of(poses, rider.client, rider.life)?;
    let Some(bone_id) = rider.bone else {
        return Some(core::array::from_fn(|axis| pose.origin[axis] + rider.local[axis]));
    };
    let bone = pose.joints.iter().find(|bone| bone.bone == bone_id)?;
    Some(core::array::from_fn(|axis| {
        let mut shift = 0.0;
        for k in 0..3 {
            shift += bone.axes[k][axis] * rider.local[k];
        }
        bone.center[axis] + shift
    }))
}
