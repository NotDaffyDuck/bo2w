//! The small records a model skeleton is made of.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{fpv_mesh_catalog::TagViewBind, skel_model::ModelSkel};

pub use xmodel_runtime::BoneCollision;

/// A bone's place in the model's bind pose, relative to its parent.
#[derive(Clone, Copy, Debug)]
pub struct BoneBind {
    pub quat: [f32; 4],
    pub trans: [f32; 3],
}

impl From<BoneBind> for TagViewBind {
    fn from(bind: BoneBind) -> Self {
        Self {
            quat: bind.quat,
            trans: bind.trans,
        }
    }
}

/// How one vertex is skinned: up to four bones and the share each one has.
#[derive(Clone, Copy, Debug, Default)]
pub struct VertSkin {
    pub bones: [u16; 4],
    pub weights: [f32; 4],
    /// The same shares as fixed-point numbers, 0 to 65535.
    pub weight_u16: [u16; 4],
}

/// A first-person arm or weapon rig is a model like any other.
pub type FpvSkel = ModelSkel;

/// Skeletons shared between callers, by model name, each with the number of
/// callers that hold it.
#[derive(Clone, Debug, Default)]
pub struct SharedXModelSurfaces(pub BTreeMap<String, (Arc<ModelSkel>, usize)>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bind_carries_over_to_a_tag_bind_unchanged() {
        let bind = BoneBind {
            quat: [0.0, 0.0, 0.7, 0.7],
            trans: [1.0, 2.0, 3.0],
        };
        let tag = TagViewBind::from(bind);
        assert_eq!((tag.quat, tag.trans), (bind.quat, bind.trans));
    }
}
