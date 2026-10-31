//! Frame publication: the installed world and the per-frame data the extract
//! step seals and the render world reads. Moved out of the SM3 colour pass.

use super::ExtractedRenderFrameOutputs;
use bevy::prelude::*;
use render_frame::{MaterialRunFrame, DaylightShadowForcedFrame};
use render_material::MaterialEpochId;
use std::sync::Arc;

pub const ENGINE_TEXTURE_FLOATZ: u32 = 0x0f;

pub const ENGINE_TEXTURE_RESOLVED_POST_DAYLIGHT: u32 = 9;

pub const ENGINE_TEXTURE_SHADOW_MAP_DAYLIGHT: u32 = 6;

pub const ENGINE_TEXTURE_SHADOW_MAP_CONE: u32 = 7;

/// bo2mp: BO2's UI3D texture (the sight screens' 512 x 512 picture).
pub const CODE_TEXTURE_UI3D: u32 = 0x21;

/// Geometry that survives frame replacement within a world generation.
#[derive(Clone, Debug, Default)]
pub struct StaticGeometryExtract {
    pub level_vertex_total: Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>,
    pub world_layer: Arc<Vec<u8>>,
    pub level_indices: Arc<Vec<u32>>,
    pub world_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub world_vertex_refusal: Option<render_frame::WorldVertexRefusal>,
    pub static_model_verts_used: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub static_model_slots_used: Arc<Vec<u32>>,
    pub smodel_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub smodel_vertex_refusal: Option<render_frame::PackedVertexRefusal>,
    pub smodel_cached_vertices: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub smodel_surface_verts: Arc<Vec<(u32, u32)>>,
}

/// Installed world and material resources. Longer-lived than a colour frame:
/// static geometry and image handles stay here.
#[derive(Resource, Clone, Debug, Default)]
pub struct InstalledRenderWorld(Arc<RenderWorldData>);

impl InstalledRenderWorld {
    pub fn new(data: RenderWorldData) -> Self {
        Self(Arc::new(data))
    }
}

impl std::ops::Deref for InstalledRenderWorld {
    type Target = RenderWorldData;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Consumed by publication; no mutable access through an installed handle.
#[derive(Clone, Debug, Default)]
pub struct RenderWorldData {
    pub generation: MaterialEpochId,
    pub level_generation: frame::LevelGeneration,
    pub world_products: frame::WorldProducts,
    pub prop_cache_edition: Option<u64>,
    pub static_geometry: Arc<StaticGeometryExtract>,
    pub prop_prebuilt_indices: std::sync::Arc<Vec<u16>>,
    pub prop_index_layout_edition: u64,
    pub smc_index_baked: Arc<Vec<u16>>,
    pub image_handles: super::gpu_bufs::LiveImageHandles,
    pub sun_effects: Option<render_frame::SunEffectsDef>,
}

/// Commands and changed data of the current frame. Names the installed world by
/// generation instead of carrying world/material tables again.
#[derive(Resource, Clone, Debug, Default)]
pub struct PublishedRenderFrame {
    world: InstalledRenderWorld,
    data: Arc<RenderFrameData>,
}

impl PublishedRenderFrame {
    pub fn seal(world: InstalledRenderWorld, data: RenderFrameData) -> Self {
        assert_eq!(world.generation, data.generation);
        assert_eq!(world.level_generation, data.level_generation);
        Self {
            world,
            data: Arc::new(data),
        }
    }
    pub fn world(&self) -> &InstalledRenderWorld {
        &self.world
    }
}

impl std::ops::Deref for PublishedRenderFrame {
    type Target = RenderFrameData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderFrameData {
    pub frame_products: ExtractedRenderFrameOutputs,
    pub generation: MaterialEpochId,
    pub level_generation: frame::LevelGeneration,
    pub sun_shadow: Option<DaylightShadowForcedFrame>,
    pub sun_effects: Option<render_frame::SunEffectsFrame>,
    pub prewarmed_pipelines: bool,
    pub pipeline_level_materials: Arc<std::collections::HashSet<u16>>,
    pub pipeline_prop_materials: Arc<std::collections::HashSet<u16>>,
    pub pipeline_demand_revision: u64,
    pub smc_vb_patches: Vec<(lighting_t6::PropCachePatchLock, Vec<u8>)>,
    pub prop_cache_ib_patches: Vec<(u32, Vec<u8>)>,
    pub xmodel_verts: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub figure_indices: Arc<Vec<u32>>,
    pub figure_facet_ranges: Arc<Vec<(u32, u32)>>,
    pub xmodel_vertex_refusal: Option<render_frame::PackedVertexRefusal>,
    pub fx_verts: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub effect_indices: Arc<Vec<u32>>,
    pub fx_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub fx_vertex_refusal: Option<render_frame::PackedVertexRefusal>,
    pub effect_edition: u64,
    pub figure_edition: u64,
    pub figure_topology_edition: u64,
    pub xmodel_packed_segments: render_frame::CompactSegments,
    pub cloud_verts: Arc<Vec<[u8; fx_t6::runtime::POINT_UV_CORNER_BYTES]>>,
    pub cloud_indices: Arc<Vec<u32>>,
    pub particle_cloud_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub particle_cloud_template: (usize, usize),
    pub particle_cloud_revision: u64,
    pub mark_verts: Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>,
    pub stain_mesh_indices: Arc<Vec<u16>>,
    pub mark_mesh_surface_ranges: Arc<Vec<(u32, u32)>>,
    /// Per mark-mesh draw: its vertices are model rows (packed), not world rows.
    pub mark_mesh_surface_packed: Arc<Vec<bool>>,
    pub stain_mesh_edition: u64,
    pub exec_frame: MaterialRunFrame,
}
