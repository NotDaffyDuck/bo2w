//! Techniques and technique sets as the renderer holds them at runtime.

use asset_core::AssetNamespace;

use super::ids::{RuntimeShaderPair, RuntimeTechniqueSetId, TECHNIQUE_SLOT_COUNT};
use super::table::CatalogBuildError;
use crate::{PassColorSpace, RuntimeArgumentBinding, TechType};

/// Where a material's technique set comes from after remapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemapResolution {
    SelfSet,
    Resolved(RuntimeTechniqueSetId),
    Missing { pointer_identity: u32 },
    Cycle { first_set: RuntimeTechniqueSetId },
}

/// One draw pass. `arguments` is laid out as the per-primitive, per-object
/// and stable runs, in that order, with the counts saying where each ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePass {
    pub shader_pair: Option<RuntimeShaderPair>,
    pub custom_sampler_flags: u8,
    pub t5_custom_sampler_flags: u8,
    pub per_prim_arg_count: u8,
    pub per_obj_arg_count: u8,
    pub stable_arg_count: u8,
    pub arguments: Vec<RuntimeArgumentBinding>,
    pub color_space: PassColorSpace,
}

/// Orders each run of a pass's arguments by sort key. A pass whose counts do
/// not cover its argument list is left alone.
pub fn sort_pass_args(pass: &mut RuntimePass) {
    let runs = [
        usize::from(pass.per_prim_arg_count),
        usize::from(pass.per_obj_arg_count),
        usize::from(pass.stable_arg_count),
    ];
    if runs.iter().sum::<usize>() != pass.arguments.len() {
        return;
    }
    let mut rest = pass.arguments.as_mut_slice();
    for len in runs {
        let (run, tail) = rest.split_at_mut(len);
        run.sort_by_key(|argument| argument.runtime_sort_key());
        rest = tail;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTechnique {
    pub flags: u16,
    pub passes: Vec<RuntimePass>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTechniqueSet {
    slots: Vec<Option<RuntimeTechnique>>,
    pub world_vert_format: u8,
    pub namespace: AssetNamespace,
    pub name: String,
}

impl RuntimeTechniqueSet {
    pub fn new(slots: Vec<Option<RuntimeTechnique>>) -> Result<Self, CatalogBuildError> {
        if slots.len() == TECHNIQUE_SLOT_COUNT {
            return Ok(Self {
                slots,
                world_vert_format: 0,
                namespace: AssetNamespace::T6,
                name: String::new(),
            });
        }
        Err(CatalogBuildError::TechniqueSlotCount {
            expected: TECHNIQUE_SLOT_COUNT,
            actual: slots.len(),
        })
    }

    pub fn technique(&self, tech_type: TechType) -> Option<&RuntimeTechnique> {
        self.slots.get(usize::from(tech_type.0))?.as_ref()
    }

    pub fn with_identity(
        self,
        world_vert_format: u8,
        namespace: AssetNamespace,
        name: impl Into<String>,
    ) -> Self {
        Self {
            world_vert_format,
            namespace,
            name: name.into(),
            ..self
        }
    }
}
