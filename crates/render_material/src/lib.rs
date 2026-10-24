pub mod argument;
pub mod catalog;
pub mod code_sources;
pub mod decl_table;
mod depth_state;
pub mod execute;
pub mod pass_color_space;
mod polygon_offset;
pub mod prepared;
pub mod setup;
pub mod sm3;
pub mod sm3_abi;
pub mod stage;
pub mod vertex_decl;
pub mod vertex_layout;

pub use argument::RuntimeArgumentBinding;
pub use catalog::{
    CAMERA_REGION_DEPTH_HACK, CAMERA_REGION_EMISSIVE, CAMERA_REGION_LIT_OPAQUE,
    CAMERA_REGION_LIT_TRANS, CAMERA_REGION_NONE, CatalogBuildError, MaterialAssetId,
    MaterialGenerationId, PortId, RemapResolution, RuntimeImageId, RuntimeMaterial,
    RuntimeMaterialCatalog, RuntimePass, RuntimeProgramIdentity, RuntimeShaderPair,
    RuntimeShaderProgram, RuntimeShaderProgramId, RuntimeSortedMaterialTable, RuntimeTechnique,
    RuntimeTechniqueSet, RuntimeTechniqueSetId, RuntimeTextureBinding, SortedMaterialOrdinal,
    TECHNIQUE_SLOT_COUNT, sort_band, sort_pass_args,
};
pub use code_sources::{CodeSourceError, CodeSourceLookup, LayeredCodeSources, RuntimeCodeSources};
pub use depth_state::{
    D3DCMP_ALWAYS, D3DCMP_EQUAL, D3DCMP_LESS, D3DCMP_LESSEQUAL, D3DRS_ZENABLE, D3DRS_ZFUNC,
    D3DRS_ZWRITEENABLE, GFXS1_DEPTHTEST_ALWAYS, GFXS1_DEPTHTEST_DISABLE, GFXS1_DEPTHTEST_EQUAL,
    GFXS1_DEPTHTEST_LESS, GFXS1_DEPTHTEST_LESSEQUAL, GFXS1_DEPTHTEST_MASK, GFXS1_DEPTHTEST_SHIFT,
    GFXS1_DEPTHWRITE, S_DEPTH_TEST_TABLE, d3d_zfunc_from_word1, depth_state_from_state_bits,
    depth_test_enable, depth_test_field, depth_write_enable,
};
pub use execute::{
    ExecutablePass, ExecutablePassView, MaterialDrawKey, MaterialExecution, MaterialRefusal,
    PackedCodeConstantLane, PackedCodeConstants, PackedCodeSamplerLane, PackedCodeSamplers,
    StableMaterialShell, StablePassShell, UnsupportedStateFields, add_surf_has_technique,
    capture_stable_shell, draw_binds_code_texture, draw_code_sampler_mask, execute_material,
    execute_material_with_shell, prepared_draw_technique, rebind_stable_material,
    resolve_material_technique, resolve_sorted_material, smodel_tess_vertex_type,
    world_tess_vertex_type, world_tess_vertex_type_authored,
};
pub use pass_color_space::{PassColorSpace, pass_color_space};
pub use polygon_offset::{
    GFXS1_POLYGON_OFFSET_MASK, GFXS1_POLYGON_OFFSET_SHADOWMAP_LEVEL, GFXS1_POLYGON_OFFSET_SHIFT,
    POLYGON_OFFSET_BIAS_TO_D3D, R_POLYGON_OFFSET_BIAS_DEFAULT, R_POLYGON_OFFSET_BIAS_MIN,
    R_POLYGON_OFFSET_MAX, R_POLYGON_OFFSET_SCALE_DEFAULT, R_POLYGON_OFFSET_SCALE_MIN,
    SM_POLYGON_OFFSET_BIAS_DEFAULT, SM_POLYGON_OFFSET_BIAS_MAX, SM_POLYGON_OFFSET_SCALE_DEFAULT,
    SM_POLYGON_OFFSET_SCALE_MAX, d3d_depth_bias_to_wgpu_constant, polygon_offset_d3d,
    polygon_offset_d3d_defaults, polygon_offset_level, polygon_offset_wgpu_defaults,
};
pub use prepared::{
    AdmittedPortFacts, ExecutableArgumentBinding, GfxPassStateBits, PackedCodeArg,
    PackedLocalBanks, PackedLocalSamplerLane, PackedLocalSamplers, PreparedArgBelts,
    PreparedArgSlice, PreparedMaterial, PreparedMaterialTable, PreparedPass, PreparedTableCensus,
    PreparedTechnique, pack_local_banks, pack_local_samplers,
};
pub use setup::{SetupArm, TechType};
pub use sm3::{
    MATERIAL_ALPHA_TESTS, OpcodeSurfaceCoverage, SUPPORTED_OPCODE_SURFACE, Sm3Destination,
    Sm3Instruction, Sm3InstructionBody, Sm3IrError, Sm3LoweringRefusal, Sm3Opcode, Sm3ProgramIr,
    Sm3Register, Sm3RegisterFile, Sm3RelativeAddress, Sm3Source, Sm3SourceModifier,
    alpha_test_fragment_entry, decode_sm3_program, measure_shader_surface_coverage,
    named_sm3_opcode_count, validate_supported_opcode_surface,
};
pub use sm3_abi::{
    ConstantBinding, ConstantSource, PassAbiRefusal, PassProgramAbi, SamplerBinding, SamplerSource,
    SamplerTextureDimension, Semantic, VaryingBinding, VertexAttribute, VertexInputBinding,
    build_pass_abi, format_vertex_semantic,
};
pub use stage::RuntimeShaderStage;
pub use vertex_decl::RuntimeVertexDecl;
pub use vertex_layout::{T5_WORLD_LAYER_HOST_STRIDE, VertexLayoutFamily};
