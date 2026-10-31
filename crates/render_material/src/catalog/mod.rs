//! The runtime material catalog and the identifiers it is keyed by.

mod hash;
mod ids;
mod mat_entry;
mod table;
mod technique;

pub(crate) use hash::hash_text_64;
pub(crate) use hash::fnv_extend;
pub use ids::{
    CAMERA_REGION_NEAR_DEPTH_TRICK, VIEW_STAGE_SELF_LIT, VIEW_STAGE_SOLID_LIT,
    VIEW_STAGE_BLENDED_LIT, VIEW_STAGE_ABSENT, MaterialAssetKey, MaterialEpochId, ShaderPortKey,
    LiveImageId, LiveShaderIdentity, LiveShaderPair, ActiveShaderCombo,
    LiveShaderProgramId, LiveTechniqueBundleKey, LiveTextureBinding, SortedMaterialRank,
    RENDER_RECIPE_SLOT_TOTAL, sort_band,
};
pub use mat_entry::LiveMaterial;
pub use table::{LedgerBuildError, LiveMaterialLedger, LiveSortedMaterialTable};
pub use technique::{
    MaterialRemapOutcome, LivePassRecord, LiveShadingTechnique, LiveTechniqueBundle, sort_pass_args,
};
