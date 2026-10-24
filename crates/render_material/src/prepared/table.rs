//! The table of every material's passes, prepared once at load: shader
//! ports per vertex type, state bits, argument belts and packed local data.

use std::sync::Arc;

use asset_core::AssetNamespace;

use super::args::{PackedCodeArg, PreparedArgBelts};
use super::banks::{PackedLocalBanks, PackedLocalSamplers, pack_local_banks, pack_local_samplers};
use crate::catalog::{
    MaterialAssetId, PortId, RemapResolution, RuntimeMaterial, RuntimeMaterialCatalog,
    RuntimePass, RuntimeShaderPair, RuntimeTechnique, TECHNIQUE_SLOT_COUNT,
};
use crate::TechType;
use crate::decl_table::VERTEX_TYPE_COUNT;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxPassStateBits {
    pub namespace: AssetNamespace,
    pub word0: u32,
    pub word1: u32,
}

/// What the port admitter reports for a pass compiled against one vertex type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedPortFacts {
    pub id: PortId,
    pub vertex_constant_len: usize,
    pub pixel_constant_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPass {
    pub port: [Option<PortId>; VERTEX_TYPE_COUNT],
    pub shader_pair: Option<RuntimeShaderPair>,
    pub state: GfxPassStateBits,
    pub per_prim_arg_count: u8,
    pub per_obj_arg_count: u8,
    pub stable_arg_count: u8,
    pub belts: Option<PreparedArgBelts>,
    pub local_banks: [Option<Arc<PackedLocalBanks>>; VERTEX_TYPE_COUNT],
    pub local_samplers: Option<Arc<PackedLocalSamplers>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTechnique {
    pub flags: u16,
    pub passes: Vec<PreparedPass>,
    /// Bit `n` is set when some pass binds code texture `n` (for `n < 64`).
    pub code_sampler_mask: u64,
}

impl PreparedTechnique {
    pub fn binds_code_texture(&self, index: u32) -> bool {
        if index < 64 {
            return self.code_sampler_mask & (1 << index) != 0;
        }
        self.passes
            .iter()
            .filter_map(|pass| pass.belts.as_ref())
            .flat_map(|belts| belts.in_bank_order())
            .flat_map(|slice| &slice.code)
            .any(|arg| matches!(arg, PackedCodeArg::Sampler { index: bound, .. } if *bound == index))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMaterial {
    pub tech: Vec<Option<PreparedTechnique>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparedTableCensus {
    pub port_slot_n: usize,
    pub local_bank_n: usize,
    pub local_sampler_n: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedMaterialTable {
    materials: Vec<PreparedMaterial>,
    census: PreparedTableCensus,
}

impl PreparedMaterialTable {
    pub fn from_catalog(
        catalog: &RuntimeMaterialCatalog,
        mut admit: impl FnMut(&RuntimePass, u8) -> Option<AdmittedPortFacts>,
    ) -> Self {
        let materials: Vec<_> = catalog
            .materials
            .iter()
            .map(|material| prepare_material(catalog, &mut admit, material))
            .collect();
        let census = census_of(&materials);
        Self { materials, census }
    }

    pub fn material(&self, id: MaterialAssetId) -> Option<&PreparedMaterial> {
        self.materials.get(usize::from(id.0))
    }

    pub fn technique(&self, id: MaterialAssetId, tech_type: TechType) -> Option<&PreparedTechnique> {
        self.material(id)?
            .tech
            .get(usize::from(tech_type.0))?
            .as_ref()
    }

    pub fn port(
        &self,
        id: MaterialAssetId,
        tech_type: TechType,
        pass_index: u8,
        vertex_type: u8,
    ) -> Option<PortId> {
        let pass = self
            .technique(id, tech_type)?
            .passes
            .get(usize::from(pass_index))?;
        pass.port.get(usize::from(vertex_type)).copied().flatten()
    }

    /// Every prepared pass of technique `tech`, tagged with its material.
    pub fn passes_for_tech(
        &self,
        tech: TechType,
    ) -> impl Iterator<Item = (MaterialAssetId, &PreparedPass)> {
        self.materials
            .iter()
            .enumerate()
            .filter_map(move |(index, material)| {
                let technique = material.tech.get(usize::from(tech.0))?.as_ref()?;
                Some((MaterialAssetId(index as u16), technique))
            })
            .flat_map(|(id, technique)| technique.passes.iter().map(move |pass| (id, pass)))
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    pub fn port_slot_n(&self) -> usize {
        self.census.port_slot_n
    }

    pub fn local_bank_n(&self) -> usize {
        self.census.local_bank_n
    }

    pub fn local_sampler_n(&self) -> usize {
        self.census.local_sampler_n
    }
}

fn census_of(materials: &[PreparedMaterial]) -> PreparedTableCensus {
    let mut census = PreparedTableCensus::default();
    let passes = materials
        .iter()
        .flat_map(|material| material.tech.iter().flatten())
        .flat_map(|technique| &technique.passes);
    for pass in passes {
        census.port_slot_n += pass.port.iter().flatten().count();
        census.local_bank_n += pass.local_banks.iter().flatten().count();
        census.local_sampler_n += usize::from(pass.local_samplers.is_some());
    }
    census
}

fn prepare_material(
    catalog: &RuntimeMaterialCatalog,
    admit: &mut impl FnMut(&RuntimePass, u8) -> Option<AdmittedPortFacts>,
    material: &RuntimeMaterial,
) -> PreparedMaterial {
    let mut tech = vec![None; TECHNIQUE_SLOT_COUNT];
    let set_id = match material.remap {
        RemapResolution::SelfSet => material.local_technique_set,
        RemapResolution::Resolved(set) => set,
        RemapResolution::Missing { .. } | RemapResolution::Cycle { .. } => {
            return PreparedMaterial { tech };
        }
    };
    let (Some(set), Some(rows)) = (
        catalog.technique_sets.get(set_id.0 as usize),
        material.state_bits_entry.as_ref(),
    ) else {
        return PreparedMaterial { tech };
    };
    for (slot, entry) in tech.iter_mut().enumerate() {
        let tech_type = TechType(u8::try_from(slot).unwrap_or(u8::MAX));
        let base_row = rows.get(slot).copied().unwrap_or(0xff);
        let Some(technique) = set.technique(tech_type) else {
            continue;
        };
        if technique.passes.is_empty() || base_row == 0xff {
            continue;
        }
        *entry = Some(prepare_technique(admit, material, technique, base_row));
    }
    PreparedMaterial { tech }
}

fn prepare_technique(
    admit: &mut impl FnMut(&RuntimePass, u8) -> Option<AdmittedPortFacts>,
    material: &RuntimeMaterial,
    technique: &RuntimeTechnique,
    base_row: u8,
) -> PreparedTechnique {
    let passes: Vec<_> = technique
        .passes
        .iter()
        .enumerate()
        .map(|(index, pass)| {
            let row = usize::from(base_row).saturating_add(index);
            prepare_pass(admit, material, pass, material.state_bits_table.get(row).copied())
        })
        .collect();
    PreparedTechnique {
        flags: technique.flags,
        code_sampler_mask: code_sampler_mask(&passes),
        passes,
    }
}

fn prepare_pass(
    admit: &mut impl FnMut(&RuntimePass, u8) -> Option<AdmittedPortFacts>,
    material: &RuntimeMaterial,
    pass: &RuntimePass,
    load_bits: Option<[u32; 2]>,
) -> PreparedPass {
    let mut port = [None; VERTEX_TYPE_COUNT];
    let mut bank_lens = [None; VERTEX_TYPE_COUNT];
    if pass.shader_pair.is_some() && load_bits.is_some() {
        for vertex_type in 0..VERTEX_TYPE_COUNT {
            let wanted = u8::try_from(vertex_type).unwrap_or(u8::MAX);
            if let Some(facts) = admit(pass, wanted) {
                port[vertex_type] = Some(facts.id);
                bank_lens[vertex_type] = Some((facts.vertex_constant_len, facts.pixel_constant_len));
            }
        }
    }
    let belts = PreparedArgBelts::from_pass(material, pass);
    let mut local_banks = [const { None }; VERTEX_TYPE_COUNT];
    if let Some(belts) = &belts {
        for (bank, lens) in local_banks.iter_mut().zip(bank_lens) {
            if let Some((vertex_len, pixel_len)) = lens {
                *bank = pack_local_banks(vertex_len, pixel_len, belts).map(Arc::new);
            }
        }
    }
    let [word0, word1] = load_bits.unwrap_or([0, 0]);
    PreparedPass {
        port,
        shader_pair: pass.shader_pair,
        state: GfxPassStateBits {
            namespace: material.namespace,
            word0,
            word1,
        },
        per_prim_arg_count: pass.per_prim_arg_count,
        per_obj_arg_count: pass.per_obj_arg_count,
        stable_arg_count: pass.stable_arg_count,
        local_samplers: belts
            .as_ref()
            .map(|belts| Arc::new(pack_local_samplers(belts))),
        belts,
        local_banks,
    }
}

fn code_sampler_mask(passes: &[PreparedPass]) -> u64 {
    passes
        .iter()
        .filter_map(|pass| pass.belts.as_ref())
        .flat_map(|belts| belts.in_bank_order())
        .flat_map(|slice| &slice.code)
        .fold(0, |mask, arg| match *arg {
            PackedCodeArg::Sampler { index, .. } if index < 64 => mask | 1 << index,
            _ => mask,
        })
}
