//! One argument of a pass: where a shader register or sampler gets its value.

use crate::RuntimeShaderStage;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeArgumentBinding {
    MaterialTexture {
        destination: u16,
        name_hash: u32,
    },
    CodeTexture {
        destination: u16,
        index: u32,
    },
    MaterialConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        name_hash: u32,
    },
    LiteralConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        words: Option<[u32; 4]>,
    },
    CodeConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        index: u16,
        first_row: u8,
        row_count: u8,
    },
    Unknown {
        argument_type: u16,
        raw: [u8; 8],
    },
}

/// Argument kind numbers; a pass lists its arguments in this order.
pub mod mtl_arg {
    pub const MATERIAL_VERTEX_CONST: u16 = 0;
    pub const LITERAL_VERTEX_CONST: u16 = 1;
    pub const MATERIAL_PIXEL_SAMPLER: u16 = 2;
    pub const CODE_VERTEX_CONST: u16 = 3;
    pub const CODE_PIXEL_SAMPLER: u16 = 4;
    pub const CODE_PIXEL_CONST: u16 = 5;
    pub const MATERIAL_PIXEL_CONST: u16 = 6;
    pub const LITERAL_PIXEL_CONST: u16 = 7;
}

impl RuntimeArgumentBinding {
    /// The kind number this argument sorts under.
    pub fn mtl_arg_type(&self) -> u16 {
        use RuntimeShaderStage::{Pixel, Vertex};
        match self {
            Self::MaterialTexture { .. } => mtl_arg::MATERIAL_PIXEL_SAMPLER,
            Self::CodeTexture { .. } => mtl_arg::CODE_PIXEL_SAMPLER,
            Self::MaterialConstant { stage, .. } => match stage {
                Vertex => mtl_arg::MATERIAL_VERTEX_CONST,
                Pixel => mtl_arg::MATERIAL_PIXEL_CONST,
            },
            Self::LiteralConstant { stage, .. } => match stage {
                Vertex => mtl_arg::LITERAL_VERTEX_CONST,
                Pixel => mtl_arg::LITERAL_PIXEL_CONST,
            },
            Self::CodeConstant { stage, .. } => match stage {
                Vertex => mtl_arg::CODE_VERTEX_CONST,
                Pixel => mtl_arg::CODE_PIXEL_CONST,
            },
            Self::Unknown { argument_type, .. } => *argument_type,
        }
    }

    fn destination(&self) -> Option<u16> {
        match self {
            Self::MaterialTexture { destination, .. }
            | Self::CodeTexture { destination, .. }
            | Self::MaterialConstant { destination, .. }
            | Self::LiteralConstant { destination, .. }
            | Self::CodeConstant { destination, .. } => Some(*destination),
            Self::Unknown { .. } => None,
        }
    }

    /// Kind first, then the material name for material-fed arguments and the
    /// register for the rest.
    pub fn runtime_sort_key(&self) -> (u16, u32) {
        let tie_break = match self {
            Self::MaterialConstant { name_hash, .. } | Self::MaterialTexture { name_hash, .. } => {
                *name_hash
            }
            _ => u32::from(self.destination().unwrap_or(u16::MAX)),
        };
        (self.mtl_arg_type(), tie_break)
    }
}
