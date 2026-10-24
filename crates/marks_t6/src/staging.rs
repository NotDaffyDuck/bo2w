//! A mark's cut pieces before they go into the pool: triangles that name their corners by
//! number, and the corner points.

use crate::GfxMarkContext;

/// The most triangles one mark may stage.
pub const R_MARK_FRAGMENTS_MAX_TRIS: u32 = 0xff;
/// The most points one mark may stage.
pub const R_MARK_FRAGMENTS_MAX_POINTS: u32 = 0x2fd;

/// One staged triangle: its three corners (numbers into the mark's points) and the surface
/// it was cut from. Laid out as BO2 lays it out, 14 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxMarkStagingTri {
    pub indices: [u16; 3],
    pub context: GfxMarkContext,
    pub _pad: u8,
}

const _: () = assert!(size_of::<FxMarkStagingTri>() == 14);

impl FxMarkStagingTri {
    pub const ZERO: Self = Self {
        indices: [0; 3],
        context: GfxMarkContext::ZERO,
        _pad: 0,
    };
}

/// One staged point: where it is, its spot on the surface's lightmap and the surface's
/// normal there. 32 bytes, as in BO2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxMarkStagingPoint {
    pub xyz: [f32; 3],
    pub lmap_coord: [f32; 2],
    pub normal: [f32; 3],
}

const _: () = assert!(size_of::<FxMarkStagingPoint>() == 32);

impl FxMarkStagingPoint {
    pub const ZERO: Self = Self {
        xyz: [0.0; 3],
        lmap_coord: [0.0; 2],
        normal: [0.0; 3],
    };
}
