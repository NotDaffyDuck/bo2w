//! Once a world's images are resident: the colour maps the effects draw
//! with, the geometry of the effects' models, and the typed image registry
//! the draw-surface stage looks textures up in.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use render_fx::{ColorMaps, EffectLevelColorImages, ReadyFxBook, StagedStreaks};
use xmodel_runtime::{PosedSkeleton, ModelPoseQuery, RigPoseRecord};

use crate::assemble::draw_items::mesh_build::figure_mesh::bound_lit_figure_pass_material;
use crate::assemble::draw_items::{LiveImageHandles, LiveLightbakeHandles, LiveMaterial};
use render_anim::motion::figure_pose::{skin_chosen_surfaces, rigid_lod_surface};

use super::world::LevelStage;

const MIB: f64 = 1024.0 * 1024.0;

/// A material's sort key and its (unused here) second key half.

/// Logs how many images the asset store holds, how much of their pixel data the
/// main world still keeps, how much went to the GPU, and the process's resident size.
fn report_image_memory(store: &Assets<Image>) {
    let mut assets = 0usize;
    // Bytes held for each side: [main world, render world].
    let mut held = [0usize; 2];
    let sides = [RenderAssetUsages::MAIN_WORLD, RenderAssetUsages::RENDER_WORLD];
    for (_, img) in store.iter() {
        assets += 1;
        let size = img.data.as_ref().map_or(0, Vec::len);
        for (total, side) in held.iter_mut().zip(sides) {
            if img.asset_usage.contains(side) {
                *total += size;
            }
        }
    }
    let rss = asset_transport::working_set_size().map_or(0.0, |n| n as f64 / MIB);
    let [kept, sent] = held.map(|n| n as f64 / MIB);
    diag::info!(
        World,
        "image asset memory: assets={assets} main_world_retained={kept:.1}MiB uploaded={sent:.1}MiB rss={rss:.0}MiB"
    );
}

/// The exact-pool image a material's colour map (or, failing that, its
/// plain 2D texture) points at.
fn colour_handle(material: &LiveMaterial, exact: &[Option<Handle<Image>>]) -> Option<Handle<Image>> {
    let colour_map = material.semantic_for_image(asset_material::SEMANTIC_COLOR);
    let bound = colour_map.or_else(|| material.semantic_for_image(asset_material::TS_2D))?;
    exact.get(bound.0 as usize)?.clone()
}

/// The colour maps and sort keys of the materials effects bind by asset id.
#[derive(Default)]
struct ByAsset {
    colors: HashMap<usize, Handle<Image>>,
    keys: HashMap<usize, (u8, u16)>,
}

impl ByAsset {
    /// Records the material of `asset_id`: its key always, its colour map if
    /// it has one resident.
    fn note_material(&mut self, scene: &LevelStage, exact: &[Option<Handle<Image>>], asset_id: usize) {
        let order = assets::SurfaceMaterialId::from_sequence(asset_id);
        if let Some(found) = scene.live_material_ledger.derived(order) {
            self.keys.insert(asset_id, (found.draw_sort_key, 0));
            if let Some(image) = colour_handle(found, exact) {
                self.colors.insert(asset_id, image);
            }
        }
    }

    /// Records every id in `ids` and says how many there were (repeats count).
    fn add_all(&mut self, scene: &LevelStage, exact: &[Option<Handle<Image>>], ids: impl Iterator<Item = usize>) -> usize {
        ids.map(|id| self.note_material(scene, exact, id)).count()
    }
}

/// What the effect catalogs say about which materials need a colour map, and
/// how many bindings said so.
struct Bindings {
    sprites: ByAsset,
    marks: ByAsset,
    /// Distinct tracer colour maps, before the effect elements add theirs.
    tracer_maps: usize,
    tracer_refs: usize,
    visual_refs: usize,
    mark_refs: usize,
}

fn collect_bindings(
    scene: &LevelStage,
    exact: &[Option<Handle<Image>>],
    tracer_defs: Option<&StagedStreaks>,
    effects: Option<&ReadyFxBook>,
) -> Bindings {
    let mut sprites = ByAsset::default();
    let tracer_ids = tracer_defs.into_iter().flat_map(|set| set.0.defs()).filter_map(|def| def.material.extent_position());
    let tracer_refs = sprites.add_all(scene, exact, tracer_ids);
    let tracer_maps = sprites.colors.len();

    // Every visual of every element of every effect that names a material.
    let elems = effects.into_iter().flat_map(|set| set.0.effects()).flat_map(|effect| &effect.elems);
    let visual_ids = elems.flat_map(|elem| &elem.visuals).filter_map(|visual| visual.extent_position());
    let visual_refs = sprites.add_all(scene, exact, visual_ids);

    let mut marks = ByAsset::default();
    let mark_ids = effects.into_iter().flat_map(|set| set.0.distinct_decal_hints()).map(|(id, _)| id);
    let mark_refs = marks.add_all(scene, exact, mark_ids);

    Bindings { sprites, marks, tracer_maps, tracer_refs, visual_refs, mark_refs }
}

/// Sprite bindings whose material has no colour map, as `id:name` rows split
/// into distortion ones (which need none) and the rest. Logged only.
fn log_unmapped_bindings(scene: &LevelStage, effects: &ReadyFxBook, sprites: &ByAsset) {
    let known = &scene.live_material_ledger.materials;
    let mut distortion = 0usize;
    let mut rest = Vec::new();
    for (id, hint) in effects.0.distinct_binding_clues() {
        if sprites.colors.contains_key(&id) {
            continue;
        }
        let name = known.get(id).map_or(hint, |m| m.name.as_str());
        let row = format!("{id}:{name}");
        if row.contains("distortion") {
            distortion += 1;
        } else {
            rest.push(row);
        }
    }
    let total = distortion + rest.len();
    if total == 0 {
        return;
    }
    let sample = if rest.is_empty() { String::from("-") } else { rest.join("; ") };
    let unique = effects.0.surface_look_distinct_binding_total();
    let others = rest.len();
    diag::info!(
        World,
        "fx elem Bound without color map: {total} of {unique} unique (distortion={distortion}, other={others}) other_sample: {sample}"
    );
}

/// Drops the unused second half of each sort key.
fn first_halves<K: std::hash::Hash + Eq>(keys: HashMap<K, (u8, u16)>) -> HashMap<K, u8> {
    keys.into_iter().map(|(at, (sort, _))| (at, sort)).collect()
}

pub(crate) fn world_color_images(
    scene: &LevelStage,
    exact_pool: &[Option<Handle<Image>>],
    tracer_defs: Option<&StagedStreaks>,
    effects: Option<&ReadyFxBook>,
) -> EffectLevelColorImages {
    let found = collect_bindings(scene, exact_pool, tracer_defs, effects);

    // Effects that name their material look the map up by that name.
    let mut handles_named: HashMap<String, Handle<Image>> = Default::default();
    let mut keys_named = HashMap::new();
    let catalog = &scene.live_material_ledger;
    for (&id, image) in found.sprites.colors.iter().chain(&found.marks.colors) {
        if let Some(m) = catalog.derived(assets::SurfaceMaterialId::from_sequence(id)) {
            asset_game::add_tint_picture(&mut handles_named, &m.name, image.clone());
            asset_game::add_tint_picture(&mut keys_named, &m.name, (m.draw_sort_key, 0));
        }
    }
    let stub_aliases = asset_game::effect_tint_map_placeholders(&mut handles_named);
    asset_game::effect_tint_map_placeholders(&mut keys_named);

    let (named, named_keys) = (handles_named.len(), keys_named.len());
    diag::info!(
        World,
        "effect colour maps bound by name: {named} (stub aliases {stub_aliases}, material keys {named_keys}); handles come from the exact pool, never CPU copies"
    );
    let (tracer_maps, tracer_refs) = (found.tracer_maps, found.tracer_refs);
    diag::info!(World, "fx tracer color maps by asset: {tracer_maps} unique / {tracer_refs} bound defs");
    let (sprite_maps, visual_refs) = (found.sprites.colors.len(), found.visual_refs);
    diag::info!(World, "fx elem color maps by asset: {sprite_maps} unique / {visual_refs} bound visuals");
    let (mark_maps, mark_refs) = (found.marks.colors.len(), found.mark_refs);
    diag::info!(
        World,
        "fx mark color maps by asset: {mark_maps} unique / {mark_refs} unique Bound — not sprite colors_by_asset"
    );
    if let Some(set) = effects {
        log_unmapped_bindings(scene, set, &found.sprites);
    }

    let by_name = ColorMaps { colors: handles_named, sort_keys: first_halves(keys_named) };
    let by_asset = ColorMaps { colors: found.sprites.colors, sort_keys: first_halves(found.sprites.keys) };
    EffectLevelColorImages { by_name, by_asset }
}

/// An effect model posed at bind: the object, the request that resolved it,
/// and one skin matrix per bone.

/// The bind-pose skinning of one effect model. None if any step refuses.
fn bind_pose_skin(name: &str, entry: &asset_game::EffectMeshRecord) -> Option<(PosedSkeleton, ModelPoseQuery, Vec<Mat4>)> {
    let skeleton = entry.bone_rig.pose.as_ref()?;
    let object = PosedSkeleton::build(&[(skeleton, None)]).ok()?;
    let posed_as = RigPoseRecord::neutral_pose(name.to_owned(), 1, 1).look_up_pose_query(|_| None).ok()?;
    let bones = xmodel_runtime::pose_dobj(&object, &posed_as, Mat4::default()).ok()?;
    let joint_transforms = object.skin_matrices(&bones);
    Some((object, posed_as, joint_transforms))
}

pub(crate) fn prepare_fx_model_geometry(
    scene: &LevelStage,
    models: Option<&render_fx::ReadyFxProps>,
) -> render_fx::PreparedFxModelPlan {
    let catalog = &scene.live_material_ledger;
    let mut plan = render_fx::FxModelDrawList::without_geometry(catalog.revision_stamp.0);
    // Models are lit from the world's model lighting atlas; without one, none are drawn.
    let lighting = scene.model_illum_image.clone().zip(scene.model_illum_dims);
    let (Some(models), Some((image, dims))) = (models, lighting) else {
        return render_fx::PreparedFxModelPlan(plan);
    };
    let atlas = super::model_illum_atlas::LevelModelIllumAtlas { image, dims };
    let set = &models.0;

    for slot in 0..set.len() {
        let Some(entry) = set.record_at(slot) else {
            continue;
        };
        let name = set.label_at(slot).unwrap_or_default();
        let Some((object, posed_as, joint_transforms)) = bind_pose_skin(name, entry) else {
            diag::warn!(World, "FX model `{name}` refused: bind pose unavailable");
            continue;
        };
        let bone_rig = &entry.bone_rig;
        // Up to four LODs; a model without a selector has just the one.
        let detail_tiers = bone_rig.lod.map_or(1, |pick| pick.detail_tier_total().clamp(0, 4) as u8);
        let shown = |surface: usize| object.facet_visible(0, &bone_rig.face_parts[surface], &posed_as.hidden_mask);
        for lod in 0..detail_tiers {
            let rigid = |surface| rigid_lod_surface(bone_rig, lod, &[], 0, surface);
            let Some(surfaces) = skin_chosen_surfaces(bone_rig, |bone| joint_transforms[bone], &[], shown, rigid, lod) else {
                diag::warn!(World, "FX model `{name}` LOD {lod} refused: geometry unavailable");
                continue;
            };
            let passes: Vec<_> = surfaces
                .iter()
                .map(|s| entry.surface_slot(s.surface_slot_number).and_then(|m| bound_lit_figure_pass_material(&atlas, catalog, m)))
                .collect();
            // A model is drawn whole or not at all.
            if passes.iter().any(Option::is_none) {
                diag::warn!(World, "FX model `{name}` LOD {lod} refused: material binding unavailable");
                continue;
            }
            render_fx::push_effect_model_asset(&mut plan, slot, lod, &surfaces, &passes);
        }
    }
    render_fx::PreparedFxModelPlan(plan)
}

pub fn register_into(
    commands: &mut Commands,
    scene: &LevelStage,
    store: &Assets<Image>,
    exact_pool: Vec<Option<Handle<Image>>>,
    probe_pool: Vec<Option<Handle<Image>>>,
    lightmap_pool: Vec<Option<LiveLightbakeHandles>>,
    tracer_defs: Option<&StagedStreaks>,
    effects: Option<&ReadyFxBook>,
) {
    commands.insert_resource(world_color_images(scene, &exact_pool, tracer_defs, effects));

    // Filled slots out of all slots, per pool; a lightmap page counts only with both halves.
    let resident = |pool: &[Option<Handle<Image>>]| pool.iter().filter(|slot| slot.is_some()).count();
    let materials = format!("{}/{}", resident(&exact_pool), exact_pool.len());
    let probes = format!("{}/{}", resident(&probe_pool), probe_pool.len());
    let whole_pages = lightmap_pool.iter().flatten().filter(|p| p.primary.is_some() && p.secondary.is_some()).count();
    let lightmaps = format!("{whole_pages}/{}", lightmap_pool.len());

    let generation = scene.live_material_ledger.revision_stamp;
    let names = scene.precise_material_names.clone();
    commands.insert_resource(LiveImageHandles::build_from_stores(generation, exact_pool, names, probe_pool, lightmap_pool, None));
    report_image_memory(store);
    diag::info!(
        World,
        "drawsurf typed image registry: material={materials} probe={probes} lightmap_exact={lightmaps}"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asset_core::AssetCategoryScope;
    use bevy::ecs::system::RunSystemOnce;
    use render_material::{
        MaterialAssetKey, MaterialEpochId, MaterialRemapOutcome, LiveImageId, LiveMaterial,
        LiveMaterialLedger, LiveTechniqueBundleKey, LiveTextureBinding, RENDER_RECIPE_SLOT_TOTAL,
    };

    use super::*;

    const COLOR: u8 = asset_material::SEMANTIC_COLOR;
    const PLAIN: u8 = asset_material::TS_2D;

    /// A texture slot bound to pool image `slot` with the given meaning.
    fn binding(semantic: u8, slot: u32) -> (u32, Option<LiveTextureBinding>) {
        let bound = LiveTextureBinding { image: LiveImageId(slot), texture_sampling_mode: 0, semantic };
        (u32::from(semantic) + 100, Some(bound))
    }

    fn mat_row(
        id: u16,
        name: &str,
        order_key: u8,
        textures: Vec<(u32, Option<LiveTextureBinding>)>,
    ) -> LiveMaterial {
        let cull_mode = None;
        LiveMaterial {
            asset_id: MaterialAssetKey(id),
            name: name.to_owned(),
            namespace: AssetCategoryScope::T6,
            method_set: String::new(),
            baked_facet_draw: None,
            local_technique_set: LiveTechniqueBundleKey(0),
            remap: MaterialRemapOutcome::SelfSet,
            state_row_for: None::<[u8; RENDER_RECIPE_SLOT_TOTAL]>,
            state_rows: Vec::new(),
            camera_zone: 0,
            draw_sort_key: order_key,
            game_flag_info: 0,
            condition_bits: 0,
            facet_type_bits: None,
            unlit: false,
            uses_model_illum_const: false,
            colour_map_is_square: false,
            occluder_exclusive: false,
            t6_draw: None,
            cull_mode,
            uv_scroll_bits: [0; 4],
            attenuation_terms_raw: [0; 4],
            attenuation_start_raw: [0; 4],
            attenuation_stop_raw: [0; 4],
            reflection_map_param_bits: [0; 4],
            sheet_bits: [0; 5],
            textures,
            constants: Vec::new(),
        }
    }

    /// A world whose catalog holds `materials` under generation 9.
    fn scene_with(materials: Vec<LiveMaterial>) -> LevelStage {
        let mut catalog = LiveMaterialLedger::default();
        catalog.revision_stamp = MaterialEpochId(9);
        catalog.materials = materials;
        let mut scene = LevelStage::default();
        scene.live_material_ledger = Arc::new(catalog);
        scene
    }

    /// An exact pool whose slot `i` holds a distinct image.
    fn pool(images: &mut Assets<Image>, n: usize) -> Vec<Option<Handle<Image>>> {
        (0..n).map(|_| Some(images.add(Image::default()))).collect()
    }

    #[test]
    fn a_colour_map_binding_picks_the_image_in_its_pool_slot() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = mat_row(0, "m", 4, vec![binding(COLOR, 2)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[2].as_ref().unwrap().id());
    }

    #[test]
    fn the_colour_map_wins_over_a_plain_texture() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = mat_row(0, "m", 4, vec![binding(PLAIN, 0), binding(COLOR, 1)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[1].as_ref().unwrap().id());
    }

    #[test]
    fn a_material_with_only_a_plain_texture_falls_back_to_it() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = mat_row(0, "m", 4, vec![binding(PLAIN, 0)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[0].as_ref().unwrap().id());
    }

    #[test]
    fn no_colour_handle_without_a_binding_or_a_resident_image() {
        let mut images = Assets::<Image>::default();
        let mut exact = pool(&mut images, 3);
        // Neither kind of binding.
        assert!(colour_handle(&mat_row(0, "m", 4, vec![binding(7, 0)]), &exact).is_none());
        assert!(colour_handle(&mat_row(0, "m", 4, vec![]), &exact).is_none());
        // A texture slot with nothing bound to it.
        assert!(colour_handle(&mat_row(0, "m", 4, vec![(1, None)]), &exact).is_none());
        // Bound to an image that is not resident, or is not in the pool.
        exact[1] = None;
        assert!(colour_handle(&mat_row(0, "m", 4, vec![binding(COLOR, 1)]), &exact).is_none());
        assert!(colour_handle(&mat_row(0, "m", 4, vec![binding(COLOR, 40)]), &exact).is_none());
    }

    #[test]
    fn a_material_is_recorded_by_its_asset_id_with_its_sort_key() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 2);
        let scene = scene_with(vec![
            mat_row(0, "a", 3, vec![binding(COLOR, 0)]),
            mat_row(1, "b", 6, vec![binding(COLOR, 1)]),
        ]);
        let mut by_asset = ByAsset::default();
        by_asset.note_material(&scene, &exact, 1);
        assert_eq!(by_asset.keys.get(&1), Some(&(6, 0)));
        assert_eq!(
            by_asset.colors.get(&1).map(Handle::id),
            Some(exact[1].as_ref().unwrap().id())
        );
        assert!(!by_asset.keys.contains_key(&0));
    }

    #[test]
    fn a_material_without_a_resident_colour_map_still_records_its_key() {
        let scene = scene_with(vec![mat_row(0, "a", 3, vec![binding(COLOR, 0)])]);
        let mut by_asset = ByAsset::default();
        by_asset.note_material(&scene, &[None], 0);
        assert_eq!(by_asset.keys.get(&0), Some(&(3, 0)));
        assert!(by_asset.colors.is_empty());
    }

    #[test]
    fn an_asset_id_the_catalog_does_not_know_records_nothing() {
        let scene = scene_with(vec![mat_row(0, "a", 3, Vec::new())]);
        let mut by_asset = ByAsset::default();
        by_asset.note_material(&scene, &[], 5);
        assert!(by_asset.keys.is_empty() && by_asset.colors.is_empty());

        // A row sitting at an index other than its own asset id is not
        // that material.
        let scene = scene_with(vec![mat_row(3, "stray", 3, Vec::new())]);
        by_asset.note_material(&scene, &[], 0);
        assert!(by_asset.keys.is_empty());
    }

    #[test]
    fn with_no_tracers_or_effects_nothing_is_bound() {
        let scene = scene_with(vec![mat_row(0, "a", 3, vec![binding(COLOR, 0)])]);
        let bindings = collect_bindings(&scene, &[None], None, None);
        assert!(bindings.sprites.colors.is_empty() && bindings.sprites.keys.is_empty());
        assert!(bindings.marks.colors.is_empty() && bindings.marks.keys.is_empty());
        assert_eq!(
            (bindings.tracer_maps, bindings.tracer_refs, bindings.visual_refs, bindings.mark_refs),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn a_world_without_effects_has_no_effect_colour_maps() {
        let scene = scene_with(vec![mat_row(0, "a", 3, vec![binding(COLOR, 0)])]);
        let images = world_color_images(&scene, &[None], None, None);
        assert!(images.is_empty());
    }

    #[test]
    fn the_effect_model_plan_carries_the_material_generation_even_when_empty() {
        let scene = scene_with(Vec::new());
        let plan = prepare_fx_model_geometry(&scene, None);
        assert_eq!(plan.0.generation, 9);
        assert_eq!(plan.0.emitted, 0);

        // Models are on offer but the world has no model lighting atlas yet.
        let models = render_fx::ReadyFxProps::default();
        let plan = prepare_fx_model_geometry(&scene, Some(&models));
        assert_eq!(plan.0.generation, 9);
        assert_eq!(plan.0.emitted, 0);
    }

    #[test]
    fn installing_registers_the_typed_image_pools_under_the_materials_generation() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let scene = scene_with(vec![mat_row(0, "a", 3, vec![binding(COLOR, 0)])]);
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 2);
        let expected_ids: Vec<_> = exact.iter().flatten().map(Handle::id).collect();
        let probes = vec![Some(images.add(Image::default())), None, None];

        world
            .run_system_once(move |mut commands: Commands, store: Res<Assets<Image>>| {
                register_into(
                    &mut commands,
                    &scene,
                    &store,
                    exact.clone(),
                    probes.clone(),
                    vec![None],
                    None,
                    None,
                );
            })
            .unwrap();

        let handles = world.resource::<LiveImageHandles>();
        assert_eq!(handles.revision_stamp, MaterialEpochId(9));
        assert_eq!(handles.shader_textures.len(), 2);
        assert_eq!(
            handles
                .shader_textures
                .iter()
                .flatten()
                .map(|handle| handle.id())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(handles.probe_images.len(), 3);
        assert_eq!(handles.lightmaps.len(), 1);
        assert!(handles.model_light.is_none());
        assert!(world.contains_resource::<EffectLevelColorImages>());
    }

    #[test]
    fn the_memory_report_counts_every_kind_of_image_without_complaint() {
        let mut images = Assets::<Image>::default();
        images.add(Image::default());
        let mut render_only = Image::default();
        render_only.asset_usage = RenderAssetUsages::RENDER_WORLD;
        images.add(render_only);
        report_image_memory(&images);
        report_image_memory(&Assets::<Image>::default());
    }
}
