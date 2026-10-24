//! What the frontend knows about the material corpus at run time: the catalog
//! captured from the authored definitions and the sorted-material table that
//! fixes every material's draw-surf ordinal.

mod capture;
mod sort_table;

pub use render_frame::{
    SurfaceLightmapId, SurfaceReflectionProbeId, SurfaceSamplerInputs, TextureBindIdentity,
};
pub use render_material::{
    CatalogBuildError, CodeSourceError, CodeSourceLookup, ExecutableArgumentBinding,
    ExecutablePass, ExecutablePassView, GfxPassStateBits, LayeredCodeSources, MaterialAssetId,
    MaterialExecution, MaterialGenerationId, MaterialRefusal, PackedCodeArg,
    PackedCodeConstantLane, PackedCodeConstants, PackedCodeSamplerLane, PackedCodeSamplers,
    PackedLocalBanks, PackedLocalSamplerLane, PackedLocalSamplers, PortId, PreparedArgBelts,
    PreparedArgSlice, PreparedMaterial, PreparedMaterialTable, PreparedPass, PreparedTableCensus,
    PreparedTechnique, RemapResolution, RuntimeArgumentBinding, RuntimeCodeSources, RuntimeImageId,
    RuntimeMaterial, RuntimeMaterialCatalog, RuntimePass, RuntimeProgramIdentity,
    RuntimeShaderPair, RuntimeShaderProgram, RuntimeShaderProgramId, RuntimeShaderStage,
    RuntimeSortedMaterialTable, RuntimeTechnique, RuntimeTechniqueSet, RuntimeTechniqueSetId,
    RuntimeTextureBinding, RuntimeVertexDecl, SortedMaterialOrdinal, StableMaterialShell,
    StablePassShell, TECHNIQUE_SLOT_COUNT, add_surf_has_technique, capture_stable_shell,
    draw_binds_code_texture, draw_code_sampler_mask, execute_material, pack_local_banks,
    pack_local_samplers, prepared_draw_technique, rebind_stable_material,
    resolve_material_technique, resolve_sorted_material, smodel_tess_vertex_type, sort_pass_args,
    world_tess_vertex_type, world_tess_vertex_type_authored,
};
pub use render_scene::runtime_cull_face;

pub use capture::capture_runtime_catalog;
