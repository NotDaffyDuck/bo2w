//! Why a draw cannot be turned into an executable material.

use asset_core::AssetNamespace;

use crate::prepared::GfxPassStateBits;
use crate::{
    CatalogBuildError, MaterialAssetId, MaterialGenerationId, RuntimeShaderPair,
    RuntimeShaderStage, RuntimeTechniqueSetId, TechType,
};

/// Pass-state fields the host cannot reproduce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnsupportedStateFields {
    pub unknown_blend_factor: bool,
    pub unknown_blend_operation: bool,
    pub stencil: bool,
}

impl UnsupportedStateFields {
    pub fn any(self) -> bool {
        self.unknown_blend_factor || self.unknown_blend_operation || self.stencil
    }
}

impl GfxPassStateBits {
    /// `None` when every blend field of this pass maps onto the host.
    pub fn unsupported_host_fields(self) -> Option<UnsupportedStateFields> {
        use d3d9_state::BlendFactor;

        let colour_op = ((self.word0 >> 8) & 0x7) as u8;
        let alpha_word = (self.word0 >> 16) & 0x7ff;
        let alpha_op = (alpha_word >> 8) & 0x7;

        let colour_known = BlendFactor::from_raw(self.word0 & 0xf).is_known()
            && BlendFactor::from_raw((self.word0 >> 4) & 0xf).is_known();
        let alpha_known = BlendFactor::from_raw(alpha_word & 0xf).is_known()
            && BlendFactor::from_raw((alpha_word >> 4) & 0xf).is_known();

        let fields = UnsupportedStateFields {
            unknown_blend_factor: colour_op != 0
                && (!colour_known || (alpha_op != 0 && !alpha_known)),
            unknown_blend_operation: colour_op > 5 || (colour_op != 0 && alpha_op > 5),
            stencil: false,
        };
        fields.any().then_some(fields)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialRefusal {
    SortedMaterialTableMissing,
    SortedMaterialOrdinalOutOfRange {
        ordinal: u32,
        table_len: usize,
    },
    SortedMaterialBuildFailed {
        cause: CatalogBuildError,
    },
    StaleMaterialGeneration {
        retained: MaterialGenerationId,
        current: MaterialGenerationId,
    },
    MaterialOutOfRange {
        material: MaterialAssetId,
    },
    LocalTechniqueSetOutOfRange {
        set: RuntimeTechniqueSetId,
    },
    RemappedTechniqueSetOutOfRange {
        set: RuntimeTechniqueSetId,
    },
    RemapMissing {
        pointer_identity: u32,
    },
    RemapCycle {
        first_set: RuntimeTechniqueSetId,
    },
    TechniqueAbsent {
        tech_type: TechType,
    },
    EmptyTechnique {
        tech_type: TechType,
    },
    StateEntriesMissing,
    StateEntryOutOfRange {
        tech_type: TechType,
    },
    StateRowOutOfRange {
        row: usize,
        pass_index: u8,
    },
    UnsupportedState {
        pass_index: u8,
        fields: UnsupportedStateFields,
        word0: u32,
        word1: u32,
    },
    PassIndexOverflow {
        pass_index: usize,
    },
    ShaderProgramMissing {
        pass_index: u8,
    },
    UnsupportedShaderPair {
        pass_index: u8,
        pair: RuntimeShaderPair,
    },
    MissingTexture {
        pass_index: u8,
        name_hash: u32,
    },
    MissingMaterialConstant {
        pass_index: u8,
        name_hash: u32,
    },
    MissingCodeConstant {
        pass_index: u8,
        stage: RuntimeShaderStage,
        index: u16,
    },
    CodeConstantRowsOutOfRange {
        pass_index: u8,
        stage: RuntimeShaderStage,
        index: u16,
        first_row: u8,
        row_count: u8,
        available_rows: usize,
    },
    MissingCodeTexture {
        pass_index: u8,
        index: u32,
    },
    MissingLiteralConstant {
        pass_index: u8,
    },
    UnknownArgumentType {
        pass_index: u8,
        argument_type: u16,
    },
    PassArgCountMismatch {
        pass_index: u8,
        per_prim: u8,
        per_obj: u8,
        stable: u8,
        arguments: usize,
    },
    TechniqueSetNamespaceMismatch {
        want: AssetNamespace,
        got: AssetNamespace,
    },
}
