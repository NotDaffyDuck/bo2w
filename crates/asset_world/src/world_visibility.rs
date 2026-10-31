//! The visibility side of a loaded world: its cells, portals, plane tree and
//! the bounds each cell draws from.

use dpvs_t6::{AabbTree, Bounds, PlaneEquation};

use crate::mesh_report::{BoxSource, LevelMeshFault};

/// A doorway between two cells.
#[derive(Clone, Debug)]
pub struct SelfContainedVisibilityPortal {
    pub split: [f32; 4],
    /// The cell on the far side.
    pub adjacent_cell: u16,
    /// Where this portal's corners sit in `VisibilityLevelData::portal_vertices`.
    pub vertex_first: usize,
    pub vertex_tally: usize,
    /// Two axes of the portal's hull, when it has one.
    pub hull_axes_dir: Option<[[f32; 3]; 2]>,
}

/// Which kind of surface a camera range holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraSpanKind {
    LitOpaque,
    LitTrans,
    Decal,
    Emissive,
}

/// A run of sorted surfaces `[begin, end)` of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewSurfaceSpan {
    pub kind: CameraSpanKind,
    pub from: u32,
    pub until: u32,
}

/// The camera's surface runs. There is always at least one, so the first is
/// kept apart from the rest.
#[derive(Clone, Debug)]
pub struct ViewSurfaceSpans {
    lead: ViewSurfaceSpan,
    others: Vec<ViewSurfaceSpan>,
}

impl ViewSurfaceSpans {
    pub fn new(lead: ViewSurfaceSpan, others: Vec<ViewSurfaceSpan>) -> Self {
        Self { others, lead }
    }

    pub fn iter(&self) -> impl Iterator<Item = ViewSurfaceSpan> + '_ {
        let tail = self.others.iter().copied();
        std::iter::once(self.lead).chain(tail)
    }
}

/// Everything the culler reads from a world.
#[derive(Clone, Debug)]
pub struct VisibilityLevelData {
    pub planes: Vec<PlaneEquation>,
    pub tree_nodes: Vec<u16>,
    pub cell_total: usize,
    /// Per cell, its bounding-box tree.
    pub aabb_trees: Vec<Vec<AabbTree>>,
    /// Per cell, the static models its tree points at.
    pub aabb_prop_indices: Vec<Vec<u16>>,
    pub sorted_facet_index: Vec<u16>,
    pub static_facet_tally: usize,
    pub static_facet_tally_no_stain: usize,
    pub facet_bounds: Vec<Bounds>,
    pub static_prop_boxes: Vec<Bounds>,
    pub portal_vertices: Vec<[f32; 3]>,
    pub portals_per_room: Vec<Vec<SelfContainedVisibilityPortal>>,
    pub room_env_probes: Vec<Vec<u8>>,
    pub lit_solid_begin: u32,
    pub lit_solid_end: u32,
    pub camera_spans: ViewSurfaceSpans,
    pub emissive_facets_begin: u32,
    pub emissive_facets_end: u32,
    /// How many boxes arrived already cleared (set by `vetted`).
    pub emptied_volumes: usize,
}

impl VisibilityLevelData {
    /// An empty world around its camera ranges.
    pub fn new(ranges: ViewSurfaceSpans) -> Self {
        Self {
            emptied_volumes: Default::default(),
            emissive_facets_end: Default::default(),
            emissive_facets_begin: Default::default(),
            camera_spans: ranges,
            lit_solid_end: Default::default(),
            lit_solid_begin: Default::default(),
            room_env_probes: Default::default(),
            portals_per_room: Default::default(),
            portal_vertices: Default::default(),
            static_prop_boxes: Default::default(),
            facet_bounds: Default::default(),
            static_facet_tally_no_stain: Default::default(),
            static_facet_tally: Default::default(),
            sorted_facet_index: Default::default(),
            aabb_prop_indices: Default::default(),
            aabb_trees: Default::default(),
            cell_total: Default::default(),
            tree_nodes: Default::default(),
            planes: Default::default(),
        }
    }

    /// Looks over every bounds table. A box with a negative half-extent is an
    /// error; a box that was deliberately cleared is counted and let through.
    pub(crate) fn vetted(mut self) -> Result<Self, LevelMeshFault> {
        let mut running = 0usize;
        for (cell, per_cell) in self.aabb_trees.iter().enumerate() {
            running += count_cleared(
                BoxSource::TreeNode { cell },
                per_cell.iter().map(AabbTree::bounds),
            )?;
        }
        running += count_cleared(BoxSource::Surface, self.facet_bounds.iter().copied())?;
        running += count_cleared(BoxSource::PropInstance, self.static_prop_boxes.iter().copied())?;
        self.emptied_volumes = running;
        Ok(self)
    }
}

/// How many of `boxes` are cleared; the first with a negative half-extent is
/// an error naming its table, index and axis.
fn count_cleared(
    which: BoxSource,
    candidates: impl Iterator<Item = Bounds>,
) -> Result<usize, LevelMeshFault> {
    let mut tally = 0usize;
    for (index, found) in candidates.enumerate() {
        if found.has_been_reset() {
            tally += 1;
            continue;
        }
        let Some(axis) = found.minus_axis_direction() else {
            continue;
        };
        let half = found.half()[axis];
        return Err(LevelMeshFault::InsideOutBox { half, axis, index, table: which });
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges() -> ViewSurfaceSpans {
        let span = |kind, begin, end| ViewSurfaceSpan { kind, from: begin, until: end };
        let lead = span(CameraSpanKind::LitOpaque, 0, 4);
        let tail = vec![span(CameraSpanKind::Decal, 4, 6)];
        ViewSurfaceSpans::new(lead, tail)
    }

    #[test]
    fn the_first_range_leads_the_rest() {
        let kinds: Vec<_> = ranges().iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [CameraSpanKind::LitOpaque, CameraSpanKind::Decal]);
    }

    #[test]
    fn an_empty_world_checks_clean() {
        let world = VisibilityLevelData::new(ranges()).vetted().unwrap();
        assert_eq!(world.emptied_volumes, 0);
        assert_eq!(world.cell_total, 0);
    }
}
