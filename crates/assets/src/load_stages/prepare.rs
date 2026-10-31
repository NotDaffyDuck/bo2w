//! Prepares one match: opens the map's zone, takes the shared common set,
//! loads the map against it, and ties the weapons, effects, models and
//! materials of both together.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::*;
use crate::ReadiedMapData;
use asset_core::{AssetCategoryScope, MaterialKey};
use asset_game::TracerRoster;
use asset_material::{MaterialDefinitions, TS_2D, SEMANTIC_COLOR};
use asset_model::BodyMeshBuild;
use asset_world::LevelDrawCall;

static NEXT_PRODUCTS_ID: AtomicU64 = AtomicU64::new(1);

/// What the map zone itself added to the weapons, animations and tracers of
/// the common set.
struct MapGear {
    weapons: WeaponBuild,
    first_person_meshes: FpvMeshBuild,
    level_placed_weapons: WorldWeaponBuild,
    animation_refs: XAnimBuild,
    tracers: TracerRoster,
}

/// The weapons, bodies, animations and tracers of a match while the common
/// set and the map are merged and tied together.
struct Gear {
    weapons: WeaponBuild,
    first_person_meshes: FpvMeshBuild,
    level_placed_weapons: WorldWeaponBuild,
    shell_visual_models: ProjectileMeshBuild,
    animation_refs: XAnimBuild,
    tracers: TracerRoster,
    bodies: BodyMeshBuild,
}

impl Gear {
    /// Takes in what the map defines, then ties every weapon to the
    /// animations and models it names.
    fn merge_map(
        &mut self,
        map: MapGear,
        namespace: Option<AssetCategoryScope>,
        load_log: &mut Vec<String>,
    ) {
        if !map.level_placed_weapons.is_empty() {
            self.level_placed_weapons.take_in(map.level_placed_weapons);
        }
        if !map.tracers.is_empty() {
            self.tracers.merge_in(map.tracers);
        }
        if !map.weapons.is_empty() {
            self.weapons.merge_in(map.weapons);
        }
        self.animation_refs.merge_in(map.animation_refs);
        self.weapons.link_anim_clips(&self.animation_refs);

        // Decode the clips the weapons play now, so a missing one shows in
        // the report rather than at the first shot.
        let failed: Vec<usize> = self
            .weapons
            .bound_weapon_xanim_indices()
            .into_iter()
            .filter(|&index| self.animation_refs.node_clip(index).is_none())
            .collect();
        for &index in failed.iter().take(16) {
            load_log.push(format!(
                "weapon animation `{}` did not decode",
                self.animation_refs.label_at(index).unwrap_or("<unknown>")
            ));
        }
        let note_actions = self.weapons.resolve_notetrack_actions(&self.animation_refs);
        load_log.push(format!("weapon notetrack actions linked: {note_actions}"));

        self.first_person_meshes.take_in(map.first_person_meshes);
        self.first_person_meshes.set_map_namespace(namespace);
        self.weapons.link_fpv_meshes(&self.first_person_meshes);
        load_log.push(self.weapons.resolve_fpv_hands(&self.first_person_meshes, &self.bodies));
        let assemblies = self
            .weapons
            .resolve_fpv_assemblies(&self.first_person_meshes, &self.animation_refs);
        load_log.push(format!(
            "first-person assemblies: {} built, {} refused",
            assemblies.built, assemblies.refused
        ));

        self.level_placed_weapons.seal_identity();
        self.weapons.link_world_meshes(&self.level_placed_weapons);
        let gaps = self.weapons.dependency_gaps();
        load_log.push(format!("weapon dependency gaps: {}", gaps.len()));
        for missing in &gaps {
            load_log.push(format!(
                "weapon dependency gap: {} {} `{}`",
                self.weapons.label_for(missing.id),
                missing.kind,
                missing.name
            ));
        }
    }
}

pub(super) async fn walk_prepared_match(
    zone_archive_file: Result<PathBuf, String>,
    shared_zone: Result<PathBuf, String>,
    load_progress: LoadCompletion,
) -> (MatchLoadOutcome, Option<Arc<CommonSet>>) {
    let zone_stem = zone_archive_file
        .as_ref()
        .ok()
        .and_then(|zone_path| zone_path.file_stem())
        .map(|base| base.to_string_lossy().into_owned())
        .unwrap_or_default();
    let map_open = loader_pool().spawn(mount_zone(zone_archive_file, load_progress.clone()));

    let key = CommonKey::for_match(shared_zone.as_ref().ok().map(PathBuf::as_path));
    let waiting = load_progress.begin_scoped(StageId::CommonAssets, "shared common", None);
    let (common, reach) = ensure_common(key).await;
    waiting.done();
    if load_progress.was_aborted() {
        diag::info!(
            World,
            "match walk: canceled while the common set was prepared — load retargeted"
        );
        return (MatchLoadOutcome::Canceled, None);
    }

    let cloning = Instant::now();
    let CommonProducts {
        scripts: common_scripts,
        material_seed: mat_seed,
        pooled_skins,
        stage_models: mut common_scene_models,
        light_defs: common_light_defs,
        pen_rows: shared_pen,
        wall_pierce_table_ready: common_pen_loaded,
        lochit_table: common_lochit_table,
        tracers,
        model_traversal,
        film_visions: mut shared_film_visions,
        weapons,
        first_person_meshes,
        level_placed_weapons,
        shell_visual_models,
        animation_refs,
        fx: shared_fx,
        effect_meshes: shared_fx_models,
        impact_effect: common_impact,
        strings,
        load_log: mut shared_notes,
        translation_summary,
        ..
    } = common.stage_outputs.clone();
    shared_notes.push(format!(
        "common set: {reach} for {} (prepared in {:.0}ms, {:.1}s ago); match copy {:.0}ms",
        common.key,
        common.prepared_ms,
        common.ready_at.elapsed().as_secs_f32(),
        cloning.elapsed().as_secs_f32() * 1000.0,
    ));

    let opened_map = map_open.await;
    if load_progress.was_aborted() {
        diag::info!(
            World,
            "match walk: canceled before the map walk — load retargeted"
        );
        return (MatchLoadOutcome::Canceled, None);
    }
    let (read_back, level_scope) = match opened_map {
        Ok((zone_path, zone_image)) => {
            let zone_game = zone_image.game;
            let loaded = zone_io(zone_game).read_level_geometry(
                &zone_path,
                &zone_image,
                &load_progress,
                pooled_skins,
                mat_seed,
                &mut shared_film_visions,
            );
            (loaded, Some(AssetCategoryScope::from_zone_game(zone_game)))
        }
        Err(gap) => (
            ResidentLevel::with_gap(LevelRenderRules::default(), gap),
            None,
        ),
    };
    let namespace = level_scope.unwrap_or(AssetCategoryScope::T6);

    let ResidentLevel {
        scripts: map_scripts,
        mut world,
        mat_list: materials,
        collision,
        spawns,
        bodies,
        first_person_meshes: map_fpv,
        animation_refs: map_xanims,
        weapons: map_weapons,
        level_placed_weapons: map_world_weapons,
        tracers: map_tracers,
        pen_table: map_pen_table,
        lochit_table: map_lochit_table,
        facts,
        sound,
        load_log: mut notes,
        ..
    } = read_back;
    notes.append(&mut shared_notes);

    let mut scripts = common_scripts;
    scripts.overlay(map_scripts);
    notes.push(format!(
        "GSC source assets: {} (map overrides common_mp)",
        scripts.len()
    ));

    // The penetration depths and hit-location damage table come with the
    // map's zones when the common set did not carry them.
    let (pen_table, wall_pierce_table_ready) = match (common_pen_loaded, map_pen_table) {
        (false, Some(table)) => (table, true),
        _ => (shared_pen, common_pen_loaded),
    };
    let lochit_table = common_lochit_table.or(map_lochit_table);

    let mut gear = Gear {
        weapons,
        first_person_meshes,
        level_placed_weapons,
        shell_visual_models,
        animation_refs,
        tracers,
        bodies,
    };
    notes.extend(gear.bodies.summary_rows());
    gear.merge_map(
        MapGear {
            weapons: map_weapons,
            first_person_meshes: map_fpv,
            level_placed_weapons: map_world_weapons,
            animation_refs: map_xanims,
            tracers: map_tracers,
        },
        level_scope,
        &mut notes,
    );

    let scene_names = scripts.asset_names();
    common_scene_models.retain_names(&scene_names);
    world
        .map_figure_stage_assets
        .take_in_captured(common_scene_models);

    // The map's materials were numbered in its own pool; publishing the pool
    // renumbers them, and `map_ids` carries each old number to its new one.
    let map_material_count = materials.materials.len();
    let (mut glob_set, completed_keys) = materials.publish();
    let map_ids: Vec<Option<usize>> = (0..map_material_count)
        .map(|id| completed_keys.get(id).copied().flatten())
        .collect();

    decode_images(
        &mut glob_set,
        &mut world,
        &common_light_defs,
        &gear.tracers,
        namespace,
        &load_progress,
        &mut notes,
    );
    link_effects(&mut world, shared_fx, shared_fx_models, &mut gear);
    link_materials(&glob_set, &mut world, &mut gear);
    decode_fx_colour_maps(&world.fx, &mut glob_set, &load_progress);
    world.impact_effect = world.impact_effect.take().or(common_impact);
    if world.impact_effect.is_none() {
        notes.push("impact effects missing — combat hits will find no cells".into());
    }

    if facts.compass_heading.is_none() {
        notes.push("compass gap: worldspawn has no northyaw — map is drawn north-up".into());
    }
    notes.extend(translation_summary);

    let mut fx = std::mem::take(&mut world.fx).publish();
    fx.set_map_namespace(namespace);
    let prepared = ReadiedSession {
        scripts,
        fx,
        world,
        materials: crate::MatchMaterials {
            population: Arc::new(glob_set),
            map_ids,
            common_profile_id: common.id,
            products_id: NEXT_PRODUCTS_ID.fetch_add(1, Ordering::Relaxed),
        },
        clip: collision.map(Arc::new),
        weapons: Arc::new(gear.weapons.publish()),
        first_person_meshes: gear.first_person_meshes.publish(),
        bodies: Arc::new(gear.bodies.publish()),
        level_placed_weapons: gear.level_placed_weapons.publish(),
        shell_visual_models: gear.shell_visual_models.publish(),
        animation_refs: gear.animation_refs.publish(),
        destructible_death: Vec::new(),
        tracers: gear.tracers.publish(),
        strings,
        report: notes,
        prepared_map: ReadiedMapData {
            zone: zone_stem,
            namespace: level_scope,
            spawns,
            facts,
        },
        pen_table,
        wall_pierce_table_ready,
        lochit_table,
        model_traversal,
        sound,
    };
    (MatchLoadOutcome::Ready(prepared), Some(common))
}

/// Opens the map's zone file, to be shared with the walk that reads it.
async fn mount_zone(
    zone_archive_file: Result<PathBuf, String>,
    progress: LoadCompletion,
) -> Result<(PathBuf, Arc<asset_transport::zone::ZoneTextureData>), String> {
    let path = match zone_archive_file {
        Ok(path) => path,
        Err(error) => {
            progress.record_skipped_scoped(StageId::MapAssets, "open");
            return Err(format!("zone not found: {error}"));
        }
    };
    let stage = progress.begin_scoped(StageId::MapAssets, "open", None);
    progress.start_zone_unpack();
    let mounted = open_shared_zone(&path);
    finish_zone_open(stage, &mounted);
    let image = mounted.map_err(|error| format!("open zone: {error}"))?;
    progress.log_zone_texture_sizes(image.bytes.len());
    Ok((path, image))
}

/// Decodes the images of the merged materials, the map's light attenuation
/// ramps and the tracer beams.
fn decode_images(
    global: &mut MaterialDefinitions,
    world: &mut StagedLevel,
    shared_light_rows: &[asset_world::SnapshotLightDef],
    tracers: &TracerRoster,
    namespace: AssetCategoryScope,
    progress: &LoadCompletion,
    load_log: &mut Vec<String>,
) {
    let stage = progress.begin_scoped(StageId::Images, "merged", None);
    let tally = asset_material::unpack_surface_tint_maps(global, &stage, loader_pool());
    stage.done();
    load_log.push(format!(
        "material images merged: {}/{} decoded / {} absent / {} of unknown format",
        tally.decoded, tally.requested, tally.missing, tally.unsupported
    ));

    if let Some(draw) = world.draw.as_mut() {
        resolve_world_lights(draw, global, shared_light_rows, namespace, progress, load_log);
    }
    asset_material::decode_in_zone_builtin_images(global);

    let stage = progress.begin_scoped(StageId::Images, "tracers", None);
    asset_material::decode_color_or_2d_for_keys(
        global,
        tracers.material_keys(),
        &stage,
        loader_pool(),
    );
    stage.done();
}

/// Finds the light definitions the map's lights name, decodes their
/// attenuation ramps, and settles which image is the outdoor lookup.
fn resolve_world_lights(
    draw: &mut LevelDrawCall,
    global: &mut MaterialDefinitions,
    common_light_defs: &[asset_world::SnapshotLightDef],
    namespace: AssetCategoryScope,
    progress: &LoadCompletion,
    load_log: &mut Vec<String>,
) {
    asset_world::resolve_primary_light_attenuation(draw, global, common_light_defs);

    let (light_ord, source) =
        asset_world::locate_exterior_texture(draw.exterior_texture_label.as_deref(), namespace, global);
    draw.openair_image = light_ord;
    if source == "$outdoor" && draw.exterior_texture_label.is_none() {
        draw.exterior_texture_label = Some("$outdoor".into());
    }
    load_log.push(format!(
        "outdoor image: source={source} name={} ordinal={}",
        draw.exterior_texture_label.as_deref().unwrap_or("-"),
        draw.openair_image
            .map_or_else(|| "none".to_owned(), |ordinal| ordinal.to_string()),
    ));

    let ramps: Vec<(usize, u8)> = draw
        .key_lights
        .iter()
        .filter_map(|key_light| Some((key_light.falloff_texture?, key_light.falloff_lookup)))
        .collect();
    let stage = progress.begin_scoped(StageId::Images, "attenuation", None);
    asset_material::decode_catalog_images(global, ramps, &stage, loader_pool());
    stage.done();
    asset_world::resolve_primary_light_attenuation(draw, global, common_light_defs);
}

/// Folds the common set's effects into the map's and ties them to the
/// models, weapons and tracers that use them.
fn link_effects(
    world: &mut StagedLevel,
    common_fx: EffectRoster,
    common_fx_models: asset_game::EffectMeshRoster,
    gear: &mut Gear,
) {
    world.fx.merge_in(common_fx);
    world.effect_meshes.merge_in(common_fx_models);
    world.effect_meshes.retain_wanted(&world.fx.model_hints());
    world.fx.link_effect_meshes(&world.effect_meshes);
    gear.weapons.resolve_combat_fx(&world.fx, &gear.tracers);
    gear.weapons.link_projectile_effects(&world.fx);
}

/// Ties everything that draws with a material to the merged material pool.
fn link_materials(mat_list: &MaterialDefinitions, world: &mut StagedLevel, gear: &mut Gear) {
    world.effect_meshes.link_surfaces(mat_list);
    gear.weapons.link_hud_materials(mat_list);
    gear.tracers.link_surfaces(mat_list);
    world.fx.link_surfaces(mat_list);
    world.fx.flatten_child_edges();
    gear.bodies.bind_materials(mat_list);
    gear.level_placed_weapons.bind_materials(mat_list);
    gear.first_person_meshes.bind_materials(mat_list);
    gear.shell_visual_models.bind_materials(mat_list);
}

/// Decodes the colour image of each material an effect draws with, when the
/// pool has not decoded it yet.
fn decode_fx_colour_maps(
    fx: &EffectRoster,
    global: &mut MaterialDefinitions,
    progress: &LoadCompletion,
) {
    let missing: Vec<MaterialKey> = fx
        .distinct_binding_clues()
        .into_iter()
        .chain(fx.distinct_decal_hints())
        .filter(|&(index, _)| !has_decoded_colour(global, index))
        .filter_map(|(index, _)| {
            let source = global.materials.get(index)?;
            Some(MaterialKey {
                namespace: source.namespace,
                name: source.name.as_str().to_owned(),
            })
        })
        .collect();
    if missing.is_empty() {
        return;
    }
    let stage = progress.begin_scoped(StageId::Images, "fx_elem", None);
    asset_material::decode_color_or_2d_for_keys(
        global,
        missing,
        &stage,
        loader_pool(),
    );
    stage.done();
}

/// Whether the material's colour map (or, failing that, its 2D image) has
/// been decoded.
fn has_decoded_colour(global: &MaterialDefinitions, material: usize) -> bool {
    let Some(material) = global.materials.get(material) else {
        return false;
    };
    let image_of = |semantic: u8| {
        material
            .textures
            .iter()
            .find(|texture| texture.semantic == semantic)
            .and_then(|texture| texture.image)
    };
    image_of(SEMANTIC_COLOR)
        .or_else(|| image_of(TS_2D))
        .and_then(|index| global.images.get(index))
        .is_some_and(|image| image.decoded.is_some())
}
