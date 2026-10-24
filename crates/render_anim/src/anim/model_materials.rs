//! Materials and DObjs for models, built once before the first frame so
//! drawing never has to wait on them: body and weapon materials by name, map
//! model materials by authored index, and projectile ones likewise.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use render_material::RuntimeMaterialCatalog;
use render_scene::{SmodelPassMaterial, TessMaterials, WorldModelLightingAtlas};

/// What a preparation was made from: the material catalog, the lighting
/// atlas image, the body catalog and the world-weapon catalog.
type Owner = (
    Arc<RuntimeMaterialCatalog>,
    Option<bevy::asset::AssetId<Image>>,
    Option<usize>,
    Option<u64>,
);

type SharedDObj = Arc<xmodel_runtime::DObj>;

#[derive(Resource, Default)]
pub struct PreparedModelMaterials {
    owner: Option<Owner>,
    by_name: HashMap<String, SmodelPassMaterial>,
    by_authored: HashMap<assets::MaterialIndex, SmodelPassMaterial>,
    scene_dobjs: HashMap<String, SharedDObj>,
    projectile_materials: HashMap<assets::MaterialIndex, SmodelPassMaterial>,
    projectile_dobjs: HashMap<(asset_core::AssetNamespace, String), SharedDObj>,
}

impl PreparedModelMaterials {
    fn owned_by(&self, owner: &Owner) -> bool {
        self.owner.as_ref().is_some_and(|(catalog, atlas, bodies, world)| {
            Arc::ptr_eq(catalog, &owner.0)
                && *atlas == owner.1
                && *bodies == owner.2
                && *world == owner.3
        })
    }

    /// Whether the materials here were prepared against `catalog`. Lookups
    /// by material answer nothing for any other catalog.
    pub fn settled_for(&self, catalog: &Arc<RuntimeMaterialCatalog>) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|(owned, ..)| Arc::ptr_eq(owned, catalog))
    }

    pub fn material(
        &self,
        catalog: &Arc<RuntimeMaterialCatalog>,
        name: &str,
    ) -> Option<&SmodelPassMaterial> {
        self.settled_for(catalog).then(|| self.by_name.get(name))?
    }

    pub fn authored(
        &self,
        catalog: &Arc<RuntimeMaterialCatalog>,
        authored: assets::MaterialIndex,
    ) -> Option<&SmodelPassMaterial> {
        self.settled_for(catalog)
            .then(|| self.by_authored.get(&authored))?
    }

    pub fn scene_dobj(&self, model: &str) -> Option<&SharedDObj> {
        self.scene_dobjs.get(model)
    }

    pub fn projectile_material(
        &self,
        catalog: &Arc<RuntimeMaterialCatalog>,
        authored: assets::MaterialIndex,
    ) -> Option<&SmodelPassMaterial> {
        self.settled_for(catalog)
            .then(|| self.projectile_materials.get(&authored))?
    }

    pub fn projectile_dobj(
        &self,
        namespace: asset_core::AssetNamespace,
        model: &str,
    ) -> Option<&SharedDObj> {
        self.projectile_dobjs.get(&(namespace, model.to_owned()))
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The material a map model's surface is drawn with: lit by the model
/// lighting atlas, with the maps its authored material names.
pub fn scene_lit_pass_material(
    atlas: &WorldModelLightingAtlas,
    tess: &TessMaterials,
    authored: assets::MaterialIndex,
) -> Option<SmodelPassMaterial> {
    let derived = tess.catalog.derived(authored)?;
    let ordinal = tess
        .catalog
        .sorted_materials
        .ordinal_for_asset_id(authored.order())?;
    let maps =
        render_scene::runtime_maps(Some(authored), &tess.catalog, tess.material_images.as_ref());
    let inv_height = lighting_t6::model_lighting_inv_image_height(atlas.dims.image_height)?;
    let scale = lighting_t6::model_lighting_lookup_scale(inv_height);
    Some(SmodelPassMaterial {
        model_lighting_required: true,
        color: maps.color,
        specular: maps.specular,
        probe: None,
        atlas: Some(atlas.image.clone()),
        alpha_mode: maps.alpha_mode,
        draw_mode: maps.draw_mode,
        cull_mode: maps.cull_mode,
        env_map_parms: maps.env_map_parms,
        lighting_lookup_scale: [scale.u, scale.v, scale.w, scale.q],
        atlas_lookup: [
            lighting_t6::MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
            inv_height,
            lighting_t6::MODEL_LIGHTING_VOLUME_W,
            0.0,
        ],
        sort_key: derived.sort_key,
        material_sorted_index: Some(ordinal.get()),
        t6_material: None,
    })
}

/// The names of the materials a model actually binds.
fn bound_material_names<'a>(
    keys: &'a [Option<asset_core::MaterialKey>],
    edges: &'a [assets::AssetEdge<assets::MaterialSpace>],
) -> impl Iterator<Item = &'a str> {
    keys.iter()
        .zip(edges)
        .filter(|(_, edge)| edge.is_bound())
        .filter_map(|(key, _)| Some(key.as_ref()?.name.as_str()))
}

/// Materials admitted by name, and the names that could not be (kept in the
/// order they were first met, for the report).
#[derive(Default)]
struct NameAdmission {
    admitted: HashMap<String, SmodelPassMaterial>,
    refused: Vec<String>,
    refused_set: HashSet<String>,
}

impl NameAdmission {
    fn admit<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a str>,
        atlas: &WorldModelLightingAtlas,
        catalog: &RuntimeMaterialCatalog,
    ) {
        for name in names {
            if self.admitted.contains_key(name) || self.refused_set.contains(name) {
                continue;
            }
            match crate::body_lit_pass_material(atlas, catalog, name) {
                Some(material) => {
                    self.admitted.insert(name.to_owned(), material);
                }
                None => {
                    self.refused_set.insert(name.to_owned());
                    self.refused.push(name.to_owned());
                }
            }
        }
    }
}

/// A DObj of one model alone, if it has a pose to build one from.
fn single_model_dobj(skel: &asset_model::ModelSkel) -> Option<SharedDObj> {
    let pose = skel.pose.as_ref()?;
    xmodel_runtime::DObj::build(&[(pose, None)]).ok().map(Arc::new)
}

pub fn prepare_model_materials(
    tess: Res<TessMaterials>,
    atlas: Option<Res<WorldModelLightingAtlas>>,
    bodies: Option<Res<assets::PreparedBodies>>,
    world_weapons: Option<Res<assets::PreparedWorldWeapons>>,
    scene_models: Option<Res<asset_world::MapXModelSceneCatalog>>,
    projectiles: Option<Res<assets::PreparedProjectileMeshes>>,
    mut prepared: ResMut<PreparedModelMaterials>,
) {
    if tess.material_images.is_empty() {
        return;
    }
    let owner: Owner = (
        Arc::clone(&tess.catalog),
        atlas.as_ref().map(|atlas| atlas.image.id()),
        bodies.as_ref().map(|bodies| Arc::as_ptr(&bodies.0) as usize),
        world_weapons.as_ref().map(|world| world.0.identity()),
    );
    if prepared.owned_by(&owner) {
        return;
    }
    let (Some(atlas), Some(bodies), Some(world)) = (atlas, bodies, world_weapons) else {
        diag::info!(
            World,
            "model materials: nothing to admit (lighting atlas or model catalogs missing)"
        );
        *prepared = PreparedModelMaterials {
            owner: Some(owner),
            ..Default::default()
        };
        return;
    };
    let started = std::time::Instant::now();

    let mut names = NameAdmission::default();
    for entry in bodies.0.names().filter_map(|name| bodies.0.get(name)) {
        let bound = bound_material_names(&entry.material_keys, &entry.material_edges);
        names.admit(bound, &atlas, &tess.catalog);
    }
    for entry in (0..world.0.len()).filter_map(|index| world.0.get_at(index)) {
        let bound = bound_material_names(&entry.material_keys, &entry.material_edges);
        names.admit(bound, &atlas, &tess.catalog);
    }

    let mut by_authored = HashMap::new();
    let mut scene_dobjs = HashMap::new();
    if let Some(catalog) = scene_models.as_deref() {
        for (key, asset) in catalog.iter() {
            let asset_world::MapXModelSceneAsset::T5(skel) = asset else {
                continue;
            };
            for surface in 0..skel.surface_materials.len() {
                let Some(authored) = catalog.surface_material(key, surface) else {
                    continue;
                };
                if let Entry::Vacant(slot) = by_authored.entry(authored)
                    && let Some(material) = scene_lit_pass_material(&atlas, &tess, authored)
                {
                    slot.insert(material);
                }
            }
            if let Some(dobj) = single_model_dobj(skel) {
                scene_dobjs.insert(key.0.clone(), dobj);
            }
        }
    }

    let mut projectile_materials = HashMap::new();
    let mut projectile_dobjs = HashMap::new();
    if let Some(meshes) = projectiles.as_deref() {
        for (index, entry) in (0..meshes.0.len()).filter_map(|i| Some((i, meshes.0.get_at(i)?))) {
            let key = meshes.0.key_at(index).cloned().unwrap_or_else(|| {
                asset_model::ProjectileMeshKey::new(entry.namespace, &entry.skel.name)
            });
            for surface in 0..entry.material_edges.len() {
                let Some(authored) = entry.material_index(surface) else {
                    continue;
                };
                if let Entry::Vacant(slot) = projectile_materials.entry(authored)
                    && let Some(material) = crate::authored_lit_xmodel_pass_material(
                        &atlas,
                        &tess.catalog,
                        tess.material_images.as_ref(),
                        authored,
                    )
                {
                    slot.insert(material);
                }
            }
            if let Some(dobj) = single_model_dobj(&entry.skel) {
                projectile_dobjs.insert((key.namespace, key.name), dobj);
            }
        }
    }

    let first_refused = names.refused.iter().take(8).cloned().collect::<Vec<_>>();
    diag::info!(
        World,
        "model materials prepared before Ready: admitted={} map_model_materials={} map_model_dobjs={} projectile_materials={} projectile_dobjs={} refused={} elapsed={:.1}ms{}",
        names.admitted.len(),
        by_authored.len(),
        scene_dobjs.len(),
        projectile_materials.len(),
        projectile_dobjs.len(),
        names.refused.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        if first_refused.is_empty() {
            String::new()
        } else {
            format!(" first_refused=[{}]", first_refused.join(","))
        }
    );
    *prepared = PreparedModelMaterials {
        owner: Some(owner),
        by_name: names.admitted,
        by_authored,
        scene_dobjs,
        projectile_materials,
        projectile_dobjs,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_skel::blank_skel;

    #[test]
    fn nothing_is_settled_before_preparation() {
        let prepared = PreparedModelMaterials::default();
        assert!(prepared.scene_dobj("any").is_none());
        assert!(prepared.owner.is_none());
    }

    #[test]
    fn unbound_edges_name_no_materials() {
        let keys = [None, None];
        let edges = [assets::AssetEdge::default(), assets::AssetEdge::default()];
        assert_eq!(bound_material_names(&keys, &edges).count(), 0);
    }

    #[test]
    fn a_model_without_a_pose_has_no_single_model_dobj() {
        assert!(single_model_dobj(&blank_skel("flat")).is_none());
    }

    #[test]
    fn clearing_forgets_the_owner() {
        let mut prepared = PreparedModelMaterials::default();
        prepared.scene_dobjs.insert(
            "crate".to_owned(),
            Arc::new(xmodel_runtime::DObj::build(&[]).expect("empty dobj")),
        );
        assert!(prepared.scene_dobj("crate").is_some());
        prepared.clear();
        assert!(prepared.scene_dobj("crate").is_none());
    }
}
