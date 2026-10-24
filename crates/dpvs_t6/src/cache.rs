use crate::clip::MAX_CLIP_PLANES;

/// DpvsCachedCell: one visit of a visible cell and the planes it was seen through. A cell
/// seen through two portals has two entries. `frustum_plane_count` is non-zero only for the
/// start cell, whose planes are the camera's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CachedCell {
    pub cell: u16,
    pub plane_index: u32,
    pub plane_count: u8,
    pub frustum_plane_count: u8,
}

/// One visit's planes copied out of the walk, for callers that keep a cell's plane set
/// after the scratch is reused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellClip {
    pub planes: [[f32; 4]; MAX_CLIP_PLANES],
    pub plane_count: u8,
    pub frustum_plane_count: u8,
}

impl CellClip {
    pub const EMPTY: Self = Self {
        planes: [[0.0; 4]; MAX_CLIP_PLANES],
        plane_count: 0,
        frustum_plane_count: 0,
    };

    /// Copies up to `MAX_CLIP_PLANES` of `planes`.
    #[must_use]
    pub fn new(planes: &[[f32; 4]], frustum_plane_count: u8) -> Self {
        let mut clip = Self::EMPTY;
        let n = planes.len().min(MAX_CLIP_PLANES);
        clip.planes[..n].copy_from_slice(&planes[..n]);
        clip.plane_count = n as u8;
        clip.frustum_plane_count = frustum_plane_count;
        clip
    }

    #[must_use]
    pub fn as_slice(&self) -> &[[f32; 4]] {
        &self.planes[..usize::from(self.plane_count).min(MAX_CLIP_PLANES)]
    }
}

/// dpvsGlob.childPlanes holds 0x800 planes for every visit of a walk.
pub const PLANE_POOL_MAX: usize = 0x800;
