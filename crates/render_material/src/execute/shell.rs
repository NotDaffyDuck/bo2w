//! The pass list a draw reuses between frames (the shell) and the per-draw
//! executable bound on top of it.

use std::sync::{Arc, OnceLock};

use super::build::{check_pass_state, fill_pass_code};
use super::lanes::{
    PackedCodeConstantLane, PackedCodeConstants, PackedCodeSamplerLane, PackedCodeSamplers,
};
use super::refusal::MaterialRefusal;
use super::resolve::{MaterialDrawKey, resolve_material_technique};
use crate::prepared::{
    GfxPassStateBits, PackedLocalBanks, PackedLocalSamplers, PreparedArgBelts,
    PreparedMaterialTable,
};
use crate::{CodeSourceLookup, PortId, RuntimeMaterialCatalog, RuntimeShaderPair, SetupArm, TechType};

/// What a pass keeps between draws: everything except the code lanes, which
/// are rebuilt from the live code sources.
#[derive(Clone, Debug)]
pub struct StablePassShell {
    pub pass_index: u8,
    pub port: PortId,
    pub shader_pair: RuntimeShaderPair,
    pub state: GfxPassStateBits,
    pub local_banks: Option<Arc<PackedLocalBanks>>,
    pub local_samplers: Option<Arc<PackedLocalSamplers>>,
    pub belts: Arc<PreparedArgBelts>,
}

#[derive(Clone, Debug)]
pub struct StableMaterialShell {
    pub tech_type: TechType,
    pub arm: SetupArm,
    pub vertex_type: u8,
    pub passes: Vec<StablePassShell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutablePass {
    pub pass_index: u8,
    pub port: PortId,
    pub shader_pair: RuntimeShaderPair,
    pub state: GfxPassStateBits,
    pub local_banks: Option<Arc<PackedLocalBanks>>,
    pub local_samplers: Option<Arc<PackedLocalSamplers>>,
    pub code_constants: PackedCodeConstants,
    pub code_samplers: PackedCodeSamplers,
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutablePassView<'a> {
    pub pass_index: u8,
    pub port: PortId,
    pub shader_pair: RuntimeShaderPair,
    pub state: GfxPassStateBits,
    pub local_banks: Option<&'a Arc<PackedLocalBanks>>,
    pub local_samplers: Option<&'a Arc<PackedLocalSamplers>>,
    pub code_constants: &'a [PackedCodeConstantLane],
    pub code_samplers: &'a [PackedCodeSamplerLane],
    pub code_constant_id: u64,
    pub code_sampler_id: u64,
}

impl ExecutablePass {
    pub fn view(&self) -> ExecutablePassView<'_> {
        ExecutablePassView {
            pass_index: self.pass_index,
            port: self.port,
            shader_pair: self.shader_pair,
            state: self.state,
            local_banks: self.local_banks.as_ref(),
            local_samplers: self.local_samplers.as_ref(),
            code_constants: &self.code_constants.lanes,
            code_samplers: &self.code_samplers.lanes,
            code_constant_id: self.code_constants.id,
            code_sampler_id: self.code_samplers.id,
        }
    }
}

impl StablePassShell {
    pub fn bind<'a>(
        &'a self,
        constants: &'a PackedCodeConstants,
        samplers: &'a PackedCodeSamplers,
    ) -> ExecutablePassView<'a> {
        ExecutablePassView {
            pass_index: self.pass_index,
            port: self.port,
            shader_pair: self.shader_pair,
            state: self.state,
            local_banks: self.local_banks.as_ref(),
            local_samplers: self.local_samplers.as_ref(),
            code_constants: &constants.lanes,
            code_samplers: &samplers.lanes,
            code_constant_id: constants.id,
            code_sampler_id: samplers.id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaterialExecution {
    pub tech_type: TechType,
    pub arm: SetupArm,
    pub vertex_type: u8,
    shell: Arc<StableMaterialShell>,
    code_constants: Vec<PackedCodeConstants>,
    code_samplers: Vec<PackedCodeSamplers>,
}

impl MaterialExecution {
    pub(super) fn from_parts(
        shell: StableMaterialShell,
        code_constants: Vec<PackedCodeConstants>,
        code_samplers: Vec<PackedCodeSamplers>,
    ) -> Self {
        Self {
            tech_type: shell.tech_type,
            arm: shell.arm,
            vertex_type: shell.vertex_type,
            shell: Arc::new(shell),
            code_constants,
            code_samplers,
        }
    }

    /// An execution with no passes, for slots that are filled in later.
    pub fn vacant() -> Self {
        static EMPTY: OnceLock<Arc<StableMaterialShell>> = OnceLock::new();
        let shell = EMPTY.get_or_init(|| {
            Arc::new(StableMaterialShell {
                tech_type: TechType(0),
                arm: SetupArm::Generic,
                vertex_type: 0,
                passes: Vec::new(),
            })
        });
        Self {
            tech_type: TechType(0),
            arm: SetupArm::Generic,
            vertex_type: 0,
            shell: Arc::clone(shell),
            code_constants: Vec::new(),
            code_samplers: Vec::new(),
        }
    }

    pub fn pass_count(&self) -> usize {
        self.shell.passes.len()
    }

    /// Drops the lane rows but keeps the pass list, so a later rebind only
    /// refills them.
    pub fn release_code_rows(&mut self) {
        self.code_constants
            .iter_mut()
            .for_each(|constants| constants.lanes.clear());
        self.code_samplers
            .iter_mut()
            .for_each(|samplers| samplers.lanes.clear());
    }

    pub fn shell_arc(&self) -> &Arc<StableMaterialShell> {
        &self.shell
    }

    pub fn code_constants(&self) -> &[PackedCodeConstants] {
        &self.code_constants
    }

    pub fn pass(&self, index: usize) -> Option<ExecutablePassView<'_>> {
        Some(self.shell.passes.get(index)?.bind(
            self.code_constants.get(index)?,
            self.code_samplers.get(index)?,
        ))
    }

    pub fn iter_passes(&self) -> impl Iterator<Item = ExecutablePassView<'_>> + '_ {
        (0..self.pass_count()).filter_map(|index| self.pass(index))
    }

    /// Points this execution at `shell` and refills every pass's code lanes
    /// from `code_sources`, reusing the lane buffers it already owns.
    pub fn rebind_into(
        &mut self,
        shell: &Arc<StableMaterialShell>,
        code_sources: &impl CodeSourceLookup,
    ) -> Result<(), MaterialRefusal> {
        self.tech_type = shell.tech_type;
        self.arm = shell.arm;
        self.vertex_type = shell.vertex_type;
        if !Arc::ptr_eq(&self.shell, shell) {
            self.shell = Arc::clone(shell);
        }
        let wanted = shell.passes.len();
        self.code_constants.resize_with(wanted, Default::default);
        self.code_samplers.resize_with(wanted, Default::default);
        let lanes = self.code_constants.iter_mut().zip(&mut self.code_samplers);
        for (pass, (constants, samplers)) in shell.passes.iter().zip(lanes) {
            check_pass_state(pass.pass_index, pass.state)?;
            fill_pass_code(&pass.belts, code_sources, pass.pass_index, constants, samplers)?;
        }
        Ok(())
    }
}

/// Copies out the shell behind `execution` if it still matches what the
/// catalog and prepared table would build for this draw.
pub fn capture_stable_shell(
    catalog: &RuntimeMaterialCatalog,
    prepared: &PreparedMaterialTable,
    key: MaterialDrawKey,
    tech_type: TechType,
    _vertex_type: u8,
    execution: &MaterialExecution,
) -> Option<StableMaterialShell> {
    let (material, technique) = resolve_material_technique(catalog, key, tech_type).ok()?;
    let prepared_tech = prepared.technique(material.asset_id, tech_type)?;
    let passes = execution.pass_count();
    (passes == technique.passes.len() && passes == prepared_tech.passes.len())
        .then(|| (**execution.shell_arc()).clone())
}

pub fn rebind_stable_material(
    shell: &StableMaterialShell,
    code_sources: &impl CodeSourceLookup,
) -> Result<MaterialExecution, MaterialRefusal> {
    let mut execution = MaterialExecution::vacant();
    execution.rebind_into(&Arc::new(shell.clone()), code_sources)?;
    Ok(execution)
}
