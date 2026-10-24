//! Once a world's images are resident: the colour maps the effects draw
//! with, the geometry of the effects' models, and the typed image registry
//! the draw-surface stage looks textures up in.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use render_fx::{FxWorldColorImages, PreparedFxCatalog, PreparedTracers};

use crate::assemble::drawsurf::{RuntimeImageHandles, RuntimeLightmapHandles, RuntimeMaterial};

use super::world::WorldScene;

const MIB: f64 = 1024.0 * 1024.0;

/// A material's sort key and its (unused here) second key half.
type MaterialKey = (u8, u16);

fn log_image_asset_memory(images: &Assets<Image>) {
    let (mut count, mut main_world, mut render_world) = (0usize, 0usize, 0usize);
    for (_, image) in images.iter() {
        let bytes = image.data.as_ref().map_or(0, Vec::len);
        count += 1;
        if image.asset_usage.contains(RenderAssetUsages::MAIN_WORLD) {
            main_world += bytes;
        }
        if image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) {
            render_world += bytes;
        }
    }
    diag::info!(
        World,
        "image asset memory: assets={count} main_world_retained={:.1}MiB uploaded={:.1}MiB rss={:.0}MiB",
        main_world as f64 / MIB,
        render_world as f64 / MIB,
        asset_transport::process_resident_bytes().map_or(0.0, |bytes| bytes as f64 / MIB),
    );
}

/// The exact-pool image a material's colour map (or, failing that, its
/// plain 2D texture) points at.
fn colour_handle(
    material: &RuntimeMaterial,
    exact: &[Option<Handle<Image>>],
) -> Option<Handle<Image>> {
    let binding = material
        .texture_semantic(asset_material::TS_COLOR_MAP)
        .or_else(|| material.texture_semantic(asset_material::TS_2D))?;
    exact.get(binding.0 as usize).cloned().flatten()
}

/// The colour maps and sort keys of the materials effects bind by asset id.
#[derive(Default)]
struct ByAsset {
    colors: HashMap<usize, Handle<Image>>,
    keys: HashMap<usize, MaterialKey>,
}

impl ByAsset {
    /// Records the material of `asset_id`: its key always, its colour map if
    /// it has one resident.
    fn add(&mut self, scene: &WorldScene, exact: &[Option<Handle<Image>>], asset_id: usize) {
        let catalog = &scene.runtime_material_catalog;
        let Some(material) = catalog.derived(assets::MaterialIndex::from_order(asset_id)) else {
            return;
        };
        self.keys.insert(asset_id, (material.sort_key, 0));
        if let Some(handle) = colour_handle(material, exact) {
            self.colors.insert(asset_id, handle);
        }
    }
}

/// What the effect catalogs say about which materials need a colour map, and
/// how many bindings said so.
struct Bindings {
    sprites: ByAsset,
    marks: ByAsset,
    tracer_unique: usize,
    tracer_bound: usize,
    elem_bound: usize,
    mark_bound: usize,
}

fn collect_bindings(
    scene: &WorldScene,
    exact: &[Option<Handle<Image>>],
    tracers: Option<&PreparedTracers>,
    fx_catalog: Option<&PreparedFxCatalog>,
) -> Bindings {
    let mut sprites = ByAsset::default();
    let mut tracer_bound = 0;
    let bound_tracers = tracers
        .into_iter()
        .flat_map(|tracers| tracers.0.defs())
        .filter_map(|def| def.material.bound_index());
    for asset_id in bound_tracers {
        tracer_bound += 1;
        sprites.add(scene, exact, asset_id);
    }
    let tracer_unique = sprites.colors.len();

    let mut elem_bound = 0;
    let visuals = fx_catalog
        .into_iter()
        .flat_map(|fx| fx.0.effects())
        .flat_map(|effect| &effect.elems)
        .flat_map(|elem| &elem.visuals)
        .filter_map(|visual| visual.bound_index());
    for asset_id in visuals {
        elem_bound += 1;
        sprites.add(scene, exact, asset_id);
    }

    let mut marks = ByAsset::default();
    let mut mark_bound = 0;
    for (asset_id, _) in fx_catalog
        .into_iter()
        .flat_map(|fx| fx.0.unique_decal_mark_hints())
    {
        mark_bound += 1;
        marks.add(scene, exact, asset_id);
    }
    Bindings {
        sprites,
        marks,
        tracer_unique,
        tracer_bound,
        elem_bound,
        mark_bound,
    }
}

/// Sprite bindings whose material has no colour map, as `id:name` rows split
/// into distortion ones (which need none) and the rest. Logged only.
fn log_unmapped_bindings(
    scene: &WorldScene,
    fx_catalog: &PreparedFxCatalog,
    sprites: &ByAsset,
) {
    let rows: Vec<String> = fx_catalog
        .0
        .unique_bound_hints()
        .into_iter()
        .filter(|(asset_id, _)| !sprites.colors.contains_key(asset_id))
        .map(|(asset_id, hint)| {
            let name = scene
                .runtime_material_catalog
                .materials
                .get(asset_id)
                .map_or(hint, |material| material.name.as_str());
            format!("{asset_id}:{name}")
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    let total = rows.len();
    let (distortion, other): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| row.contains("distortion"));
    let sample = if other.is_empty() {
        "-".to_string()
    } else {
        other.join("; ")
    };
    diag::info!(
        World,
        "fx elem Bound without color map: {total} of {} unique (distortion={}, other={}) other_sample: {sample}",
        fx_catalog.0.material_visual_unique_bound_count(),
        distortion.len(),
        other.len(),
    );
}

pub(crate) fn world_color_images(
    scene: &WorldScene,
    exact_material_handles: &[Option<Handle<Image>>],
    tracers: Option<&PreparedTracers>,
    fx_catalog: Option<&PreparedFxCatalog>,
) -> FxWorldColorImages {
    let Bindings {
        sprites,
        marks,
        tracer_unique,
        tracer_bound,
        elem_bound,
        mark_bound,
    } = collect_bindings(scene, exact_material_handles, tracers, fx_catalog);

    // Effects that name their material look the map up by that name.
    let mut colors_by_name = HashMap::<String, Handle<Image>>::new();
    let mut keys_by_name = HashMap::<String, MaterialKey>::new();
    for (asset_id, handle) in sprites.colors.iter().chain(&marks.colors) {
        let catalog = &scene.runtime_material_catalog;
        let Some(material) = catalog.derived(assets::MaterialIndex::from_order(*asset_id)) else {
            continue;
        };
        asset_game::insert_fx_color_image(&mut colors_by_name, &material.name, handle.clone());
        asset_game::insert_fx_color_image(&mut keys_by_name, &material.name, (material.sort_key, 0));
    }
    let stub_aliases = asset_game::alias_fx_color_map_stubs(&mut colors_by_name);
    asset_game::alias_fx_color_map_stubs(&mut keys_by_name);

    diag::info!(
        World,
        "fx color maps: {} bind keys (stub_aliases={stub_aliases}, material_keys={}) — handles from exact pool, not CPU clones",
        colors_by_name.len(),
        keys_by_name.len()
    );
    diag::info!(
        World,
        "fx tracer color maps by asset: {tracer_unique} unique / {tracer_bound} bound defs"
    );
    diag::info!(
        World,
        "fx elem color maps by asset: {} unique / {elem_bound} bound visuals",
        sprites.colors.len()
    );
    diag::info!(
        World,
        "fx mark color maps by asset: {} unique / {mark_bound} unique Bound — not sprite colors_by_asset",
        marks.colors.len()
    );
    if let Some(fx) = fx_catalog {
        log_unmapped_bindings(scene, fx, &sprites);
    }

    FxWorldColorImages {
        motion_tracker: Default::default(),
        colors: colors_by_name,
        keys: keys_by_name,
        colors_by_asset: sprites.colors,
        keys_by_asset: sprites.keys,
        mark_colors_by_asset: marks.colors,
        mark_keys_by_asset: marks.keys,
    }
}

/// The bind-pose skinning of one effect model: the object, what resolved it,
/// and the skin matrices. None if any step refuses.
fn bind_pose_skin(
    name: &str,
    entry: &asset_game::FxModelEntry,
) -> Option<(
    xmodel_runtime::DObj,
    xmodel_runtime::DObjPoseRequest,
    Vec<Mat4>,
)> {
    let pose = entry.skel.pose.as_ref()?;
    let dobj = xmodel_runtime::DObj::build(&[(pose, None)]).ok()?;
    let state = xmodel_runtime::DObjSemanticState::bind_pose(name.to_owned(), 1, 1);
    let request = state.resolve_request(|_| None).ok()?;
    let world = xmodel_runtime::pose_dobj(&dobj, &request, Mat4::IDENTITY).ok()?;
    let skin = dobj.skin_matrices(&world);
    Some((dobj, request, skin))
}

pub(crate) fn prepare_fx_model_geometry(
    scene: &WorldScene,
    models: Option<&render_fx::PreparedFxModels>,
) -> render_fx::PreparedFxModelGeometry {
    let mut plan = render_fx::FxModelDrawPlan::default();
    plan.generation = scene.runtime_material_catalog.generation_id.0;
    let (Some(models), Some((image, dims))) = (
        models,
        scene
            .model_lighting_image
            .clone()
            .zip(scene.model_lighting_dims),
    ) else {
        return render_fx::PreparedFxModelGeometry(plan);
    };
    let atlas = super::model_lighting_atlas::WorldModelLightingAtlas { image, dims };
    let models = &models.0;

    for index in 0..models.len() {
        let Some(entry) = models.get_at(index) else {
            continue;
        };
        let name = models.name_at(index).unwrap_or_default();
        let Some((dobj, request, skin)) = bind_pose_skin(name, entry) else {
            diag::warn!(World, "FX model `{name}` refused: bind pose unavailable");
            continue;
        };
        let lod_count = entry
            .skel
            .lod
            .map_or(1, |selector| selector.num_lods().clamp(0, 4) as u8);
        for lod in 0..lod_count {
            let Some(surfaces) = render_anim::anim::xmodel_pose::skin_model_filtered(
                &entry.skel,
                |bone| skin[bone],
                &[],
                |surface| {
                    dobj.surface_visible(
                        0,
                        &entry.skel.surface_part_bits[surface],
                        &request.hide_part_bits,
                    )
                },
                |surface| {
                    render_anim::anim::xmodel_pose::stream_lod_surface_rigid(
                        &entry.skel,
                        lod,
                        &[],
                        0,
                        surface,
                    )
                },
                lod,
            ) else {
                diag::warn!(World, "FX model `{name}` LOD {lod} refused: geometry unavailable");
                continue;
            };
            let materials: Vec<_> = surfaces
                .iter()
                .map(|surface| {
                    crate::assemble::drawsurf::tess::xmodel::bound_lit_xmodel_pass_material(
                        &atlas,
                        &scene.runtime_material_catalog,
                        entry.material_index(surface.surface_index)?,
                    )
                })
                .collect();
            if materials.iter().any(Option::is_none) {
                diag::warn!(
                    World,
                    "FX model `{name}` LOD {lod} refused: material binding unavailable"
                );
                continue;
            }
            render_fx::append_fx_model_asset(&mut plan, index, lod, &surfaces, &materials);
        }
    }
    render_fx::PreparedFxModelGeometry(plan)
}

pub fn install(
    commands: &mut Commands,
    scene: &WorldScene,
    images: &Assets<Image>,
    exact_material_handles: Vec<Option<Handle<Image>>>,
    reflection_probe_handles: Vec<Option<Handle<Image>>>,
    lightmap_handles: Vec<Option<RuntimeLightmapHandles>>,
    tracers: Option<&PreparedTracers>,
    fx_catalog: Option<&PreparedFxCatalog>,
) {
    commands.insert_resource(world_color_images(
        scene,
        &exact_material_handles,
        tracers,
        fx_catalog,
    ));

    let materials = exact_material_handles.iter().flatten().count();
    let probes = reflection_probe_handles.iter().flatten().count();
    let exact_lightmaps = lightmap_handles
        .iter()
        .flatten()
        .filter(|page| page.primary.is_some() && page.secondary.is_some())
        .count();
    let (material_slots, probe_slots, lightmap_slots) = (
        exact_material_handles.len(),
        reflection_probe_handles.len(),
        lightmap_handles.len(),
    );
    commands.insert_resource(RuntimeImageHandles::from_pools(
        scene.runtime_material_catalog.generation_id,
        exact_material_handles,
        scene.exact_material_names.clone(),
        reflection_probe_handles,
        lightmap_handles,
        None,
    ));
    log_image_asset_memory(images);
    diag::info!(
        World,
        "drawsurf typed image registry: material={materials}/{material_slots} probe={probes}/{probe_slots} lightmap_exact={exact_lightmaps}/{lightmap_slots}"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asset_core::AssetNamespace;
    use bevy::ecs::system::RunSystemOnce;
    use render_material::{
        MaterialAssetId, MaterialGenerationId, RemapResolution, RuntimeImageId, RuntimeMaterial,
        RuntimeMaterialCatalog, RuntimeTechniqueSetId, RuntimeTextureBinding, TECHNIQUE_SLOT_COUNT,
    };

    use super::*;

    const COLOR: u8 = asset_material::TS_COLOR_MAP;
    const PLAIN: u8 = asset_material::TS_2D;

    fn binding(semantic: u8, image: u32) -> (u32, Option<RuntimeTextureBinding>) {
        (
            u32::from(semantic) + 100,
            Some(RuntimeTextureBinding {
                image: RuntimeImageId(image),
                sampler_state: 0,
                semantic,
            }),
        )
    }

    fn material(
        id: u16,
        name: &str,
        sort_key: u8,
        textures: Vec<(u32, Option<RuntimeTextureBinding>)>,
    ) -> RuntimeMaterial {
        RuntimeMaterial {
            asset_id: MaterialAssetId(id),
            name: name.to_owned(),
            namespace: AssetNamespace::T6,
            technique_set: String::new(),
            baked_draw_surf: None,
            local_technique_set: RuntimeTechniqueSetId(0),
            remap: RemapResolution::SelfSet,
            state_bits_entry: None::<[u8; TECHNIQUE_SLOT_COUNT]>,
            state_bits_table: Vec::new(),
            camera_region: 0,
            sort_key,
            info_game_flags: 0,
            state_flags: 0,
            surface_type_bits: None,
            unlit: false,
            uses_model_lighting_const: false,
            square_color_map: false,
            shadow_only: false,
            t6_draw: None,
            cull_mode: None,
            uv_anim_bits: [0; 4],
            falloff_parms_bits: [0; 4],
            falloff_begin_bits: [0; 4],
            falloff_end_bits: [0; 4],
            env_map_parms_bits: [0; 4],
            textures,
            constants: Vec::new(),
        }
    }

    fn scene_with(materials: Vec<RuntimeMaterial>) -> WorldScene {
        WorldScene {
            runtime_material_catalog: Arc::new(RuntimeMaterialCatalog {
                generation_id: MaterialGenerationId(9),
                materials,
                ..RuntimeMaterialCatalog::default()
            }),
            ..WorldScene::default()
        }
    }

    /// An exact pool whose slot `i` holds a distinct image.
    fn pool(images: &mut Assets<Image>, n: usize) -> Vec<Option<Handle<Image>>> {
        (0..n).map(|_| Some(images.add(Image::default()))).collect()
    }

    #[test]
    fn a_colour_map_binding_picks_the_image_in_its_pool_slot() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = material(0, "m", 4, vec![binding(COLOR, 2)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[2].as_ref().unwrap().id());
    }

    #[test]
    fn the_colour_map_wins_over_a_plain_texture() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = material(0, "m", 4, vec![binding(PLAIN, 0), binding(COLOR, 1)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[1].as_ref().unwrap().id());
    }

    #[test]
    fn a_material_with_only_a_plain_texture_falls_back_to_it() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 3);
        let mat = material(0, "m", 4, vec![binding(PLAIN, 0)]);
        assert_eq!(colour_handle(&mat, &exact).unwrap().id(), exact[0].as_ref().unwrap().id());
    }

    #[test]
    fn no_colour_handle_without_a_binding_or_a_resident_image() {
        let mut images = Assets::<Image>::default();
        let mut exact = pool(&mut images, 3);
        // Neither kind of binding.
        assert!(colour_handle(&material(0, "m", 4, vec![binding(7, 0)]), &exact).is_none());
        assert!(colour_handle(&material(0, "m", 4, Vec::new()), &exact).is_none());
        // A texture slot with nothing bound to it.
        assert!(colour_handle(&material(0, "m", 4, vec![(1, None)]), &exact).is_none());
        // Bound to an image that is not resident, or is not in the pool.
        exact[1] = None;
        assert!(colour_handle(&material(0, "m", 4, vec![binding(COLOR, 1)]), &exact).is_none());
        assert!(colour_handle(&material(0, "m", 4, vec![binding(COLOR, 40)]), &exact).is_none());
    }

    #[test]
    fn a_material_is_recorded_by_its_asset_id_with_its_sort_key() {
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 2);
        let scene = scene_with(vec![
            material(0, "a", 3, vec![binding(COLOR, 0)]),
            material(1, "b", 6, vec![binding(COLOR, 1)]),
        ]);
        let mut by_asset = ByAsset::default();
        by_asset.add(&scene, &exact, 1);
        assert_eq!(by_asset.keys.get(&1), Some(&(6, 0)));
        assert_eq!(
            by_asset.colors.get(&1).map(Handle::id),
            Some(exact[1].as_ref().unwrap().id())
        );
        assert!(!by_asset.keys.contains_key(&0));
    }

    #[test]
    fn a_material_without_a_resident_colour_map_still_records_its_key() {
        let scene = scene_with(vec![material(0, "a", 3, vec![binding(COLOR, 0)])]);
        let mut by_asset = ByAsset::default();
        by_asset.add(&scene, &[None], 0);
        assert_eq!(by_asset.keys.get(&0), Some(&(3, 0)));
        assert!(by_asset.colors.is_empty());
    }

    #[test]
    fn an_asset_id_the_catalog_does_not_know_records_nothing() {
        let scene = scene_with(vec![material(0, "a", 3, Vec::new())]);
        let mut by_asset = ByAsset::default();
        by_asset.add(&scene, &[], 5);
        assert!(by_asset.keys.is_empty() && by_asset.colors.is_empty());

        // A row sitting at an index other than its own asset id is not
        // that material.
        let scene = scene_with(vec![material(3, "stray", 3, Vec::new())]);
        by_asset.add(&scene, &[], 0);
        assert!(by_asset.keys.is_empty());
    }

    #[test]
    fn with_no_tracers_or_effects_nothing_is_bound() {
        let scene = scene_with(vec![material(0, "a", 3, vec![binding(COLOR, 0)])]);
        let bindings = collect_bindings(&scene, &[None], None, None);
        assert!(bindings.sprites.colors.is_empty() && bindings.sprites.keys.is_empty());
        assert!(bindings.marks.colors.is_empty() && bindings.marks.keys.is_empty());
        assert_eq!(
            (
                bindings.tracer_unique,
                bindings.tracer_bound,
                bindings.elem_bound,
                bindings.mark_bound
            ),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn a_world_without_effects_has_no_effect_colour_maps() {
        let scene = scene_with(vec![material(0, "a", 3, vec![binding(COLOR, 0)])]);
        let images = world_color_images(&scene, &[None], None, None);
        assert!(images.colors.is_empty() && images.keys.is_empty());
        assert!(images.colors_by_asset.is_empty() && images.keys_by_asset.is_empty());
        assert!(images.mark_colors_by_asset.is_empty() && images.mark_keys_by_asset.is_empty());
    }

    #[test]
    fn the_effect_model_plan_carries_the_material_generation_even_when_empty() {
        let scene = scene_with(Vec::new());
        let plan = prepare_fx_model_geometry(&scene, None);
        assert_eq!(plan.0.generation, 9);
        assert_eq!(plan.0.generated, 0);

        // Models are on offer but the world has no model lighting atlas yet.
        let models = render_fx::PreparedFxModels::default();
        let plan = prepare_fx_model_geometry(&scene, Some(&models));
        assert_eq!(plan.0.generation, 9);
        assert_eq!(plan.0.generated, 0);
    }

    #[test]
    fn installing_registers_the_typed_image_pools_under_the_materials_generation() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let scene = scene_with(vec![material(0, "a", 3, vec![binding(COLOR, 0)])]);
        let mut images = Assets::<Image>::default();
        let exact = pool(&mut images, 2);
        let expected_ids: Vec<_> = exact.iter().flatten().map(Handle::id).collect();
        let probes = vec![Some(images.add(Image::default())), None, None];

        world
            .run_system_once(move |mut commands: Commands, store: Res<Assets<Image>>| {
                install(
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

        let handles = world.resource::<RuntimeImageHandles>();
        assert_eq!(handles.generation_id, MaterialGenerationId(9));
        assert_eq!(handles.material_images.len(), 2);
        assert_eq!(
            handles
                .material_images
                .iter()
                .flatten()
                .map(Handle::id)
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(handles.reflection_probes.len(), 3);
        assert_eq!(handles.lightmaps.len(), 1);
        assert!(handles.model_lighting.is_none());
        assert!(world.contains_resource::<FxWorldColorImages>());
    }

    #[test]
    fn the_memory_report_counts_every_kind_of_image_without_complaint() {
        let mut images = Assets::<Image>::default();
        images.add(Image::default());
        let mut render_only = Image::default();
        render_only.asset_usage = RenderAssetUsages::RENDER_WORLD;
        images.add(render_only);
        log_image_asset_memory(&images);
        log_image_asset_memory(&Assets::<Image>::default());
    }
}
