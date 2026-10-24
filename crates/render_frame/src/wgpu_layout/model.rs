//! Plain data describing a derived layout, and why a derivation can be refused.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WgpuVertexFormat {
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
pub struct WgpuVertexAttribute {
    pub location: u32,
    pub offset: u64,
    pub format: WgpuVertexFormat,
}

/// One vertex stream: its stride and the attributes read from it, ordered by
/// offset (ties broken by shader location).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgpuVertexBufferLayout {
    pub stream: u8,
    pub array_stride: u64,
    pub attributes: Vec<WgpuVertexAttribute>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WgpuShaderVisibility {
    Vertex,
    Fragment,
    VertexFragment,
}

/// The kind of texture a bindless table binding holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureTableDimension {
    D2,
    Cube,
    D3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WgpuBindingKind {
    UniformBuffer { min_size: u64 },
    ReadOnlyStorageBuffer,
    TextureArray { dimension: TextureTableDimension, count: u32 },
    SamplerArray { count: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WgpuBindLayoutEntry {
    pub group: u8,
    pub binding: u16,
    pub visibility: WgpuShaderVisibility,
    pub kind: WgpuBindingKind,
    pub register_packed: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgpuPassLayout {
    pub vertex_buffers: Vec<WgpuVertexBufferLayout>,
    pub bind_entries: Vec<WgpuBindLayoutEntry>,
}

/// Reasons a pass's interface cannot be turned into a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgpuLayoutRefusal {
    UnknownVertexFormat { source: u8, raw: u8 },
    MissingStreamExtent { vertex_type: u8, stream: u8 },
    DuplicateShaderLocation { location: u32 },
    VertexAttributePastStride { stream: u8, offset: u8, byte_len: u8, stride: u16 },
}
