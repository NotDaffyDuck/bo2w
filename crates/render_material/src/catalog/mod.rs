//! The runtime material catalog and the identifiers it is keyed by.

mod hash;
mod ids;
mod material;
mod table;
mod technique;

pub(crate) use hash::{fnv1a64, fnv1a64_more};
pub use ids::{
    CAMERA_REGION_DEPTH_HACK, CAMERA_REGION_EMISSIVE, CAMERA_REGION_LIT_OPAQUE,
    CAMERA_REGION_LIT_TRANS, CAMERA_REGION_NONE, MaterialAssetId, MaterialGenerationId, PortId,
    RuntimeImageId, RuntimeProgramIdentity, RuntimeShaderPair, RuntimeShaderProgram,
    RuntimeShaderProgramId, RuntimeTechniqueSetId, RuntimeTextureBinding, SortedMaterialOrdinal,
    TECHNIQUE_SLOT_COUNT, sort_band,
};
pub use material::RuntimeMaterial;
pub use table::{CatalogBuildError, RuntimeMaterialCatalog, RuntimeSortedMaterialTable};
pub use technique::{
    RemapResolution, RuntimePass, RuntimeTechnique, RuntimeTechniqueSet, sort_pass_args,
};
