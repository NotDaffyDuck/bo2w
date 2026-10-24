//! The visibility side of a loaded world: its cells, portals, plane tree and
//! the bounds each cell draws from.

use dpvs_t6::{AabbTree, Bounds, CPlane};

use crate::mesh_report::{BoundsTable, WorldMeshError};

/// A doorway between two cells.
#[derive(Clone, Debug)]
pub struct OwnedPortal {
    pub plane: [f32; 4],
    /// The cell on the far side.
    pub neighbor: u16,
    /// Where this portal's corners sit in `DpvsWorldData::portal_verts`.
    pub vert_start: usize,
    pub vert_count: usize,
    /// Two axes of the portal's hull, when it has one.
    pub hull_axis: Option<[[f32; 3]; 2]>,
}

/// Which kind of surface a camera range holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraRangeKind {
    LitOpaque,
    LitTrans,
    Decal,
    Emissive,
}

/// A run of sorted surfaces `[begin, end)` of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraSurfRange {
    pub kind: CameraRangeKind,
    pub begin: u32,
    pub end: u32,
}

/// The camera's surface runs. There is always at least one, so the first is
/// kept apart from the rest.
#[derive(Clone, Debug)]
pub struct CameraSurfRanges {
    first: CameraSurfRange,
    rest: Vec<CameraSurfRange>,
}

impl CameraSurfRanges {
    pub fn new(first: CameraSurfRange, rest: Vec<CameraSurfRange>) -> Self {
        Self { first, rest }
    }

    pub fn iter(&self) -> impl Iterator<Item = CameraSurfRange> + '_ {
        std::iter::once(self.first).chain(self.rest.iter().copied())
    }
}

/// Everything the culler reads from a world.
#[derive(Clone, Debug)]
pub struct DpvsWorldData {
    pub planes: Vec<CPlane>,
    pub nodes: Vec<u16>,
    pub cell_count: usize,
    /// Per cell, its bounding-box tree.
    pub aabb_trees: Vec<Vec<AabbTree>>,
    /// Per cell, the static models its tree points at.
    pub aabb_smodel_indices: Vec<Vec<u16>>,
    pub sorted_surf_index: Vec<u16>,
    pub static_surface_count: usize,
    pub static_surface_count_no_decal: usize,
    pub surface_bounds: Vec<Bounds>,
    pub smodel_bounds: Vec<Bounds>,
    pub portal_verts: Vec<[f32; 3]>,
    pub portals_per_cell: Vec<Vec<OwnedPortal>>,
    pub cell_reflection_probes: Vec<Vec<u8>>,
    pub lit_opaque_begin: u32,
    pub lit_opaque_end: u32,
    pub camera_ranges: CameraSurfRanges,
    pub emissive_surfs_begin: u32,
    pub emissive_surfs_end: u32,
    /// How many boxes arrived already cleared (set by `checked`).
    pub cleared_boxes: usize,
}

impl DpvsWorldData {
    /// An empty world around its camera ranges.
    pub fn new(camera_ranges: CameraSurfRanges) -> Self {
        Self {
            planes: vec![],
            nodes: vec![],
            cell_count: 0,
            aabb_trees: vec![],
            aabb_smodel_indices: vec![],
            sorted_surf_index: vec![],
            static_surface_count: 0,
            static_surface_count_no_decal: 0,
            surface_bounds: vec![],
            smodel_bounds: vec![],
            portal_verts: vec![],
            portals_per_cell: vec![],
            cell_reflection_probes: vec![],
            lit_opaque_begin: 0,
            lit_opaque_end: 0,
            camera_ranges,
            emissive_surfs_begin: 0,
            emissive_surfs_end: 0,
            cleared_boxes: 0,
        }
    }

    /// Looks over every bounds table. A box with a negative half-extent is an
    /// error; a box that was deliberately cleared is counted and let through.
    pub(crate) fn checked(mut self) -> Result<Self, WorldMeshError> {
        let mut cleared = 0;
        for (cell, trees) in self.aabb_trees.iter().enumerate() {
            let boxes = trees.iter().map(AabbTree::bounds);
            cleared += count_cleared(BoundsTable::AabbNode { cell }, boxes)?;
        }
        let surfaces = self.surface_bounds.iter().copied();
        cleared += count_cleared(BoundsTable::Surface, surfaces)?;
        let models = self.smodel_bounds.iter().copied();
        cleared += count_cleared(BoundsTable::SmodelInst, models)?;
        self.cleared_boxes = cleared;
        Ok(self)
    }
}

/// How many of `boxes` are cleared; the first with a negative half-extent is
/// an error naming its table, index and axis.
fn count_cleared(
    table: BoundsTable,
    boxes: impl Iterator<Item = Bounds>,
) -> Result<usize, WorldMeshError> {
    let mut cleared = 0;
    for (index, bounds) in boxes.enumerate() {
        if bounds.is_cleared() {
            cleared += 1;
        } else if let Some(axis) = bounds.negative_half_axis() {
            let half = bounds.half()[axis];
            return Err(WorldMeshError::NegativeBoundsHalf {
                table,
                index,
                axis,
                half,
            });
        }
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges() -> CameraSurfRanges {
        let run = |kind, begin, end| CameraSurfRange { kind, begin, end };
        CameraSurfRanges::new(
            run(CameraRangeKind::LitOpaque, 0, 4),
            vec![run(CameraRangeKind::Decal, 4, 6)],
        )
    }

    #[test]
    fn the_first_range_leads_the_rest() {
        let kinds: Vec<_> = ranges().iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [CameraRangeKind::LitOpaque, CameraRangeKind::Decal]);
    }

    #[test]
    fn an_empty_world_checks_clean() {
        let world = DpvsWorldData::new(ranges()).checked().unwrap();
        assert_eq!(world.cleared_boxes, 0);
        assert_eq!(world.cell_count, 0);
    }
}
