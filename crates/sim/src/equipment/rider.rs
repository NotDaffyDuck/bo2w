//! A missile stuck into a player keeps its place on his body: it is stored against the hit box it struck, in that
//! box's own axes, and carried along as the box moves.

use crate::bullet_collision::{AuthorityDObjCollisionBone, PlayerCollisionPose};
use crate::identities::LifeSequence;
use crate::ClientId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckRider {
    pub client: ClientId,
    pub life: LifeSequence,
    /// Box bone it hangs on; None: it hangs off his origin.
    pub bone: Option<u16>,
    /// Offset in the box's axes (or from the origin when `bone` is None).
    pub local: [f32; 3],
    /// It stuck to his riot shield.
    pub shield: bool,
}

fn pose_of(poses: &[PlayerCollisionPose], client: ClientId, life: LifeSequence) -> Option<&PlayerCollisionPose> {
    poses
        .iter()
        .find(|pose| pose.client == client && pose.life_sequence == life)
}

fn dist_sq(bone: &AuthorityDObjCollisionBone, at: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    for axis in 0..3 {
        let gap = bone.center[axis] - at[axis];
        sum += gap * gap;
    }
    sum
}

/// Pins a missile that landed at `at` on the box nearest to it, preferring boxes of the body part that was hit.
pub(super) fn ride_on(
    poses: &[PlayerCollisionPose],
    client: ClientId,
    life: LifeSequence,
    hitloc: u8,
    at: [f32; 3],
) -> Option<StuckRider> {
    let pose = pose_of(poses, client, life)?;
    let closest = |only_part: Option<u8>| {
        pose.bones
            .iter()
            .filter(|bone| only_part.is_none_or(|part| bone.part_classification == part))
            .min_by(|a, b| dist_sq(a, at).total_cmp(&dist_sq(b, at)))
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
pub(super) fn rider_origin(poses: &[PlayerCollisionPose], rider: StuckRider) -> Option<[f32; 3]> {
    let pose = pose_of(poses, rider.client, rider.life)?;
    let Some(bone_id) = rider.bone else {
        return Some(core::array::from_fn(|axis| pose.origin[axis] + rider.local[axis]));
    };
    let bone = pose.bones.iter().find(|bone| bone.bone == bone_id)?;
    Some(core::array::from_fn(|axis| {
        let mut shift = 0.0;
        for k in 0..3 {
            shift += bone.axes[k][axis] * rider.local[k];
        }
        bone.center[axis] + shift
    }))
}
