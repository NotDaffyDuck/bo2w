//! The sun's shadow: the basis its cascades are laid out in, the projections
//! fitted to the camera, the constants receivers read them back with, and
//! which casters are worth drawing.

mod axes;
mod casters;
mod forced;
mod projection;

pub use super::sun_shadow_clip::{
    SunShadowClipInput, SunShadowFrustumRays, sun_shadow_clip_planes, sun_shadow_frustum_rays,
};
pub use render_frame::{
    SUN_SHADOW_FORCED_PROFILE, SUN_SHADOW_PARTITION_COUNT, SunShadowAtlasProfile,
    SunShadowClipPlanes, SunShadowForcedFrame, SunShadowPartition, SunShadowReceiverConstants,
    SunShadowViewport,
};

pub use axes::{
    SUN_AXIS_HELPER_Z_MAX, scene_extents_along_dir, snap_sun_proj_origin, sun_axes_from_dir,
    sun_axes_from_light_dir, sun_shadow_forward_from_light_dir,
    sun_shadow_packed_axes_from_light_dir, sun_shadow_view_matrix, tan_half_fov_from_clip,
    view_org_in_sun_proj,
};
pub use casters::{
    STATIC_MODEL_FLAG_NO_CAST_SHADOW, SUN_SHADOW_CASTER_TECH, SunShadowSamplingPath,
    smodel_casts_sun_shadow, sun_shadow_cutout_name,
};
pub use forced::{SunShadowCamera, forced_fallback_frame, forced_fallback_metrics};
pub use projection::{
    SunShadowMapMetrics, SunShadowProjectionSetup, shadowmap_polygon_offset,
    sun_shadow_lookup_matrix, sun_shadow_ortho_projection, sun_shadow_projection_m22,
    sun_shadow_projection_m32, sun_shadowmap_pixel_adjust,
};

pub use crate::prepare::scene::frustum::clip_from_world_frustum_planes;

/// The texture the sun's shadow map is bound as.
pub const CODE_TEXTURE_SHADOWMAP_SUN: u32 = 6;

/// The texture the spot lights' shadow map is bound as.
pub const CODE_TEXTURE_SHADOWMAP_SPOT: u32 = 7;

/// Code constants the shadow passes and receivers are fed through.
pub const CODE_SHADOWMAP_POLYGON_OFFSET: u16 = 0x15;
pub const CODE_SHADOWMAP_SWITCH_PARTITION: u16 = 0x1f;
pub const CODE_SHADOWMAP_SCALE: u16 = 0x20;
pub const CODE_SUN_SHADOWMAP_PIXEL_ADJUST: u16 = 0x34;
pub const CODE_SHADOWMAP_LOOKUP_TRANSPOSE: u16 = 0x5a;

pub const SUN_SHADOW_QUARTER: f32 = 0.25;
pub const SUN_SHADOW_HALF: f32 = 0.5;
pub const SUN_SHADOW_NEG_QUARTER: f32 = -0.25;

/// A cascade covers this many texels across.
pub const SUN_SHADOW_SAMPLE_EXTENT_PIXELS: f32 = 1024.0;

/// A near-cascade texel, in world units.
pub const SM_SUN_SAMPLE_SIZE_NEAR_DEFAULT: f32 = 0.25;

/// A far-cascade texel is this many near ones.
pub const SM_SUN_PARTITION_RATIO: f32 = 4.0;

/// What a frame's sun-shadow collection works in, kept between frames so the
/// buffers are only allocated once.
#[derive(Default)]
pub(crate) struct SunShadowStaging {
    pub surface_vis: [Vec<u8>; 2],
    pub smodel_vis: [Vec<u8>; 2],
    pub frustum_draw_msb: [Vec<u32>; 2],
    pub draw_cell_n: u32,
}
