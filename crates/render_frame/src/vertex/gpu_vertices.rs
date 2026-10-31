//! Vertices as the GPU reads them, and the reasons decoded data is refused.

use bytemuck::{Pod, Zeroable};

/// Static-model vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PropVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub first_texcoord: [f32; 2],
}

/// Map-geometry vertex, with a tangent and a second UV set for lightmaps.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LevelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub color: [f32; 4],
    pub first_texcoord: [f32; 2],
    pub lightmap_uv: [f32; 2],
}

/// Why a static model's packed vertices were not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedVertexRefusal {
    /// The source stores vertices in a layout this path does not read.
    ForeignLayout { origin_layout: &'static str },
    CountsDiffer { packed: usize, decoded: usize },
    StrideMismatch { table_stride: Option<u16> },
}

/// Why map vertices were not accepted. Same reasons as [`PackedVertexRefusal`],
/// kept separate so each path can grow its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldVertexRefusal {
    ForeignLayout { origin_layout: &'static str },
    CountsDiffer { packed: usize, decoded: usize },
    StrideMismatch { table_stride: Option<u16> },
}
