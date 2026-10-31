//! BO2's world AABB-tree walk (R_AddCellStaticSurfacesInFrustum, R_AddAabbTreeSurfacesInFrustum_r):
//! which static surfaces and static models of a cell lie inside its clip planes. Built from
//! BO2's GfxAabbTree (40 bytes: mins, maxs, childCount, surfaceCount, startSurfIndex,
//! smodelIndexCount, smodelIndexes, childrenOffset).
//!
//! What differs from MW2's walk: nodes are mins/maxs boxes tested by their nearest and
//! farthest corner; there are no no-decal spans, no sorted-span bits and no cell-root span.
//! BO2 unknown: the occluder test (TestOccludersPartial) is not ported; the walk runs as if
//! the map had no occluders.

use crate::cut::CLIP_PLANE_CEILING;
use crate::plane::{box_culled, box_culled_by, plane_dist, positive_vertex};

/// A box as its middle and half size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    mid: [f32; 3],
    half: [f32; 3],
}

impl Bounds {
    /// The cleared box: no middle, a large negative size. Every plane culls it.
    pub const RESET_EMPTY: Self = Self::from_center_extent([0.0; 3], [-131072.0; 3]);

    #[must_use]
    pub const fn from_center_extent(mid: [f32; 3], half: [f32; 3]) -> Self {
        Bounds { half, mid }
    }

    #[must_use]
    pub fn from_corner_extremes(low_corner: [f32; 3], high_corner: [f32; 3]) -> Self {
        let mid = core::array::from_fn(|k| (low_corner[k] + high_corner[k]) * 0.5);
        let half = core::array::from_fn(|k| (high_corner[k] - low_corner[k]) * 0.5);
        Self::from_center_extent(mid, half)
    }

    #[must_use]
    pub const fn mid(self) -> [f32; 3] {
        self.mid
    }

    #[must_use]
    pub const fn half(self) -> [f32; 3] {
        self.half
    }

    /// One corner: the middle stepped by the half size, `sign` of it per axis.
    fn corner(self, sign: f32) -> [f32; 3] {
        core::array::from_fn(|k| if sign < 0.0 { self.mid[k] - self.half[k] } else { self.mid[k] + self.half[k] })
    }

    #[must_use]
    pub fn low_corner(self) -> [f32; 3] {
        self.corner(-1.0)
    }

    #[must_use]
    pub fn high_corner(self) -> [f32; 3] {
        self.corner(1.0)
    }

    #[must_use]
    pub fn has_been_reset(self) -> bool {
        self == Self::RESET_EMPTY
    }

    /// The first axis whose half size is negative, if any.
    #[must_use]
    pub fn minus_axis_direction(self) -> Option<usize> {
        self.half.iter().position(|&h| h < 0.0)
    }
}

/// `local` moved to `origin` and turned by `axis`, as the box that holds the turned box.
#[must_use]
pub fn box_around_oriented_frame(origin: [f32; 3], basis: [[f32; 3]; 3], local: Bounds) -> Bounds {
    let (centre, size) = (local.mid(), local.half());
    let moved = core::array::from_fn(|k| {
        (0..3).fold(origin[k], |sum, row| sum + centre[row] * basis[row][k])
    });
    let spread = core::array::from_fn(|k| {
        let first = size[0] * libm::fabsf(basis[0][k]);
        (1..3).fold(first, |sum, row| sum + size[row] * libm::fabsf(basis[row][k]))
    });
    Bounds::from_center_extent(moved, spread)
}

/// Culled when any plane has the whole box at or behind it (nearest-corner test).
#[must_use]
pub fn box_outside_planes(bounds: Bounds, cull_planes: &[[f32; 4]]) -> bool {
    box_culled(cull_planes, bounds.low_corner(), bounds.high_corner())
}

/// Size of BO2's GfxAabbTree; `sub_elements_start` counts bytes of these.
pub const AABB_TREE_SIZE: usize = 40;

/// One BO2 GfxAabbTree node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AabbTree {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub sub_element_total: u16,
    pub facet_tally: u16,
    /// First entry of the node's surfaces in `sorted_facet_index`.
    pub start_surf_index: u16,
    pub static_model_slot_total: u16,
    /// BO2 holds a pointer; here the first entry in the cell's static-model index list.
    pub static_model_slot_first: u32,
    /// Bytes from this node to its first child; children are consecutive nodes.
    pub sub_elements_start: i32,
}

impl AabbTree {
    /// The node's box. A node with nothing under it holds inverted extremes (mins
    /// +FLT_MAX, maxs -FLT_MAX), whose half overflows to -inf; it is the cleared box,
    /// which culls by every plane as the inverted one does. One inverted axis is enough.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        if (0..3).any(|k| self.lo[k] > self.hi[k]) {
            Bounds::RESET_EMPTY
        } else {
            Bounds::from_corner_extremes(self.lo, self.hi)
        }
    }

    /// Index of the first child of the node at `index`, if it has children.
    #[must_use]
    pub fn first_child(&self, index: usize) -> Option<usize> {
        if self.sub_element_total == 0 || self.sub_elements_start <= 0 {
            return None;
        }
        let offset = self.sub_elements_start as usize;
        (offset % AABB_TREE_SIZE == 0).then(|| index + offset / AABB_TREE_SIZE)
    }

    /// Whether `plane` still cuts the node: its corner farthest behind the plane is behind it.
    fn cut_by(&self, cut_plane: [f32; 4]) -> bool {
        plane_dist(cut_plane, positive_vertex(cut_plane, self.hi, self.lo)) < 0.0
    }
}

/// One cell's tree and the world arrays it indexes.
#[derive(Clone, Copy, Debug)]
pub struct BoxTreeVisibilityWalk<'a> {
    pub nodes: &'a [AabbTree],
    pub prop_indexes: &'a [u16],
    pub sorted_facet_index: &'a [u16],
    pub face_spans: &'a [Bounds],
    pub prop_spans: &'a [Bounds],
}

/// One view's visibility bytes (BO2 surfaceVisData / smodelVisData); 1 = visible.
pub struct VisibilityResultSet<'a> {
    pub facet_vis: &'a mut [u8],
    pub prop_vis: &'a mut [u8],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AabbHideStats {
    pub nodes_walked: u32,
    pub nodes_discarded: u32,
    /// Nodes with every plane dropped: all their surfaces and models taken untested.
    pub nodes_taken_entire: u32,
    pub planes_discarded: u32,
    pub surfaces_passed: u32,
    pub facets_rejected_by_bounds: u32,
    /// WelcomedIn with no bound to test (no planes left, or no bound for the surface).
    pub facets_admitted_unbounded: u32,
    pub static_models_passed: u32,
    pub props_rejected_by_bounds: u32,
    pub props_admitted_unbounded: u32,
    /// Planes past BO2's 16 that were left out.
    pub clip_plane_excess_count: u32,
}

/// What became of one listed surface or model.
#[derive(Clone, Copy)]
enum Verdict {
    WelcomedIn,
    Rejected,
    Unbounded,
}

/// The two lists a node points into.
#[derive(Clone, Copy)]
enum List {
    Surfaces,
    Smodels,
}

impl List {
    /// The stats counter a verdict on this list goes to.
    fn counter(self, stats: &mut AabbHideStats, outcome: Verdict) -> &mut u32 {
        match (self, outcome) {
            (List::Surfaces, Verdict::WelcomedIn) => &mut stats.surfaces_passed,
            (List::Surfaces, Verdict::Rejected) => &mut stats.facets_rejected_by_bounds,
            (List::Surfaces, Verdict::Unbounded) => &mut stats.facets_admitted_unbounded,
            (List::Smodels, Verdict::WelcomedIn) => &mut stats.static_models_passed,
            (List::Smodels, Verdict::Rejected) => &mut stats.props_rejected_by_bounds,
            (List::Smodels, Verdict::Unbounded) => &mut stats.props_admitted_unbounded,
        }
    }
}

/// The walk over one cell's tree.
struct Walk<'t, 'o, 'v> {
    tree: &'t BoxTreeVisibilityWalk<'t>,
    out: &'o mut VisibilityResultSet<'v>,
    stats: &'o mut AabbHideStats,
}

impl Walk<'_, '_, '_> {
    fn visit(&mut self, ix: usize, cut_planes: &[[f32; 4]]) {
        let box_tree = self.tree;
        let Some(node) = box_tree.nodes.get(ix) else {
            return;
        };
        self.stats.nodes_walked += 1;

        // Planes the node is wholly in front of are dropped; any plane with the
        // node wholly behind it rejects the node.
        let mut cutting = [[0.0f32; 4]; CLIP_PLANE_CEILING];
        let mut kept = 0usize;
        for &plane in cut_planes {
            if box_culled_by(plane, node.lo, node.hi) {
                self.stats.nodes_discarded += 1;
                return;
            }
            if node.cut_by(plane) {
                cutting[kept] = plane;
                kept += 1;
            } else {
                self.stats.planes_discarded += 1;
            }
        }
        let cutting = &cutting[..kept];

        if cutting.is_empty() {
            self.stats.nodes_taken_entire += 1;
            self.admit_node(node, cutting);
            return;
        }
        match (node.sub_element_total, node.first_child(ix)) {
            (0, _) => self.admit_node(node, cutting),
            (children, Some(first)) => {
                for child in first..first + usize::from(children) {
                    self.visit(child, cutting);
                }
            }
            (_, None) => {}
        }
    }

    fn admit_node(&mut self, bvh_node: &AabbTree, half_spaces: &[[f32; 4]]) {
        let bvh = self.tree;
        let surfaces = bvh.sorted_facet_index.iter().skip(usize::from(bvh_node.start_surf_index));
        self.admit(List::Surfaces, surfaces.take(usize::from(bvh_node.facet_tally)), half_spaces);
        let fixed_props = bvh.prop_indexes.iter().skip(bvh_node.static_model_slot_first as usize);
        self.admit(List::Smodels, fixed_props.take(usize::from(bvh_node.static_model_slot_total)), half_spaces);
    }

    /// Marks each listed entry not yet visible, unless its bound lies outside `planes`.
    fn admit<'a>(&mut self, which_list: List, ids: impl Iterator<Item = &'a u16>, cull_planes: &[[f32; 4]]) {
        let (vis, bounds) = match which_list {
            List::Surfaces => (&mut *self.out.facet_vis, self.tree.face_spans),
            List::Smodels => (&mut *self.out.prop_vis, self.tree.prop_spans),
        };
        for at in ids.map(|&entry| usize::from(entry)) {
            let Some(seen) = vis.get_mut(at).filter(|seen| **seen == 0) else {
                continue;
            };
            let verdict = match bounds.get(at) {
                None => Verdict::Unbounded,
                Some(&b) if !cull_planes.is_empty() && box_outside_planes(b, cull_planes) => Verdict::Rejected,
                Some(_) => Verdict::WelcomedIn,
            };
            *which_list.counter(self.stats, verdict) += 1;
            if !matches!(verdict, Verdict::Rejected) {
                *seen = 1;
            }
        }
    }
}

/// R_AddCellStaticSurfacesInFrustum: marks what of one cell's tree lies inside `frustum_clip_planes_for`.
/// An empty tree adds nothing; planes past 16 are left out.
pub fn admit_box_tree_surfaces_inside_frustum(
    src_bvh: &BoxTreeVisibilityWalk<'_>,
    frustum_clip_planes_for: &[[f32; 4]],
    out: &mut VisibilityResultSet<'_>,
    stats: &mut AabbHideStats,
) {
    if src_bvh.nodes.is_empty() {
        return;
    }
    let (used, extra) = frustum_clip_planes_for.split_at(frustum_clip_planes_for.len().min(CLIP_PLANE_CEILING));
    stats.clip_plane_excess_count += extra.len() as u32;
    Walk { tree: src_bvh, out, stats }.visit(0, used);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(lo: [f32; 3], hi: [f32; 3], start: u16, count: u16) -> AabbTree {
        let mut node = AabbTree { lo, hi, ..AabbTree::default() };
        (node.start_surf_index, node.facet_tally) = (start, count);
        node
    }

    /// Root over two leaves at x 0..10 and x 20..30; each leaf lists its two surfaces in
    /// swapped order so the sortedSurfIndex remap shows.
    fn two_leaf_tree() -> ([AabbTree; 3], [u16; 4], [Bounds; 4]) {
        let mut root = leaf([0.0; 3], [30.0, 10.0, 10.0], 0, 4);
        root.sub_element_total = 2;
        root.sub_elements_start = AABB_TREE_SIZE as i32;
        let nodes = [
            root,
            leaf([0.0; 3], [10.0, 10.0, 10.0], 0, 2),
            leaf([20.0, 0.0, 0.0], [30.0, 10.0, 10.0], 2, 2),
        ];
        let strip = |x0: f32| Bounds::from_corner_extremes([x0, 0.0, 0.0], [x0 + 5.0, 1.0, 1.0]);
        let bounds = [strip(0.0), strip(5.0), strip(20.0), strip(25.0)];
        (nodes, [1, 0, 3, 2], bounds)
    }

    /// Walks `bvh_view` with `planes`, starting from nothing visible.
    fn walk(bvh_view: &BoxTreeVisibilityWalk<'_>, cull_planes: &[[f32; 4]], vis: (&mut [u8], &mut [u8])) -> AabbHideStats {
        let mut tally = AabbHideStats::default();
        let mut out = VisibilityResultSet { facet_vis: vis.0, prop_vis: vis.1 };
        admit_box_tree_surfaces_inside_frustum(bvh_view, cull_planes, &mut out, &mut tally);
        tally
    }

    fn run(planes: &[[f32; 4]]) -> ([u8; 4], AabbHideStats) {
        let (nodes, sorted, bounds) = two_leaf_tree();
        let tree = BoxTreeVisibilityWalk {
            nodes: &nodes,
            prop_indexes: &[],
            sorted_facet_index: &sorted,
            face_spans: &bounds,
            prop_spans: &[],
        };
        let mut surfaces = [0u8; 4];
        let stats = walk(&tree, planes, (&mut surfaces, &mut []));
        (surfaces, stats)
    }

    /// Keeps the side x > `x`.
    fn keep_x_above(x: f32) -> [f32; 4] {
        [1.0, 0.0, 0.0, -x]
    }

    #[test]
    fn no_planes_takes_the_root_whole() {
        let (vis, stats) = run(&[]);
        assert_eq!(vis, [1; 4]);
        assert_eq!((stats.nodes_walked, stats.nodes_taken_entire), (1, 1));
    }

    #[test]
    fn plane_rejects_one_leaf_and_drops_for_the_other() {
        // The root straddles x 15, leaf 1 is behind, leaf 2 is wholly in front.
        let (vis, stats) = run(&[keep_x_above(15.0)]);
        assert_eq!(vis, [0, 0, 1, 1]);
        assert_eq!(stats.nodes_walked, 3);
        assert_eq!(stats.nodes_discarded, 1);
        assert_eq!(stats.nodes_taken_entire, 1);
    }

    #[test]
    fn leaf_tests_each_surface_bound() {
        // x 22 cuts leaf 2, so surfaces 2 (20..25) and 3 (25..30) are tested.
        let (vis, stats) = run(&[keep_x_above(22.0)]);
        assert_eq!(vis, [0, 0, 1, 1]);
        assert_eq!(stats.surfaces_passed, 2);
        // With x 26, surface 2 (20..25) is behind.
        let (vis, stats) = run(&[keep_x_above(26.0)]);
        assert_eq!(vis, [0, 0, 0, 1]);
        assert_eq!(stats.facets_rejected_by_bounds, 1);
    }

    #[test]
    fn box_touching_a_plane_is_rejected() {
        // At x 30 the root's nearest corner sits on the plane.
        let (vis, stats) = run(&[keep_x_above(30.0)]);
        assert_eq!(vis, [0; 4]);
        assert_eq!(stats.nodes_discarded, 1);
    }

    #[test]
    fn children_offset_is_bytes_of_40() {
        let mut node = AabbTree { sub_element_total: 2, ..AabbTree::default() };
        node.sub_elements_start = 80;
        assert_eq!(node.first_child(3), Some(5));
        node.sub_elements_start = 44;
        assert_eq!(node.first_child(0), None);
    }

    #[test]
    fn planes_past_16_are_left_out() {
        let (vis, stats) = run(&[[0.0, 0.0, 0.0, 1.0]; 18]);
        assert_eq!(vis, [1; 4]);
        assert_eq!(stats.clip_plane_excess_count, 2);
    }

    #[test]
    fn smodels_follow_the_node_index_list() {
        let mut node = leaf([0.0; 3], [10.0; 3], 0, 0);
        (node.static_model_slot_first, node.static_model_slot_total) = (1, 2);
        let smodel_bounds = [
            Bounds::from_corner_extremes([0.0; 3], [1.0; 3]),
            Bounds::from_corner_extremes([8.0; 3], [9.0; 3]),
        ];
        let tree = BoxTreeVisibilityWalk {
            nodes: &[node],
            prop_indexes: &[9, 0, 1],
            sorted_facet_index: &[],
            face_spans: &[],
            prop_spans: &smodel_bounds,
        };
        let mut fixed_props = [0u8; 2];
        let stats = walk(&tree, &[keep_x_above(5.0)], (&mut [], &mut fixed_props));
        assert_eq!(fixed_props, [0, 1]);
        assert_eq!(stats.props_rejected_by_bounds, 1);
    }

    #[test]
    fn empty_node_is_the_cleared_box() {
        assert!(leaf([f32::MAX; 3], [-f32::MAX; 3], 0, 0).bounds().has_been_reset());
        assert!(leaf([0.0, 2.0, 0.0], [1.0; 3], 0, 0).bounds().has_been_reset());
        let flat = [1.0, 0.0, 1.0];
        assert_eq!(leaf([0.0; 3], flat, 0, 0).bounds(), Bounds::from_corner_extremes([0.0; 3], flat));
    }

    #[test]
    fn bounds_from_origin_axis_turns_the_box() {
        let local = Bounds::from_corner_extremes([0.0; 3], [2.0, 4.0, 6.0]);
        let quarter_turn = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let turned = box_around_oriented_frame([10.0, 0.0, 0.0], quarter_turn, local);
        assert_eq!((turned.low_corner(), turned.high_corner()), ([6.0, 0.0, 0.0], [10.0, 2.0, 6.0]));
    }
}
