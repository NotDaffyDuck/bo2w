//! Immutable per-frame render output: the packed source lists the backend reads,
//! the destination-ring records, per-frame product status and the GPU layout
//! contract. Everything the renderer hands across a frame boundary is defined here.
pub mod view_math; pub mod packing; pub mod portal_eyes;
mod src_lists; mod material_frame; mod mesh_pool; mod outputs; mod kept_draws; mod sampler_identity;
mod sun_fx; mod sunshadow; mod vertex; mod wgpu_layout;
pub use src_lists::{DRAWLIST_BLOCK_WORDS, StaticModelRigidRecord, TriangleRunEntry, RigidModelBatchRecord,
    DRAWLIST_MARKER, CompactFrontendLists, STATIC_FIXED_MESH_RECORD_BYTES, PropPrebuiltRange};
pub use src_lists::{SOURCE_SMODEL_CACHED_BYTES, SOURCE_SMODEL_CACHED_PTR, SOURCE_SMODEL_PRETESS_BYTES, SOURCE_SMODEL_PRETESS_PTR,
    SOURCE_SMODEL_RIGID_COUNT, SOURCE_SMODEL_RIGID_PTR, SOURCE_SMODEL_SKINNED_BYTES, SOURCE_SMODEL_SKINNED_PTR};
pub use src_lists::{SOURCE_WORLD_COUNT, SOURCE_WORLD_PTR, SOURCE_XMODEL_RIGID_COUNT, SOURCE_XMODEL_RIGID_PTR};
pub use material_frame::{MaterialRunFrame, OpenAirProbe, ConeShadowSink};
pub use mesh_pool::{MeshPoolBlock, reserve_code_mesh, reserve_code_mesh_indices, reserve_code_mesh_verts,
    shrink_code_mesh_verts};
pub use outputs::{FrameOutputItem, FrameOutputKind, FrameOutputState, FrameOutputsSnapshot,
    AbsentOutputReason, COMPACT_SEGMENT_CLAIMANTS, CompactSegment, CompactSegments, OutputTarget,
    FocusedFrameSnapshot, InputChangeCounters, ConeShadowFrameSlot, publish_rows};
pub use kept_draws::{LevelViewLane, LightFalloffBind, RENDER_EFFECT_DEPTH_HACK, HeldDrawItem,
    HeldDrawKind, LegacyFalloffParams, MODEL_OBJECT_ID_FIRST_PERSON_WEAPON, viewmodel_fx_flags};
pub use sampler_identity::TextureBindingKey;
pub use sun_fx::{SunEffectsDef, SunEffectsFrame, angular_lerp};
pub use sunshadow::{SUN_CASTER_TECH, DAYLIGHT_SHADOW_FORCED_PROFILE, SUN_SHADOW_MAX_CLIP_PLANES};
pub use sunshadow::{SUNLIGHT_CASCADE_TOTAL, DaylightShadowAtlasProfile, DaylightShadowCasterLists, SunlightShadowRegion};
pub use sunshadow::{SunShadowClipPlanes, DaylightShadowForcedFrame, SunCascadeSlice, SunCascadeSliceLists};
pub use sunshadow::DaylightShadowReceiverConstants;
pub use vertex::{LIVE_INDEX_BUFFER_CAP, PackedVertexRefusal, PropVertex, SurfaceBakedLightId};
pub use vertex::{SurfaceCubeProbeId, FacetSamplerInputs, LevelVertex, WorldVertexRefusal};
pub use vertex::{model_tess_packed_arm, model_tess_vert_decl};
pub use wgpu_layout::{FLAT_TEXTURE_TABLE_SLOTS, VOLUME_TEXTURE_TABLE_SIZE, CUBEMAP_TEXTURE_TABLE_SIZE,
    SAMPLER_TABLE_SIZE, TextureTableDimension, GpuBindSlotDescriptor, GpuBindResourceKind, WgpuLayoutRejection, GpuPassBindingPlan,
    GpuStageVisibility, GpuVertexField, GpuVertexStreamLayout, GpuVertexElementKind,
    plan_gpu_pass_bindings, table_bind_entries};
