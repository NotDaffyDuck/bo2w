//! Per-frame sun-shadow state: what each cascade renders with, what receivers
//! sample with, and which casters each cascade found.

use glam::Mat4;

use super::atlas::SunlightShadowRegion;
use super::volume::SunShadowClipPlanes;

/// Constants the receiving shaders read to pick a cascade and scale lookups.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DaylightShadowReceiverConstants {
    pub switch_split: [f32; 4],
    pub shadow_map_scale: [f32; 4],
}

/// One cascade's camera, depth bias, atlas viewport and caster volume.
#[derive(Clone, Copy, Debug)]
pub struct SunCascadeSlice {
    pub world_to_clip: Mat4,
    pub view: Mat4,
    pub projection: Mat4,
    pub depth_bias: [f32; 4],
    pub viewport: SunlightShadowRegion,
    pub frustum_clip_planes_for: SunShadowClipPlanes,
}

/// Everything the sun-shadow pass computed for one frame.
#[derive(Clone, Copy, Debug)]
pub struct DaylightShadowForcedFrame {
    pub partitions: [SunCascadeSlice; 2],
    pub texel_offset_correction: [f32; 4],
    pub lookup: Mat4,
    pub receiver: DaylightShadowReceiverConstants,
    pub split_fraction: [f32; 4],
    pub axes: [[f32; 3]; 3],
    pub eye_forward: [f32; 3],
    pub eye_right: [f32; 3],
    pub fov_half_tan: [f32; 2],
    pub near_shadow_floor: f32,
    pub shadow_origin_texel_centre: [f32; 2],
    pub proj_offset_clip: [f32; 2],
}

/// Visible casters found for one cascade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SunCascadeSliceLists {
    pub facet_vis: Vec<u8>,
    pub prop_vis: Vec<u8>,
    pub level_faces: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DaylightShadowCasterLists {
    pub partitions: [SunCascadeSliceLists; 2],
}
