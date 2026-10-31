//! Turns what map loading prepared (the draw arrays, the match's materials and
//! the placements the map authors) into one `LevelStage`.
//!
//! The loader's records are unpacked once at the top, so every field the scene
//! takes is named exactly once; a field the loader adds later and the scene
//! should carry shows up here as an unused binding, not a silent drop.

use std::collections::BTreeSet;
use std::sync::Arc;

use asset_material::{HandBuiltTexture, MaterialDefinitions};
use asset_world::{
    StaticMeshData, MeshFaceDrawCall, ScriptSolidPlacementInfo, ScriptModelId,
    ScriptedPropPlacement, LevelDrawGroup, LevelDominantLight, LevelSunlamp,
};
use bevy::prelude::Image;

use super::bmodel::{
    designer_solid_model_level_from_local, level_debris_solids, level_debris_instances,
};
use super::dpvs::LevelSight;
use super::model_set::{
    LevelBatchGeometry, LevelDrawGeometry, LevelDebrisSolid, LevelDebrisInstance, LevelLightbake,
    LevelScriptModelInstance, LevelPropMesh, LevelPropFacet, LevelDaylight,
};
use super::scene::{MapDirKeyLight, LevelStage};
use super::daylight::{install_sun_effects, map_local_material};
use super::{LightFalloffBind, LegacyFalloffParams, WorldEyePlacement};
use crate::{
    LedgerBuildError, DynamicLightBind, LiveMaterialLedger, LiveSortedMaterialTable,
};


/// Image semantics a material samples for its look: colour, normal and the two
/// specular kinds. One of these missing its decoded copy is worth a log line.
const LOOK_SEMANTICS: [u8; 4] = [2, 3, 5, 8];

/// How many never-decoded images the warning spells out.
const NEVER_DECODED_SHOWN: usize = 24;

/// Script brushes and dynamic entities: the map places them outside the draw
/// arrays, and both the full and the empty scene carry them.
struct Placements {
    brushes: Vec<ScriptSolidPlacementInfo>,
    debris_instances: Vec<LevelDebrisInstance>,
    debris_solid_tally: usize,
}

impl Placements {
    fn install(self, scene: &mut LevelStage, debris_solids: Vec<LevelDebrisSolid>) {
        let mut gameobjects = Vec::with_capacity(self.brushes.len());
        let mut exploders = Vec::with_capacity(self.brushes.len());
        let mut targetnames = Vec::with_capacity(self.brushes.len());
        for brush in &self.brushes {
            gameobjects.push(brush.script_entity.clone());
            exploders.push(brush.scripted_detonator.clone());
            targetnames.push(brush.lookup_key.clone());
        }
        scene.script_solid_gameobjects = gameobjects;
        scene.script_solid_exploders = exploders;
        scene.script_solid_targetnames = targetnames;
        scene.script_solid_models = self.brushes;
        scene.debris_instances = self.debris_instances;
        scene.debris_solid_tally = self.debris_solid_tally;
        scene.debris_solids = debris_solids;
    }
}

/// An image slot as the GPU side indexes it; one past `u32` has no slot.
fn gpu_image_slot(index: Option<usize>) -> Option<u32> {
    index.and_then(|index| u32::try_from(index).ok())
}

fn attenuation_of(image: Option<usize>, sampler: u8) -> LightFalloffBind {
    LightFalloffBind {
        image: gpu_image_slot(image),
        sampler,
    }
}

/// The map's directional sun among its primary lights, if it authors one.
fn authored_sun(lights: &[LevelDominantLight]) -> Option<&LevelDominantLight> {
    lights
        .iter()
        .find(|light| light.is_daylight && light.light_kind == lighting_t6::LAMP_KIND_PARALLEL)
}

/// Full description of an image with no decoded copy, for the gap logs.
fn undecoded_row(index: usize, image: &HandBuiltTexture) -> String {
    let HandBuiltTexture {
        name,
        payload,
        width,
        height,
        depth,
        format,
        texture_kind: map_type,
        semantic,
        ..
    } = image;
    let bytes = payload.len();
    format!(
        "{index}:{name} payload={bytes} {width}x{height}x{depth} fmt={format} map={map_type} semantic={semantic}"
    )
}

/// Logs which images the world binds without a decoded copy, the engine's
/// `$` images either way, and why the material catalog could not be sorted.
fn log_material_gaps(
    defs: &MaterialDefinitions,
    decoded: &[Option<Arc<Image>>],
    lights: &[LevelDominantLight],
    openair_image: Option<usize>,
    catalog: &LiveMaterialLedger,
) {
    let is_decoded = |index: usize| matches!(decoded.get(index), Some(Some(_)));

    let mut built_in = Vec::new();
    for (index, image) in defs.images.iter().enumerate() {
        if !image.name.as_str().starts_with('$') {
            continue;
        }
        let HandBuiltTexture {
            name,
            payload,
            width,
            height,
            format,
            ..
        } = image;
        let (ready, bytes) = (is_decoded(index), payload.len());
        built_in.push(format!(
            "{index}:{name} decoded={ready} payload={bytes} {width}x{height} fmt={format}"
        ));
    }
    if !built_in.is_empty() {
        diag::info!(World, "drawsurf images built in: {}", built_in.join("; "));
    }

    // Every image some material samples, in index order.
    let mut bound = BTreeSet::new();
    for material in &defs.materials {
        bound.extend(material.textures.iter().filter_map(|texture| texture.image));
    }
    let missing_look: Vec<String> = bound
        .iter()
        .copied()
        .filter(|&index| !is_decoded(index))
        .filter_map(|index| {
            let image = defs.images.get(index)?;
            LOOK_SEMANTICS
                .contains(&image.semantic)
                .then(|| undecoded_row(index, image))
        })
        .collect();
    if !missing_look.is_empty() {
        diag::info!(
            World,
            "drawsurf color/normal/specular not decoded: {}",
            missing_look.join("; ")
        );
    }

    // Light attenuation and the outdoor lookup are bound too.
    bound.extend(lights.iter().filter_map(|light| light.falloff_texture));
    bound.extend(openair_image);
    let mut never_decoded = Vec::new();
    for index in bound {
        if is_decoded(index) {
            continue;
        }
        if let Some(image) = defs.images.get(index) {
            never_decoded.push(undecoded_row(index, image));
        }
    }
    if !never_decoded.is_empty() {
        let shown = never_decoded[..never_decoded.len().min(NEVER_DECODED_SHOWN)].join("; ");
        diag::warn!(
            World,
            "drawsurf bound images that never decoded: n={} {shown}",
            never_decoded.len()
        );
    }

    let LiveSortedMaterialTable::BuildFailed(LedgerBuildError::ShaderPairAbsent {
        material,
        slot,
    }) = &catalog.ranked_materials
    else {
        return;
    };
    let index = material.0;
    let name = match defs.materials.get(usize::from(index)) {
        Some(found) => found.name.as_str(),
        None => "<out-of-range>",
    };
    diag::warn!(
        World,
        "drawsurf sorted-material input: material {index} {name:?} slot {slot} has no exact shader identity"
    );
}

/// The visibility tables as loading built them, plus room for the sun's
/// caster matrix (baked once the sun is known).
fn visibility(data: asset_world::VisibilityLevelData) -> LevelSight {
    let asset_world::VisibilityLevelData {
        planes,
        tree_nodes: nodes,
        cell_total: num_cells,
        aabb_trees: trees,
        aabb_prop_indices,
        sorted_facet_index,
        static_facet_tally,
        static_facet_tally_no_stain,
        facet_bounds,
        static_prop_boxes: smodel_bounds,
        portal_vertices,
        portals_per_room,
        room_env_probes,
        lit_solid_begin,
        lit_solid_end,
        camera_spans,
        emissive_facets_begin,
        emissive_facets_end,
        // A load-time check count; the camera walk never reads it.
        emptied_volumes: _,
    } = data;
    LevelSight {
        plane_set: planes,
        node_list: nodes,
        cells_n: num_cells,
        box_trees: trees,
        aabb_prop_indices,
        sorted_facet_index,
        static_facet_tally,
        static_facet_tally_no_stain,
        facet_bounds,
        prop_bounds: smodel_bounds,
        portal_vertices,
        portals_per_room,
        room_env_probes,
        room_caster_bits: Vec::new(),
        lit_solid_begin,
        lit_solid_end,
        camera_spans,
        emissive_facets_begin,
        emissive_facets_end,
    }
}

/// Draw batches with their material moved into the match catalog. A batch
/// stays lightmapped only when its page loaded, unless the world's policy
/// lets it light without one.
fn draw_batches(
    batches: Vec<LevelDrawGroup>,
    lights: &[LevelDominantLight],
    pages: &[Option<LevelLightbake>],
    page_required: bool,
    map_ids: &[Option<usize>],
) -> Vec<LevelBatchGeometry> {
    let sun_for = |slot: u8| {
        let &LevelDominantLight {
            is_daylight: true,
            heading: direction,
            rgba: color,
            ..
        } = lights.get(usize::from(slot))?
        else {
            return None;
        };
        Some(LevelDaylight { dir_v: direction, tint: color })
    };
    let page_loaded = |slot: u8| matches!(pages.get(usize::from(slot)), Some(Some(_)));
    let mut out = Vec::with_capacity(batches.len());
    for batch in batches {
        let LevelDrawGroup {
            built_mesh: mesh,
            compact_indices,
            mat_slot: material,
            has_baked_light: has_lightmap,
            lightbake_index,
            key_light_index,
            env_probe_index,
        } = batch;
        out.push(LevelBatchGeometry {
            mesh_h: mesh,
            compact_indices,
            material: map_local_material(material, map_ids),
            lmapped: has_lightmap && (!page_required || page_loaded(lightbake_index)),
            lightbake_index,
            key_light_index,
            env_probe_index,
            sun: sun_for(key_light_index),
        });
    }
    out
}

/// A model mesh with each LOD surface's material moved into the match catalog.
fn static_mesh(model: StaticMeshData, map_ids: &[Option<usize>]) -> LevelPropMesh {
    let StaticMeshData {
        name,
        detail_facets,
        detail_prop_cache,
        lod,
        ..
    } = model;
    let surface = |draw: MeshFaceDrawCall| {
        let MeshFaceDrawCall {
            face_mesh: mesh,
            mat_slot: material,
            compact_vertices,
            figure_part_plus_1,
            figure_part_base_index,
            figure_part_vertex_offset,
            collision,
        } = draw;
        LevelPropFacet {
            mesh_h: mesh,
            material: map_local_material(material, map_ids),
            compact_vertices,
            figure_part_plus_1,
            figure_part_base_index,
            figure_part_vertex_offset,
            collision,
        }
    };
    LevelPropMesh {
        name,
        detail_prop_cache,
        lod,
        detail_facets: detail_facets.map(|surfaces| surfaces.into_iter().map(surface).collect()),
    }
}

/// Script models as the scene draws them, each tagged with the authority
/// owner installed for it (if any). Entity numbers come later, at spawn.
fn script_models(
    placed: Vec<ScriptedPropPlacement>,
    owners: &[(ScriptModelId, sim::ServerModelSource)],
) -> Vec<LevelScriptModelInstance> {
    let mut out = Vec::with_capacity(placed.len());
    for model in placed {
        let ScriptedPropPlacement {
            handle: id,
            active_mesh,
            transform,
            illum_origin,
            puppet_state,
            metadata,
        } = model;
        let server_owned_model_owner = owners
            .iter()
            .find(|(owner_id, _)| *owner_id == id)
            .map(|&(_, owner)| owner);
        out.push(LevelScriptModelInstance {
            id,
            server_owned_model_owner,
            active_mesh,
            place: transform,
            illum_origin,
            puppet_state,
            metadata,
            game_actor_number: None,
        });
    }
    out
}

/// The map's directional light. A T5 sun record wins (its diffuse supplies
/// the colour); otherwise the directional sun among the primary lights.
fn directional_light(
    t5_sun: Option<LevelSunlamp>,
    lights: &[LevelDominantLight],
) -> Option<MapDirKeyLight> {
    let (lamp_type, direction, color, diffuse, specular) = match t5_sun {
        Some(LevelSunlamp {
            direction,
            diffuse,
            specular,
        }) => {
            let [r, g, b, _] = diffuse;
            let kind = lighting_t6::LAMP_KIND_PARALLEL;
            (kind, direction, [r, g, b], Some(diffuse), Some(specular))
        }
        None => {
            let sun = authored_sun(lights)?;
            let (diffuse, specular) = (sun.classic_diffuse_color, sun.classic_specular_color);
            (sun.light_kind, sun.heading, sun.rgba, diffuse, specular)
        }
    };
    let scale = lighting_t6::COLOR_GAIN_INITIAL;
    Some(MapDirKeyLight {
        light_kind: lamp_type,
        direction,
        color,
        diffuse_gain: scale,
        specular_gain: scale,
        classic_diffuse_color: diffuse,
        classic_specular_color: specular,
    })
}

/// The T5 falloff values of one primary light, as the shader reads them.
fn classic_falloff(light: &LevelDominantLight) -> LegacyFalloffParams {
    let LevelDominantLight {
        classic_diffuse_color: diffuse,
        classic_specular_color: specular,
        classic_attenuation: attenuation,
        classic_falloff: falloff,
        classic_a_ab_b: pair_ab_vs_b,
        classic_angle: angle,
        classic_cookie0: projector_mask_first,
        classic_cookie1: projector_mask_second,
        classic_cookie2: projector_mask_third,
        ..
    } = *light;
    LegacyFalloffParams {
        diffuse,
        specular,
        attenuation,
        falloff,
        pair_ab_vs_b,
        yaw_degrees_opt: angle.map(|[_, _, z, _]| z),
        projector_mask_first,
        projector_mask_second,
        projector_mask_third,
    }
}

/// Fills the scene's per-primary-light tables, all indexed by light slot.
fn install_primary_lights(scene: &mut LevelStage, lights: &[LevelDominantLight]) {
    let count = lights.len();
    let mut types = Vec::with_capacity(count);
    let mut cull = Vec::with_capacity(count);
    let mut pack = Vec::with_capacity(count);
    let mut attenuation = Vec::with_capacity(count);
    let mut def_names = Vec::with_capacity(count);
    let mut falloff = Vec::with_capacity(count);
    for light in lights {
        types.push(light.light_kind);
        cull.push(light.cull_input());
        pack.push(light.gfx_light_pack());
        attenuation.push(attenuation_of(light.falloff_texture, light.falloff_lookup));
        def_names.push(light.definition_name.clone());
        falloff.push(classic_falloff(light));
    }
    scene.key_light_types = types;
    scene.key_light_hide = cull;
    scene.key_light_pack = pack;
    scene.key_light_attenuation = attenuation;
    scene.key_light_def_names = def_names;
    scene.key_light_classic_falloff = falloff;
}

fn log_canonical_materials(scene: &LevelStage) {
    let catalog_n = scene.live_material_ledger.materials.len();
    let on_batches = scene.batches.iter().filter_map(|batch| batch.material).count();
    let on_models = scene
        .prop_meshes
        .iter()
        .flat_map(LevelPropMesh::facet_materials)
        .flatten()
        .count();
    diag::info!(
        World,
        "canonical materials: n={catalog_n} on batches={on_batches} on static models={on_models} (MaterialIndex into the decoded catalog; the runtime one is derived)"
    );
}

pub fn level_stage_from_draw(
    world: assets::StagedLevel,
    materials: assets::MatchMaterials,
    installed_owners: &[(asset_world::ScriptModelId, sim::ServerModelSource)],
) -> Result<LevelStage, asset_world::FacetMaterialStampError> {
    let assets::StagedLevel {
        draw,
        dynamic_light,
        prop_meshes,
        prop_instances,
        mut map_figure_stage_assets,
        scripted_prop_placements,
        script_solid_models,
        debris_pieces,
        prop_illum_samples,
        light_grid: lighting_grid,
        t6_glass,
        env_probe_images,
        scoreboard_eye,
        exp_haze,
        colour_grade,
        vision_sets: film_visions,
        art_profile_name,
        min,
        max,
        level_bounds,
        policy,
        ..
    } = world;
    let assets::MatchMaterials {
        population: defs,
        map_ids,
        common_profile_id,
        products_id,
    } = materials;

    let (debris_instances, debris_solid_tally) = level_debris_instances(&debris_pieces);
    diag::info!(
        World,
        "{} models admitted={}",
        debris_pieces.summary_text(),
        debris_instances.len(),
    );
    let placements = Placements {
        brushes: script_solid_models,
        debris_instances,
        debris_solid_tally,
    };

    let Some(draw) = draw else {
        // No geometry loaded: the scene still carries the placements.
        let mut scene = LevelStage {
            common_profile_id,
            products_id,
            t6_glass,
            ..Default::default()
        };
        let brushes = level_debris_solids(&debris_pieces.brushes, &[], &[]);
        placements.install(&mut scene, brushes);
        return Ok(scene);
    };
    let asset_world::LevelDrawCall {
        batches,
        sky_dome_model,
        lightmap,
        compact_vertices,
        compact_indices,
        capture,
        dpvs,
        warp_sort_key,
        vertex_tier,
        spots: positions,
        surface_directions,
        tangent_rows: tangents,
        colors,
        texture_coords,
        lightbake_uvs,
        facet_vertex_layer,
        facet_first_vertex,
        facet_draw_fields,
        facet_index_ranges,
        facet_batch_ranges,
        facet_lightbake_indices,
        facet_env_probes,
        facet_key_lights,
        facet_materials,
        solid_models,
        solid_model_bounds,
        key_lights,
        daylight_key_light_tally,
        light_zone_hulls,
        shadow_geometry: caster_geometry,
        reflection_probes: reflect_probes,
        openair_image,
        openair_lookup,
        sun_effects,
        classic_daylight_parse_exposure,
        classic_sky_dynamic_intensity,
        t5_sun_light: t5_daylight,
        classic_exposure_volume_tally,
        t6_vision_saturation,
        t6_sky_parms,
        t6_vision_grade,
        t6_script_grades,
        t6_lights,
        t6_fog_banks,
        t6_fog_modifiers,
        t6_exposure_volumes,
        t6_lut,
        t6_lut_volumes,
        t6_light_tweaks,
        t6_flicker,
        t6_probes,
        t6_layer_uvs,
        t6_brush_surfaces,
        ..
    } = draw;
    let debris_solids =
        level_debris_solids(&debris_pieces.brushes, &solid_model_bounds, &solid_models);

    // Materials: the runtime catalog, plus logs for anything bound but not decoded.
    let catalog = crate::assemble::draw_items::capture_runtime_catalog(&defs);
    let decoded: Vec<Option<Arc<Image>>> =
        defs.images.iter().map(|image| image.decoded.clone()).collect();
    log_material_gaps(&defs, &decoded, &key_lights, openair_image, &catalog);

    let facet_materials: Vec<Option<assets::SurfaceMaterialId>> = facet_materials
        .into_iter()
        .map(|local| map_local_material(local, &map_ids))
        .collect();
    let facet_sort_keys: Vec<u8> = facet_materials
        .iter()
        .map(|&id| match id.and_then(|id| catalog.derived(id)) {
            Some(material) => material.draw_sort_key,
            None => 0,
        })
        .collect();

    // A world whose lightmap failed to load draws with no pages at all.
    let lightmaps = lightmap.unwrap_or_default();
    let mut dpvs = visibility(dpvs);
    if let Some(sun) = authored_sun(&key_lights) {
        dpvs.bake_cell_casters(sun.heading);
    }
    let batches = draw_batches(
        batches,
        &key_lights,
        &lightmaps,
        policy.lightmap_needs_texture,
        &map_ids,
    );

    let census = map_figure_stage_assets.link_skin_surfaces(&map_ids);
    let asset_world::SceneMaterialTally {
        model_total: models,
        surface_total: surfaces,
        linked: bound,
        unlinked: unbound,
        missing: absent,
    } = census;
    diag::info!(
        World,
        "map xmodel surface materials: models={models} surfaces={surfaces} bound={bound} unbound={unbound} absent={absent}"
    );

    let solid_model_level_from_local =
        designer_solid_model_level_from_local(solid_models.len(), &placements.brushes, &debris_pieces.brushes);
    let env_probe_origins: Vec<[f32; 3]> =
        reflect_probes.iter().map(|probe| probe.spot).collect();
    let geometry = LevelDrawGeometry {
        batches,
        lightmaps,
        reflect_probes: env_probe_images,
        compact_vertices,
        vertex_tier,
        facet_vertex_layer,
        facet_first_vertex,
        facet_draw_fields,
        pos_list: positions,
        surface_directions,
        tangent_list: tangents,
        colors,
        texture_coords,
        lightbake_uvs,
        t6_layer_uvs,
        compact_indices,
        facet_index_ranges,
        facet_batch_ranges,
        facet_materials,
        facet_lightbake_indices,
        facet_env_probes,
        facet_key_lights,
        warp_sort_key,
        facet_sort_keys,
        capture,
        solid_models,
        solid_model_bounds,
        solid_model_level_from_local,
        t6_brush_surfaces,
        prop_meshes: prop_meshes
            .into_iter()
            .map(|model| static_mesh(model, &map_ids))
            .collect(),
        prop_instances,
        map_figure_stage_assets,
        scripted_prop_placements: script_models(scripted_prop_placements, installed_owners),
        prop_illum_samples,
    };
    let camera = scoreboard_eye
        .map(|asset_world::BetweenRoundsCamera { spot: pos, turn_deg: angles }| WorldEyePlacement { origin: pos, turn: angles });

    let catalog = Arc::new(catalog);
    let t6_lights = Arc::new(t6_lights);
    let mut scene = LevelStage {
        t6_glass,
        level_bounds,
        env_probe_origins,
        asset_ref: asset_material::AssetPointerDumpTally::build_from_index(&defs),
        exact_material_common: defs.images.iter().map(|image| image.common_owned).collect(),
        exact_material_variants: defs.images.iter().map(|image| image.decoded_variant).collect(),
        precise_material_names: defs
            .images
            .iter()
            .map(|image| image.name.as_str().to_owned())
            .collect(),
        precise_material_images: decoded,
        common_profile_id,
        products_id,
        light_grid: lighting_grid,
        sky_dome_model: sky_dome_model.map(|model| static_mesh(model, &map_ids)),
        exp_haze,
        colour_grade,
        film_visions,
        art_profile_name,
        dir_key_light: directional_light(t5_daylight, &key_lights),
        classic_daylight_parse_exposure,
        classic_sky_dynamic_intensity,
        classic_exposure_volume_tally,
        t6_vision_saturation,
        t6_sky_parms,
        t6_vision_grade,
        t6_vision_grade_loaded: t6_vision_grade,
        t6_script_grades: Arc::new(t6_script_grades),
        t6_lights_loaded: Arc::clone(&t6_lights),
        t6_lights,
        t6_fog_banks,
        t6_fog_modifiers,
        t6_exposure_volumes: Arc::new(t6_exposure_volumes),
        t6_lut_pick: [0.0, 0.0, 1.0, if t6_lut.is_some() { 1.0 } else { 0.0 }],
        t6_lut,
        t6_lut_volumes: Arc::new(t6_lut_volumes),
        t6_lut_script: None,
        t6_light_tweaks,
        t6_sun_sky_scale: [1.0; 4],
        t6_flicker,
        t6_fog_bank: 0,
        t6_probes: Arc::new(t6_probes),
        dynamic_light: dynamic_light.map(|def| DynamicLightBind {
            attenuation: attenuation_of(def.falloff_texture, def.falloff_lookup),
            falloff_map_width: def.falloff_map_width,
            lightmap_lookup_first: def.lightmap_first_texel,
        }),
        daylight_key_light_tally,
        light_zone_hulls,
        shadow_geometry: caster_geometry,
        openair_image: gpu_image_slot(openair_image),
        openair_lookup,
        sun_effects: install_sun_effects(sun_effects.as_ref(), &map_ids, &catalog),
        live_material_ledger: Arc::clone(&catalog),
        ..LevelStage::from_geometry(geometry, min, max, dpvs, camera)
    };
    if let Some(cull) = scene.culling.as_mut() {
        LevelStage::stamp_facet_materials(cull, &catalog)?;
    }
    log_canonical_materials(&scene);
    install_primary_lights(&mut scene, &key_lights);
    placements.install(&mut scene, debris_solids);
    Ok(scene)
}
