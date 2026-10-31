//! A material's reusable plan and the per-draw lane sets bound onto it.
//!
//! The plan holds everything about a technique's passes that does not depend
//! on the frame (ports, states, local banks, argument belts). The code lanes
//! do depend on the live code sources, so each execution owns its own and
//! refills them in place when it is pointed at a plan again.

use std::sync::{Arc, OnceLock};

use super::build::{check_pass_state, fill_pass_code};
use super::lanes::{CodeConstantBundle, CompactEngineSamplers};
use super::refusal::MaterialExecutionDenial;
use super::resolve::{SurfaceSortKeyPair, select_material_technique};
use crate::prepared::{
    VisualPassStateBits, LocalSamplerBanks, LocalSamplerBundle, PreparedParamStrips,
    StagedMaterialTable,
};
use crate::{EngineValueSource, ShaderPortKey, LiveMaterialLedger, LiveShaderPair, SetupVariantBranch, MethodType};

/// One pass of a plan.
#[derive(Clone, Debug)]
pub struct PassPlan {
    pub technique_pass_ordinal: u8,
    pub port: ShaderPortKey,
    pub stage_program_duo: LiveShaderPair,
    pub state: VisualPassStateBits,
    pub local_banks: Option<Arc<LocalSamplerBanks>>,
    pub local_samplers: Option<Arc<LocalSamplerBundle>>,
    pub belts: Arc<PreparedParamStrips>,
}

/// The frame-independent part of a material execution.
#[derive(Clone, Debug)]
pub struct MaterialPlan {
    pub shading_technique_kind: MethodType,
    pub arm: SetupVariantBranch,
    pub vertex_format_code: u8,
    pub passes: Vec<PassPlan>,
}

impl MaterialPlan {
    fn empty() -> Self {
        Self {
            shading_technique_kind: MethodType(0),
            arm: SetupVariantBranch::Generic,
            vertex_format_code: 0,
            passes: Vec::default(),
        }
    }
}

/// The code constants and samplers one pass reads from the live sources.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeLanes {
    pub constants: CodeConstantBundle,
    pub samplers: CompactEngineSamplers,
}

impl CodeLanes {
    fn drop_rows(&mut self) {
        self.constants.lanes.clear();
        self.samplers.lanes.clear();
    }
}

/// A pass of a plan together with the lanes bound to it.
#[derive(Clone, Copy, Debug)]
pub struct BoundPass<'a> {
    pub plan: &'a PassPlan,
    pub lanes: &'a CodeLanes,
}

impl<'a> BoundPass<'a> {
    pub fn technique_pass_ordinal(&self) -> u8 {
        self.plan.technique_pass_ordinal
    }

    pub fn port(&self) -> ShaderPortKey {
        self.plan.port
    }

    pub fn stage_program_duo(&self) -> LiveShaderPair {
        self.plan.stage_program_duo
    }

    pub fn state(&self) -> VisualPassStateBits {
        self.plan.state
    }

    pub fn local_banks(&self) -> Option<&'a Arc<LocalSamplerBanks>> {
        self.plan.local_banks.as_ref()
    }

    pub fn local_samplers(&self) -> Option<&'a Arc<LocalSamplerBundle>> {
        self.plan.local_samplers.as_ref()
    }

    pub fn constants(&self) -> &'a CodeConstantBundle {
        &self.lanes.constants
    }

    pub fn samplers(&self) -> &'a CompactEngineSamplers {
        &self.lanes.samplers
    }
}

#[derive(Clone, Debug)]
pub struct MaterialRunPlan {
    plan: Arc<MaterialPlan>,
    lanes: Vec<CodeLanes>,
}

impl MaterialRunPlan {
    pub(super) fn from_parts(plan: MaterialPlan, lanes: Vec<CodeLanes>) -> Self {
        Self {
            plan: Arc::new(plan),
            lanes,
        }
    }

    /// An execution that has no passes yet, for slots filled in later.
    pub fn blank() -> Self {
        static NOTHING: OnceLock<Arc<MaterialPlan>> = OnceLock::new();
        Self {
            plan: Arc::clone(NOTHING.get_or_init(|| Arc::new(MaterialPlan::empty()))),
            lanes: Vec::default(),
        }
    }

    pub fn shading_technique_kind(&self) -> MethodType {
        self.plan.shading_technique_kind
    }

    pub fn arm(&self) -> SetupVariantBranch {
        self.plan.arm
    }

    pub fn vertex_format_code(&self) -> u8 {
        self.plan.vertex_format_code
    }

    pub fn stage_total(&self) -> usize {
        self.plan.passes.len()
    }

    pub fn plan(&self) -> &Arc<MaterialPlan> {
        &self.plan
    }

    pub fn lanes(&self) -> &[CodeLanes] {
        &self.lanes
    }

    /// Empties every lane set but keeps the passes, so pointing the execution
    /// at a plan again only has to refill them.
    pub fn release_code_rows(&mut self) {
        self.lanes.iter_mut().for_each(CodeLanes::drop_rows);
    }

    pub fn pass_at(&self, index: usize) -> Option<BoundPass<'_>> {
        Some(BoundPass {
            plan: self.plan.passes.get(index)?,
            lanes: self.lanes.get(index)?,
        })
    }

    pub fn iter_passes(&self) -> impl Iterator<Item = BoundPass<'_>> {
        self.plan
            .passes
            .iter()
            .zip(&self.lanes)
            .map(|(plan, lanes)| BoundPass { plan, lanes })
    }

    /// Adopts `plan` and refills each pass's lanes from `engine_sources`,
    /// keeping the lane buffers this execution already has.
    pub fn reattach_onto(
        &mut self,
        plan: &Arc<MaterialPlan>,
        engine_sources: &impl EngineValueSource,
    ) -> Result<(), MaterialExecutionDenial> {
        if !Arc::ptr_eq(&self.plan, plan) {
            self.plan = Arc::clone(plan);
        }
        self.lanes.resize_with(plan.passes.len(), CodeLanes::default);
        for (pass, lanes) in plan.passes.iter().zip(&mut self.lanes) {
            check_pass_state(pass.technique_pass_ordinal, pass.state)?;
            fill_pass_code(&pass.belts, engine_sources, pass.technique_pass_ordinal, lanes)?;
        }
        Ok(())
    }
}

/// The plan behind `execution`, copied out, provided the catalog and the
/// prepared table still describe the same number of passes for this draw.
pub fn capture_plan(
    catalog: &LiveMaterialLedger,
    prepared: &StagedMaterialTable,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
    execution: &MaterialRunPlan,
) -> Option<MaterialPlan> {
    let (picked, technique) = select_material_technique(catalog, key, shading_technique_kind).ok()?;
    let prepared_tech = prepared.recipe_for(picked.asset_id, shading_technique_kind)?;
    let wanted = execution.stage_total();
    let agrees = wanted == technique.passes.len() && wanted == prepared_tech.passes.len();
    agrees.then(|| MaterialPlan::clone(execution.plan()))
}

/// A fresh execution for `plan`, with its lanes filled from `engine_sources`.
pub fn execute_plan(
    plan: &MaterialPlan,
    engine_sources: &impl EngineValueSource,
) -> Result<MaterialRunPlan, MaterialExecutionDenial> {
    let mut run = MaterialRunPlan::blank();
    run.reattach_onto(&Arc::new(plan.clone()), engine_sources)?;
    Ok(run)
}
