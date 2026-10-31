//! What the GPU contract needs to know about a pass's vertex inputs.

use crate::decl_table::VertexStreamArrangement;
use crate::vertex_layout::VertexFormatLineage;

/// One vertex attribute a pass reads: which source it comes from, where that
/// source sits in its stream, and the shader input location it feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassVertexInput {
    pub source: u8,
    pub layout: VertexStreamArrangement,
    pub location: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassShaderContract {
    pub vertex_family: VertexFormatLineage,
    pub vertex_format_code: u8,
    pub attributes: Vec<PassVertexInput>,
}

/// Where a shader constant register gets its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstantOrigin {
    Literal { lane_bits: Option<[u32; 4]> },
    Material { label_digest: u32 },
    Code { entry: u16, row_at: u8 },
    ShaderSupplied,
}

/// Where a sampler register gets its texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerOrigin {
    ShaderSamplerImage { label_digest: u32 },
    EngineSuppliedTexture { index: u32 },
    SurfaceCubemapProbe,
    SurfaceMainBakedLight,
    SurfaceAuxiliaryBakedLight,
    SurfaceTertiaryBakedLight,
}
