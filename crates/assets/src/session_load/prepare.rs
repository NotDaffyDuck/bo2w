//! Prepares one match: opens the map's zone, takes the shared common set,
//! loads the map against it, and ties the weapons, effects, models and
//! materials of both together.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::*;
use crate::PreparedMap;
use asset_core::{AssetNamespace, MaterialKey};
use asset_game::TracerCatalog;
use asset_material::{MaterialDefinitions, TS_2D, TS_COLOR_MAP};
use asset_model::BodyMeshBuild;
use asset_world::WorldDraw;

static NEXT_PRODUCTS_ID: AtomicU64 = AtomicU64::new(1);

/// What the map zone itself added to the weapons, animations and tracers of
/// the common set.
struct MapGear {
    weapons: WeaponBuild,
    fpv_meshes: FpvMeshBuild,
    world_weapons: WorldWeaponBuild,
    xanims: XAnimBuild,
    tracers: TracerCatalog,
}

/// The weapons, bodies, animations and tracers of a match while the common
/// set and the map are merged and tied together.
struct Gear {
    weapons: WeaponBuild,
    fpv_meshes: FpvMeshBuild,
    world_weapons: WorldWeaponBuild,
    projectile_meshes: ProjectileMeshBuild,
    xanims: XAnimBuild,
    tracers: TracerCatalog,
    bodies: BodyMeshBuild,
}

impl Gear {
    /// Takes in what the map defines, then ties every weapon to the
    /// animations and models it names.
    fn merge_map(
        &mut self,
        map: MapGear,
        namespace: Option<AssetNamespace>,
        report: &mut Vec<String>,
    ) {
        if !map.world_weapons.is_empty() {
            self.world_weapons.absorb(map.world_weapons);
        }
        if !map.tracers.is_empty() {
            self.tracers.absorb(map.tracers);
        }
        if !map.weapons.is_empty() {
            self.weapons.absorb(map.weapons);
        }
        self.xanims.absorb(map.xanims);
        self.weapons.resolve_sz_xanim_edges(&self.xanims);

        // Decode the clips the weapons play now, so a missing one shows in
        // the report rather than at the first shot.
        let failed: Vec<usize> = self
            .weapons
            .bound_weapon_xanim_indices()
            .into_iter()
            .filter(|&index| self.xanims.clip_at(index).is_none())
            .collect();
        for &index in failed.iter().take(16) {
            report.push(format!(
                "weapon animation `{}` did not decode",
                self.xanims.name_at(index).unwrap_or("<unknown>")
            ));
        }
        let note_actions = self.weapons.resolve_notetrack_actions(&self.xanims);
        report.push(format!("weapon notetrack actions linked: {note_actions}"));

        self.fpv_meshes.absorb(map.fpv_meshes);
        self.fpv_meshes.set_map_namespace(namespace);
        self.weapons.resolve_fpv_mesh_edges(&self.fpv_meshes);
        self.weapons.resolve_fpv_hands(&self.fpv_meshes, &self.bodies);
        let assemblies = self
            .weapons
            .resolve_fpv_assemblies(&self.fpv_meshes, &self.xanims);
        report.push(format!(
            "first-person assemblies: {} built, {} refused",
            assemblies.built, assemblies.refused
        ));

        self.world_weapons.seal_identity();
        self.weapons.resolve_world_model_edges(&self.world_weapons);
        let gaps = self.weapons.dependency_gaps();
        report.push(format!("weapon dependency gaps: {}", gaps.len()));
        for gap in &gaps {
            report.push(format!(
                "weapon dependency gap: {} {} `{}`",
                self.weapons.name_of(gap.id),
                gap.kind,
                gap.name
            ));
        }
    }
}

pub(super) async fn walk_prepared_match(
    zone_ff: Result<PathBuf, String>,
    common_mp: Result<PathBuf, String>,
    progress: LoadProgress,
) -> (MatchLoadOutcome, Option<Arc<CommonSet>>) {
    let zone_name = zone_ff
        .as_ref()
        .ok()
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let map_open = load_pool().spawn(open_zone(zone_ff, progress.clone()));

    let key = CommonKey::for_match(common_mp.as_ref().ok().map(PathBuf::as_path));
    let waiting = progress.begin_scoped(StageId::CommonAssets, "shared common", None);
    let (common, reach) = ensure_common(key).await;
    waiting.done();
    if progress.is_canceled() {
        diag::info!(
            World,
            "match walk: canceled while the common set was prepared — load retargeted"
        );
        return (MatchLoadOutcome::Canceled, None);
    }

    let cloning = Instant::now();
    let CommonProducts {
        scripts: common_scripts,
        material_seed,
        shared_surfaces,
        scene_models: mut common_scene_models,
        light_defs: common_light_defs,
        pen_table: common_pen_table,
        pen_table_loaded: common_pen_loaded,
        lochit_table: common_lochit_table,
        tracers,
        xmodel_walk,
        film_visions: mut common_film_visions,
        weapons,
        fpv_meshes,
        world_weapons,
        projectile_meshes,
        xanims,
        fx: common_fx,
        fx_models: common_fx_models,
        impact_fx: common_impact,
        strings,
        report: mut common_report,
        localize_report,
        ..
    } = common.products.clone();
    common_report.push(format!(
        "common set: {reach} for {} (prepared in {:.0}ms, {:.1}s ago); match copy {:.0}ms",
        common.key,
        common.prepared_ms,
        common.ready_at.elapsed().as_secs_f32(),
        cloning.elapsed().as_secs_f32() * 1000.0,
    ));

    let opened_map = map_open.await;
    if progress.is_canceled() {
        diag::info!(
            World,
            "match walk: canceled before the map walk — load retargeted"
        );
        return (MatchLoadOutcome::Canceled, None);
    }
    let (loaded, map_namespace) = match opened_map {
        Ok((path, image)) => {
            let game = image.game;
            let loaded = lane(game).load_world(
                &path,
                &image,
                &progress,
                shared_surfaces,
                material_seed,
                &mut common_film_visions,
            );
            (loaded, Some(AssetNamespace::from_zone_game(game)))
        }
        Err(gap) => (
            LoadedWorld::with_gap(WorldDrawPolicy::default(), gap),
            None,
        ),
    };
    let namespace = map_namespace.unwrap_or(AssetNamespace::T6);

    let LoadedWorld {
        scripts: map_scripts,
        mut world,
        materials,
        collision,
        spawns,
        bodies,
        fpv_meshes: map_fpv,
        xanims: map_xanims,
        weapons: map_weapons,
        world_weapons: map_world_weapons,
        tracers: map_tracers,
        pen_table: map_pen_table,
        lochit_table: map_lochit_table,
        facts,
        sound,
        mut report,
        ..
    } = loaded;
    report.append(&mut common_report);

    let mut scripts = common_scripts;
    scripts.overlay(map_scripts);
    report.push(format!(
        "GSC source assets: {} (map overrides common_mp)",
        scripts.len()
    ));

    // The penetration depths and hit-location damage table come with the
    // map's zones when the common set did not carry them.
    let (pen_table, pen_table_loaded) = match (common_pen_loaded, map_pen_table) {
        (false, Some(table)) => (table, true),
        _ => (common_pen_table, common_pen_loaded),
    };
    let lochit_table = common_lochit_table.or(map_lochit_table);

    let mut gear = Gear {
        weapons,
        fpv_meshes,
        world_weapons,
        projectile_meshes,
        xanims,
        tracers,
        bodies,
    };
    report.extend(gear.bodies.report_lines());
    gear.merge_map(
        MapGear {
            weapons: map_weapons,
            fpv_meshes: map_fpv,
            world_weapons: map_world_weapons,
            xanims: map_xanims,
            tracers: map_tracers,
        },
        map_namespace,
        &mut report,
    );

    let scene_names = scripts.asset_names();
    common_scene_models.retain_names(&scene_names);
    world
        .map_xmodel_scene_assets
        .absorb_captured(common_scene_models);

    // The map's materials were numbered in its own pool; publishing the pool
    // renumbers them, and `map_ids` carries each old number to its new one.
    let map_material_count = materials.materials.len();
    let (mut global, finalized_ids) = materials.publish();
    let map_ids: Vec<Option<usize>> = (0..map_material_count)
        .map(|id| finalized_ids.get(id).copied().flatten())
        .collect();

    decode_images(
        &mut global,
        &mut world,
        &common_light_defs,
        &gear.tracers,
        namespace,
        &progress,
        &mut report,
    );
    link_effects(&mut world, common_fx, common_fx_models, &mut gear);
    link_materials(&global, &mut world, &mut gear);
    decode_fx_colour_maps(&world.fx, &mut global, &progress);
    world.impact_fx = world.impact_fx.take().or(common_impact);
    if world.impact_fx.is_none() {
        report.push("impact effects missing — combat hits will find no cells".into());
    }

    if facts.north_yaw.is_none() {
        report.push("compass gap: worldspawn has no northyaw — map is drawn north-up".into());
    }
    report.extend(localize_report);

    let mut fx = std::mem::take(&mut world.fx).publish();
    fx.set_map_namespace(namespace);
    let prepared = PreparedMatch {
        scripts,
        fx,
        world,
        materials: crate::MatchMaterials {
            population: Arc::new(global),
            map_ids,
            common_profile_id: common.id,
            products_id: NEXT_PRODUCTS_ID.fetch_add(1, Ordering::Relaxed),
        },
        clip: collision.map(Arc::new),
        weapons: Arc::new(gear.weapons.publish()),
        fpv_meshes: gear.fpv_meshes.publish(),
        bodies: Arc::new(gear.bodies.publish()),
        world_weapons: gear.world_weapons.publish(),
        projectile_meshes: gear.projectile_meshes.publish(),
        xanims: gear.xanims.publish(),
        destructible_death: Vec::new(),
        tracers: gear.tracers.publish(),
        strings,
        report,
        prepared_map: PreparedMap {
            zone: zone_name,
            namespace: map_namespace,
            spawns,
            facts,
        },
        pen_table,
        pen_table_loaded,
        lochit_table,
        xmodel_walk,
        sound,
    };
    (MatchLoadOutcome::Ready(prepared), Some(common))
}

/// Opens the map's zone file, to be shared with the walk that reads it.
async fn open_zone(
    zone_ff: Result<PathBuf, String>,
    progress: LoadProgress,
) -> Result<(PathBuf, Arc<asset_transport::zone::ZoneImage>), String> {
    let path = match zone_ff {
        Ok(path) => path,
        Err(error) => {
            progress.record_skipped_scoped(StageId::MapAssets, "open");
            return Err(format!("zone not found: {error}"));
        }
    };
    let stage = progress.begin_scoped(StageId::MapAssets, "open", None);
    progress.begin_zone_open();
    let opened = open_zone_shared(&path);
    finish_zone_open(stage, &opened);
    let image = opened.map_err(|error| format!("open zone: {error}"))?;
    progress.record_zone_image_bytes(image.bytes.len());
    Ok((path, image))
}

/// Decodes the images of the merged materials, the map's light attenuation
/// ramps and the tracer beams.
fn decode_images(
    global: &mut MaterialDefinitions,
    world: &mut PreparedWorld,
    common_light_defs: &[asset_world::CapturedLightDef],
    tracers: &TracerCatalog,
    namespace: AssetNamespace,
    progress: &LoadProgress,
    report: &mut Vec<String>,
) {
    let stage = progress.begin_scoped(StageId::Images, "merged", None);
    let stats = asset_material::decode_material_color_maps(global, &stage, load_pool());
    stage.done();
    report.push(format!(
        "merged material images: {}/{} decoded, {} missing, {} unsupported",
        stats.decoded, stats.requested, stats.missing, stats.unsupported
    ));

    if let Some(draw) = world.draw.as_mut() {
        resolve_world_lights(draw, global, common_light_defs, namespace, progress, report);
    }
    asset_material::decode_in_zone_builtin_images(global);

    let stage = progress.begin_scoped(StageId::Images, "tracers", None);
    asset_material::decode_color_or_2d_for_keys(
        global,
        tracers.material_keys(),
        &stage,
        load_pool(),
    );
    stage.done();
}

/// Finds the light definitions the map's lights name, decodes their
/// attenuation ramps, and settles which image is the outdoor lookup.
fn resolve_world_lights(
    draw: &mut WorldDraw,
    global: &mut MaterialDefinitions,
    common_light_defs: &[asset_world::CapturedLightDef],
    namespace: AssetNamespace,
    progress: &LoadProgress,
    report: &mut Vec<String>,
) {
    asset_world::resolve_primary_light_attenuation(draw, global, common_light_defs);

    let (ordinal, source) =
        asset_world::resolve_outdoor_image(draw.outdoor_image_name.as_deref(), namespace, global);
    draw.outdoor_image = ordinal;
    if source == "$outdoor" && draw.outdoor_image_name.is_none() {
        draw.outdoor_image_name = Some("$outdoor".into());
    }
    report.push(format!(
        "outdoor image: source={source} name={} ordinal={}",
        draw.outdoor_image_name.as_deref().unwrap_or("-"),
        draw.outdoor_image
            .map_or_else(|| "none".to_owned(), |ordinal| ordinal.to_string()),
    ));

    let ramps: Vec<(usize, u8)> = draw
        .primary_lights
        .iter()
        .filter_map(|light| Some((light.attenuation_image?, light.attenuation_sampler)))
        .collect();
    let stage = progress.begin_scoped(StageId::Images, "attenuation", None);
    asset_material::decode_catalog_images(global, ramps, &stage, load_pool());
    stage.done();
    asset_world::resolve_primary_light_attenuation(draw, global, common_light_defs);
}

/// Folds the common set's effects into the map's and ties them to the
/// models, weapons and tracers that use them.
fn link_effects(
    world: &mut PreparedWorld,
    common_fx: FxCatalog,
    common_fx_models: asset_game::FxModelCatalog,
    gear: &mut Gear,
) {
    world.fx.absorb(common_fx);
    world.fx_models.absorb(common_fx_models);
    world.fx_models.keep_referenced(&world.fx.model_hints());
    world.fx.resolve_model_edges(&world.fx_models);
    gear.weapons.resolve_combat_fx(&world.fx, &gear.tracers);
    gear.weapons.resolve_projectile_fx_edges(&world.fx);
}

/// Ties everything that draws with a material to the merged material pool.
fn link_materials(materials: &MaterialDefinitions, world: &mut PreparedWorld, gear: &mut Gear) {
    world.fx_models.resolve_materials(materials);
    gear.weapons.resolve_hud_material_edges(materials);
    gear.tracers.resolve_materials(materials);
    world.fx.resolve_materials(materials);
    world.fx.resolve_nested_edges();
    gear.bodies.resolve_materials(materials);
    gear.world_weapons.resolve_materials(materials);
    gear.fpv_meshes.resolve_materials(materials);
    gear.projectile_meshes.resolve_materials(materials);
}

/// Decodes the colour image of each material an effect draws with, when the
/// pool has not decoded it yet.
fn decode_fx_colour_maps(
    fx: &FxCatalog,
    global: &mut MaterialDefinitions,
    progress: &LoadProgress,
) {
    let missing: Vec<MaterialKey> = fx
        .unique_bound_hints()
        .into_iter()
        .chain(fx.unique_decal_mark_hints())
        .filter(|&(index, _)| !has_decoded_colour(global, index))
        .filter_map(|(index, _)| {
            let material = global.materials.get(index)?;
            Some(MaterialKey {
                namespace: material.namespace,
                name: material.name.as_str().to_owned(),
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
        load_pool(),
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
    image_of(TS_COLOR_MAP)
        .or_else(|| image_of(TS_2D))
        .and_then(|index| global.images.get(index))
        .is_some_and(|image| image.decoded.is_some())
}
