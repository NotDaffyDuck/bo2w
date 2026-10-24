//! A loaded world as the renderer takes it: batches, vertex rows, per-surface
//! tables, and the lights, probes and sky it was captured with.

use bevy::prelude::Mesh;

use crate::world_lights::{
    CapturedLightDef, WorldLightRegionHull, WorldPrimaryLight, WorldReflectionProbe,
    WorldShadowGeometry, WorldSunLight,
};
use crate::world_lightmap::{WorldLightmap, WorldLightmapGap};
use crate::mesh_report::WorldMeshStats;
use crate::world_visibility::DpvsWorldData;
use crate::{T6BrushSurfaces, WorldCapture};

/// Where a brush model's surfaces start, and how many it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxBrushModelSurfs {
    pub start_surf: u16,
    pub surface_count: u16,
    pub surface_count_no_decal: u16,
}

/// A brush model's box, as centre and half-extent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GfxBrushModelBounds {
    pub mid: [f32; 3],
    pub half: [f32; 3],
}

/// The per-surface numbers a draw call is built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceDrawFields {
    pub first_vertex: u32,
    pub tri_count: u16,
    pub base_index: u32,
    pub lightmap_index: u8,
    pub reflection_probe_index: u8,
    pub primary_light_index: u8,
}

/// The world's packed vertex rows, when it has them.
#[derive(Clone, Debug)]
pub enum WorldVertexPayload {
    /// bo2zm: Black Ops II world rows, as the map holds them.
    T6(Vec<[u8; asset_t6::T6_WORLD_VERTEX]>),
    /// No rows; says why.
    Unavailable { source_layout: &'static str },
}

impl Default for WorldVertexPayload {
    fn default() -> Self {
        Self::Unavailable {
            source_layout: "world vertex payload not installed",
        }
    }
}

impl WorldVertexPayload {
    /// The rows of the first vertex stream, or why there are none.
    pub fn type2_stream0(&self) -> Result<&[[u8; asset_t6::T6_WORLD_VERTEX]], &'static str> {
        match self {
            Self::T6(rows) => Ok(rows),
            Self::Unavailable { source_layout } => Err(source_layout),
        }
    }

    /// A short name for log lines.
    pub fn dump_kind(&self) -> &'static str {
        match self {
            Self::T6(_) => "t6",
            Self::Unavailable { source_layout } => source_layout,
        }
    }
}

/// One mesh drawn with one material.
#[derive(Clone, Debug)]
pub struct WorldBatch {
    pub mesh: Mesh,
    pub packed_indices: Vec<u32>,
    pub material: Option<usize>,
    pub lightmapped: bool,
    pub lightmap_index: u8,
    pub primary_light_index: u8,
    pub reflection_probe_index: u8,
}

/// The sun's glow, flare, blinding and glare, from the zone. Material slots
/// are indices local to the zone's walk.
#[derive(Clone, Copy, Debug)]
pub struct SunEffectsCapture {
    pub sprite_material: Option<usize>,
    pub flare_material: Option<usize>,
    pub sprite_size: f32,
    pub flare_min_size: f32,
    pub flare_min_dot: f32,
    pub flare_max_size: f32,
    pub flare_max_dot: f32,
    pub flare_max_alpha: f32,
    pub flare_fade_in_ms: i32,
    pub flare_fade_out_ms: i32,
    pub blind_min_dot: f32,
    pub blind_max_dot: f32,
    pub blind_max_darken: f32,
    pub blind_fade_in_ms: i32,
    pub blind_fade_out_ms: i32,
    pub glare_min_dot: f32,
    pub glare_max_dot: f32,
    pub glare_max_lighten: f32,
    pub glare_fade_in_ms: i32,
    pub glare_fade_out_ms: i32,
    pub direction: [f32; 3],
}

/// Everything captured from a world that the renderer draws it with.
#[derive(Clone, Debug)]
pub struct WorldDraw {
    // The draw itself.
    pub batches: Vec<WorldBatch>,
    pub sky_model: Option<crate::model_draw::ModelMesh>,
    pub lightmap: Result<Vec<Option<WorldLightmap>>, WorldLightmapGap>,
    pub stats: WorldMeshStats,
    pub packed_vertices: WorldVertexPayload,
    pub packed_indices: Vec<u32>,
    pub capture: WorldCapture,
    pub dpvs: DpvsWorldData,
    pub sort_key_distortion: Option<u32>,

    // Vertex rows, one entry per world vertex.
    pub vertex_layer: Vec<u8>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub texture_uvs: Vec<[f32; 2]>,
    pub lightmap_uvs: Vec<[f32; 2]>,

    // Tables with one entry per surface.
    pub surface_vertex_layer: Vec<i32>,
    pub surface_first_vertex: Vec<u32>,
    pub surface_draw_fields: Vec<SurfaceDrawFields>,
    pub surface_index_ranges: Vec<(u32, u32)>,
    pub surface_batch_ranges: Vec<(usize, u32, u32)>,
    pub surface_lightmapped: Vec<bool>,
    pub surface_lightmap_indices: Vec<u8>,
    pub surface_reflection_probes: Vec<u8>,
    pub surface_primary_lights: Vec<u8>,
    pub surface_materials: Vec<Option<usize>>,

    // Brush models.
    pub brush_models: Vec<GfxBrushModelSurfs>,
    pub brush_model_bounds: Vec<GfxBrushModelBounds>,

    // Lights and probes.
    pub primary_lights: Vec<WorldPrimaryLight>,
    pub light_defs: Vec<CapturedLightDef>,
    pub sun_primary_light_count: u32,
    pub light_region_hulls: Option<Vec<Vec<WorldLightRegionHull>>>,
    pub shadow_geometry: Vec<WorldShadowGeometry>,
    pub reflection_probes: Vec<WorldReflectionProbe>,

    // The outdoor-light lookup.
    pub outdoor_image_name: Option<String>,
    pub outdoor_image: Option<usize>,
    pub outdoor_lookup: [u32; 16],

    // The sky and sun.
    pub sun_effects: Option<SunEffectsCapture>,
    pub t5_sun_parse_exposure: Option<f32>,
    pub t5_sky_dynamic_intensity: Option<[f32; 4]>,
    pub t5_sun_light: Option<WorldSunLight>,
    pub t5_tree_scatter_intensity: Option<f32>,
    pub t5_tree_scatter_amount: Option<f32>,
    pub t5_exposure_volume_count: u32,

    // bo2zm: what a Black Ops II world adds; all empty for any other.
    /// Primary lights for the fallback draw, four vec4 each (see
    /// `assets::lane::t6`).
    pub t6_lights: Vec<[f32; 16]>,
    /// Fog banks (a fog volume's bank, as the client scripts'
    /// `setworldfogactivebank` picks): per bank, the two fog rows of
    /// `t6_lights` (slots 30 and 31) it draws with.
    pub t6_fog_banks: Vec<(u32, [[f32; 16]; 2])>,
    /// Per reflection probe, its `lightingSH` (three vec4) for the fallback
    /// draw's reflections.
    pub t6_probes: Vec<[[f32; 4]; 3]>,
    /// Per world vertex (parallel to `positions`), a layered surface's layer
    /// 1 and 2 texcoords.
    pub t6_layer_uvs: Vec<[f32; 4]>,
    /// codmp: the `script_brushmodel` submodels the world draws, their
    /// surfaces placed where their entities stand.
    pub t6_brush_surfaces: Vec<T6BrushSurfaces>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_without_rows_says_why() {
        let payload = WorldVertexPayload::default();
        assert_eq!(
            payload.type2_stream0().unwrap_err(),
            "world vertex payload not installed"
        );
        assert_eq!(payload.dump_kind(), "world vertex payload not installed");
    }

    #[test]
    fn t6_rows_read_back_as_they_were_given() {
        let payload = WorldVertexPayload::T6(vec![[7; asset_t6::T6_WORLD_VERTEX]]);
        assert_eq!(payload.type2_stream0().unwrap().len(), 1);
        assert_eq!(payload.dump_kind(), "t6");
    }
}
