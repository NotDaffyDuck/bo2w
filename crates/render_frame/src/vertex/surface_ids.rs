//! Small selectors a surface carries, and the tess-info decoding for dynamic
//! models.

/// Index of the reflection probe a surface samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceReflectionProbeId(pub u8);

/// Index of a lightmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceLightmapId(pub u8);

/// The per-surface texture picks that are not part of the material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SurfaceSamplerInputs {
    pub reflection_probe: Option<SurfaceReflectionProbeId>,
    pub primary_lightmap: Option<SurfaceLightmapId>,
    pub secondary_lightmap: Option<SurfaceLightmapId>,
}

/// Size, in indices, of the buffer dynamic geometry streams into.
pub const DYNAMIC_INDEX_BUFFER_CAPACITY: u32 = 1 << 20;

/// Whether a tess-info byte selects the packed vertex arm (values 1, 3 and 4).
#[must_use]
pub const fn xmodel_tess_info_packed_arm(tess_info_byte_10: u8) -> bool {
    matches!(tess_info_byte_10, 1 | 3 | 4)
}

/// Vertex declaration type for a tess-info byte: 1 on the packed arm, else 2.
#[must_use]
pub const fn xmodel_tess_info_vert_decl_type(tess_info_byte_10: u8) -> u8 {
    if xmodel_tess_info_packed_arm(tess_info_byte_10) { 1 } else { 2 }
}
