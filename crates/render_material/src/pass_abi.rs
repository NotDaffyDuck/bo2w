//! What the GPU contract needs to know about a pass's vertex inputs.

use crate::decl_table::StreamSourceLayout;
use crate::vertex_layout::VertexLayoutFamily;

/// One vertex attribute a pass reads: which source it comes from, where that
/// source sits in its stream, and the shader input location it feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexAttribute {
    pub source: u8,
    pub layout: StreamSourceLayout,
    pub location: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassProgramAbi {
    pub vertex_family: VertexLayoutFamily,
    pub vertex_type: u8,
    pub attributes: Vec<VertexAttribute>,
}

/// Where a shader constant register gets its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstantSource {
    Literal { words: Option<[u32; 4]> },
    Material { name_hash: u32 },
    Code { index: u16, row: u8 },
    ProgramDefined,
}

/// Where a sampler register gets its texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerSource {
    MaterialTexture { name_hash: u32 },
    CodeTexture { index: u32 },
    SurfaceReflectionProbe,
    SurfacePrimaryLightmap,
    SurfaceSecondaryLightmap,
    SurfaceSecondaryBLightmap,
}
