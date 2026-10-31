//! Plain data describing a derived layout, and why a derivation can be refused.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuVertexElementKind {
    Float32x2,
    Float16x2,
    Float32x3,
    Float32x4,
    Unorm8x4,
    Uint8x4,
    Uint32,
    Uint32x4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GpuVertexField {
    pub location: u32,
    pub offset: u64,
    pub format: GpuVertexElementKind,
}

/// One vertex stream: its stride and the attributes read from it, ordered by
/// offset (ties broken by shader location).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuVertexStreamLayout {
    pub slot: u8,
    pub array_stride: u64,
    pub attributes: Vec<GpuVertexField>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuStageVisibility {
    Vertex,
    Fragment,
    VertexAndPixelStage,
}

/// The kind of texture a bindless table binding holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureTableDimension {
    D2,
    Cube,
    D3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuBindResourceKind {
    UniformBuffer { min_size: u64 },
    ReadableDataBuffer,
    TextureList { dimension: TextureTableDimension, count: u32 },
    SamplerArray { count: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GpuBindSlotDescriptor {
    pub band: u8,
    pub binding: u16,
    pub visibility: GpuStageVisibility,
    pub kind: GpuBindResourceKind,
    pub register_packed: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuPassBindingPlan {
    pub vertex_buffers: Vec<GpuVertexStreamLayout>,
    pub bind_entries: Vec<GpuBindSlotDescriptor>,
}

/// Reasons a pass's interface cannot be turned into a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgpuLayoutRejection {
    UnlistedVertexFormat { stream_slot: u8, decl_code: u8 },
    StreamExtentAbsent { vertex_format_code: u8, stream: u8 },
    DuplicateShaderLocation { location: u32 },
    AttrOverrunsStride { stream: u8, offset: u8, size: u8, stride_bytes: u16 },
}
