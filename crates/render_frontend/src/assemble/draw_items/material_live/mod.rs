//! What the frontend knows about the material corpus at run time: the catalog
//! captured from the authored definitions and the sorted-material table that
//! fixes every material's draw-surf ordinal.

mod capture;
mod sort_table;

pub use render_frame::{
    SurfaceBakedLightId, SurfaceCubeProbeId, FacetSamplerInputs, TextureBindingKey,
};
pub use render_material::LedgerBuildError;
pub use render_material::EngineSourceError;
pub use render_material::EngineValueSource;
pub use render_material::RunnableParamSource;
pub use render_material::BoundPass;
pub use render_material::CodeLanes;
pub use render_material::VisualPassStateBits;
pub use render_material::StackedEngineInputs;
pub use render_material::MaterialAssetKey;
pub use render_material::MaterialRunPlan;
pub use render_material::MaterialEpochId;
pub use render_material::MaterialPlan;
pub use render_material::MaterialExecutionDenial;
pub use render_material::PackedCodeParam;
pub use render_material::CodeConstantChannel;
pub use render_material::CodeConstantBundle;
pub use render_material::CodeSamplerChannel;
pub use render_material::CompactEngineSamplers;
pub use render_material::LocalSamplerBanks;
pub use render_material::CompactSamplerSlot;
pub use render_material::PassPlan;
pub use render_material::LocalSamplerBundle;
pub use render_material::ShaderPortKey;
pub use render_material::PreparedParamStrips;
pub use render_material::PreparedParamSpan;
pub use render_material::StagedMaterial;
pub use render_material::StagedMaterialTable;
pub use render_material::ResolvedDrawPass;
pub use render_material::ResolvedTableTally;
pub use render_material::ResolvedShadingRecipe;
pub use render_material::MaterialRemapOutcome;
pub use render_material::LiveArgumentBinding;
pub use render_material::LiveEngineSources;
pub use render_material::LiveImageId;
pub use render_material::LiveMaterial;
pub use render_material::LiveMaterialLedger;
pub use render_material::LivePassRecord;
pub use render_material::LiveShaderIdentity;
pub use render_material::LiveShaderPair;
pub use render_material::ActiveShaderCombo;
pub use render_material::LiveShaderProgramId;
pub use render_material::LiveShaderStage;
pub use render_material::LiveSortedMaterialTable;
pub use render_material::LiveShadingTechnique;
pub use render_material::LiveTechniqueBundle;
pub use render_material::LiveTechniqueBundleKey;
pub use render_material::LiveTextureBinding;
pub use render_material::LiveVertexFormat;
pub use render_material::SortedMaterialRank;
pub use render_material::RENDER_RECIPE_SLOT_TOTAL;
pub use render_material::surface_add_gains_technique;
pub use render_material::capture_plan;
pub use render_material::draw_uses_engine_texture;
pub use render_material::draw_engine_sampler_bits;
pub use render_material::run_material_pipeline;
pub use render_material::execute_plan;
pub use render_material::encode_local_register_banks;
pub use render_material::pack_local_samplers;
pub use render_material::resolved_draw_technique;
pub use render_material::select_material_technique;
pub use render_material::lookup_ordered_material;
pub use render_material::smodel_tess_vertex_type;
pub use render_material::sort_pass_args;
pub use render_material::tess_vertex_kind;
pub use render_material::level_stitch_vertex_type_designer;
pub use render_scene::culled_face_from_mode;

pub use capture::capture_runtime_catalog;
