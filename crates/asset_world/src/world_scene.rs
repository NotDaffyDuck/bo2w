//! A loaded world as the renderer takes it: batches, vertex rows, per-surface
//! tables, and the lights, probes and sky it was captured with.

use bevy::prelude::Mesh;

use crate::world_lights::{
    SnapshotLightDef, LevelLightZoneHull, LevelDominantLight, LevelEnvReflectionCapture,
    LevelShadowMesh, LevelSunlamp,
};
use crate::world_lightmap::{LevelLightbake, LevelLightmapHole};
use crate::mesh_report::LevelMeshMetrics;
use crate::world_visibility::VisibilityLevelData;
use crate::{T6BrushSurfaces, LevelSnapshot};

/// Where a brush model's surfaces start, and how many it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SolidModelFaces {
    pub start_facet: u16,
    pub facet_tally: u16,
    pub plain_face_total: u16,
}

/// A brush model's box, as centre and half-extent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SolidModelExtents {
    pub centre: [f32; 3],
    pub half: [f32; 3],
}

/// The per-surface numbers a draw call is built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceDrawAttributes {
    pub vertex_from: u32,
    pub tri_tally: u16,
    pub index_from: u32,
    pub lightbake_index: u8,
    pub env_probe_index: u8,
    pub key_light_index: u8,
}

/// The world's packed vertex rows, when it has them.
#[derive(Clone, Debug)]
pub enum WorldVertexPayload {
    /// bo2zm: Black Ops II world rows, as the map holds them.
    T6(Vec<[u8; asset_t6::T6_WORLD_VERTEX]>),
    /// No rows; says why.
    Unavailable { origin_layout: &'static str },
}

const NOT_INSTALLED: &str = "world vertex payload not installed";

impl Default for WorldVertexPayload {
    fn default() -> Self {
        let reason = NOT_INSTALLED;
        Self::Unavailable { origin_layout: reason }
    }
}

impl WorldVertexPayload {
    /// The rows of the first vertex stream, if they were installed
    /// (`dump_variety_name` says why they were not).
    pub fn type2_stream0(&self) -> Option<&[[u8; asset_t6::T6_WORLD_VERTEX]]> {
        match self {
            Self::T6(rows) => Some(rows),
            Self::Unavailable { .. } => None,
        }
    }

    /// A short name for log lines.
    pub fn dump_variety_name(&self) -> &'static str {
        if let Self::Unavailable { origin_layout: reason } = self {
            return reason;
        }
        "t6"
    }
}

/// One mesh drawn with one material.
#[derive(Clone, Debug)]
pub struct LevelDrawGroup {
    pub built_mesh: Mesh,
    pub compact_indices: Vec<u32>,
    pub mat_slot: Option<usize>,
    pub has_baked_light: bool,
    pub lightbake_index: u8,
    pub key_light_index: u8,
    pub env_probe_index: u8,
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
    pub heading: [f32; 3],
}

/// bo2zm: one flickering lamp: its primary light (`pl#`, the row of
/// `LevelDrawCall::t6_lights`), its own intensity, and the script keys the
/// client's `_lights.csc` hands the engine (`setmixerlightparam`): the
/// intensity range, the delay between changes, and the electrical bursts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6Flicker {
    pub light: u16,
    pub electrical: bool,
    pub base: f32,
    pub min: f32,
    pub max: f32,
    pub delay: [f32; 2],
    pub burst: [f32; 2],
    pub burst_time: f32,
    pub wait: [f32; 2],
}

/// bo2zm: one Black Ops II fog volume (`GfxWorldFogVolume`); its box is
/// zero in the data, its planes bound it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6FogVolume {
    /// Inside where `dot(n, p) + d <= 0` for every `[n, d]` (as exposure
    /// volumes).
    pub planes: Vec<[f32; 4]>,
    /// The fog banks it belongs to, a bit mask (bank 1 = bit 0); 0 = none.
    pub banks: u32,
    /// The two fog rows of `t6_lights` (slots 256 and 257) it draws with.
    pub rows: [[f32; 16]; 2],
}

/// bo2zm: one Black Ops II fog modifier volume (`GfxWorldFogModifierVolume`):
/// while the camera is in it the world fog's half distance is multiplied by
/// its depth scale and the fog and sun fog colours and opacities by its
/// colour adjust (the game also stores a height scale it does not draw with).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6FogModifier {
    /// Inside where `dot(n, p) + d <= 0` for every `[n, d]`.
    pub planes: Vec<[f32; 4]>,
    /// The fog banks it belongs to, a bit mask (as `T6FogVolume::banks`).
    pub banks: u32,
    pub depth_scale: f32,
    pub color_adjust: [f32; 4],
}

/// bo2zm: one Black Ops II exposure volume: the planes bounding it (inside
/// where `dot(n, p) + d <= 0` for every `[n, d]`), its exposure in stops,
/// its luminance increase and decrease scales and its feather range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6ExposureVolume {
    pub planes: Vec<[f32; 4]>,
    pub exposure: f32,
    pub increase_scale: f32,
    pub decrease_scale: f32,
    pub feather_range: f32,
}

/// bo2zm: a Black Ops II map's colour tables (`hdr_create_lut2dv` samples
/// one before the vision's grade): RGBA8 rows 1024 wide, 32 slices of 32x32
/// side by side (x % 32 red, x / 32 the blue slice, y % 32 green), one table
/// per 32 rows (Buried 3, the others 2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6Lut {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// bo2zm: a Black Ops II map's own values for the two lighting dvars its
/// client scripts tweak: `r_lightTweakSunLight` (the sun's brightness,
/// `sunParse.initWorldSun` `sunCd.w`; the sun colour is drawn times it) and
/// the `r_sky_intensity_*` four (`GfxWorld::skyDynIntensity`: angle0,
/// angle1, factor0, factor1), whose blend multiplies the sky; and
/// `r_lightTweakSunColor` (`sunCd.xyz`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct T6LightTweaks {
    pub sun_light: f32,
    pub sun_color: [f32; 3],
    pub sky_intensity: [f32; 4],
}

/// bo2zm: one Black Ops II colour table volume: its planes (inside where
/// `dot(n, p) + d <= 0` for every one), the banks it belongs to (the low
/// byte of its `control`), its table and its transition time (seconds; 0:
/// the game's quick default).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct T6LutVolume {
    pub side_planes: Vec<[f32; 4]>,
    pub banks: u32,
    pub table: u32,
    pub fade_duration: f32,
}

/// Everything captured from a world that the renderer draws it with.
#[derive(Clone, Debug)]
pub struct LevelDrawCall {
    // The draw itself.
    pub batches: Vec<LevelDrawGroup>,
    pub sky_dome_model: Option<crate::prop_meshes::StaticMeshData>,
    pub lightmap: Result<Vec<Option<LevelLightbake>>, LevelLightmapHole>,
    pub tallies: LevelMeshMetrics,
    pub compact_vertices: WorldVertexPayload,
    pub compact_indices: Vec<u32>,
    pub capture: LevelSnapshot,
    pub dpvs: VisibilityLevelData,
    pub warp_sort_key: Option<u32>,

    // Vertex rows, one entry per world vertex.
    pub vertex_tier: Vec<u8>,
    pub spots: Vec<[f32; 3]>,
    pub surface_directions: Vec<[f32; 3]>,
    pub tangent_rows: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub texture_coords: Vec<[f32; 2]>,
    pub lightbake_uvs: Vec<[f32; 2]>,

    // Tables with one entry per surface.
    pub facet_vertex_layer: Vec<i32>,
    pub facet_first_vertex: Vec<u32>,
    pub facet_draw_fields: Vec<FaceDrawAttributes>,
    pub facet_index_ranges: Vec<(u32, u32)>,
    pub facet_batch_ranges: Vec<(usize, u32, u32)>,
    pub face_has_baked_light: Vec<bool>,
    pub facet_lightbake_indices: Vec<u8>,
    pub facet_env_probes: Vec<u8>,
    pub facet_key_lights: Vec<u8>,
    pub facet_materials: Vec<Option<usize>>,

    // Brush models.
    pub solid_models: Vec<SolidModelFaces>,
    pub solid_model_bounds: Vec<SolidModelExtents>,

    // Lights and probes.
    pub key_lights: Vec<LevelDominantLight>,
    pub light_defs: Vec<SnapshotLightDef>,
    pub daylight_key_light_tally: u32,
    pub light_zone_hulls: Option<Vec<Vec<LevelLightZoneHull>>>,
    pub shadow_geometry: Vec<LevelShadowMesh>,
    pub reflection_probes: Vec<LevelEnvReflectionCapture>,

    // The outdoor-light lookup.
    pub exterior_texture_label: Option<String>,
    pub openair_image: Option<usize>,
    pub openair_lookup: [u32; 16],

    // The sky and sun.
    pub sun_effects: Option<SunEffectsCapture>,
    pub classic_daylight_parse_exposure: Option<f32>,
    pub classic_sky_dynamic_intensity: Option<[f32; 4]>,
    pub t5_sun_light: Option<LevelSunlamp>,
    pub t5_foliage_scatter_strength: Option<f32>,
    pub t5_foliage_scatter_share: Option<f32>,
    pub classic_exposure_volume_tally: u32,

    // bo2zm: what a Black Ops II world adds; all empty for any other.
    /// The saturation of the map's vision (see `T6WorldLighting::saturation`).
    pub t6_vision_saturation: Option<f32>,
    /// How the sky decodes and fogs (see `T6WorldLighting::sky_parms`).
    pub t6_sky_parms: Option<[f32; 2]>,
    /// The map's vision colour grade (see `T6WorldLighting::grade`).
    pub t6_vision_grade: Option<[[f32; 4]; 14]>,
    /// The other visions a script can turn on, by file
    /// (`vision/zm_afterlife.vision`), graded as `t6_vision_grade`.
    pub t6_script_grades: Vec<(String, [[f32; 4]; 14])>,
    /// Primary lights for the fallback draw, four vec4 each (see
    /// `assets::lane::t6`).
    pub t6_lights: Vec<[f32; 16]>,
    /// Fog volumes (the client scripts' `setworldfogactivebank` picks which
    /// banks are on).
    pub t6_fog_banks: Vec<T6FogVolume>,
    /// Fog modifier volumes (`GfxWorld::worldFogModifierVolumes`).
    pub t6_fog_modifiers: Vec<T6FogModifier>,
    /// Exposure volumes (`GfxWorld::exposureVolumes`): the camera's volume
    /// sets the exposure.
    pub t6_exposure_volumes: Vec<T6ExposureVolume>,
    /// The map's own colour table (`GfxWorld::lutMaterial`, see `T6Lut`).
    pub t6_lut: Option<std::sync::Arc<T6Lut>>,
    /// The volumes that pick among its tables (`GfxWorld::lutVolumes`).
    pub t6_lut_volumes: Vec<T6LutVolume>,
    /// The map's sun light and sky intensity (see `T6LightTweaks`).
    pub t6_light_tweaks: Option<T6LightTweaks>,
    /// The map's flickering lamps (map `light` entities whose targetname is
    /// one of the engine's built-in mixer behaviours, `fire_flicker` /
    /// `electrical_flicker`).
    pub t6_flicker: Vec<T6Flicker>,
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
        let empty = WorldVertexPayload::default();
        assert!(empty.type2_stream0().is_none());
        assert_eq!(empty.dump_variety_name(), NOT_INSTALLED);
    }

    #[test]
    fn t6_rows_read_back_as_they_were_given() {
        let rows = WorldVertexPayload::T6(vec![[7; asset_t6::T6_WORLD_VERTEX]]);
        assert_eq!(rows.dump_variety_name(), "t6");
        assert_eq!(rows.type2_stream0().unwrap().len(), 1);
    }
}
