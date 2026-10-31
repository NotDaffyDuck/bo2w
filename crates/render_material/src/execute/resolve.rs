//! Finding the material, technique set and prepared technique for a draw.

use dpvs_t6::SortedDraw;

use super::refusal::MaterialExecutionDenial;
use crate::decl_table;
use crate::catalog::LiveTechniqueBundle;
use crate::prepared::{StagedMaterialTable, ResolvedShadingRecipe};
use crate::{
    MaterialAssetKey, MaterialRemapOutcome, LiveMaterial, LiveMaterialLedger,
    LiveSortedMaterialTable, LiveShadingTechnique, LiveTechniqueBundleKey, MethodType,

};

/// A sorted draw's material: either named directly by asset id or by its
/// ordinal in the sorted table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SurfaceSortKeyPair {
    pub named_asset: Option<MaterialAssetKey>,
    pub packed: u64,
    pub sort_rank: u32,
}

impl SurfaceSortKeyPair {
    pub const fn new(key_bits: u64, sort_rank: u32) -> Self {
        Self {
            named_asset: None,
            packed: key_bits,
            sort_rank,
        }
    }

    pub const fn with_material_id(self, asset: Option<MaterialAssetKey>) -> Self {
        let mut key = self;
        key.named_asset = asset;
        key
    }

    pub const fn facet_draw(self) -> SortedDraw {
        SortedDraw {
            packed: self.packed,
        }
    }
}

/// The material a draw key names, by asset id or by sorted ordinal.
pub fn lookup_ordered_material(
    catalog: &LiveMaterialLedger,
    key: SurfaceSortKeyPair,
) -> Result<&LiveMaterial, MaterialExecutionDenial> {
    let by_id = |id: MaterialAssetKey| {
        let row_at = usize::from(id.0);
        catalog
            .materials
            .get(row_at)
            .ok_or(MaterialExecutionDenial::SurfaceSlotBeyondTable { material: id })
    };
    if let Some(id) = key.named_asset {
        return by_id(id).and_then(|found| {
            (found.asset_id == id)
                .then_some(found)
                .ok_or(MaterialExecutionDenial::SurfaceSlotBeyondTable { material: id })
        });
    }
    match &catalog.ranked_materials {
        LiveSortedMaterialTable::Missing => Err(MaterialExecutionDenial::OrderedMaterialTableAbsent),
        LiveSortedMaterialTable::BuildFailed(cause) => {
            Err(MaterialExecutionDenial::OrderedMaterialConstructionFailed {
                cause: Clone::clone(cause),
            })
        }
        LiveSortedMaterialTable::Ready {
            asset_ids_by_ordinal: ids,
            ..
        } => {
            let id = ids.get(key.sort_rank as usize).copied().ok_or(
                MaterialExecutionDenial::OrderedMaterialRankBeyondTable {
                    sorted_pos: key.sort_rank,
                    table_size: ids.len(),
                },
            )?;
            by_id(id)
        }
    }
}

/// A resolved material together with the catalog it came from, so the
/// technique set and technique behind it can be walked to step by step.
struct Located<'c> {
    catalog: &'c LiveMaterialLedger,
    material: &'c LiveMaterial,
}

impl<'c> Located<'c> {
    fn find(
        catalog: &'c LiveMaterialLedger,
        key: SurfaceSortKeyPair,
    ) -> Result<Self, MaterialExecutionDenial> {
        let material = lookup_ordered_material(catalog, key)?;
        Ok(Self { catalog, material })
    }

    /// The technique set the material's remap leads to.
    fn technique_set_ident(&self) -> Result<LiveTechniqueBundleKey, MaterialExecutionDenial> {
        let material = self.material;
        match material.remap {
            MaterialRemapOutcome::SelfSet => Ok(material.local_technique_set),
            MaterialRemapOutcome::Resolved(set) => Ok(set),
            MaterialRemapOutcome::Missing { address_fingerprint } => {
                Err(MaterialExecutionDenial::SubstitutionTargetAbsent { address_fingerprint })
            }
            MaterialRemapOutcome::Cycle { first_set } => Err(MaterialExecutionDenial::SubstitutionLoop { first_set }),
        }
    }

    /// The set when the remap resolves and the catalog holds it.
    fn set_if_present(&self) -> Option<&'c LiveTechniqueBundle> {
        let id = self.technique_set_ident().ok()?;
        self.catalog.technique_sets.get(id.0 as usize)
    }

    /// The set, refusing when it is out of range or from another namespace.
    fn set(&self) -> Result<&'c LiveTechniqueBundle, MaterialExecutionDenial> {
        let id = self.technique_set_ident()?;
        let held = self.catalog.technique_sets.get(id.0 as usize).ok_or_else(|| {
            if matches!(self.material.remap, MaterialRemapOutcome::SelfSet) {
                MaterialExecutionDenial::OwnRecipeSetBeyondTable { set: id }
            } else {
                MaterialExecutionDenial::SubstitutedRecipeSetBeyondTable { set: id }
            }
        })?;
        match held.namespace == self.material.namespace {
            true => Ok(held),
            false => Err(MaterialExecutionDenial::SetNamespaceDiffers {
                want: self.material.namespace,
                got: held.namespace,
            }),
        }
    }

    fn technique(&self, shading_technique_kind: MethodType) -> Result<&'c LiveShadingTechnique, MaterialExecutionDenial> {
        let technique = self
            .set()?
            .recipe_for(shading_technique_kind)
            .ok_or(MaterialExecutionDenial::RenderTechniqueNotFound { shading_technique_kind })?;
        match technique.passes.is_empty() {
            true => Err(MaterialExecutionDenial::RecipeHasNoPasses { shading_technique_kind }),
            false => Ok(technique),
        }
    }

    /// The world vertex type the technique's own declaration names.
    fn authored_vertex_type(&self, technique: &LiveShadingTechnique) -> u8 {
        self.set_if_present().map_or(decl_table::WORLD_VERTEX_LAYOUT_ID, |set| {
            decl_table::world_decl_kind(technique.flags, set.level_format)
        })
    }

    /// Whether the first pass's vertex declaration routes a source past the
    /// plain world layout (a stream element at slot 5 or above).
    fn first_pass_routes_extra(&self, technique: &LiveShadingTechnique) -> bool {
        let first = technique.passes.first();
        let decl = first
            .and_then(|pass| pass.stage_program_duo)
            .and_then(|pair| self.catalog.decl_for(pair.vertex_format_slot));
        decl.is_some_and(|decl| decl.route_rows().iter().any(|route| route[0] >= 5))
    }
}

/// Anything that cannot be resolved counts as "has the technique": the
/// caller only wants to skip draws that are known to lack it.
pub fn surface_add_gains_technique(
    catalog: &LiveMaterialLedger,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> bool {
    Located::find(catalog, key)
        .ok()
        .and_then(|found| found.set_if_present())
        .is_none_or(|set| set.recipe_for(shading_technique_kind).is_some())
}

pub fn select_material_technique(
    catalog: &LiveMaterialLedger,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> Result<(&LiveMaterial, &LiveShadingTechnique), MaterialExecutionDenial> {
    let found = Located::find(catalog, key)?;
    let chosen = found.technique(shading_technique_kind)?;
    Ok((found.material, chosen))
}

pub fn tess_vertex_kind(
    catalog: &LiveMaterialLedger,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> u8 {
    let world = decl_table::WORLD_VERTEX_LAYOUT_ID;
    let resolved = Located::find(catalog, key).and_then(|found| {
        let tech = found.technique(shading_technique_kind)?;
        Ok((found, tech))
    });
    let Ok((found, tech)) = resolved else {
        return world;
    };
    match found.authored_vertex_type(tech) {
        authored if authored == world && found.first_pass_routes_extra(tech) => {
            world.saturating_add(1)
        }
        authored => authored,
    }
}

pub fn level_stitch_vertex_type_designer(
    catalog: &LiveMaterialLedger,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> u8 {
    let authored = Located::find(catalog, key).and_then(|found| {
        let technique = found.technique(shading_technique_kind)?;
        Ok(found.authored_vertex_type(technique))
    });
    authored.unwrap_or(decl_table::WORLD_VERTEX_LAYOUT_ID)
}

pub fn smodel_tess_vertex_type(stream: Option<lighting_t6::PropFacetPath>) -> u8 {
    use lighting_t6::PropFacetPath::{Cached, Pretess};
    if matches!(stream, Some(Cached | Pretess)) {
        decl_table::STATIC_CACHE_VERTEX_LAYOUT_ID
    } else {
        decl_table::COMPRESSED_VERTEX_KIND
    }
}

/// The refusal that explains why a resolved technique has no prepared form.
pub(super) fn missing_prepared_technique(
    material: &LiveMaterial,
    technique: &LiveShadingTechnique,
    shading_technique_kind: MethodType,
) -> MaterialExecutionDenial {
    let Some(entries) = &material.state_row_for else {
        return MaterialExecutionDenial::RenderStateRecordsAbsent;
    };
    let Some(&row) = entries.get(usize::from(shading_technique_kind.0)) else {
        return MaterialExecutionDenial::RenderStateRecordBeyondBounds { shading_technique_kind };
    };
    if row == 0xff {
        return MaterialExecutionDenial::RenderTechniqueNotFound { shading_technique_kind };
    }
    let pair = technique.passes.first().and_then(|pass| pass.stage_program_duo);
    pair.map_or(
        MaterialExecutionDenial::ShaderProgramAbsent { technique_pass_ordinal: 0 },
        |pair| MaterialExecutionDenial::UnhandledShaderCombination {
            technique_pass_ordinal: 0,
            pair,
        },
    )
}

pub fn resolved_draw_technique<'a>(
    catalog: &'a LiveMaterialLedger,
    prepared: &'a StagedMaterialTable,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> Result<(&'a LiveMaterial, &'a ResolvedShadingRecipe), MaterialExecutionDenial> {
    let (picked, technique) = select_material_technique(catalog, key, shading_technique_kind)?;
    prepared
        .recipe_for(picked.asset_id, shading_technique_kind)
        .map(|found| (picked, found))
        .ok_or_else(|| missing_prepared_technique(picked, technique, shading_technique_kind))
}

pub fn draw_engine_sampler_bits(
    catalog: &LiveMaterialLedger,
    prepared: &StagedMaterialTable,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
) -> u64 {
    resolved_draw_technique(catalog, prepared, key, shading_technique_kind)
        .map_or(0, |(_, found)| found.engine_sampler_bitmask)
}

pub fn draw_uses_engine_texture(
    catalog: &LiveMaterialLedger,
    prepared: &StagedMaterialTable,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
    index: u32,
) -> bool {
    resolved_draw_technique(catalog, prepared, key, shading_technique_kind)
        .is_ok_and(|pair| pair.1.uses_engine_texture(index))
}
