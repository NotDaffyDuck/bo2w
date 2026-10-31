use crate::cut::CLIP_PLANE_CEILING;

/// DpvsCachedCell: one visit of a visible cell and the planes it was seen through. A cell
/// seen through two portals has two entries. `view_wall_total` is non-zero only for the
/// start cell, whose planes are the camera's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CachedCell {
    pub cell: u16,
    pub plane_index: u32,
    pub plane_tally: u8,
    pub view_wall_total: u8,
}

/// One visit's planes copied out of the walk, for callers that keep a cell's plane set
/// after the scratch is reused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellClip {
    pub split_planes: [[f32; 4]; CLIP_PLANE_CEILING],
    pub plane_tally: u8,
    pub view_wall_total: u8,
}

impl CellClip {
    pub const BLANK: Self = Self {
        split_planes: [[0.0; 4]; CLIP_PLANE_CEILING],
        plane_tally: 0,
        view_wall_total: 0,
    };

    /// Copies up to `CLIP_PLANE_CEILING` of `planes`.
    #[must_use]
    pub fn new(planes: &[[f32; 4]], view_wall_total: u8) -> Self {
        let mut clip = Self::BLANK;
        let n = planes.len().min(CLIP_PLANE_CEILING);
        clip.split_planes[..n].copy_from_slice(&planes[..n]);
        clip.plane_tally = n as u8;
        clip.view_wall_total = view_wall_total;
        clip
    }

    #[must_use]
    pub fn as_slice(&self) -> &[[f32; 4]] {
        &self.split_planes[..usize::from(self.plane_tally).min(CLIP_PLANE_CEILING)]
    }
}

/// dpvsGlob.childPlanes holds 0x800 planes for every visit of a walk.
pub const PLANE_POOL_MAX: usize = 0x800;
