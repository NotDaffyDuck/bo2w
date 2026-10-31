//! Small identifier and handle types shared by the material catalog.

use super::table::LedgerBuildError;

/// Technique slots per technique set, in the renderer's own numbering.
pub const RENDER_RECIPE_SLOT_TOTAL: usize = 36;

/// Camera passes a material can draw in.
pub const VIEW_STAGE_SOLID_LIT: u8 = 0;
pub const VIEW_STAGE_BLENDED_LIT: u8 = 1;
pub const VIEW_STAGE_SELF_LIT: u8 = 2;
pub const CAMERA_REGION_NEAR_DEPTH_TRICK: u8 = 3;
pub const VIEW_STAGE_ABSENT: u8 = 4;

/// Index of a material in the catalog's material list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialAssetKey(pub u16);

/// Position of a material in the draw-sorted material order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SortedMaterialRank(u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialEpochId(pub u64);

/// Ranks past the retail limit all share the last sort band.
pub const fn sort_band(rank: u32) -> u16 {
    let top = SortedMaterialRank::SHIPPED_SLOT_CEILING - 1;
    if (rank as usize) < top { rank as u16 } else { top as u16 }
}

impl SortedMaterialRank {
    pub const SLOT_CEILING: usize = 0x8000;
    pub const SHIPPED_SLOT_CEILING: usize = 0x1000;

    pub fn new(rank: u32) -> Result<Self, LedgerBuildError> {
        let slots_needed = rank as usize + 1;
        if slots_needed > Self::SLOT_CEILING {
            return Err(LedgerBuildError::RankTableFull {
                count: slots_needed,
                capacity: Self::SLOT_CEILING,
            });
        }
        Ok(Self(rank))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn sort_band(self) -> u16 {
        sort_band(self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveTechniqueBundleKey(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveImageId(pub u32);

/// An image plus the sampler state and semantic a material binds it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveTextureBinding {
    pub image: LiveImageId,
    pub texture_sampling_mode: u8,
    pub semantic: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveShaderProgramId {
    pub asset_table_position: u32,
    pub program_digest: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveShaderPair {
    pub vertex: LiveShaderProgramId,
    pub pixel: LiveShaderProgramId,
    pub vertex_format_slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveShaderIdentity {
    pub vertex_shader_digest: u64,
    pub fragment_shader_digest: u64,
    pub vertex_format_slot: u32,
}

/// Key of one compiled shader port: a pass compiled against one vertex type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderPortKey {
    pub vertex_shader_digest: u64,
    pub fragment_shader_digest: u64,
    pub vertex_format_slot: u32,
    pub vertex_format_code: u8,
    pub sampler_custom_bits: u8,
    pub argument_digest: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveShaderCombo {
    pub id: LiveShaderProgramId,
    pub stage: crate::LiveShaderStage,
    pub name: String,
    pub program: Vec<u8>,
}
