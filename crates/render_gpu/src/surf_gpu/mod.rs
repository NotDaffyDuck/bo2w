// Frame data the extract publishes and the passes read.
mod frame_publish;
mod image_registry;
mod postfx_frame;
mod frame_outputs;

// Pass setup and per-pass state.
mod blend_state;
mod bo2_afterlife; // bo2zm
mod bo2_bloom;
mod bo2_dof;
mod z_layers;
mod draw;
mod scene_geo;
mod guest_view;
mod hud_quads;
mod portal_view;
mod dof_bloom;
mod sun_effects;
mod ui3d;
pub(crate) mod scene_depth;

// GPU-side resources and uploads.
mod gpu_bufs;
mod model_lighting_tiles;
mod spot_shadow_gpu;
mod sun_shadow_gpu;
mod prop_cache;

pub mod gpu_types {
    pub use render_frame::{
        FLAT_TEXTURE_TABLE_SLOTS, VOLUME_TEXTURE_TABLE_SIZE, CUBEMAP_TEXTURE_TABLE_SIZE,
        SAMPLER_TABLE_SIZE, GpuBindSlotDescriptor, GpuBindResourceKind, WgpuLayoutRejection,
        GpuPassBindingPlan, GpuStageVisibility, GpuVertexField, GpuVertexStreamLayout,
        GpuVertexElementKind, plan_gpu_pass_bindings, table_bind_entries,
    };
}
pub use gpu_types::*;

pub use render_frame::{
    HeldDrawItem, HeldDrawKind, SUN_CASTER_TECH, SUNLIGHT_CASCADE_TOTAL,
    FacetSamplerInputs,
};
pub use render_material::{
    CodeSamplerChannel, LiveTextureBinding, SamplerOrigin, SamplerTexShape, MethodType,
};

pub(crate) use draw::register_facet_draw_render;
pub use frame_publish::{
    ENGINE_TEXTURE_FLOATZ, ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT, ENGINE_TEXTURE_SHADOW_MAP_CONE,
    ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT, CODE_TEXTURE_UI3D, StaticGeometryExtract, InstalledRenderWorld,
    PublishedRenderFrame, RenderFrameData, RenderWorldData,
};
pub use scene_geo::{
    ExtractedInspectGeometry, ExtractedT6Dynamic, T6DeferredFill, T6DynamicDraw, T6SkinShape,
    T6Skinned, T6StaticDraw, geometry_inspect_enabled,
};
pub use gpu_bufs::*;
pub use guest_view::{GuestViewFrame, GuestViewTexture};
pub use model_lighting_tiles::{ModelLightingTileUpload, ModelLightingTileUploads};
pub use dof_bloom::{FocusBlur, BokehFrame, BLOOM_APPLY_MATERIAL, BLOOM_SETUP_MATERIAL, BloomFrame};
pub use postfx_frame::ExtractedPostEffect;
pub use frame_outputs::ExtractedRenderFrameOutputs;
pub use spot_shadow_gpu::*;
pub use sun_shadow_gpu::*;
pub use prop_cache::*;
pub use ui3d::{UI3D_SIZE, Ui3dFrame, Ui3dQuad};
