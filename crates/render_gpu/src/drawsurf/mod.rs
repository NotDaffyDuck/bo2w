mod depth_range;
mod draw;
mod frame_publish; // bo2mp: frame publication, kept when the MW2 SM3 pass goes
mod postfx_frame; // bo2mp: post-fx frame data, kept when the MW2 SM3 pass goes
mod blend_state; // bo2mp: pass blend decode, kept when the MW2 SM3 pass goes
mod image_registry; // bo2mp: uploaded images, kept when the MW2 SM3 pass goes
mod bo2_bloom; // bo2zm
mod bo2_dof; // bo2mp
mod geometry_diagnostic;
mod gpu_prepare;
mod gpu_resources;
mod guest_view; // bo2zm: the Special Weapons tab's guest weapons
mod portal_view; // bo2mp: seeing through a linked portal
mod ui3d; // bo2mp: the UI3D texture (Target Finder screen)
mod hud_quads;
mod model_lighting_tiles;
mod postfx_dof;
mod products;
pub(crate) mod scene_depth;
mod shadowmap_spot_gpu;
mod shadowmap_sun_gpu;
mod smodel_cache_gpu;
mod sun_effects;

pub use products::ExtractedRenderFrameProducts;
pub use render_frame::{
    RetainedDrawItem, RetainedDrawKind, SUN_SHADOW_CASTER_TECH, SUN_SHADOW_PARTITION_COUNT,
    SurfaceSamplerInputs,
};
pub use render_material::{PackedCodeSamplerLane, RuntimeTextureBinding, TechType};
pub use render_material::{SamplerSource, SamplerTextureDimension};
pub mod gpu_contract {
    pub use render_frame::{
        TEXTURE_TABLE_2D_CAPACITY, TEXTURE_TABLE_3D_CAPACITY, TEXTURE_TABLE_CUBE_CAPACITY,
        TEXTURE_TABLE_SAMPLER_CAPACITY, WgpuBindLayoutEntry, WgpuBindingKind, WgpuLayoutRefusal,
        WgpuPassLayout, WgpuShaderVisibility, WgpuVertexAttribute, WgpuVertexBufferLayout,
        WgpuVertexFormat, derive_wgpu_pass_layout, texture_table_bind_entries,
    };
}
pub use gpu_contract::*;
pub use gpu_prepare::{
    ConstantPackRefusal, PassConstantBuffers, overlay_packed_code_on_banks, split_bind_layout,
};

pub use frame_publish::{
    CODE_TEXTURE_FLOATZ, CODE_TEXTURE_RESOLVED_POST_SUN, CODE_TEXTURE_SHADOWMAP_SPOT,
    CODE_TEXTURE_SHADOWMAP_SUN, CODE_TEXTURE_UI3D, ExtractedStaticGeometry, InstalledRenderWorld,
    PublishedRenderFrame, RenderFrameData, RenderWorldData,
};
pub(crate) use draw::register_drawsurf_render;
pub use guest_view::{GuestViewFrame, GuestViewTexture};
pub use ui3d::{UI3D_SIZE, Ui3dFrame, Ui3dQuad};
pub use geometry_diagnostic::{
    ExtractedDiagnosticGeometry, ExtractedT6Dynamic, T6DeferredFill, T6DynamicDraw, T6SkinShape,
    T6Skinned, T6StaticDraw, geometry_diagnostic_enabled,
};
pub use gpu_resources::*;
pub use model_lighting_tiles::{ModelLightingTileUpload, ModelLightingTileUploads};
pub use postfx_frame::ExtractedPostFx;
pub use postfx_dof::{DepthOfField, DofFrame, GLOW_APPLY_MATERIAL, GLOW_SETUP_MATERIAL, GlowFrame};
pub use shadowmap_spot_gpu::*;
pub use shadowmap_sun_gpu::*;
pub use smodel_cache_gpu::*;
