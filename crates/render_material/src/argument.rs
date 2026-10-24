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

/// Shader argument kinds, in the order passes list their arguments.
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
    pub fn mtl_arg_type(&self) -> u16 {
        match self {
            Self::MaterialConstant {
                stage: RuntimeShaderStage::Vertex,
                ..
            } => mtl_arg::MATERIAL_VERTEX_CONST,
            Self::LiteralConstant {
                stage: RuntimeShaderStage::Vertex,
                ..
            } => mtl_arg::LITERAL_VERTEX_CONST,
            Self::MaterialTexture { .. } => mtl_arg::MATERIAL_PIXEL_SAMPLER,
            Self::CodeConstant {
                stage: RuntimeShaderStage::Vertex,
                ..
            } => mtl_arg::CODE_VERTEX_CONST,
            Self::CodeTexture { .. } => mtl_arg::CODE_PIXEL_SAMPLER,
            Self::CodeConstant {
                stage: RuntimeShaderStage::Pixel,
                ..
            } => mtl_arg::CODE_PIXEL_CONST,
            Self::MaterialConstant {
                stage: RuntimeShaderStage::Pixel,
                ..
            } => mtl_arg::MATERIAL_PIXEL_CONST,
            Self::LiteralConstant {
                stage: RuntimeShaderStage::Pixel,
                ..
            } => mtl_arg::LITERAL_PIXEL_CONST,
            Self::Unknown { argument_type, .. } => *argument_type,
        }
    }

    fn dest(&self) -> u16 {
        match self {
            Self::MaterialTexture { destination, .. }
            | Self::CodeTexture { destination, .. }
            | Self::MaterialConstant { destination, .. }
            | Self::LiteralConstant { destination, .. }
            | Self::CodeConstant { destination, .. } => *destination,
            Self::Unknown { .. } => u16::MAX,
        }
    }

    pub fn runtime_sort_key(&self) -> (u16, u32) {
        let ty = self.mtl_arg_type();
        let secondary = match self {
            Self::MaterialConstant { name_hash, .. } | Self::MaterialTexture { name_hash, .. } => {
                *name_hash
            }
            _ => u32::from(self.dest()),
        };
        (ty, secondary)
    }
}
