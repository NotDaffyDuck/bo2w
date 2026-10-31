//! One argument of a pass: where a shader register or sampler gets its value.

use crate::LiveShaderStage;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LiveArgumentBinding {
    ShaderSamplerImage {
        destination: u16,
        label_digest: u32,
    },
    EngineSuppliedTexture {
        destination: u16,
        index: u32,
    },
    ShaderParameterValue {
        stage: LiveShaderStage,
        destination: u16,
        label_digest: u32,
    },
    InlineConstant {
        stage: LiveShaderStage,
        destination: u16,
        words: Option<[u32; 4]>,
    },
    EngineConstant {
        stage: LiveShaderStage,
        destination: u16,
        index: u16,
        row_base: u8,
        line_total: u8,
    },
    Unknown {
        kind_code: u16,
        raw: [u8; 8],
    },
}

/// Argument kind numbers; a pass lists its arguments in this order.
pub mod arg_kind {
    pub const SURFACE_VERTEX_CONSTANT: u16 = 0;
    pub const INLINE_VERTEX_CONSTANT: u16 = 1;
    pub const SURFACE_FRAGMENT_SAMPLER: u16 = 2;
    pub const ENGINE_VERTEX_CONSTANT: u16 = 3;
    pub const ENGINE_FRAGMENT_SAMPLER: u16 = 4;
    pub const ENGINE_FRAGMENT_CONSTANT: u16 = 5;
    pub const SURFACE_FRAGMENT_CONSTANT: u16 = 6;
    pub const INLINE_FRAGMENT_CONSTANT: u16 = 7;
}

impl LiveArgumentBinding {
    /// The kind number this argument sorts under.
    pub fn material_argument_kind(&self) -> u16 {
        use LiveShaderStage::{Pixel, Vertex};
        match self {
            Self::ShaderSamplerImage { .. } => arg_kind::SURFACE_FRAGMENT_SAMPLER,
            Self::EngineSuppliedTexture { .. } => arg_kind::ENGINE_FRAGMENT_SAMPLER,
            Self::ShaderParameterValue { stage, .. } => match stage {
                Vertex => arg_kind::SURFACE_VERTEX_CONSTANT,
                Pixel => arg_kind::SURFACE_FRAGMENT_CONSTANT,
            },
            Self::InlineConstant { stage, .. } => match stage {
                Vertex => arg_kind::INLINE_VERTEX_CONSTANT,
                Pixel => arg_kind::INLINE_FRAGMENT_CONSTANT,
            },
            Self::EngineConstant { stage, .. } => match stage {
                Vertex => arg_kind::ENGINE_VERTEX_CONSTANT,
                Pixel => arg_kind::ENGINE_FRAGMENT_CONSTANT,
            },
            LiveArgumentBinding::Unknown { kind_code: kind, .. } => *kind,
        }
    }

    fn destination(&self) -> Option<u16> {
        match self {
            Self::ShaderSamplerImage { destination: reg, .. }
            | Self::EngineSuppliedTexture { destination: reg, .. }
            | Self::ShaderParameterValue { destination: reg, .. }
            | Self::InlineConstant { destination: reg, .. }
            | Self::EngineConstant { destination: reg, .. } => Some(*reg),
            Self::Unknown { .. } => None,
        }
    }

    /// Kind first, then the material name for material-fed arguments and the
    /// register for the rest.
    pub fn draw_order_pair(&self) -> (u16, u32) {
        let tie_break = match self {
            Self::ShaderParameterValue { label_digest, .. } | Self::ShaderSamplerImage { label_digest, .. } => {
                *label_digest
            }
            _ => u32::from(self.destination().unwrap_or(u16::MAX)),
        };
        (self.material_argument_kind(), tie_break)
    }
}
