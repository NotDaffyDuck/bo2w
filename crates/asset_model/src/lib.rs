//! The models the game draws, and the data that goes with them: skeletons,
//! meshes, material links and the choice of detail level.

mod body_mesh_set;
mod fpv_hands;
mod fpv_mesh_catalog;
mod fpv_mount;
pub mod link;
mod material_slots;
mod model_kind;
pub mod model_lighting;
mod model_lod;
mod model_skel_t6;
mod packed_vertex;
mod projectile_mesh_set;
mod skel_lod;
mod skel_model;
mod skel_parts;
mod soldier_kit;
mod weapon_world_entry;
mod weapon_world_set;

pub use asset_core::*;
pub use body_mesh_set::*;
pub use fpv_hands::*;
pub use fpv_mesh_catalog::*;
pub use fpv_mount::*;
pub use model_kind::*;
pub use model_lighting::*;
pub use model_lod::*;
pub use model_skel_t6::*;
pub use packed_vertex::*;
pub use projectile_mesh_set::*;
pub use skel_model::*;
pub use skel_parts::*;
pub use soldier_kit::*;
pub use weapon_world_entry::*;
pub use weapon_world_set::*;

/// The asset core's names together with the material-slot links, for the
/// code that wires models to materials.
pub mod asset_graph {
    pub use crate::link::*;
    pub use asset_core::*;
}

pub mod dobj {
    pub use xmodel_runtime::ModelPoseSrc;
}
