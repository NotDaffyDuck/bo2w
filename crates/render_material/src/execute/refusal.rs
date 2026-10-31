//! Why a draw cannot be turned into an executable material.

use asset_core::AssetCategoryScope;

use crate::prepared::VisualPassStateBits;
use crate::{
    LedgerBuildError, MaterialAssetKey, MaterialEpochId, LiveShaderPair,
    LiveShaderStage, LiveTechniqueBundleKey, MethodType,

};

/// Pass-state fields the host cannot reproduce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnhandledStateFields {
    pub odd_blend_factor: bool,
    pub odd_blend_op: bool,
    pub stencil: bool,
}

impl UnhandledStateFields {
    pub fn any(self) -> bool {
        self.stencil || self.odd_blend_factor || self.odd_blend_op
    }
}

impl VisualPassStateBits {
    /// `None` when every blend field of this pass maps onto the host.
    pub fn unsupported_host_fields(self) -> Option<UnhandledStateFields> {
        use d3d9_state::BlendWeight;

        let colour_op = ((self.dword0 >> 8) & 0x7) as u8;
        let alpha_word = (self.dword0 >> 16) & 0x7ff;
        let alpha_op = (alpha_word >> 8) & 0x7;

        let colour_known = BlendWeight::from_raw(self.dword0 & 0xf).is_listed()
            && BlendWeight::from_raw((self.dword0 >> 4) & 0xf).is_listed();
        let alpha_known = BlendWeight::from_raw(alpha_word & 0xf).is_listed()
            && BlendWeight::from_raw((alpha_word >> 4) & 0xf).is_listed();

        let unhandled = UnhandledStateFields {
            odd_blend_factor: colour_op != 0
                && (!colour_known || (alpha_op != 0 && !alpha_known)),
            odd_blend_op: colour_op > 5 || (colour_op != 0 && alpha_op > 5),
            stencil: false,
        };
        unhandled.any().then_some(unhandled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialExecutionDenial {
    OrderedMaterialTableAbsent,
    OrderedMaterialRankBeyondTable {
        sorted_pos: u32,
        table_size: usize,
    },
    OrderedMaterialConstructionFailed {
        cause: LedgerBuildError,
    },
    ObsoleteMaterialRevision {
        retained: MaterialEpochId,
        current: MaterialEpochId,
    },
    SurfaceSlotBeyondTable {
        material: MaterialAssetKey,
    },
    OwnRecipeSetBeyondTable {
        set: LiveTechniqueBundleKey,
    },
    SubstitutedRecipeSetBeyondTable {
        set: LiveTechniqueBundleKey,
    },
    SubstitutionTargetAbsent {
        address_fingerprint: u32,
    },
    SubstitutionLoop {
        first_set: LiveTechniqueBundleKey,
    },
    RenderTechniqueNotFound {
        shading_technique_kind: MethodType,
    },
    RecipeHasNoPasses {
        shading_technique_kind: MethodType,
    },
    RenderStateRecordsAbsent,
    RenderStateRecordBeyondBounds {
        shading_technique_kind: MethodType,
    },
    RenderStateRowBeyondBounds {
        row: usize,
        technique_pass_ordinal: u8,
    },
    UnhandledRenderState {
        technique_pass_ordinal: u8,
        fields: UnhandledStateFields,
        dword0: u32,
        dword1: u32,
    },
    PassNumberTooLarge {
        technique_pass_ordinal: usize,
    },
    ShaderProgramAbsent {
        technique_pass_ordinal: u8,
    },
    UnhandledShaderCombination {
        technique_pass_ordinal: u8,
        pair: LiveShaderPair,
    },
    TextureUnavailable {
        technique_pass_ordinal: u8,
        label_digest: u32,
    },
    SurfaceConstantUnavailable {
        technique_pass_ordinal: u8,
        label_digest: u32,
    },
    EngineConstantUnavailable {
        technique_pass_ordinal: u8,
        stage: LiveShaderStage,
        index: u16,
    },
    EngineConstantRowsBeyondTable {
        technique_pass_ordinal: u8,
        stage: LiveShaderStage,
        index: u16,
        row_base: u8,
        line_total: u8,
        rows_remaining: usize,
    },
    EngineTextureUnavailable {
        technique_pass_ordinal: u8,
        index: u32,
    },
    InlineConstantUnavailable {
        technique_pass_ordinal: u8,
    },
    UnrecognisedArgumentKind {
        technique_pass_ordinal: u8,
        kind_code: u16,
    },
    PassArgumentTotalWrong {
        technique_pass_ordinal: u8,
        each_triangle_batch_inputs: u8,
        each_mesh_inputs: u8,
        stable: u8,
        arg_total: usize,
    },
    SetNamespaceDiffers {
        want: AssetCategoryScope,
        got: AssetCategoryScope,
    },
}
