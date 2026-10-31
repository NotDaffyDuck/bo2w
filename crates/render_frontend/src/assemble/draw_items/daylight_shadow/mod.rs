//! The sun's shadow: the basis its cascades are laid out in, the projections
//! fitted to the camera, the constants receivers read them back with, and
//! which casters are worth drawing.

mod axes;
mod casters;
mod forced;
mod proj_m;

pub use super::sun_shadow_clip::{
    SunShadowClipInput, SunShadowFrustumRays, sun_shadow_clip_planes, sun_shadow_frustum_rays,
};
pub use render_frame::{
    DAYLIGHT_SHADOW_FORCED_PROFILE, SUNLIGHT_CASCADE_TOTAL, DaylightShadowAtlasProfile,
    SunShadowClipPlanes, DaylightShadowForcedFrame, SunCascadeSlice, DaylightShadowReceiverConstants,
    SunlightShadowRegion,
};

pub use axes::{
    DAYLIGHT_AXIS_HELPER_Z_MAX, stage_extents_along_dir, snap_daylight_proj_origin, daylight_axes_from_dir,
    daylight_axes_from_light_dir, daylight_shadow_forward_from_light_dir,
    daylight_shadow_compact_axes_from_light_dir, sun_shadow_view_matrix, half_fov_tan_of_clip,
    eye_org_in_daylight_proj,
};
pub use casters::{
    PROP_FLAG_NO_CAST_SHADOW, DAYLIGHT_CASTER_TECH, DaylightShadowSamplingPath,
    prop_casts_daylight_shadow, daylight_shadow_cutout_name,
};
pub use forced::{DaylightShadowCamera, forced_backup_frame, forced_fallback_metrics};
pub use proj_m::{
    DaylightShadowMapMetrics, DaylightShadowProjectionSetup, shadow_map_polygon_offset,
    daylight_shadow_lookup_matrix, daylight_shadow_ortho_projection, sun_shadow_projection_m22,
    sun_shadow_projection_m32, daylight_shadow_map_pixel_adjust,
};

pub use crate::prepare::scene::view_planes::frustum_planes_from_clip_matrix;

/// The texture the sun's shadow map is bound as.
pub const ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT: u32 = 6;

/// The texture the spot lights' shadow map is bound as.
pub const ENGINE_TEXTURE_SHADOW_MAP_CONE: u32 = 7;

/// Code constants the shadow passes and receivers are fed through.
pub const ENGINE_SHADOW_MAP_POLYGON_OFFSET: u16 = 0x15;
pub const ENGINE_SHADOW_MAP_SWITCH_PARTITION: u16 = 0x1f;
pub const ENGINE_SHADOW_MAP_SCALE: u16 = 0x20;
pub const ENGINE_DAYLIGHT_SHADOW_MAP_PIXEL_ADJUST: u16 = 0x34;
pub const ENGINE_SHADOW_MAP_LOOKUP_TRANSPOSE: u16 = 0x5a;

pub const DAYLIGHT_SHADOW_QUARTER: f32 = 0.25;
pub const DAYLIGHT_SHADOW_HALF: f32 = 0.5;
pub const DAYLIGHT_SHADOW_NEG_QUARTER: f32 = -0.25;

/// A cascade covers this many texels across.
pub const DAYLIGHT_SHADOW_SAMPLE_EXTENT_PIXELS: f32 = 1024.0;

/// A near-cascade texel, in world units.
pub const SM_DAYLIGHT_SAMPLE_SIZE_NEAR_DEFAULT: f32 = 0.25;

/// A far-cascade texel is this many near ones.
pub const SM_DAYLIGHT_PARTITION_RATIO: f32 = 4.0;

/// What a frame's sun-shadow collection works in, kept between frames so the
/// buffers are only allocated once.
#[derive(Default)]
pub(crate) struct DaylightShadowStaging {
    pub facet_vis: [Vec<u8>; 2],
    pub prop_vis: [Vec<u8>; 2],
    pub view_volume_draw_msb: [Vec<u32>; 2],
    pub draw_room_tally: u32,
}
