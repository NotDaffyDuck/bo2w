//! `LevelStage`: the loaded map as a main-world resource, plus the read-only
//! queries other systems make against it (light regions, cells, probes).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::mesh::PrimitiveTopology;

use super::dpvs::{LevelHide, LevelSight};
use super::model_set::{
    LevelBatchGeometry, LevelDebrisInstance, LevelDebrisSolid, LevelDrawGeometry, LevelLightbake,
    StaticPropLightSample, FixedPropPlacement, LevelPropMesh, LevelScriptModelInstance, PropStainCpu,
};
use super::{LegacyFalloffParams, LightFalloffBind, WorldEyePlacement};
use crate::assemble::draw_items::{DynamicLightBind, LiveMaterialLedger};

/// How many client views the model-lighting atlas is sized for.
pub const PROP_ILLUM_MAX_CLIENT_VIEWS: u32 = 1;

/// Vision presets by name; a preset that failed to decode keeps its error.


/// The sun: the one directional primary light, in the form material
/// constants are filled from.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapDirKeyLight {
    pub light_kind: u8,
    pub direction: [f32; 3],
    pub color: [f32; 3],
    /// Gain applied to `color` for the diffuse term when no classic colour is set.
    pub diffuse_gain: f32,
    /// Gain applied to `color` for the specular term when no classic colour is set.
    pub specular_gain: f32,
    pub classic_diffuse_color: Option<[f32; 4]>,
    pub classic_specular_color: Option<[f32; 4]>,
}

#[derive(Resource, Default)]
pub struct LevelStage {
    // --- draw batches and materials ---
    pub sky_dome_model: Option<LevelPropMesh>,
    pub batches: Vec<LevelBatchGeometry>,
    pub live_material_ledger: Arc<LiveMaterialLedger>,
    pub precise_material_images: Vec<Option<Arc<Image>>>,
    pub common_profile_id: u64,
    pub products_id: u64,
    pub exact_material_common: Vec<bool>,
    /// The decoded variant behind each image slot, when known. Equal variants
    /// mean equal bytes, sampler and colour space, so the uploader shares one
    /// texture between those slots.
    pub exact_material_variants: Vec<Option<asset_material::ImageVariantId>>,
    pub precise_material_names: Vec<String>,
    pub lightmaps: Vec<Option<LevelLightbake>>,
    /// Cube images for the reflection probes; taken by the image upload.
    pub env_probe_images: Vec<Option<Image>>,
    pub env_probe_origins: Vec<[f32; 3]>,

    // --- static and scripted models ---
    pub prop_meshes: Vec<LevelPropMesh>,
    pub prop_mesh_names: Vec<String>,
    pub script_solid_gameobjects: Vec<String>,
    pub script_solid_exploders: Vec<String>,
    pub script_solid_targetnames: Vec<String>,
    pub script_solid_models: Vec<asset_world::ScriptSolidPlacementInfo>,
    pub prop_instances: Vec<Option<FixedPropPlacement>>,
    pub smodel_mark_cpu: Option<PropStainCpu>,
    pub map_figure_stage_assets: asset_world::LevelModelPlacementRoster,
    pub scripted_prop_placements: Vec<LevelScriptModelInstance>,
    pub debris_instances: Vec<LevelDebrisInstance>,
    pub debris_solid_tally: usize,
    pub debris_solids: Vec<LevelDebrisSolid>,
    pub prop_illum_samples: Vec<StaticPropLightSample>,
    pub light_grid: Option<asset_model::DetachedLightGrid>,
    pub model_illum_image: Option<Handle<Image>>,
    pub model_illum_dims: Option<lighting_t6::ModelIllumAtlasDims>,

    // --- extent, culling, cameras ---
    pub mid: Vec3,
    pub rad: f32,
    pub level_bounds: Option<[f32; 6]>,
    pub culling: Option<LevelHide>,
    pub scoreboard_eye: Option<WorldEyePlacement>,
    pub t6_glass: Option<asset_world::T6Glass>,

    // --- vertex streams kept on the CPU side ---
    pub retained_packed_vertices: asset_world::WorldVertexPayload,
    pub held_vertex_layer: Vec<u8>,
    pub facet_vertex_layer: Vec<i32>,
    pub facet_first_vertex: Vec<u32>,
    pub held_positions: Vec<[f32; 3]>,
    pub held_normals: Vec<[f32; 3]>,
    pub held_tangents: Vec<[f32; 4]>,
    pub held_colors: Vec<[f32; 4]>,
    pub held_texture_uvs: Vec<[f32; 2]>,
    pub held_lightbake_uvs: Vec<[f32; 2]>,
    /// Same layout as `asset_world::LevelDrawCall::t6_layer_uvs`.
    pub retained_t6_layer_uvs: Vec<[f32; 4]>,

    // --- look: haze, grading, sun ---
    pub exp_haze: Option<asset_world::ExpHaze>,
    pub colour_grade: Option<asset_world::ScreenGradeLook>,
    pub film_visions: BTreeMap<String, Result<asset_world::ScreenGradeLook, asset_world::GradeVisionDecodeFault>>,
    pub art_profile_name: Option<String>,
    pub dir_key_light: Option<MapDirKeyLight>,
    pub classic_daylight_parse_exposure: Option<f32>,
    pub classic_sky_dynamic_intensity: Option<[f32; 4]>,
    /// Near sun-shadow partition texel size in world units (0 = map left it unset).
    pub t6_sun_sample_size_near: f32,
    pub classic_exposure_volume_tally: u32,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_vision_saturation`.
    pub t6_vision_saturation: Option<f32>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_sky_parms`.
    pub t6_sky_parms: Option<[f32; 2]>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_vision_grade`.
    pub t6_vision_grade: Option<[[f32; 4]; 14]>,
    /// bo2zm: the map's own grade as loaded, and the visions a script can
    /// turn on (`adapters::t6_vision` picks among them).
    pub t6_vision_grade_loaded: Option<[[f32; 4]; 14]>,
    pub t6_script_grades: Arc<Vec<(String, [[f32; 4]; 14])>>,

    // --- light tables ---
    /// Same layout as `asset_world::LevelDrawCall::t6_lights`.
    pub t6_lights: Arc<Vec<[f32; 16]>>,
    /// Every fog bank the map ships (layout as in
    /// `asset_world::LevelDrawCall::t6_fog_banks`). `t6_lights_loaded` is the
    /// light table untouched, since `t6_lights` can be patched with another
    /// bank's fog; `t6_fog_bank` names the bank on screen, 0 being the map's.
    pub t6_fog_banks: Vec<asset_world::T6FogVolume>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_fog_modifiers`.
    pub t6_fog_modifiers: Vec<asset_world::T6FogModifier>,
    pub t6_lights_loaded: Arc<Vec<[f32; 16]>>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_flicker`.
    pub t6_flicker: Vec<asset_world::T6Flicker>,
    pub t6_fog_bank: u32,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_exposure_volumes`.
    pub t6_exposure_volumes: Arc<Vec<asset_world::T6ExposureVolume>>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_lut` and `t6_lut_volumes`.
    pub t6_lut: Option<Arc<asset_world::T6Lut>>,
    pub t6_lut_volumes: Arc<Vec<asset_world::T6LutVolume>>,
    /// bo2zm: the colour tables drawn this frame (`adapters::t6_lut`): the
    /// first rows of tables A and B, A's weight, 1 when the map has them.
    pub t6_lut_pick: [f32; 4],
    /// bo2zm: the table a client script forces (`setlutscriptindex`), if any.
    pub t6_lut_script: Option<u32>,
    /// bo2zm: see `asset_world::LevelDrawCall::t6_light_tweaks`.
    pub t6_light_tweaks: Option<asset_world::T6LightTweaks>,
    /// bo2zm: this frame's sun and sky multipliers (`adapters::t6_light_tweaks`):
    /// the scripts' sun colour times light over the map's (red, green, blue),
    /// and the sky intensity blend.
    pub t6_sun_sky_scale: [f32; 4],
    /// Same layout as `asset_world::LevelDrawCall::t6_probes`.
    pub t6_probes: Arc<Vec<[[f32; 4]; 3]>>,
    pub key_light_types: Vec<u8>,
    pub key_light_hide: Vec<lighting_t6::LampReach>,
    pub key_light_pack: Vec<lighting_t6::LampRecord>,
    pub key_light_attenuation: Vec<LightFalloffBind>,
    pub dynamic_light: Option<DynamicLightBind>,
    pub key_light_def_names: Vec<Option<String>>,
    pub key_light_classic_falloff: Vec<LegacyFalloffParams>,
    pub daylight_key_light_tally: u32,
    pub light_zone_hulls: Option<Vec<Vec<asset_world::LevelLightZoneHull>>>,
    pub shadow_geometry: Vec<asset_world::LevelShadowMesh>,
    pub openair_image: Option<u32>,
    pub openair_lookup: [u32; 16],
    pub sun_effects: Option<render_frame::SunEffectsDef>,

    // --- bookkeeping ---
    pub spawned: bool,
    pub asset_ref: asset_material::AssetPointerDumpTally,
}

/// The borrowed hull shape `lighting_t6` walks, for one stored hull.
fn borrow_hull(stored: &asset_world::LevelLightZoneHull) -> lighting_t6::LightVolumeHull<'_> {
    lighting_t6::LightVolumeHull {
        bounds_center_point: stored.bounds_center_point,
        bounds_half_extent: stored.bounds_half_extent,
        axes: &stored.axes,
    }
}

/// An empty mesh that lives only in the main world; batches keep it as a
/// stand-in once their real geometry has gone to the GPU side.
fn hollow_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD)
}

impl LevelStage {
    /// A stage with nothing in it but the extent of the box `lo..hi`. The
    /// radius never drops below one unit.
    fn spanning(lo: [f32; 3], hi: [f32; 3]) -> Self {
        let (lo, hi) = (Vec3::from_array(lo), Vec3::from_array(hi));
        let diagonal = hi - lo;
        Self {
            mid: (lo + hi) * 0.5,
            rad: f32::max(diagonal.length() / 2.0, 1.0),
            t6_sun_sky_scale: [1.0; 4],
            ..Self::default()
        }
    }

    /// A stand-in stage made of a single unlit batch holding `mesh`.
    pub fn from_bounds(shape: Mesh, lo: [f32; 3], hi: [f32; 3]) -> Self {
        let material = None;
        let only_batch = LevelBatchGeometry {
            mesh_h: shape,
            compact_indices: Vec::new(),
            material,
            lmapped: false,
            lightbake_index: 0,
            key_light_index: 0,
            env_probe_index: 0,
            sun: None,
        };
        let mut stage = Self::spanning(lo, hi);
        stage.batches.push(only_batch);
        stage.retained_packed_vertices = asset_world::WorldVertexPayload::Unavailable {
            origin_layout: "synthetic bounds mesh",
        };
        stage
    }

    /// Takes ownership of the loaded map geometry. The per-batch meshes and
    /// index lists are dropped right away (the GPU copy is built from the
    /// shared arrays instead); the surface tables move into the cull state.
    pub fn from_geometry(
        geometry: LevelDrawGeometry,
        lo: [f32; 3],
        hi: [f32; 3],
        sight: LevelSight,
        scoreboard_eye: Option<WorldEyePlacement>,
    ) -> Self {
        let LevelDrawGeometry {
            mut batches,
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
            prop_meshes,
            prop_instances,
            map_figure_stage_assets,
            scripted_prop_placements,
            prop_illum_samples,
        } = geometry;

        let mut lit_flags = Vec::with_capacity(batches.len());
        for batch in batches.iter_mut() {
            lit_flags.push(batch.lmapped);
            batch.mesh_h = hollow_mesh();
            batch.compact_indices = Vec::new();
        }

        let hide = LevelHide {
            compact_indices,
            facet_index_ranges,
            facet_draw_fields,
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
            dpvs: sight,
            batches_n: u32::try_from(lit_flags.len()).unwrap_or(u32::MAX),
            batch_lightbaked: lit_flags,
            // Per-frame working state starts empty.
            facet_vis: Vec::new(),
            draw_batch_list: Vec::new(),
            item_surfs: Vec::new(),
            level_tree_run_scratch: Vec::new(),
            draw_batch_list_id: 0,
            g0_facets: Vec::new(),
            prop_entities: Vec::new(),
            prop_hide_dists: Vec::new(),
            prop_vis: Vec::new(),
            room_vis: Vec::new(),
            room_vis_all: false,
        };

        let mut stage = Self::spanning(lo, hi);
        stage.batches = batches;
        stage.lightmaps = lightmaps;
        stage.env_probe_images = env_probe_images;
        stage.prop_meshes = prop_meshes;
        stage.prop_instances = prop_instances;
        stage.map_figure_stage_assets = map_figure_stage_assets;
        stage.scripted_prop_placements = scripted_prop_placements;
        stage.prop_illum_samples = prop_illum_samples;
        stage.retained_packed_vertices = compact_vertices;
        stage.held_vertex_layer = vertex_tier;
        stage.facet_vertex_layer = facet_vertex_layer;
        stage.facet_first_vertex = facet_first_vertex;
        stage.held_positions = positions;
        stage.held_normals = surface_directions;
        stage.held_tangents = tangents;
        stage.held_colors = colors;
        stage.held_texture_uvs = texture_coords;
        stage.held_lightbake_uvs = lightbake_uvs;
        stage.retained_t6_layer_uvs = t6_layer_uvs;
        stage.culling = Some(hide);
        stage.scoreboard_eye = scoreboard_eye;
        stage
    }

    /// Writes every surface's packed sort word, taken from the material it
    /// uses, into the level snapshot.
    pub(crate) fn stamp_facet_materials(
        hide: &mut LevelHide,
        ledger: &LiveMaterialLedger,
    ) -> Result<(), asset_world::FacetMaterialStampError> {
        let LevelHide { facet_materials, facet_key_lights, capture, .. } = hide;
        let mats = &ledger.materials;
        let sort_word_by_material: Vec<Option<dpvs_t6::SortedDraw>> = mats
            .iter()
            .map(|entry| entry.baked_facet_draw.map(dpvs_t6::SortedDraw::unpack_from_bits))
            .collect();
        let material_slot_by_facet: Vec<Option<usize>> =
            facet_materials.iter().map(|slot| slot.map(|material| material.order())).collect();
        asset_world::stamp_compact_facet_materials(
            &material_slot_by_facet,
            facet_key_lights,
            &sort_word_by_material,
            capture,
        )
    }

    /// The secondary-light walk result for a box, as a byte; when the walk
    /// cannot run (no box, or no light regions) the vacant-sun value is used.
    pub fn loose_at_spot_lookup_fallback(&self, mid: [f32; 3], half_extent: Option<[f32; 3]>) -> u8 {
        match self.loose_at_spot_walk_trace(mid, half_extent) {
            Some(log) => log.first_hit as u8,
            None => lighting_t6::GRID_SAMPLE_VACANT_SUN_BAND,
        }
    }

    /// Copies what the at-spot light lookup needs into the shared resource.
    pub fn publish_loose_at_spot(&self, target: &mut render_scene::LooseAtSpotLookup) {
        let culls = self.key_light_hide.clone();
        let regions = self.light_zone_hulls.clone();
        target.load(culls, self.daylight_key_light_tally, regions);
    }

    /// Copies the cell tree into the shared resource, or empties it when the
    /// stage has no cull data.
    pub fn publish_sight_rooms(&self, rooms: &mut render_scene::LevelSightRooms) {
        match &self.culling {
            Some(hide) => {
                let tree = &hide.dpvs;
                rooms.planes.clone_from(&tree.plane_set);
                rooms.knots.clone_from(&tree.node_list);
                rooms.cells_in = tree.cells_n;
            }
            None => *rooms = render_scene::LevelSightRooms::default(),
        }
    }

    /// Hands `visit` the light-region hulls in borrowed form (`None` when the
    /// map defines no regions) and returns what it returns.
    pub fn with_light_zone_hulls<R>(
        &self,
        visit: impl FnOnce(Option<&[lighting_t6::LightVolumeHullList<'_>]>) -> R,
    ) -> R {
        let Some(stored) = self.light_zone_hulls.as_deref() else {
            return visit(None);
        };
        let borrowed: Vec<Vec<lighting_t6::LightVolumeHull<'_>>> = stored
            .iter()
            .map(|region| region.iter().map(borrow_hull).collect())
            .collect();
        let region_slices: Vec<lighting_t6::LightVolumeHullList<'_>> =
            borrowed.iter().map(Vec::as_slice).collect();
        visit(Some(region_slices.as_slice()))
    }

    /// Runs the secondary-light walk for the box at `mid`. `None` without a
    /// box or without light regions.
    pub fn loose_at_spot_walk_trace(
        &self,
        mid: [f32; 3],
        half_extent: Option<[f32; 3]>,
    ) -> Option<lighting_t6::SecondaryLightWalkLog> {
        let extent = half_extent?;
        self.with_light_zone_hulls(|regions| {
            regions.map(|regions| {
                lighting_t6::secondary_main_light_walk_trace(
                    &self.key_light_hide,
                    self.daylight_key_light_tally,
                    Some(regions),
                    mid,
                    extent,
                )
            })
        })
    }

    /// Picks the reflection probe for a lighting origin: among the probes the
    /// containing cell lists when the point lands in a cell, otherwise the
    /// nearest of all probes (slot 0 excluded).
    pub fn env_probe_for_illum_origin(&self, pos: [f32; 3]) -> u8 {
        let all_origins = &self.env_probe_origins[..];
        let cell_candidates = self.culling.as_ref().and_then(|hide| {
            let box_tree = &hide.dpvs;
            let lookup = dpvs_t6::VisibilityCellPlanes {
                planes: &box_tree.plane_set,
                nodes: &box_tree.node_list,
                leaf_cells: box_tree.cells_n as u32,
            };
            let room = lookup.cell_holding(pos)?;
            Some(box_tree.room_env_probes.get(room).map(Vec::as_slice).unwrap_or_default())
        });
        if let Some(options) = cell_candidates {
            lighting_t6::nearest_cube_in_cells(all_origins, options, pos)
        } else {
            lighting_t6::nearest_cube_probe(all_origins, pos)
        }
    }
}
