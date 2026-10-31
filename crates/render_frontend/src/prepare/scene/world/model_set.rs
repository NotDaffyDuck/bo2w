//! The pieces a world scene is assembled from: draw batches, lightmap pages,
//! static models and the dynamic-entity brush records.

use bevy::prelude::*;
use dpvs_t6::Bounds;
use assets::SurfaceMaterialId;

pub use render_scene::{LevelDebrisInstance, LevelScriptModelInstance};

// Lightmap pages, model placements and their lighting tiles reach the scene
// exactly as loading built them, so the scene names the loader's types.
pub use asset_model::StaticPropLightSample;
pub use asset_world::{FixedPropPlacement, LevelLightbake};

/// One mesh drawn with one material, lightmap page and primary light.
#[derive(Debug)]
pub struct LevelBatchGeometry {
    pub mesh_h: Mesh,
    pub compact_indices: Vec<u32>,
    pub material: Option<SurfaceMaterialId>,
    pub lmapped: bool,
    pub lightbake_index: u8,
    pub key_light_index: u8,
    pub env_probe_index: u8,
    pub sun: Option<LevelDaylight>,
}

#[derive(Clone, Copy, Debug)]
pub struct LevelDaylight {
    pub dir_v: [f32; 3],
    pub tint: [f32; 3],
}

#[derive(Debug)]
pub struct LevelPropMesh {
    pub name: String,
    pub detail_facets: [Vec<LevelPropFacet>; 4],
    pub detail_prop_cache: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelDetailSelector>,
}

impl LevelPropMesh {
    pub fn facet_materials(&self) -> impl Iterator<Item = Option<assets::SurfaceMaterialId>> + '_ {
        self.detail_facets
            .iter()
            .flatten()
            .map(|facet| facet.material)
    }
}

#[derive(Debug)]
pub struct LevelPropFacet {
    pub mesh_h: Mesh,
    pub material: Option<SurfaceMaterialId>,
    pub compact_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub figure_part_plus_1: Option<u8>,
    pub figure_part_base_index: u16,
    pub figure_part_vertex_offset: u16,
    pub collision: asset_world::XSurfaceCollisionPayload,
}

/// CPU copy of the static-model geometry that decal marking reads.
#[derive(Clone, Debug, Default)]
pub struct PropStainCpu {
    pub pos_list: Vec<[f32; 3]>,
    pub surface_directions: Vec<[f32; 3]>,
    pub tri_list: Vec<u32>,
    pub meshes: Vec<Vec<PropStainFacet>>,
}

#[derive(Clone, Debug)]
pub struct PropStainFacet {
    pub first_triangle_slot: u32,
    pub index_total: u32,
    pub material: Option<SurfaceMaterialId>,
    pub collision: asset_world::XSurfaceCollisionPayload,
}

/// A brush-model dynamic entity with its posed bounds.
#[derive(Clone, Copy, Debug)]
pub struct LevelDebrisSolid {
    pub index: u16,
    pub at_point: [f32; 3],
    pub bbox: Option<Bounds>,
    pub solid_model: u16,
    pub facet_tally: u16,
}

/// Everything `LevelStage::from_draw` takes in.
#[derive(Debug)]
pub struct LevelDrawGeometry {
    pub batches: Vec<LevelBatchGeometry>,
    pub lightmaps: Vec<Option<LevelLightbake>>,
    pub reflect_probes: Vec<Option<Image>>,
    pub compact_vertices: asset_world::WorldVertexPayload,
    pub vertex_tier: Vec<u8>,
    pub facet_vertex_layer: Vec<i32>,
    pub facet_first_vertex: Vec<u32>,
    pub facet_draw_fields: Vec<asset_world::FaceDrawAttributes>,
    pub pos_list: Vec<[f32; 3]>,
    pub surface_directions: Vec<[f32; 3]>,
    pub tangent_list: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub texture_coords: Vec<[f32; 2]>,
    pub lightbake_uvs: Vec<[f32; 2]>,
    /// See `asset_world::LevelDrawCall::t6_layer_uvs`.
    pub t6_layer_uvs: Vec<[f32; 4]>,
    pub compact_indices: Vec<u32>,
    pub facet_index_ranges: Vec<(u32, u32)>,
    pub facet_batch_ranges: Vec<(usize, u32, u32)>,
    pub facet_materials: Vec<Option<assets::SurfaceMaterialId>>,
    pub facet_lightbake_indices: Vec<u8>,
    pub facet_env_probes: Vec<u8>,
    pub facet_key_lights: Vec<u8>,
    pub warp_sort_key: Option<u32>,
    pub facet_sort_keys: Vec<u8>,
    pub capture: asset_world::LevelSnapshot,
    pub solid_models: Vec<asset_world::SolidModelFaces>,
    pub solid_model_bounds: Vec<asset_world::SolidModelExtents>,
    pub solid_model_level_from_local: Vec<Mat4>,
    pub t6_brush_surfaces: Vec<asset_world::T6BrushSurfaces>,
    pub prop_meshes: Vec<LevelPropMesh>,
    pub prop_instances: Vec<Option<FixedPropPlacement>>,
    pub map_figure_stage_assets: asset_world::LevelModelPlacementRoster,
    pub scripted_prop_placements: Vec<LevelScriptModelInstance>,
    pub prop_illum_samples: Vec<StaticPropLightSample>,
}
