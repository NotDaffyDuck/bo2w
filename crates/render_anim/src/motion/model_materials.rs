//! Materials and DObjs for models, built once before the first frame so
//! drawing never has to wait on them: body and weapon materials by name, map
//! model materials by authored index, and projectile ones likewise.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use asset_core::AssetCategoryScope;
use assets::SurfaceMaterialId;
use bevy::prelude::*;
use render_material::LiveMaterialLedger;
use render_scene::PropPassMaterial;
use render_scene::{StitchMaterials, LevelModelIllumAtlas};

#[derive(Resource, Default)]
pub struct PreparedModelMaterials {
    owner: Option<(Arc<LiveMaterialLedger>, Option<bevy::asset::AssetId<Image>>, Option<usize>, Option<u64>)>,
    by_label: HashMap<String, PropPassMaterial>,
    by_authored: HashMap<SurfaceMaterialId, PropPassMaterial>,
    stage_puppets: HashMap<String, Arc<xmodel_runtime::PosedSkeleton>>,
    projectile_materials: HashMap<SurfaceMaterialId, PropPassMaterial>,
    projectile_dobjs: HashMap<(AssetCategoryScope, String), Arc<xmodel_runtime::PosedSkeleton>>,
}

impl PreparedModelMaterials {
    fn owned_by(&self, (catalog, atlas, bodies, world): &(Arc<LiveMaterialLedger>, Option<bevy::asset::AssetId<Image>>, Option<usize>, Option<u64>)) -> bool {
        let Some((mine, my_atlas, my_bodies, my_world)) = &self.owner else {
            return false;
        };
        (my_atlas, my_bodies, my_world) == (atlas, bodies, world) && Arc::ptr_eq(mine, catalog)
    }

    /// Whether the materials here were prepared against `catalog`. Lookups
    /// by material answer nothing for any other catalog.
    pub fn settled_for(&self, catalog: &Arc<LiveMaterialLedger>) -> bool {
        match &self.owner {
            Some((owned, ..)) => Arc::ptr_eq(owned, catalog),
            None => false,
        }
    }

    /// `table`, but only when it was prepared against `built_for`.
    fn settled<'a, V>(
        &self,
        built_for: &Arc<LiveMaterialLedger>,
        table: &'a HashMap<SurfaceMaterialId, V>,
    ) -> Option<&'a HashMap<SurfaceMaterialId, V>> {
        self.settled_for(built_for).then_some(table)
    }

    pub fn mat(&self, built_for: &Arc<LiveMaterialLedger>, name: &str) -> Option<&PropPassMaterial> {
        let prepared = self.settled_for(built_for);
        prepared.then(|| self.by_label.get(name)).flatten()
    }

    pub fn authored(&self, built_for: &Arc<LiveMaterialLedger>, index: SurfaceMaterialId) -> Option<&PropPassMaterial> {
        self.settled(built_for, &self.by_authored)?.get(&index)
    }

    pub fn stage_puppet(&self, mdl: &str) -> Option<&Arc<xmodel_runtime::PosedSkeleton>> {
        self.stage_puppets.get(mdl)
    }

    pub fn projectile_material(&self, built_for: &Arc<LiveMaterialLedger>, index: SurfaceMaterialId) -> Option<&PropPassMaterial> {
        let table = self.settled(built_for, &self.projectile_materials)?;
        table.get(&index)
    }

    pub fn projectile_dobj(&self, space: AssetCategoryScope, mdl: &str) -> Option<&Arc<xmodel_runtime::PosedSkeleton>> {
        let key = (space, mdl.to_owned());
        self.projectile_dobjs.get(&key)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// How a model's lighting lookup is scaled, from the atlas image's height.
fn lookup_scale(inv_height: f32) -> [f32; 4] {
    let scale = lighting_t6::atlas_lookup_scale(inv_height);
    [scale.u, scale.v, scale.w, scale.q]
}

/// The material a map model's surface is drawn with: lit by the model
/// lighting atlas, with the maps its authored material names.
pub fn scene_lit_pass_material(
    index: SurfaceMaterialId,
    mat_list: &StitchMaterials,
    lit: &LevelModelIllumAtlas,
) -> Option<PropPassMaterial> {
    let catalog = &mat_list.catalog;
    let image_height = lit.dims.rows_tall;
    let inv_height = lighting_t6::light_atlas_reciprocal_height(image_height)?;
    let derived_in = catalog.derived(index)?;
    let slot = catalog
        .ranked_materials
        .rank_for_asset_id(index.order())?;
    let images = mat_list.shader_textures.as_ref();
    let maps_in = render_scene::live_model_tables(Some(index), catalog, images);
    Some(PropPassMaterial {
        probe: None,
        t6_material: None,
        atlas: Some(lit.image.clone()),
        atlas_find: [
            lighting_t6::ATLAS_INV_WIDTH as f32,
            inv_height,
            lighting_t6::MODEL_LIGHT_VOLUME_DEPTH_SPAN,
            0.0,
        ],
        illum_lookup_scale: lookup_scale(inv_height),
        model_illum_required: true,
        material_draw_rank_slot: Some(slot.get()),
        order_key: derived_in.draw_sort_key,
        color: maps_in.color,
        specular: maps_in.specular,
        see_through: maps_in.see_through,
        paint_mode: maps_in.paint_mode,
        face_cull: maps_in.cull_mode,
        env_map_params: maps_in.env_map_params,
    })
}

/// The names of the materials a model actually binds.
fn bound_material_names<'a>(
    keys: &'a [Option<asset_core::MaterialKey>],
    edges: &'a [assets::AssetDependencyLink<assets::SurfaceShaderSpace>],
) -> impl Iterator<Item = &'a str> {
    keys.iter()
        .zip(edges)
        .filter(|(_k, edge)| edge.is_attached())
        .filter_map(|(key, _)| Some(key.as_ref()?.name.as_str()))
}

/// Materials admitted by name, and the names that could not be (kept in the
/// order they were first met, for the report).
#[derive(Default)]
struct NameAdmission {
    admitted: HashMap<String, PropPassMaterial>,
    refused: Vec<String>,
    refused_set: HashSet<String>,
}

impl NameAdmission {
    fn knows(&self, caption: &str) -> bool {
        self.admitted.contains_key(caption) || self.refused_set.contains(caption)
    }

    fn admit<'a>(
        &mut self,
        (lighting, materials): (&LevelModelIllumAtlas, &LiveMaterialLedger),
        wanted: impl IntoIterator<Item = &'a str>,
    ) {
        for name in wanted {
            if self.knows(name) {
                continue;
            }
            match crate::body_lighting_material(lighting, materials, name) {
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
fn single_model_dobj(bone_rig: &asset_model::MeshRig) -> Option<Arc<xmodel_runtime::PosedSkeleton>> {
    let pose = bone_rig.pose.as_ref()?;
    xmodel_runtime::PosedSkeleton::build(&[(pose, None)])
        .ok()
        .map(Arc::new)
}

/// The map models' materials by authored index, and their DObjs by name.
#[derive(Default)]
struct SceneModels {
    mat_list: HashMap<SurfaceMaterialId, PropPassMaterial>,
    dobjs: HashMap<String, Arc<xmodel_runtime::PosedSkeleton>>,
}

impl SceneModels {
    fn gather(
        catalog: &asset_world::LevelModelPlacementRoster,
        tess: &StitchMaterials,
        atlas: &LevelModelIllumAtlas,
    ) -> Self {
        let mut found = SceneModels::default();
        for (key, asset) in catalog.iter() {
            let asset_world::LevelModelPlacementAsset::T5(bone_rig) = asset else {
                continue;
            };
            let authored = (0..bone_rig.facet_materials.len())
                .filter_map(|surface| catalog.facet_material(key, surface));
            for index in authored {
                let Entry::Vacant(slot) = found.mat_list.entry(index) else {
                    continue;
                };
                if let Some(material) = scene_lit_pass_material(index, tess, atlas) {
                    slot.insert(material);
                }
            }
            if let Some(dobj) = single_model_dobj(bone_rig) {
                found.dobjs.insert(key.0.clone(), dobj);
            }
        }
        found
    }
}

/// The projectiles' materials by authored index, and their DObjs by model.
#[derive(Default)]
struct ProjectileModels {
    mat_list: HashMap<SurfaceMaterialId, PropPassMaterial>,
    dobjs: HashMap<(AssetCategoryScope, String), Arc<xmodel_runtime::PosedSkeleton>>,
}

impl ProjectileModels {
    fn gather(
        meshes: &asset_model::MissileMeshRoster,
        tessellation: &StitchMaterials,
        atlas: &LevelModelIllumAtlas,
    ) -> Self {
        let mut found = ProjectileModels::default();
        let entries = (0..meshes.len()).filter_map(|at| Some((at, meshes.at_row(at)?)));
        for (at, entry) in entries {
            let authored = (0..entry.surface_links.len()).filter_map(|s| entry.bound_material(s));
            for index in authored {
                let Entry::Vacant(slot) = found.mat_list.entry(index) else {
                    continue;
                };
                let lit = crate::designer_lit_mesh_material(
                    atlas,
                    &tessellation.catalog,
                    tessellation.shader_textures.as_ref(),
                    index,
                );
                if let Some(material) = lit {
                    slot.insert(material);
                }
            }
            let Some(dobj) = single_model_dobj(&entry.bone_rig) else {
                continue;
            };
            let key = meshes.key_at(at).cloned().unwrap_or_else(|| {
                asset_model::ProjectileMeshKey::new(entry.namespace, &entry.bone_rig.name)
            });
            found.dobjs.insert((key.namespace, key.name), dobj);
        }
        found
    }
}

pub fn prepare_model_materials(
    tess: Res<StitchMaterials>,
    atlas: Option<Res<LevelModelIllumAtlas>>,
    bodies: Option<Res<assets::ReadiedCharacterBodies>>,
    level_placed_weapons: Option<Res<assets::ReadiedWorldWeapons>>,
    stage_models: Option<Res<asset_world::LevelModelPlacementRoster>>,
    projectiles: Option<Res<assets::ReadiedProjectileMeshes>>,
    mut prepared: ResMut<PreparedModelMaterials>,
) {
    if tess.shader_textures.is_empty() {
        return;
    }
    let owner: (Arc<LiveMaterialLedger>, Option<bevy::asset::AssetId<Image>>, Option<usize>, Option<u64>) = (
        Arc::clone(&tess.catalog),
        atlas.as_ref().map(|atlas| atlas.image.id()),
        bodies
            .as_ref()
            .map(|bodies| Arc::as_ptr(&bodies.0) as usize),
        level_placed_weapons.as_ref().map(|world| world.0.identity()),
    );
    if prepared.owned_by(&owner) {
        return;
    }
    let (Some(atlas), Some(bodies), Some(world)) = (atlas, bodies, level_placed_weapons) else {
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
    let t0 = Instant::now();

    let mut names = NameAdmission::default();
    let lit_by = (&*atlas, &*tess.catalog);
    let body_entries = bodies.0.labels().filter_map(|name| bodies.0.by_label(name));
    for entry in body_entries {
        let bound = bound_material_names(&entry.material_keys, &entry.surface_links);
        names.admit(lit_by, bound);
    }
    let weapon_entries = (0..world.0.len()).filter_map(|at| world.0.at_row(at));
    for entry in weapon_entries {
        let bound = bound_material_names(&entry.material_keys, &entry.surface_links);
        names.admit(lit_by, bound);
    }

    let scene = stage_models
        .as_deref()
        .map(|catalog| SceneModels::gather(catalog, &tess, &atlas))
        .unwrap_or_default();
    let missiles = projectiles
        .as_deref()
        .map(|meshes| ProjectileModels::gather(&meshes.0, &tess, &atlas))
        .unwrap_or_default();

    let first_refused = names.refused.iter().take(8).cloned().collect::<Vec<_>>();
    let refused_note = match first_refused.is_empty() {
        true => String::new(),
        false => format!(" first_refused=[{}]", first_refused.join(",")),
    };
    diag::info!(
        World,
        "model materials prepared before Ready: admitted={} map_model_materials={} map_model_dobjs={} projectile_materials={} projectile_dobjs={} refused={} elapsed={:.1}ms{}",
        names.admitted.len(),
        scene.mat_list.len(),
        scene.dobjs.len(),
        missiles.mat_list.len(),
        missiles.dobjs.len(),
        names.refused.len(),
        t0.elapsed().as_secs_f64() * 1e3,
        refused_note
    );
    *prepared = PreparedModelMaterials {
        owner: Some(owner),
        by_label: names.admitted,
        by_authored: scene.mat_list,
        stage_puppets: scene.dobjs,
        projectile_materials: missiles.mat_list,
        projectile_dobjs: missiles.dobjs,
    };
}

mod tests {
    #![cfg(test)]
    use super::*;
    use crate::motion::test_skel::blank_skel;

    #[test]
    fn nothing_is_settled_before_preparation() {
        let prepared = PreparedModelMaterials::default();
        assert!(prepared.stage_puppet("any").is_none());
        assert!(prepared.owner.is_none());
    }

    #[test]
    fn unbound_edges_name_no_materials() {
        let keys = [None, None];
        let edges = [assets::AssetDependencyLink::default(), assets::AssetDependencyLink::default()];
        assert_eq!(bound_material_names(&keys, &edges).count(), 0);
    }

    #[test]
    fn a_model_without_a_pose_has_no_single_model_dobj() {
        assert!(single_model_dobj(&blank_skel("flat")).is_none());
    }

    #[test]
    fn clearing_forgets_the_owner() {
        let mut prepared = PreparedModelMaterials::default();
        prepared.stage_puppets.insert(
            "crate".to_owned(),
            Arc::new(xmodel_runtime::PosedSkeleton::build(&[]).expect("empty dobj")),
        );
        assert!(prepared.stage_puppet("crate").is_some());
        prepared.clear();
        assert!(prepared.stage_puppet("crate").is_none());
    }
}
