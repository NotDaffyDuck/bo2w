//! Building a `WorldScene` from what loading prepared: the draw arrays, the
//! match's materials and the authored placements.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::bmodel::{
    authored_bmodel_world_from_local, world_dyn_ent_brushes, world_dyn_ent_instances,
};
use super::dpvs::{WorldDpvs, WorldPortal};
use super::models::{
    WorldBatchGeometry, WorldDrawGeometry, WorldDynEntBrush, WorldDynEntInstance, WorldLightmap,
    WorldScriptModelInstance, WorldSmodelLightingSample, WorldStaticModelInstance,
    WorldStaticModelMesh, WorldStaticModelSurface, WorldSun,
};
use super::scene::{MapDirPrimaryLight, WorldScene};
use super::sun::{install_sun_effects, remap_local_material};
use super::{LightAttenuationBind, T5LightFalloffPack, WorldCameraPose};
use crate::assemble::drawsurf::DynamicLightBind;

type SurfaceStampError = asset_world::SurfaceMaterialStampError;

/// The placements the map authors outside the draw arrays.
struct Authored {
    brushes: Vec<asset_world::ScriptBrushModelPlacement>,
    dyn_ent_instances: Vec<WorldDynEntInstance>,
    dyn_ent_brush_n: usize,
}

impl Authored {
    fn attach(self, scene: &mut WorldScene, dyn_ent_brushes: Vec<WorldDynEntBrush>) {
        let names = |pick: fn(&asset_world::ScriptBrushModelPlacement) -> &String| -> Vec<String> {
            self.brushes.iter().map(|brush| pick(brush).clone()).collect()
        };
        scene.script_brush_gameobjects = names(|brush| &brush.gameobject);
        scene.script_brush_exploders = names(|brush| &brush.script_exploder);
        scene.script_brush_targetnames = names(|brush| &brush.targetname);
        scene.script_brush_models = self.brushes;
        scene.dyn_ent_instances = self.dyn_ent_instances;
        scene.dyn_ent_brush_n = self.dyn_ent_brush_n;
        scene.dyn_ent_brushes = dyn_ent_brushes;
    }
}

fn image_slot(index: Option<usize>) -> Option<u32> {
    index.and_then(|index| u32::try_from(index).ok())
}

fn attenuation_bind(image: Option<usize>, sampler: u8) -> LightAttenuationBind {
    LightAttenuationBind {
        image: image_slot(image),
        sampler,
    }
}

/// One line describing an image that has no decoded copy.
fn image_row(index: usize, image: &asset_material::AuthoredImage) -> String {
    format!(
        "{index}:{} payload={} {}x{}x{} fmt={} map={} semantic={}",
        image.name,
        image.payload.len(),
        image.width,
        image.height,
        image.depth,
        image.format,
        image.map_type,
        image.semantic
    )
}

/// Logs the images the draw binds but nothing decoded, and a catalog that
/// could not be sorted.
fn log_material_gaps(
    defs: &asset_material::MaterialDefinitions,
    has_decoded: &dyn Fn(usize) -> bool,
    lights: &[asset_world::WorldPrimaryLight],
    outdoor_image: Option<usize>,
    catalog: &crate::assemble::drawsurf::RuntimeMaterialCatalog,
) {
    let builtin: Vec<String> = defs
        .images
        .iter()
        .enumerate()
        .filter(|(_, image)| image.name.as_str().starts_with('$'))
        .map(|(index, image)| {
            format!(
                "{index}:{} decoded={} payload={} {}x{} fmt={}",
                image.name,
                has_decoded(index),
                image.payload.len(),
                image.width,
                image.height,
                image.format
            )
        })
        .collect();
    if !builtin.is_empty() {
        diag::info!(World, "drawsurf builtin images: {}", builtin.join("; "));
    }

    let material_images: BTreeSet<usize> = defs
        .materials
        .iter()
        .flat_map(|material| material.textures.iter())
        .filter_map(|texture| texture.image)
        .collect();
    let undecoded: Vec<String> = material_images
        .iter()
        .filter(|&&index| !has_decoded(index))
        .filter_map(|&index| {
            let image = defs.images.get(index)?;
            matches!(image.semantic, 2 | 3 | 5 | 8).then(|| image_row(index, image))
        })
        .collect();
    if !undecoded.is_empty() {
        diag::info!(
            World,
            "drawsurf undecoded color/normal/specular: {}",
            undecoded.join("; ")
        );
    }

    let mut bound = material_images;
    bound.extend(lights.iter().filter_map(|light| light.attenuation_image));
    bound.extend(outdoor_image);
    let never: Vec<String> = bound
        .into_iter()
        .filter(|&index| !has_decoded(index))
        .filter_map(|index| Some(image_row(index, defs.images.get(index)?)))
        .collect();
    if !never.is_empty() {
        diag::warn!(
            World,
            "drawsurf bound images that never decoded: n={} {}",
            never.len(),
            never.iter().take(24).cloned().collect::<Vec<_>>().join("; ")
        );
    }

    if let crate::assemble::drawsurf::RuntimeSortedMaterialTable::BuildFailed(
        crate::assemble::drawsurf::CatalogBuildError::ShaderIdentityMissing { material, slot },
    ) = &catalog.sorted_materials
    {
        let name = defs
            .materials
            .get(usize::from(material.0))
            .map_or("<out-of-range>", |material| material.name.as_str());
        diag::warn!(
            World,
            "drawsurf sorted-material input: material {} {name:?} slot {slot} has no exact shader identity",
            material.0
        );
    }
}

fn lightmap_pages(
    pages: Result<Vec<Option<asset_world::WorldLightmap>>, asset_world::WorldLightmapGap>,
) -> Vec<Option<WorldLightmap>> {
    let Ok(pages) = pages else {
        return Vec::new();
    };
    pages
        .into_iter()
        .map(|page| {
            page.map(|map| WorldLightmap {
                primary_image: map.primary_image,
                secondary_image: map.secondary_image,
                secondary_b_image: map.secondary_b_image,
                ambient_image: map.ambient_image,
                directional_image: map.directional_image,
                sun_mask_image: map.sun_mask_image,
                ambient_source_name: map.ambient_source_name,
                sun_mask_source_name: map.sun_mask_source_name,
                ambient_size: map.ambient_size,
                sun_mask_size: map.sun_mask_size,
            })
        })
        .collect()
}

fn world_dpvs(data: asset_world::DpvsWorldData) -> WorldDpvs {
    WorldDpvs {
        planes: data.planes,
        nodes: data.nodes,
        cell_count: data.cell_count,
        aabb_trees: data.aabb_trees,
        aabb_smodel_indices: data.aabb_smodel_indices,
        sorted_surf_index: data.sorted_surf_index,
        static_surface_count: data.static_surface_count,
        static_surface_count_no_decal: data.static_surface_count_no_decal,
        surface_bounds: data.surface_bounds,
        smodel_bounds: data.smodel_bounds,
        portal_verts: data.portal_verts,
        cell_reflection_probes: data.cell_reflection_probes,
        lit_opaque_begin: data.lit_opaque_begin,
        lit_opaque_end: data.lit_opaque_end,
        camera_ranges: data.camera_ranges,
        emissive_surfs_begin: data.emissive_surfs_begin,
        emissive_surfs_end: data.emissive_surfs_end,
        portals_per_cell: data
            .portals_per_cell
            .into_iter()
            .map(|cell| {
                cell.into_iter()
                    .map(|p| WorldPortal {
                        plane: p.plane,
                        neighbor: p.neighbor,
                        vert_start: p.vert_start,
                        vert_count: p.vert_count,
                        hull_axis: p.hull_axis,
                    })
                    .collect()
            })
            .collect(),
        cell_caster_bits: Vec::new(),
    }
}

fn batch_geometry(
    batches: Vec<asset_world::WorldBatch>,
    lights: &[asset_world::WorldPrimaryLight],
    lightmaps: &[Option<WorldLightmap>],
    lightmap_requires_image: bool,
    map_ids: &[Option<usize>],
) -> Vec<WorldBatchGeometry> {
    batches
        .into_iter()
        .map(|batch| {
            let sun = lights
                .get(usize::from(batch.primary_light_index))
                .filter(|light| light.is_sun)
                .map(|light| WorldSun {
                    direction: light.direction,
                    color: light.color,
                });
            let has_page = lightmaps
                .get(usize::from(batch.lightmap_index))
                .is_some_and(Option::is_some);
            WorldBatchGeometry {
                mesh: batch.mesh,
                packed_indices: batch.packed_indices,
                material: remap_local_material(batch.material, map_ids),
                lightmapped: batch.lightmapped && (has_page || !lightmap_requires_image),
                lightmap_index: batch.lightmap_index,
                primary_light_index: batch.primary_light_index,
                reflection_probe_index: batch.reflection_probe_index,
                sun,
            }
        })
        .collect()
}

fn convert_model(model: asset_world::ModelMesh, map_ids: &[Option<usize>]) -> WorldStaticModelMesh {
    WorldStaticModelMesh {
        name: model.name,
        lod_smc: model.lod_smc,
        lod: model.lod,
        lod_surfaces: model.lod_surfaces.map(|lod| {
            lod.into_iter()
                .map(|surface| WorldStaticModelSurface {
                    mesh: surface.mesh,
                    material: remap_local_material(surface.material, map_ids),
                    packed_vertices: surface.packed_vertices,
                    xsurface_plus_1: surface.xsurface_plus_1,
                    xsurface_base_index: surface.xsurface_base_index,
                    xsurface_vert_offset: surface.xsurface_vert_offset,
                    collision: surface.collision,
                })
                .collect()
        }),
    }
}

/// The directional light the map authors: the T5 sun when present, else the
/// sun among the primary lights.
fn directional_light(
    t5_sun: Option<asset_world::WorldSunLight>,
    lights: &[asset_world::WorldPrimaryLight],
) -> Option<MapDirPrimaryLight> {
    let default_scale = lighting_t6::R_COLOR_SCALE_DEFAULT;
    if let Some(sun) = t5_sun {
        return Some(MapDirPrimaryLight {
            light_type: lighting_t6::GFX_LIGHT_TYPE_DIR,
            direction: sun.direction,
            color: [sun.diffuse[0], sun.diffuse[1], sun.diffuse[2]],
            diffuse_color_scale: default_scale,
            specular_color_scale: default_scale,
            t5_diffuse_color: Some(sun.diffuse),
            t5_specular_color: Some(sun.specular),
        });
    }
    let light = lights
        .iter()
        .find(|light| light.is_sun && light.light_type == lighting_t6::GFX_LIGHT_TYPE_DIR)?;
    Some(MapDirPrimaryLight {
        light_type: light.light_type,
        direction: light.direction,
        color: light.color,
        diffuse_color_scale: default_scale,
        specular_color_scale: default_scale,
        t5_diffuse_color: light.t5_diffuse_color,
        t5_specular_color: light.t5_specular_color,
    })
}

/// Copies the per-primary-light tables onto the scene.
fn attach_primary_lights(scene: &mut WorldScene, lights: &[asset_world::WorldPrimaryLight]) {
    scene.primary_light_types = lights.iter().map(|light| light.light_type).collect();
    scene.primary_light_cull = lights
        .iter()
        .map(asset_world::WorldPrimaryLight::cull_input)
        .collect();
    scene.primary_light_pack = lights
        .iter()
        .map(asset_world::WorldPrimaryLight::gfx_light_pack)
        .collect();
    scene.primary_light_attenuation = lights
        .iter()
        .map(|light| attenuation_bind(light.attenuation_image, light.attenuation_sampler))
        .collect();
    scene.primary_light_def_names = lights.iter().map(|light| light.def_name.clone()).collect();
    scene.primary_light_t5_falloff = lights
        .iter()
        .map(|light| T5LightFalloffPack {
            diffuse: light.t5_diffuse_color,
            specular: light.t5_specular_color,
            attenuation: light.t5_attenuation,
            falloff: light.t5_falloff,
            a_ab_b: light.t5_a_ab_b,
            angle_z: light.t5_angle.map(|angle| angle[2]),
            cookie0: light.t5_cookie0,
            cookie1: light.t5_cookie1,
            cookie2: light.t5_cookie2,
        })
        .collect();
}

fn log_canonical_materials(scene: &WorldScene) {
    let batch_materials = scene
        .batches
        .iter()
        .filter(|batch| batch.material.is_some())
        .count();
    let smodel_materials = scene
        .static_model_meshes
        .iter()
        .flat_map(|mesh| mesh.lod_surfaces.iter().flatten())
        .filter(|surface| surface.material.is_some())
        .count();
    diag::info!(
        World,
        "canonical materials: n={} batch_mat={} smodel_mat={} (MaterialIndex into decoded catalog; runtime is derived)",
        scene.runtime_material_catalog.materials.len(),
        batch_materials,
        smodel_materials,
    );
}

pub fn world_scene_from_draw(
    world: assets::PreparedWorld,
    materials: assets::MatchMaterials,
    installed_owners: &[(asset_world::ScriptModelId, sim::AuthorityModelOwner)],
) -> Result<WorldScene, SurfaceStampError> {
    let (dyn_ent_instances, dyn_ent_brush_n) = world_dyn_ent_instances(&world.dyn_ents);
    diag::info!(
        World,
        "{} admitted_models={}",
        world.dyn_ents.report_line(),
        dyn_ent_instances.len(),
    );
    let authored = Authored {
        brushes: world.script_brush_models.clone(),
        dyn_ent_instances,
        dyn_ent_brush_n,
    };
    let assets::MatchMaterials {
        population: defs,
        map_ids,
        common_profile_id,
        products_id,
    } = materials;

    let Some(mut draw) = world.draw else {
        let mut empty = WorldScene {
            common_profile_id,
            products_id,
            t6_glass: world.t6_glass,
            ..Default::default()
        };
        let brushes = world_dyn_ent_brushes(&world.dyn_ents.brushes, &[], &[]);
        authored.attach(&mut empty, brushes);
        return Ok(empty);
    };
    let dyn_ent_brushes = world_dyn_ent_brushes(
        &world.dyn_ents.brushes,
        &draw.brush_model_bounds,
        &draw.brush_models,
    );
    let policy = world.policy;

    let catalog = crate::assemble::drawsurf::capture_runtime_catalog(&defs);
    let decoded: Vec<_> = defs.images.iter().map(|image| image.decoded.clone()).collect();
    log_material_gaps(
        &defs,
        &|index| decoded.get(index).is_some_and(Option::is_some),
        &draw.primary_lights,
        draw.outdoor_image,
        &catalog,
    );

    let surface_material_ids: Vec<Option<assets::MaterialIndex>> = draw
        .surface_materials
        .iter()
        .map(|&local| remap_local_material(local, &map_ids))
        .collect();
    let surface_sort_keys: Vec<u8> = surface_material_ids
        .iter()
        .map(|id| {
            id.and_then(|id| catalog.derived(id))
                .map_or(0, |material| material.sort_key)
        })
        .collect();

    let lightmaps = lightmap_pages(draw.lightmap);
    let mut dpvs = world_dpvs(draw.dpvs);
    if let Some(sun) = draw
        .primary_lights
        .iter()
        .find(|light| light.is_sun && light.light_type == lighting_t6::GFX_LIGHT_TYPE_DIR)
    {
        dpvs.bake_cell_casters(sun.direction);
    }
    let batches = batch_geometry(
        draw.batches,
        &draw.primary_lights,
        &lightmaps,
        policy.lightmap_requires_image,
        &map_ids,
    );

    let sky_model = draw.sky_model.take().map(|model| convert_model(model, &map_ids));
    let static_model_meshes: Vec<WorldStaticModelMesh> = world
        .static_model_meshes
        .into_iter()
        .map(|model| convert_model(model, &map_ids))
        .collect();
    let static_model_instances: Vec<Option<WorldStaticModelInstance>> = world
        .static_model_instances
        .into_iter()
        .map(|placement| {
            placement.map(|p| WorldStaticModelInstance {
                mesh: p.mesh,
                transform: p.transform,
                origin: p.origin,
                axis: p.axis,
                scale: p.scale,
                cull_dist: p.cull_dist,
                reflection_probe_index: p.reflection_probe_index,
                primary_light_index: p.primary_light_index,
                flags: p.flags,
                packed_lighting: p.packed_lighting,
                t6_primary: p.t6_primary,
                t6_light: p.t6_light,
            })
        })
        .collect();
    let script_model_instances: Vec<WorldScriptModelInstance> = world
        .script_model_instances
        .into_iter()
        .map(|p| WorldScriptModelInstance {
            id: p.id,
            authority_owner: installed_owners
                .iter()
                .find(|(id, _)| *id == p.id)
                .map(|(_, owner)| *owner),
            current_model: p.current_model,
            transform: p.transform,
            lighting_origin: p.lighting_origin,
            dobj_state: p.dobj_state,
            metadata: p.metadata,
            gentity_number: None,
        })
        .collect();
    let smodel_lighting_samples: Vec<WorldSmodelLightingSample> = world
        .smodel_lighting_samples
        .into_iter()
        .map(|sample| WorldSmodelLightingSample {
            authored_slot: sample.authored_slot,
            lighting_origin: sample.lighting_origin,
            tile_rgba: sample.tile_rgba,
            packed_lighting: sample.packed_lighting,
        })
        .collect();

    let mut map_xmodel_scene_assets = world.map_xmodel_scene_assets;
    let xmodel_materials = map_xmodel_scene_assets.resolve_surface_materials(&map_ids);
    diag::info!(
        World,
        "map xmodel surface materials: models={} surfaces={} bound={} unbound={} absent={}",
        xmodel_materials.models,
        xmodel_materials.surfaces,
        xmodel_materials.bound,
        xmodel_materials.unbound,
        xmodel_materials.absent,
    );

    let bmodel_world_from_local = authored_bmodel_world_from_local(
        draw.brush_models.len(),
        &world.script_brush_models,
        &world.dyn_ents.brushes,
    );
    let reflection_probe_origins: Vec<[f32; 3]> =
        draw.reflection_probes.iter().map(|probe| probe.origin).collect();
    let dir_primary_light = directional_light(draw.t5_sun_light, &draw.primary_lights);

    let mut scene = WorldScene::from_draw(
        WorldDrawGeometry {
            batches,
            lightmaps,
            reflection_probes: world.reflection_probe_images,
            packed_vertices: draw.packed_vertices,
            vertex_layer: draw.vertex_layer,
            surface_vertex_layer: draw.surface_vertex_layer,
            surface_first_vertex: draw.surface_first_vertex,
            surface_draw_fields: draw.surface_draw_fields,
            positions: draw.positions,
            normals: draw.normals,
            tangents: draw.tangents,
            colors: draw.colors,
            texture_uvs: draw.texture_uvs,
            lightmap_uvs: draw.lightmap_uvs,
            t6_layer_uvs: draw.t6_layer_uvs,
            packed_indices: draw.packed_indices,
            surface_index_ranges: draw.surface_index_ranges,
            surface_batch_ranges: draw.surface_batch_ranges,
            surface_materials: surface_material_ids,
            surface_lightmap_indices: draw.surface_lightmap_indices,
            surface_reflection_probes: draw.surface_reflection_probes,
            surface_primary_lights: draw.surface_primary_lights,
            sort_key_distortion: draw.sort_key_distortion,
            surface_sort_keys,
            capture: draw.capture,
            brush_models: draw.brush_models,
            brush_model_bounds: draw.brush_model_bounds,
            bmodel_world_from_local,
            t6_brush_surfaces: draw.t6_brush_surfaces,
            static_model_meshes,
            static_model_instances,
            map_xmodel_scene_assets,
            script_model_instances,
            smodel_lighting_samples,
        },
        world.min,
        world.max,
        dpvs,
        world.intermission_view.map(|view| WorldCameraPose {
            origin: view.origin,
            angles: view.angles,
        }),
    );

    scene.t6_glass = world.t6_glass;
    scene.world_bounds = world.world_bounds;
    scene.reflection_probe_origins = reflection_probe_origins;
    scene.asset_ref = asset_material::AssetRefDumpCensus::from_catalog(&defs);
    scene.runtime_material_catalog = Arc::new(catalog);
    if let Some(cull) = scene.cull.as_mut() {
        WorldScene::stamp_surface_materials(cull, &scene.runtime_material_catalog)?;
    }
    scene.exact_material_common = defs.images.iter().map(|image| image.common_owned).collect();
    scene.exact_material_variants = defs.images.iter().map(|image| image.decoded_variant).collect();
    scene.exact_material_names = defs
        .images
        .iter()
        .map(|image| image.name.as_str().to_owned())
        .collect();
    scene.exact_material_images = decoded;
    scene.common_profile_id = common_profile_id;
    scene.products_id = products_id;
    log_canonical_materials(&scene);

    scene.light_grid = world.light_grid;
    scene.sky_model = sky_model;
    scene.exp_fog = world.exp_fog;
    scene.film_vision = world.film_vision;
    scene.film_visions = world.film_visions;
    scene.createart_name = world.createart_name;
    scene.dir_primary_light = dir_primary_light;
    scene.t5_sun_parse_exposure = draw.t5_sun_parse_exposure;
    scene.t5_sky_dynamic_intensity = draw.t5_sky_dynamic_intensity;
    scene.t5_exposure_volume_count = draw.t5_exposure_volume_count;
    scene.t6_lights = Arc::new(draw.t6_lights.clone());
    scene.t6_lights_loaded = Arc::clone(&scene.t6_lights);
    scene.t6_fog_banks = draw.t6_fog_banks.clone();
    scene.t6_fog_bank = 0;
    scene.t6_probes = Arc::new(draw.t6_probes.clone());
    attach_primary_lights(&mut scene, &draw.primary_lights);
    scene.dynamic_light = world.dynamic_light.map(|light| DynamicLightBind {
        attenuation: attenuation_bind(light.attenuation_image, light.attenuation_sampler),
        falloff_image_width: light.falloff_image_width,
        lmap_lookup_start: light.lmap_lookup_start,
    });
    scene.sun_primary_light_count = draw.sun_primary_light_count;
    scene.light_region_hulls = draw.light_region_hulls;
    scene.shadow_geometry = draw.shadow_geometry;
    scene.outdoor_image = image_slot(draw.outdoor_image);
    scene.outdoor_lookup = draw.outdoor_lookup;
    scene.sun_effects = install_sun_effects(
        draw.sun_effects.as_ref(),
        &map_ids,
        &scene.runtime_material_catalog,
    );
    authored.attach(&mut scene, dyn_ent_brushes);
    Ok(scene)
}
