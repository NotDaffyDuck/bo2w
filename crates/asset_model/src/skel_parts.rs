//! The small records a model skeleton is made of.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::fpv_mesh_catalog::AttachPointBinding;
use crate::skel_model::MeshRig;

pub use xmodel_runtime::BoneHitShape;

/// A bone's place in the model's bind pose, relative to its parent.
#[derive(Clone, Copy, Debug)]
pub struct JointAttachment {
    pub spin: [f32; 4],
    pub shift_offset: [f32; 3],
}

impl From<JointAttachment> for AttachPointBinding {
    fn from(rest: JointAttachment) -> Self {
        let JointAttachment { spin: quat, shift_offset } = rest;
        Self { turn: quat, at: shift_offset }
    }
}

/// How one vertex is skinned: up to four bones and the share each one has.
#[derive(Clone, Copy, Debug, Default)]
pub struct CornerWeights {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
    /// The same shares as fixed-point numbers, 0 to 65535.
    pub pull_u16: [u16; 4],
}

/// Older spelling FpvSkel still calls; the rename sweep moves it over.
/// (A first-person arm or weapon rig is a model like any other.)
pub use crate::skel_model::MeshRig as FpvSkel;

/// Skeletons shared between callers, by model name, each with the number of
/// callers that hold it.
#[derive(Clone, Debug, Default)]
pub struct PooledMeshSkins(pub BTreeMap<String, (Arc<MeshRig>, usize)>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rest_pose_moves_to_an_attach_point_as_is() {
        let rest = JointAttachment {
            spin: [0.0, 0.0, 0.7, 0.7],
            shift_offset: [1.0, 2.0, 3.0],
        };
        let tag = AttachPointBinding::from(rest);
        assert_eq!((tag.turn, tag.at), (rest.spin, rest.shift_offset));
    }
}
