//! Immutable per-frame render output: the packed source lists the backend reads,
//! the destination-ring records, per-frame product status and the GPU layout
//! contract. Everything the renderer hands across a frame boundary is defined here.
pub mod code_math; pub mod packing; pub mod portal_eyes;
mod frontend; mod material_frame; mod mesh_pool; mod products; mod retained; mod sampler_identity;
mod sun_fx; mod sunshadow; mod vertex; mod wgpu_layout;
pub use frontend::{FRONTEND_DRAW_LISTS_DWORDS, GfxSmodelRigidEntry, GfxTrianglesListEntry, GfxXModelRigidEntry,
    LIST_TOKEN, PackedFrontendLists, SMODEL_RIGID_ENTRY_STRIDE, SmodelPretessRange};
pub use frontend::{SRC_SMODEL_CACHED_BYTES, SRC_SMODEL_CACHED_PTR, SRC_SMODEL_PRETESS_BYTES, SRC_SMODEL_PRETESS_PTR,
    SRC_SMODEL_RIGID_COUNT, SRC_SMODEL_RIGID_PTR, SRC_SMODEL_SKINNED_BYTES, SRC_SMODEL_SKINNED_PTR};
pub use frontend::{SRC_WORLD_COUNT, SRC_WORLD_PTR, SRC_XMODEL_RIGID_COUNT, SRC_XMODEL_RIGID_PTR};
pub use material_frame::{MaterialExecFrame, OutdoorLookup, SpotShadowReceiver};
pub use mesh_pool::{GfxMeshData, reserve_code_mesh, reserve_code_mesh_indices, reserve_code_mesh_verts,
    shrink_code_mesh_verts};
pub use products::{FrameProduct, FrameProductKind, FrameProductStatus, FrameProductsSnapshot,
    MissingProductCause, PACKED_SEGMENT_OWNERS, PackedSegment, PackedSegments, ProductTarget,
    RenderFocusFrame, SourceRevisions, SpotShadowFrameSlot, publish_rows};
pub use retained::{BspCameraLane, LightAttenuationBind, RENDER_FX_DEPTH_HACK, RetainedDrawItem,
    RetainedDrawKind, T5LightFalloffPack, XMODEL_OBJECT_ID_VIEWMODEL, host_viewmodel_render_fx_flags};
pub use sampler_identity::TextureBindIdentity;
pub use sun_fx::{SunEffectsDef, SunEffectsFrame, angular_lerp};
pub use sunshadow::{SUN_SHADOW_CASTER_TECH, SUN_SHADOW_FORCED_PROFILE, SUN_SHADOW_MAX_CLIP_PLANES};
pub use sunshadow::{SUN_SHADOW_PARTITION_COUNT, SunShadowAtlasProfile, SunShadowCasterLists, SunShadowViewport};
pub use sunshadow::{SunShadowClipPlanes, SunShadowForcedFrame, SunShadowPartition, SunShadowPartitionLists};
pub use sunshadow::SunShadowReceiverConstants;
pub use vertex::{DYNAMIC_INDEX_BUFFER_CAPACITY, PackedVertexRefusal, SmodelVertex, SurfaceLightmapId};
pub use vertex::{SurfaceReflectionProbeId, SurfaceSamplerInputs, WorldVertex, WorldVertexRefusal};
pub use vertex::{xmodel_tess_info_packed_arm, xmodel_tess_info_vert_decl_type};
pub use wgpu_layout::{TEXTURE_TABLE_2D_CAPACITY, TEXTURE_TABLE_3D_CAPACITY, TEXTURE_TABLE_CUBE_CAPACITY,
    TEXTURE_TABLE_SAMPLER_CAPACITY, TextureTableDimension, WgpuBindLayoutEntry, WgpuBindingKind, WgpuLayoutRefusal, WgpuPassLayout,
    WgpuShaderVisibility, WgpuVertexAttribute, WgpuVertexBufferLayout, WgpuVertexFormat,
    derive_wgpu_pass_layout, texture_table_bind_entries};
