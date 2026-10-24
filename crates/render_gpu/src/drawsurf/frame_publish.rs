//! Frame publication: the installed world and the per-frame data the extract
//! step seals and the render world reads. Moved out of the SM3 colour pass.

use super::ExtractedRenderFrameProducts;
use bevy::prelude::*;
use render_frame::{MaterialExecFrame, SunShadowForcedFrame};
use render_material::MaterialGenerationId;
use std::sync::Arc;

pub const CODE_TEXTURE_FLOATZ: u32 = 0x0f;

pub const CODE_TEXTURE_RESOLVED_POST_SUN: u32 = 9;

pub const CODE_TEXTURE_SHADOWMAP_SUN: u32 = 6;

pub const CODE_TEXTURE_SHADOWMAP_SPOT: u32 = 7;

/// bo2mp: BO2's UI3D texture (the sight screens' 512 x 512 picture).
pub const CODE_TEXTURE_UI3D: u32 = 0x21;

/// Geometry that survives frame replacement within a world generation.
#[derive(Clone, Debug, Default)]
pub struct ExtractedStaticGeometry {
    pub world_vertices: Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>,
    pub world_layer: Arc<Vec<u8>>,
    pub world_indices: Arc<Vec<u32>>,
    pub world_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub world_vertex_refusal: Option<render_frame::WorldVertexRefusal>,
    pub smodel_vertices: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub smodel_indices: Arc<Vec<u32>>,
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
    pub generation: MaterialGenerationId,
    pub world_generation: frame::WorldGeneration,
    pub world_products: frame::WorldProducts,
    pub smc_revision: Option<u64>,
    pub static_geometry: Arc<ExtractedStaticGeometry>,
    pub smodel_pretess_indices: std::sync::Arc<Vec<u16>>,
    pub smodel_index_layout_revision: u64,
    pub smc_index_baked: Arc<Vec<u16>>,
    pub image_handles: super::gpu_resources::RuntimeImageHandles,
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
        assert_eq!(world.world_generation, data.world_generation);
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
    pub frame_products: ExtractedRenderFrameProducts,
    pub generation: MaterialGenerationId,
    pub world_generation: frame::WorldGeneration,
    pub sun_shadow: Option<SunShadowForcedFrame>,
    pub sun_effects: Option<render_frame::SunEffectsFrame>,
    pub warm_pipelines: bool,
    pub pipeline_world_materials: Arc<std::collections::HashSet<u16>>,
    pub pipeline_smodel_materials: Arc<std::collections::HashSet<u16>>,
    pub pipeline_demand_revision: u64,
    pub smc_vb_patches: Vec<(lighting_t6::SmcPatchLock, Vec<u8>)>,
    pub smc_ib_patches: Vec<(u32, Vec<u8>)>,
    pub xmodel_vertices: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub xmodel_indices: Arc<Vec<u32>>,
    pub xmodel_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub xmodel_vertex_refusal: Option<render_frame::PackedVertexRefusal>,
    pub fx_vertices: Arc<Vec<[u8; asset_t6::T6_PACKED_VERTEX]>>,
    pub fx_indices: Arc<Vec<u32>>,
    pub fx_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub fx_vertex_refusal: Option<render_frame::PackedVertexRefusal>,
    pub fx_revision: u64,
    pub xmodel_revision: u64,
    pub xmodel_topology_revision: u64,
    pub xmodel_packed_segments: render_frame::PackedSegments,
    pub particle_cloud_vertices: Arc<Vec<[u8; fx_t6::runtime::GFX_POS_TEX_VERTEX_STRIDE]>>,
    pub particle_cloud_indices: Arc<Vec<u32>>,
    pub particle_cloud_surface_ranges: Arc<Vec<(u32, u32)>>,
    pub particle_cloud_template: (usize, usize),
    pub particle_cloud_revision: u64,
    pub mark_mesh_vertices: Arc<Vec<[u8; asset_t6::T6_WORLD_VERTEX]>>,
    pub mark_mesh_indices: Arc<Vec<u16>>,
    pub mark_mesh_surface_ranges: Arc<Vec<(u32, u32)>>,
    /// Per mark-mesh draw: its vertices are model rows (packed), not world rows.
    pub mark_mesh_surface_packed: Arc<Vec<bool>>,
    pub mark_mesh_revision: u64,
    pub exec_frame: MaterialExecFrame,
}
