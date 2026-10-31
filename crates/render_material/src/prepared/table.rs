//! Every material's passes, prepared once at load: for each vertex type the
//! shader port that fits it and the constant banks packed for it, plus the
//! state words, argument belts and sampler layout the pass shares.

use std::sync::Arc;

use asset_core::AssetCategoryScope;

use super::args::{PackedCodeParam, PreparedParamStrips};
use super::banks::LocalSamplerBanks;
use super::banks::LocalSamplerBundle;
use super::banks::encode_local_register_banks;
use super::banks::pack_local_samplers;
use crate::MethodType;
use crate::catalog::{
    MaterialAssetKey, ShaderPortKey, MaterialRemapOutcome, LiveMaterial, LiveMaterialLedger,
    LivePassRecord, LiveShaderPair, LiveShadingTechnique, RENDER_RECIPE_SLOT_TOTAL,
};
use crate::decl_table::VERTEX_LAYOUT_KIND_TOTAL;

/// The two state words of a pass and the namespace they were authored in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualPassStateBits {
    pub namespace: AssetCategoryScope,
    pub dword0: u32,
    pub dword1: u32,
}

/// What the port admitter reports for a pass compiled against one vertex type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptedPortSummary {
    pub id: ShaderPortKey,
    pub vertex_rows_used: usize,
    pub pixel_rows_used: usize,
}

/// A pass as compiled for one vertex type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexVariant {
    pub port: ShaderPortKey,
    /// Present when the pass has argument belts to pack.
    pub banks: Option<Arc<LocalSamplerBanks>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDrawPass {
    pub stage_program_duo: Option<LiveShaderPair>,
    pub state: VisualPassStateBits,
    pub belts: Option<PreparedParamStrips>,
    pub local_samplers: Option<Arc<LocalSamplerBundle>>,
    variants: [Option<VertexVariant>; VERTEX_LAYOUT_KIND_TOTAL],
}

impl ResolvedDrawPass {
    pub fn variant(&self, vertex_format_code: u8) -> Option<&VertexVariant> {
        self.variants.get(usize::from(vertex_format_code))?.as_ref()
    }

    pub fn port(&self, vertex_format_code: u8) -> Option<ShaderPortKey> {
        self.variant(vertex_format_code).map(|variant| variant.port)
    }

    pub fn local_banks(&self, vertex_format_code: u8) -> Option<&Arc<LocalSamplerBanks>> {
        self.variant(vertex_format_code)?.banks.as_ref()
    }

    fn admitted(&self) -> impl Iterator<Item = &VertexVariant> {
        self.variants.iter().flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedShadingRecipe {
    pub flags: u16,
    pub passes: Vec<ResolvedDrawPass>,
    /// Bit `n` is set when some pass binds code texture `n` (for `n < 64`).
    pub engine_sampler_bitmask: u64,
}

/// All code arguments of a technique's passes, in bank order.
fn code_args(passes: &[ResolvedDrawPass]) -> impl Iterator<Item = &PackedCodeParam> {
    passes
        .iter()
        .filter_map(|pass| pass.belts.as_ref())
        .flat_map(|belts| belts.in_bank_order())
        .flat_map(|slice| &slice.code)
}

impl ResolvedShadingRecipe {
    pub fn uses_engine_texture(&self, tex_no: u32) -> bool {
        match tex_no < 64 {
            true => self.engine_sampler_bitmask >> tex_no & 1 != 0,
            false => code_args(&self.passes)
                .any(|arg| matches!(arg, PackedCodeParam::Sampler { index: bound, .. } if *bound == tex_no)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedMaterial {
    pub techniques: Vec<Option<ResolvedShadingRecipe>>,
}

impl StagedMaterial {
    fn unprepared() -> Self {
        Self {
            techniques: vec![None; RENDER_RECIPE_SLOT_TOTAL],
        }
    }

    fn passes(&self) -> impl Iterator<Item = &ResolvedDrawPass> {
        self.techniques
            .iter()
            .flatten()
            .flat_map(|technique| &technique.passes)
    }
}

/// How many of each kind of prepared item the table holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedTableTally {
    pub port_slot_total: usize,
    pub local_bank_total: usize,
    pub local_sampler_total: usize,
}

impl ResolvedTableTally {
    fn count(materials: &[StagedMaterial]) -> Self {
        let mut census = Self::default();
        for pass in materials.iter().flat_map(StagedMaterial::passes) {
            census.port_slot_total += pass.admitted().count();
            census.local_bank_total += pass.admitted().filter(|v| v.banks.is_some()).count();
            census.local_sampler_total += usize::from(pass.local_samplers.is_some());
        }
        census
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StagedMaterialTable {
    prepared: Vec<StagedMaterial>,
    census: ResolvedTableTally,
}

impl StagedMaterialTable {
    pub fn build_from_index(
        catalog: &LiveMaterialLedger,
        admit: impl FnMut(&LivePassRecord, u8) -> Option<AcceptedPortSummary>,
    ) -> Self {
        let mut preparer = Preparer { catalog, admit };
        let sources = &catalog.materials;
        let prepared_rows: Vec<_> = sources
            .iter()
            .map(|material| preparer.material(material))
            .collect();
        let census = ResolvedTableTally::count(&prepared_rows);
        Self { prepared: prepared_rows, census }
    }

    pub fn staged_for(&self, id: MaterialAssetKey) -> Option<&StagedMaterial> {
        self.prepared.get(usize::from(id.0))
    }

    pub fn recipe_for(&self, id: MaterialAssetKey, shading_technique_kind: MethodType) -> Option<&ResolvedShadingRecipe> {
        let entry = self.staged_for(id)?;
        entry.techniques.get(usize::from(shading_technique_kind.0))?.as_ref()
    }

    pub fn pass_port(
        &self,
        id: MaterialAssetKey,
        shading_technique_kind: MethodType,
        technique_pass_ordinal: u8,
        vertex_format_code: u8,
    ) -> Option<ShaderPortKey> {
        let technique = self.recipe_for(id, shading_technique_kind)?;
        technique.passes.get(usize::from(technique_pass_ordinal))?.port(vertex_format_code)
    }

    /// Every prepared pass of technique `tech`, tagged with its material.
    pub fn stages_for_technique(
        &self,
        tech: MethodType,
    ) -> impl Iterator<Item = (MaterialAssetKey, &ResolvedDrawPass)> {
        let slot = usize::from(tech.0);
        (0..).map(|index| MaterialAssetKey(index as u16))
            .zip(&self.prepared)
            .filter_map(move |(id, material)| Some((id, material.techniques.get(slot)?.as_ref()?)))
            .flat_map(|pair| {
                let (id, recipe) = pair;
                recipe.passes.iter().map(move |pass| (id, pass))
            })
    }

    pub fn len(&self) -> usize {
        self.prepared.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prepared.is_empty()
    }

    pub fn port_slot_total(&self) -> usize {
        self.census.port_slot_total
    }

    pub fn local_bank_total(&self) -> usize {
        self.census.local_bank_total
    }

    pub fn local_sampler_total(&self) -> usize {
        self.census.local_sampler_total
    }
}

/// Walks the catalog once, asking `admit` for the port of every pass and
/// vertex type.
struct Preparer<'a, A> {
    catalog: &'a LiveMaterialLedger,
    admit: A,
}

impl<A: FnMut(&LivePassRecord, u8) -> Option<AcceptedPortSummary>> Preparer<'_, A> {
    fn material(&mut self, material: &LiveMaterial) -> StagedMaterial {
        let technique_set_ident = match material.remap {
            MaterialRemapOutcome::SelfSet => material.local_technique_set,
            MaterialRemapOutcome::Resolved(set) => set,
            MaterialRemapOutcome::Missing { .. } | MaterialRemapOutcome::Cycle { .. } => {
                return StagedMaterial::unprepared();
            }
        };
        let mut prepared = StagedMaterial::unprepared();
        let (Some(set), Some(rows)) = (
            self.catalog.technique_sets.get(technique_set_ident.0 as usize),
            Option::as_ref(&material.state_row_for),
        ) else {
            return prepared;
        };
        for (slot, place) in prepared.techniques.iter_mut().enumerate() {
            let base_row = rows.get(slot).copied().unwrap_or(0xff);
            let shading_technique_kind = MethodType(u8::try_from(slot).unwrap_or(u8::MAX));
            if let Some(technique) = set.recipe_for(shading_technique_kind)
                && !technique.passes.is_empty()
                && base_row != 0xff
            {
                *place = Some(self.technique(material, technique, base_row));
            }
        }
        prepared
    }

    fn technique(
        &mut self,
        material: &LiveMaterial,
        recipe_src: &LiveShadingTechnique,
        base_row: u8,
    ) -> ResolvedShadingRecipe {
        let passes: Vec<_> = recipe_src
            .passes
            .iter()
            .zip(usize::from(base_row)..)
            .map(|(pass, row)| {
                let words = material.state_rows.get(row).copied();
                self.pass(material, pass, words)
            })
            .collect();
        let engine_sampler_bitmask = code_args(&passes).fold(0, |mask, arg| match *arg {
            PackedCodeParam::Sampler { index, .. } if index < 64 => mask | 1 << index,
            _ => mask,
        });
        ResolvedShadingRecipe {
            flags: recipe_src.flags,
            engine_sampler_bitmask,
            passes,
        }
    }

    fn pass(
        &mut self,
        mat: &LiveMaterial,
        pass: &LivePassRecord,
        state_words: Option<[u32; 2]>,
    ) -> ResolvedDrawPass {
        let belts = PreparedParamStrips::from_pass(mat, pass);
        let mut variants = [const { None }; VERTEX_LAYOUT_KIND_TOTAL];
        if pass.stage_program_duo.is_some() && state_words.is_some() {
            for (vertex_format_code, place) in (0..).zip(&mut variants) {
                let Some(facts) = (self.admit)(pass, vertex_format_code) else {
                    continue;
                };
                let banks = belts.as_ref().and_then(|belts| {
                    encode_local_register_banks(facts.vertex_rows_used, facts.pixel_rows_used, belts)
                });
                *place = Some(VertexVariant {
                    port: facts.id,
                    banks: banks.map(Arc::new),
                });
            }
        }
        let [dword0, dword1] = state_words.unwrap_or_default();
        ResolvedDrawPass {
            stage_program_duo: pass.stage_program_duo,
            state: VisualPassStateBits {
                namespace: mat.namespace,
                dword0,
                dword1,
            },
            local_samplers: belts.as_ref().map(|belts| Arc::new(pack_local_samplers(belts))),
            belts,
            variants,
        }
    }
}
