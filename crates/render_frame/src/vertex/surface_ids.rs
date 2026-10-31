//! Small selectors a surface carries, and the tess-info decoding for dynamic
//! models.

/// Index of the reflection probe a surface samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceCubeProbeId(pub u8);

/// Index of a lightmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceBakedLightId(pub u8);

/// The per-surface texture picks that are not part of the material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FacetSamplerInputs {
    pub reflection_probe: Option<SurfaceCubeProbeId>,
    pub primary_lightmap: Option<SurfaceBakedLightId>,
    pub secondary_lightmap: Option<SurfaceBakedLightId>,
}

/// Size, in indices, of the buffer dynamic geometry streams into.
pub const LIVE_INDEX_BUFFER_CAP: u32 = 1 << 20;

/// Whether a tess-info byte selects the packed vertex arm (values 1, 3 and 4).
#[must_use]
pub const fn model_tess_packed_arm(tessellation_info_flags: u8) -> bool {
    matches!(tessellation_info_flags, 1 | 3 | 4)
}

/// Vertex declaration type for a tess-info byte: 1 on the packed arm, else 2.
#[must_use]
pub const fn model_tess_vert_decl(tessellation_info_flags: u8) -> u8 {
    if model_tess_packed_arm(tessellation_info_flags) { 1 } else { 2 }
}
