//! Small identifier and handle types shared by the material catalog.

use super::table::CatalogBuildError;

/// Technique slots per technique set, in the renderer's own numbering.
pub const TECHNIQUE_SLOT_COUNT: usize = 36;

/// Camera passes a material can draw in.
pub const CAMERA_REGION_LIT_OPAQUE: u8 = 0;
pub const CAMERA_REGION_LIT_TRANS: u8 = 1;
pub const CAMERA_REGION_EMISSIVE: u8 = 2;
pub const CAMERA_REGION_DEPTH_HACK: u8 = 3;
pub const CAMERA_REGION_NONE: u8 = 4;

/// Index of a material in the catalog's material list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialAssetId(pub u16);

/// Position of a material in the draw-sorted material order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SortedMaterialOrdinal(u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialGenerationId(pub u64);

/// Ranks past the retail limit all share the last sort band.
pub const fn sort_band(rank: u32) -> u16 {
    let top = SortedMaterialOrdinal::RETAIL_LIMIT - 1;
    if (rank as usize) < top { rank as u16 } else { top as u16 }
}

impl SortedMaterialOrdinal {
    pub const LIMIT: usize = 0x8000;
    pub const RETAIL_LIMIT: usize = 0x1000;

    pub fn new(value: u32) -> Result<Self, CatalogBuildError> {
        match (value as usize) < Self::LIMIT {
            true => Ok(Self(value)),
            false => Err(CatalogBuildError::SortedMaterialCapacity {
                count: value as usize + 1,
                capacity: Self::LIMIT,
            }),
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn sort_band(self) -> u16 {
        sort_band(self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeTechniqueSetId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeImageId(pub u32);

/// An image plus the sampler state and semantic a material binds it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeTextureBinding {
    pub image: RuntimeImageId,
    pub sampler_state: u8,
    pub semantic: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeShaderProgramId {
    pub asset_slot: u32,
    pub program_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeShaderPair {
    pub vertex: RuntimeShaderProgramId,
    pub pixel: RuntimeShaderProgramId,
    pub vertex_decl_slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeProgramIdentity {
    pub vertex_program_hash: u64,
    pub pixel_program_hash: u64,
    pub vertex_decl_slot: u32,
}

/// Key of one compiled shader port: a pass compiled against one vertex type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PortId {
    pub vertex_program_hash: u64,
    pub pixel_program_hash: u64,
    pub vertex_decl_slot: u32,
    pub vertex_type: u8,
    pub custom_sampler_flags: u8,
    pub arguments_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeShaderProgram {
    pub id: RuntimeShaderProgramId,
    pub stage: crate::RuntimeShaderStage,
    pub name: String,
    pub program: Vec<u8>,
}
