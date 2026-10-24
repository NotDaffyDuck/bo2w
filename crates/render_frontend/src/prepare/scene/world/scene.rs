//! The spawned world as the main world holds it.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;

use super::dpvs::{WorldCull, WorldDpvs};
use super::models::{
    SmodelMarkCpu, WorldBatchGeometry, WorldDrawGeometry, WorldDynEntBrush, WorldDynEntInstance,
    WorldLightmap, WorldScriptModelInstance, WorldSmodelLightingSample, WorldStaticModelInstance,
    WorldStaticModelMesh,
};
use super::{LightAttenuationBind, T5LightFalloffPack, WorldCameraPose};
use crate::assemble::drawsurf::{DynamicLightBind, RuntimeMaterialCatalog};

pub const SMODEL_LIGHTING_MAX_CLIENT_VIEWS: u32 = 1;

/// The map's directional (sun) primary light as the material side reads it.
#[derive(Clone, Copy, Debug, Resource)]
pub struct MapDirPrimaryLight {
    pub light_type: u8,
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub diffuse_color_scale: f32,
    pub specular_color_scale: f32,
    pub t5_diffuse_color: Option<[f32; 4]>,
    pub t5_specular_color: Option<[f32; 4]>,
}

#[derive(Resource, Default)]
pub struct WorldScene {
    pub sky_model: Option<WorldStaticModelMesh>,
    pub batches: Vec<WorldBatchGeometry>,
    pub runtime_material_catalog: Arc<RuntimeMaterialCatalog>,
    pub exact_material_images: Vec<Option<Arc<Image>>>,
    pub common_profile_id: u64,
    pub products_id: u64,
    pub exact_material_common: Vec<bool>,
    /// Which prepared variant each of those images is, where it came from a
    /// decode plan. Two slots carrying the same variant hold byte-identical
    /// images under the same sampler and colour space, so the upload gives
    /// them one asset and a cloned handle instead of two textures.
    pub exact_material_variants: Vec<Option<asset_material::ImageVariantId>>,
    pub exact_material_names: Vec<String>,
    pub lightmaps: Vec<Option<WorldLightmap>>,
    pub reflection_probes: Vec<Option<Image>>,
    pub reflection_probe_origins: Vec<[f32; 3]>,
    pub static_model_meshes: Vec<WorldStaticModelMesh>,
    pub smodel_mesh_names: Vec<String>,
    pub script_brush_gameobjects: Vec<String>,
    pub script_brush_exploders: Vec<String>,
    pub script_brush_targetnames: Vec<String>,
    pub script_brush_models: Vec<asset_world::ScriptBrushModelPlacement>,
    pub static_model_instances: Vec<Option<WorldStaticModelInstance>>,
    pub smodel_mark_cpu: Option<SmodelMarkCpu>,
    pub map_xmodel_scene_assets: asset_world::MapXModelSceneCatalog,
    pub script_model_instances: Vec<WorldScriptModelInstance>,
    pub dyn_ent_instances: Vec<WorldDynEntInstance>,
    pub dyn_ent_brush_n: usize,
    pub dyn_ent_brushes: Vec<WorldDynEntBrush>,
    pub smodel_lighting_samples: Vec<WorldSmodelLightingSample>,
    pub light_grid: Option<asset_model::OwnedLightGrid>,
    pub model_lighting_image: Option<Handle<Image>>,
    pub model_lighting_dims: Option<lighting_t6::ModelLightingAtlasDims>,
    pub center: Vec3,
    pub radius: f32,
    pub world_bounds: Option<[f32; 6]>,
    pub cull: Option<WorldCull>,
    pub intermission_view: Option<WorldCameraPose>,
    pub t6_glass: Option<asset_world::T6Glass>,
    pub retained_packed_vertices: asset_world::WorldVertexPayload,
    pub retained_vertex_layer: Vec<u8>,
    pub surface_vertex_layer: Vec<i32>,
    pub surface_first_vertex: Vec<u32>,
    pub retained_positions: Vec<[f32; 3]>,
    pub retained_normals: Vec<[f32; 3]>,
    pub retained_tangents: Vec<[f32; 4]>,
    pub retained_colors: Vec<[f32; 4]>,
    pub retained_texture_uvs: Vec<[f32; 2]>,
    pub retained_lightmap_uvs: Vec<[f32; 2]>,
    /// See `asset_world::WorldDraw::t6_layer_uvs`.
    pub retained_t6_layer_uvs: Vec<[f32; 4]>,
    pub exp_fog: Option<asset_world::ExpFog>,
    pub film_vision: Option<asset_world::FilmVision>,
    pub film_visions:
        BTreeMap<String, Result<asset_world::FilmVision, asset_world::FilmVisionParseError>>,
    pub createart_name: Option<String>,
    pub dir_primary_light: Option<MapDirPrimaryLight>,
    pub t5_sun_parse_exposure: Option<f32>,
    pub t5_sky_dynamic_intensity: Option<[f32; 4]>,
    /// The map's `sm_sunSampleSizeNear`: units per texel of the near sun-shadow
    /// partition; 0 means not set.
    pub t6_sun_sample_size_near: f32,
    pub t5_exposure_volume_count: u32,
    /// See `asset_world::WorldDraw::t6_lights`.
    pub t6_lights: Arc<Vec<[f32; 16]>>,
    /// See `asset_world::WorldDraw::t6_fog_banks`: the table as loaded
    /// (`t6_lights` may carry another bank's fog), and the bank shown
    /// (0 is the map's own).
    pub t6_fog_banks: Vec<(u32, [[f32; 16]; 2])>,
    pub t6_lights_loaded: Arc<Vec<[f32; 16]>>,
    pub t6_fog_bank: u32,
    /// See `asset_world::WorldDraw::t6_probes`.
    pub t6_probes: Arc<Vec<[[f32; 4]; 3]>>,
    pub primary_light_types: Vec<u8>,
    pub primary_light_cull: Vec<lighting_t6::PrimaryLightCull>,
    pub primary_light_pack: Vec<lighting_t6::GfxLightPack>,
    pub primary_light_attenuation: Vec<LightAttenuationBind>,
    pub dynamic_light: Option<DynamicLightBind>,
    pub primary_light_def_names: Vec<Option<String>>,
    pub primary_light_t5_falloff: Vec<T5LightFalloffPack>,
    pub sun_primary_light_count: u32,
    pub light_region_hulls: Option<Vec<Vec<asset_world::WorldLightRegionHull>>>,
    pub shadow_geometry: Vec<asset_world::WorldShadowGeometry>,
    pub outdoor_image: Option<u32>,
    pub outdoor_lookup: [u32; 16],
    pub sun_effects: Option<render_frame::SunEffectsDef>,
    pub spawned: bool,
    pub asset_ref: asset_material::AssetRefDumpCensus,
}

/// Borrowed views of the light-region hulls, the shape `lighting_t6` walks.
fn hull_views(
    lists: &[Vec<asset_world::WorldLightRegionHull>],
) -> Vec<Vec<lighting_t6::LightRegionHull<'_>>> {
    lists
        .iter()
        .map(|list| {
            list.iter()
                .map(|hull| lighting_t6::LightRegionHull {
                    kdop_mid: hull.kdop_mid,
                    kdop_half: hull.kdop_half,
                    axes: &hull.axes,
                })
                .collect()
        })
        .collect()
}

impl WorldScene {
    /// An empty scene sized to the given world bounds.
    fn around(min: [f32; 3], max: [f32; 3]) -> Self {
        let (min, max) = (Vec3::from_array(min), Vec3::from_array(max));
        WorldScene {
            center: (min + max) * 0.5,
            radius: ((max - min).length() * 0.5).max(1.0),
            ..Default::default()
        }
    }

    /// A one-batch placeholder world: just a mesh and its bounds.
    pub fn from_bounds(mesh: Mesh, min: [f32; 3], max: [f32; 3]) -> Self {
        WorldScene {
            batches: vec![WorldBatchGeometry {
                mesh,
                packed_indices: Vec::new(),
                material: None,
                lightmapped: false,
                lightmap_index: 0,
                primary_light_index: 0,
                reflection_probe_index: 0,
                sun: None,
            }],
            retained_packed_vertices: asset_world::WorldVertexPayload::Unavailable {
                source_layout: "synthetic bounds mesh",
            },
            ..Self::around(min, max)
        }
    }

    /// The scene for loaded world geometry. Batch meshes are emptied here: the
    /// GPU copy is built elsewhere and the main world keeps only the arrays.
    pub fn from_draw(
        draw: WorldDrawGeometry,
        min: [f32; 3],
        max: [f32; 3],
        dpvs: WorldDpvs,
        intermission_view: Option<WorldCameraPose>,
    ) -> Self {
        let WorldDrawGeometry {
            mut batches,
            lightmaps,
            reflection_probes,
            packed_vertices,
            vertex_layer,
            surface_vertex_layer,
            surface_first_vertex,
            surface_draw_fields,
            positions,
            normals,
            tangents,
            colors,
            texture_uvs,
            lightmap_uvs,
            t6_layer_uvs,
            packed_indices,
            surface_index_ranges,
            surface_batch_ranges,
            surface_materials,
            surface_lightmap_indices,
            surface_reflection_probes,
            surface_primary_lights,
            sort_key_distortion,
            surface_sort_keys,
            capture,
            brush_models,
            brush_model_bounds,
            bmodel_world_from_local,
            t6_brush_surfaces,
            static_model_meshes,
            static_model_instances,
            map_xmodel_scene_assets,
            script_model_instances,
            smodel_lighting_samples,
        } = draw;

        let batch_lightmapped: Vec<bool> = batches.iter().map(|batch| batch.lightmapped).collect();
        let batch_count = batches.len() as u32;
        for batch in &mut batches {
            batch.mesh = Mesh::new(
                bevy::render::mesh::PrimitiveTopology::TriangleList,
                bevy::asset::RenderAssetUsages::MAIN_WORLD,
            );
            batch.packed_indices.clear();
        }

        let cull = WorldCull {
            packed_indices,
            surface_index_ranges,
            surface_draw_fields,
            surface_batch_ranges,
            surface_materials,
            surface_lightmap_indices,
            surface_reflection_probes,
            surface_primary_lights,
            sort_key_distortion,
            surface_sort_keys,
            capture,
            brush_models,
            brush_model_bounds,
            bmodel_world_from_local,
            t6_brush_surfaces,
            dpvs,
            batch_count,
            batch_lightmapped,
            surface_vis: Vec::new(),
            draw_items: Vec::new(),
            item_surfs: Vec::new(),
            bsp_run_scratch: Vec::new(),
            draw_items_id: 0,
            g0_surfs: Vec::new(),
            static_model_entities: Vec::new(),
            static_model_cull_dists: Vec::new(),
            smodel_vis: Vec::new(),
            cell_vis: Vec::new(),
            cell_vis_all: false,
        };

        WorldScene {
            batches,
            lightmaps,
            reflection_probes,
            static_model_meshes,
            static_model_instances,
            map_xmodel_scene_assets,
            script_model_instances,
            smodel_lighting_samples,
            retained_packed_vertices: packed_vertices,
            retained_vertex_layer: vertex_layer,
            surface_vertex_layer,
            surface_first_vertex,
            retained_positions: positions,
            retained_normals: normals,
            retained_tangents: tangents,
            retained_colors: colors,
            retained_texture_uvs: texture_uvs,
            retained_lightmap_uvs: lightmap_uvs,
            retained_t6_layer_uvs: t6_layer_uvs,
            cull: Some(cull),
            intermission_view,
            ..Self::around(min, max)
        }
    }

    /// Bakes each surface's packed draw-surf from its material.
    pub(crate) fn stamp_surface_materials(
        cull: &mut WorldCull,
        catalog: &RuntimeMaterialCatalog,
    ) -> Result<(), asset_world::SurfaceMaterialStampError> {
        let baked: Vec<Option<dpvs_t6::GfxDrawSurf>> = catalog
            .materials
            .iter()
            .map(|material| material.baked_draw_surf.map(dpvs_t6::GfxDrawSurf::from_packed))
            .collect();
        let slots: Vec<Option<usize>> = cull
            .surface_materials
            .iter()
            .map(|id| id.map(|index| index.order()))
            .collect();
        asset_world::stamp_packed_surface_materials(
            &slots,
            &cull.surface_primary_lights,
            &baked,
            &mut cull.capture,
        )
    }

    pub fn dyn_atpoint_lookup_fallback(&self, mid: [f32; 3], box_half: Option<[f32; 3]>) -> u8 {
        self.dyn_atpoint_walk_trace(mid, box_half)
            .map_or(lighting_t6::LIGHT_GRID_ATPOINT_EMPTY_PRIMARY, |trace| {
                trace.walk as u8
            })
    }

    pub fn publish_dyn_atpoint(&self, lookup: &mut render_scene::DynAtPointLookup) {
        lookup.replace(
            self.primary_light_cull.clone(),
            self.sun_primary_light_count,
            self.light_region_hulls.clone(),
        );
    }

    pub fn publish_dpvs_cells(&self, cells: &mut render_scene::WorldDpvsCells) {
        let Some(cull) = self.cull.as_ref() else {
            *cells = render_scene::WorldDpvsCells::default();
            return;
        };
        cells.planes.clone_from(&cull.dpvs.planes);
        cells.nodes.clone_from(&cull.dpvs.nodes);
        cells.cell_count = cull.dpvs.cell_count;
    }

    /// Runs `f` over the light-region hulls, or `None` when the map has none.
    pub fn with_light_region_hulls<R>(
        &self,
        f: impl FnOnce(Option<&[lighting_t6::LightRegionHulls<'_>]>) -> R,
    ) -> R {
        let Some(lists) = self.light_region_hulls.as_ref() else {
            return f(None);
        };
        let hulls = hull_views(lists);
        let regions: Vec<lighting_t6::LightRegionHulls<'_>> =
            hulls.iter().map(Vec::as_slice).collect();
        f(Some(&regions))
    }

    pub fn dyn_atpoint_walk_trace(
        &self,
        mid: [f32; 3],
        box_half: Option<[f32; 3]>,
    ) -> Option<lighting_t6::NonSunPrimaryWalkTrace> {
        let half = box_half?;
        let hulls = hull_views(self.light_region_hulls.as_ref()?);
        let regions: Vec<lighting_t6::LightRegionHulls<'_>> =
            hulls.iter().map(Vec::as_slice).collect();
        Some(lighting_t6::non_sun_primary_light_walk_trace(
            &self.primary_light_cull,
            self.sun_primary_light_count,
            Some(&regions),
            mid,
            half,
        ))
    }

    /// The reflection probe nearest `origin`, preferring the ones listed for the
    /// cell the point stands in.
    pub fn reflection_probe_for_lighting_origin(&self, origin: [f32; 3]) -> u8 {
        let origins = self.reflection_probe_origins.as_slice();
        let cell = self.cull.as_ref().and_then(|cull| {
            let dpvs = &cull.dpvs;
            let planes = dpvs_t6::DpvsPlanes {
                planes: &dpvs.planes,
                nodes: &dpvs.nodes,
                cell_count: dpvs.cell_count as u32,
            };
            let cell = planes.cell_for_point(origin)?;
            let list = dpvs.cell_reflection_probes.get(cell).map_or(&[][..], Vec::as_slice);
            Some(list)
        });
        match cell {
            None => lighting_t6::nearest_reflection_probe_skip_0(origins, origin),
            Some(list) => lighting_t6::nearest_reflection_probe_in_cell_list(origins, list, origin),
        }
    }
}
