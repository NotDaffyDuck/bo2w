//! Techniques and technique sets as the renderer holds them at runtime.

use asset_core::AssetCategoryScope;

use super::ids::{LiveShaderPair, LiveTechniqueBundleKey, RENDER_RECIPE_SLOT_TOTAL};
use super::table::LedgerBuildError;
use crate::{PassColorEncoding, LiveArgumentBinding, MethodType};

/// Where a material's technique set comes from after remapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialRemapOutcome {
    SelfSet,
    Resolved(LiveTechniqueBundleKey),
    Missing { address_fingerprint: u32 },
    Cycle { first_set: LiveTechniqueBundleKey },
}

/// One draw pass. `arguments` is laid out as the per-primitive, per-object
/// and stable runs (in that order); the counts mark where each run ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivePassRecord {
    pub stage_program_duo: Option<LiveShaderPair>,
    pub sampler_custom_bits: u8,
    pub classic_custom_sampler_flags: u8,
    pub per_prim_arg_tally: u8,
    pub per_obj_arg_tally: u8,
    pub stable_arg_tally: u8,
    pub arguments: Vec<LiveArgumentBinding>,
    pub color_space: PassColorEncoding,
}

/// Orders each run of a pass's arguments by sort key. A pass whose counts do
/// not cover its argument list is left alone.
pub fn sort_pass_args(pass: &mut LivePassRecord) {
    let runs = [
        usize::from(pass.per_prim_arg_tally),
        usize::from(pass.per_obj_arg_tally),
        usize::from(pass.stable_arg_tally),
    ];
    if runs.iter().sum::<usize>() != pass.arguments.len() {
        return;
    }
    let mut rest = pass.arguments.as_mut_slice();
    for len in runs {
        let (run, tail) = rest.split_at_mut(len);
        run.sort_by_key(|argument| argument.draw_order_pair());
        rest = tail;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveShadingTechnique {
    pub flags: u16,
    pub passes: Vec<LivePassRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTechniqueBundle {
    slots: Vec<Option<LiveShadingTechnique>>,
    pub level_format: u8,
    pub namespace: AssetCategoryScope,
    pub name: String,
}

impl LiveTechniqueBundle {
    pub fn new(slots: Vec<Option<LiveShadingTechnique>>) -> Result<Self, LedgerBuildError> {
        if slots.len() == RENDER_RECIPE_SLOT_TOTAL {
            return Ok(Self {
                slots,
                level_format: 0,
                namespace: AssetCategoryScope::T6,
                name: String::new(),
            });
        }
        Err(LedgerBuildError::RenderTechniqueSlotTotal {
            want_slots: RENDER_RECIPE_SLOT_TOTAL,
            got_slots: Vec::len(&slots),
        })
    }

    pub fn recipe_for(&self, shading_technique_kind: MethodType) -> Option<&LiveShadingTechnique> {
        self.slots.get(usize::from(shading_technique_kind.0))?.as_ref()
    }

    pub fn with_identity<N: Into<String>>(
        self,
        level_format: u8,
        namespace: AssetCategoryScope,
        name: N,
    ) -> Self {
        Self {
            level_format,
            namespace,
            name: name.into(),
            ..self
        }
    }
}
