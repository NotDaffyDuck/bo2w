//! A mark's cut pieces before they go into the pool: triangles that name their corners by
//! number, and the corner points.

use crate::GfxMarkContext;

/// The most triangles one mark may stage.
pub const DECAL_FRAGMENT_TRIANGLE_CEILING: u32 = 0xff;
/// The most points one mark may stage.
pub const DECAL_FRAGMENT_VERTEX_CEILING: u32 = 0x2fd;

/// One staged triangle: its three corners (numbers into the mark's points) and the surface
/// it was cut from. Laid out as BO2 lays it out, 14 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectStainStagingTri {
    pub indices: [u16; 3],
    pub context: GfxMarkContext,
    pub tail_fill: u8,
}

const _: () = assert!(size_of::<EffectStainStagingTri>() == 14);

impl EffectStainStagingTri {
    pub const ZERO: Self = Self {
        indices: [0; 3],
        context: GfxMarkContext::ZERO,
        tail_fill: 0,
    };
}

/// One staged point: where it is, its spot on the surface's lightmap and the surface's
/// normal there. 32 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectStainStagingPoint {
    pub spot: [f32; 3],
    pub baked_light_uv: [f32; 2],
    pub normal: [f32; 3],
}

const _: () = assert!(size_of::<EffectStainStagingPoint>() == 32);

impl EffectStainStagingPoint {
    pub const ZERO: Self = Self {
        spot: [0.0; 3],
        baked_light_uv: [0.0; 2],
        normal: [0.0; 3],
    };
}
