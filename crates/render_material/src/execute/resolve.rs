//! Finding the material, technique set and prepared technique for a draw.

use dpvs_t6::GfxDrawSurf;

use super::refusal::MaterialRefusal;
use crate::decl_table;
use crate::prepared::{PreparedMaterialTable, PreparedTechnique};
use crate::{
    MaterialAssetId, RemapResolution, RuntimeMaterial, RuntimeMaterialCatalog,
    RuntimeSortedMaterialTable, RuntimeTechnique, RuntimeTechniqueSetId, TechType,
};

/// A sorted draw's material: either named directly by asset id or by its
/// ordinal in the sorted table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialDrawKey {
    pub material_id: Option<MaterialAssetId>,
    pub packed: u64,
    pub rank: u32,
}

impl MaterialDrawKey {
    pub const fn new(packed: u64, rank: u32) -> Self {
        Self {
            material_id: None,
            packed,
            rank,
        }
    }

    pub const fn with_material_id(mut self, material_id: Option<MaterialAssetId>) -> Self {
        self.material_id = material_id;
        self
    }

    pub const fn draw_surf(self) -> GfxDrawSurf {
        GfxDrawSurf {
            packed: self.packed,
        }
    }
}

pub fn resolve_sorted_material(
    catalog: &RuntimeMaterialCatalog,
    key: MaterialDrawKey,
) -> Result<&RuntimeMaterial, MaterialRefusal> {
    if let Some(id) = key.material_id {
        return match catalog.materials.get(usize::from(id.0)) {
            Some(material) if material.asset_id == id => Ok(material),
            _ => Err(MaterialRefusal::MaterialOutOfRange { material: id }),
        };
    }
    let id = match &catalog.sorted_materials {
        RuntimeSortedMaterialTable::Missing => {
            return Err(MaterialRefusal::SortedMaterialTableMissing);
        }
        RuntimeSortedMaterialTable::BuildFailed(cause) => {
            return Err(MaterialRefusal::SortedMaterialBuildFailed {
                cause: cause.clone(),
            });
        }
        RuntimeSortedMaterialTable::Ready {
            asset_ids_by_ordinal,
            ..
        } => match asset_ids_by_ordinal.get(key.rank as usize) {
            Some(&id) => id,
            None => {
                return Err(MaterialRefusal::SortedMaterialOrdinalOutOfRange {
                    ordinal: key.rank,
                    table_len: asset_ids_by_ordinal.len(),
                });
            }
        },
    };
    catalog
        .materials
        .get(usize::from(id.0))
        .ok_or(MaterialRefusal::MaterialOutOfRange { material: id })
}

/// The technique set a material draws from, after following its remap.
fn technique_set_id(material: &RuntimeMaterial) -> Result<RuntimeTechniqueSetId, MaterialRefusal> {
    match material.remap {
        RemapResolution::SelfSet => Ok(material.local_technique_set),
        RemapResolution::Resolved(set) => Ok(set),
        RemapResolution::Missing { pointer_identity } => {
            Err(MaterialRefusal::RemapMissing { pointer_identity })
        }
        RemapResolution::Cycle { first_set } => Err(MaterialRefusal::RemapCycle { first_set }),
    }
}

/// Anything that cannot be resolved counts as "has the technique": the
/// caller only wants to skip draws that are known to lack it.
pub fn add_surf_has_technique(
    catalog: &RuntimeMaterialCatalog,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> bool {
    let Ok(material) = resolve_sorted_material(catalog, key) else {
        return true;
    };
    let Ok(set_id) = technique_set_id(material) else {
        return true;
    };
    match catalog.technique_sets.get(set_id.0 as usize) {
        Some(set) => set.technique(tech_type).is_some(),
        None => true,
    }
}

pub fn resolve_material_technique(
    catalog: &RuntimeMaterialCatalog,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> Result<(&RuntimeMaterial, &RuntimeTechnique), MaterialRefusal> {
    let material = resolve_sorted_material(catalog, key)?;
    let set_id = technique_set_id(material)?;
    let Some(set) = catalog.technique_sets.get(set_id.0 as usize) else {
        return Err(if matches!(material.remap, RemapResolution::SelfSet) {
            MaterialRefusal::LocalTechniqueSetOutOfRange { set: set_id }
        } else {
            MaterialRefusal::RemappedTechniqueSetOutOfRange { set: set_id }
        });
    };
    if set.namespace != material.namespace {
        return Err(MaterialRefusal::TechniqueSetNamespaceMismatch {
            want: material.namespace,
            got: set.namespace,
        });
    }
    let technique = set
        .technique(tech_type)
        .ok_or(MaterialRefusal::TechniqueAbsent { tech_type })?;
    if technique.passes.is_empty() {
        return Err(MaterialRefusal::EmptyTechnique { tech_type });
    }
    Ok((material, technique))
}

/// The vertex type the technique's authored world-vertex declaration names.
fn authored_world_vertex_type(
    catalog: &RuntimeMaterialCatalog,
    material: &RuntimeMaterial,
    technique: &RuntimeTechnique,
) -> u8 {
    let Ok(set_id) = technique_set_id(material) else {
        return decl_table::WORLD_VERTEX_TYPE;
    };
    match catalog.technique_sets.get(set_id.0 as usize) {
        Some(set) => decl_table::world_vert_decl_type(technique.flags, set.world_vert_format),
        None => decl_table::WORLD_VERTEX_TYPE,
    }
}

/// Whether the first pass's vertex declaration routes a source past the
/// plain world layout (a stream element at slot 5 or above).
fn first_pass_routes_extra_source(
    catalog: &RuntimeMaterialCatalog,
    technique: &RuntimeTechnique,
) -> bool {
    technique
        .passes
        .first()
        .and_then(|pass| pass.shader_pair)
        .and_then(|pair| catalog.vertex_decl(pair.vertex_decl_slot))
        .is_some_and(|decl| decl.routed().iter().any(|route| route[0] >= 5))
}

pub fn world_tess_vertex_type(
    catalog: &RuntimeMaterialCatalog,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> u8 {
    let Ok((material, technique)) = resolve_material_technique(catalog, key, tech_type) else {
        return decl_table::WORLD_VERTEX_TYPE;
    };
    let authored = authored_world_vertex_type(catalog, material, technique);
    if authored == decl_table::WORLD_VERTEX_TYPE
        && first_pass_routes_extra_source(catalog, technique)
    {
        decl_table::WORLD_VERTEX_TYPE.saturating_add(1)
    } else {
        authored
    }
}

pub fn world_tess_vertex_type_authored(
    catalog: &RuntimeMaterialCatalog,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> u8 {
    match resolve_material_technique(catalog, key, tech_type) {
        Ok((material, technique)) => authored_world_vertex_type(catalog, material, technique),
        Err(_) => decl_table::WORLD_VERTEX_TYPE,
    }
}

pub fn smodel_tess_vertex_type(stream: Option<lighting_t6::SmodelSurfPath>) -> u8 {
    use lighting_t6::SmodelSurfPath::{Cached, Pretess};
    match stream {
        Some(Cached | Pretess) => decl_table::STATICMODELCACHE_VERTEX_TYPE,
        _ => decl_table::PACKED_VERTEX_TYPE,
    }
}

/// The refusal that explains why a resolved technique has no prepared form.
pub(super) fn missing_prepared_technique(
    material: &RuntimeMaterial,
    technique: &RuntimeTechnique,
    tech_type: TechType,
) -> MaterialRefusal {
    let Some(entries) = material.state_bits_entry.as_ref() else {
        return MaterialRefusal::StateEntriesMissing;
    };
    match entries.get(usize::from(tech_type.0)) {
        None => MaterialRefusal::StateEntryOutOfRange { tech_type },
        Some(0xff) => MaterialRefusal::TechniqueAbsent { tech_type },
        Some(_) => {
            let first_pair = technique.passes.first().and_then(|pass| pass.shader_pair);
            match first_pair {
                Some(pair) => MaterialRefusal::UnsupportedShaderPair {
                    pass_index: 0,
                    pair,
                },
                None => MaterialRefusal::ShaderProgramMissing { pass_index: 0 },
            }
        }
    }
}

pub fn prepared_draw_technique<'a>(
    catalog: &'a RuntimeMaterialCatalog,
    prepared: &'a PreparedMaterialTable,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> Result<(&'a RuntimeMaterial, &'a PreparedTechnique), MaterialRefusal> {
    let (material, technique) = resolve_material_technique(catalog, key, tech_type)?;
    match prepared.technique(material.asset_id, tech_type) {
        Some(found) => Ok((material, found)),
        None => Err(missing_prepared_technique(material, technique, tech_type)),
    }
}

pub fn draw_code_sampler_mask(
    catalog: &RuntimeMaterialCatalog,
    prepared: &PreparedMaterialTable,
    key: MaterialDrawKey,
    tech_type: TechType,
) -> u64 {
    prepared_draw_technique(catalog, prepared, key, tech_type)
        .map_or(0, |(_, found)| found.code_sampler_mask)
}

pub fn draw_binds_code_texture(
    catalog: &RuntimeMaterialCatalog,
    prepared: &PreparedMaterialTable,
    key: MaterialDrawKey,
    tech_type: TechType,
    index: u32,
) -> bool {
    prepared_draw_technique(catalog, prepared, key, tech_type)
        .is_ok_and(|(_, found)| found.binds_code_texture(index))
}
