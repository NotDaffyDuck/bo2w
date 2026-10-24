//! The pieces a world scene is assembled from: draw batches, lightmap pages,
//! static models and the dynamic-entity brush records.

use bevy::prelude::*;
use dpvs_t6::Bounds;

pub use render_scene::{WorldDynEntInstance, WorldScriptModelInstance};

/// One mesh drawn with one material, lightmap page and primary light.
#[derive(Debug)]
pub struct WorldBatchGeometry {
    pub mesh: Mesh,
    pub packed_indices: Vec<u32>,
    pub material: Option<assets::MaterialIndex>,
    pub lightmapped: bool,
    pub lightmap_index: u8,
    pub primary_light_index: u8,
    pub reflection_probe_index: u8,
    pub sun: Option<WorldSun>,
}

#[derive(Clone, Copy, Debug)]
pub struct WorldSun {
    pub direction: [f32; 3],
    pub color: [f32; 3],
}

/// The images of one lightmap page.
#[derive(Debug)]
pub struct WorldLightmap {
    pub primary_image: Option<Image>,
    pub secondary_image: Option<Image>,
    pub secondary_b_image: Option<Image>,
    pub ambient_image: Image,
    pub directional_image: Image,
    pub sun_mask_image: Image,
    pub ambient_source_name: String,
    pub sun_mask_source_name: String,
    pub ambient_size: UVec2,
    pub sun_mask_size: UVec2,
}

/// One sampled lighting tile of a static model.
#[derive(Clone, Debug)]
pub struct WorldSmodelLightingSample {
    pub authored_slot: usize,
    pub lighting_origin: [f32; 3],
    pub tile_rgba: [u8; 256],
    pub packed_lighting: [u8; 4],
}

#[derive(Debug)]
pub struct WorldStaticModelMesh {
    pub name: String,
    pub lod_surfaces: [Vec<WorldStaticModelSurface>; 4],
    pub lod_smc: Option<[[u8; 4]; 4]>,
    pub lod: Option<asset_model::ModelLodSelector>,
}

impl WorldStaticModelMesh {
    pub fn surface_materials(&self) -> impl Iterator<Item = Option<assets::MaterialIndex>> + '_ {
        self.lod_surfaces
            .iter()
            .flatten()
            .map(|surface| surface.material)
    }
}

#[derive(Debug)]
pub struct WorldStaticModelSurface {
    pub mesh: Mesh,
    pub material: Option<assets::MaterialIndex>,
    pub packed_vertices: Vec<[u8; asset_t6::T6_PACKED_VERTEX]>,
    pub xsurface_plus_1: Option<u8>,
    pub xsurface_base_index: u16,
    pub xsurface_vert_offset: u16,
    pub collision: asset_world::XSurfaceCollisionPayload,
}

#[derive(Clone, Copy, Debug)]
pub struct WorldStaticModelInstance {
    pub mesh: usize,
    pub transform: Transform,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub scale: f32,
    pub cull_dist: u16,
    pub reflection_probe_index: u8,
    pub primary_light_index: u8,
    pub flags: u8,
    /// See `asset_world::StaticModelPlacement::packed_lighting`.
    pub packed_lighting: Option<[u8; 4]>,
    /// See `asset_world::StaticModelPlacement::t6_primary`.
    pub t6_primary: [u8; 2],
    /// See `asset_world::StaticModelPlacement::t6_light`.
    pub t6_light: u32,
}

/// CPU copy of the static-model geometry that decal marking reads.
#[derive(Clone, Debug, Default)]
pub struct SmodelMarkCpu {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub meshes: Vec<Vec<SmodelMarkSurface>>,
}

#[derive(Clone, Debug)]
pub struct SmodelMarkSurface {
    pub index_start: u32,
    pub index_count: u32,
    pub material: Option<assets::MaterialIndex>,
    pub collision: asset_world::XSurfaceCollisionPayload,
}

/// A brush-model dynamic entity with its posed bounds.
#[derive(Clone, Copy, Debug)]
pub struct WorldDynEntBrush {
    pub index: u16,
    pub origin: [f32; 3],
    pub bounds: Option<Bounds>,
    pub brush_model: u16,
    pub surface_count: u16,
}

/// Everything `WorldScene::from_draw` takes in.
#[derive(Debug)]
pub struct WorldDrawGeometry {
    pub batches: Vec<WorldBatchGeometry>,
    pub lightmaps: Vec<Option<WorldLightmap>>,
    pub reflection_probes: Vec<Option<Image>>,
    pub packed_vertices: asset_world::WorldVertexPayload,
    pub vertex_layer: Vec<u8>,
    pub surface_vertex_layer: Vec<i32>,
    pub surface_first_vertex: Vec<u32>,
    pub surface_draw_fields: Vec<asset_world::SurfaceDrawFields>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub texture_uvs: Vec<[f32; 2]>,
    pub lightmap_uvs: Vec<[f32; 2]>,
    /// See `asset_world::WorldDraw::t6_layer_uvs`.
    pub t6_layer_uvs: Vec<[f32; 4]>,
    pub packed_indices: Vec<u32>,
    pub surface_index_ranges: Vec<(u32, u32)>,
    pub surface_batch_ranges: Vec<(usize, u32, u32)>,
    pub surface_materials: Vec<Option<assets::MaterialIndex>>,
    pub surface_lightmap_indices: Vec<u8>,
    pub surface_reflection_probes: Vec<u8>,
    pub surface_primary_lights: Vec<u8>,
    pub sort_key_distortion: Option<u32>,
    pub surface_sort_keys: Vec<u8>,
    pub capture: asset_world::WorldCapture,
    pub brush_models: Vec<asset_world::GfxBrushModelSurfs>,
    pub brush_model_bounds: Vec<asset_world::GfxBrushModelBounds>,
    pub bmodel_world_from_local: Vec<Mat4>,
    pub t6_brush_surfaces: Vec<asset_world::T6BrushSurfaces>,
    pub static_model_meshes: Vec<WorldStaticModelMesh>,
    pub static_model_instances: Vec<Option<WorldStaticModelInstance>>,
    pub map_xmodel_scene_assets: asset_world::MapXModelSceneCatalog,
    pub script_model_instances: Vec<WorldScriptModelInstance>,
    pub smodel_lighting_samples: Vec<WorldSmodelLightingSample>,
}
