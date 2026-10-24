//! Per-frame sun-shadow state: what each cascade renders with, what receivers
//! sample with, and which casters each cascade found.

use glam::Mat4;

use super::atlas::SunShadowViewport;
use super::volume::SunShadowClipPlanes;

/// Constants the receiving shaders read to pick a cascade and scale lookups.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunShadowReceiverConstants {
    pub switch_partition: [f32; 4],
    pub shadowmap_scale: [f32; 4],
}

/// One cascade's camera, depth bias, atlas viewport and caster volume.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowPartition {
    pub clip_from_world: Mat4,
    pub view: Mat4,
    pub projection: Mat4,
    pub polygon_offset: [f32; 4],
    pub viewport: SunShadowViewport,
    pub clip_planes: SunShadowClipPlanes,
}

/// Everything the sun-shadow pass computed for one frame.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowForcedFrame {
    pub partitions: [SunShadowPartition; 2],
    pub pixel_adjust: [f32; 4],
    pub lookup: Mat4,
    pub receiver: SunShadowReceiverConstants,
    pub partition_fraction: [f32; 4],
    pub axes: [[f32; 3]; 3],
    pub view_forward: [f32; 3],
    pub view_right: [f32; 3],
    pub tan_half_fov: [f32; 2],
    pub near_shadow_min_dist: f32,
    pub shadow_org_pixel_center: [f32; 2],
    pub projection_offset_clip: [f32; 2],
}

/// Visible casters found for one cascade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SunShadowPartitionLists {
    pub surface_vis: Vec<u8>,
    pub smodel_vis: Vec<u8>,
    pub world_surfs: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SunShadowCasterLists {
    pub partitions: [SunShadowPartitionLists; 2],
}
