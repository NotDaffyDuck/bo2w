//! BO2's world AABB-tree walk (R_AddCellStaticSurfacesInFrustum, R_AddAabbTreeSurfacesInFrustum_r):
//! which static surfaces and static models of a cell lie inside its clip planes. Built from
//! BO2's GfxAabbTree (40 bytes: mins, maxs, childCount, surfaceCount, startSurfIndex,
//! smodelIndexCount, smodelIndexes, childrenOffset).
//!
//! What differs from MW2's walk: nodes are mins/maxs boxes tested by their nearest and
//! farthest corner; there are no no-decal spans, no sorted-span bits and no cell-root span.
//! BO2 unknown: the occluder test (TestOccludersPartial) is not ported; the walk runs as if
//! the map had no occluders.

use crate::clip::MAX_CLIP_PLANES;
use crate::plane::{box_culled, box_culled_by, plane_dist};

/// A box as its middle and half size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    mid: [f32; 3],
    half: [f32; 3],
}

impl Bounds {
    #[must_use]
    pub const fn from_mid_half(mid: [f32; 3], half: [f32; 3]) -> Self {
        Self { mid, half }
    }

    #[must_use]
    pub fn from_mins_maxs(mins: [f32; 3], maxs: [f32; 3]) -> Self {
        Self {
            mid: [
                (mins[0] + maxs[0]) * 0.5,
                (mins[1] + maxs[1]) * 0.5,
                (mins[2] + maxs[2]) * 0.5,
            ],
            half: [
                (maxs[0] - mins[0]) * 0.5,
                (maxs[1] - mins[1]) * 0.5,
                (maxs[2] - mins[2]) * 0.5,
            ],
        }
    }

    #[must_use]
    pub const fn mid(self) -> [f32; 3] {
        self.mid
    }

    #[must_use]
    pub const fn half(self) -> [f32; 3] {
        self.half
    }

    #[must_use]
    pub fn mins(self) -> [f32; 3] {
        [
            self.mid[0] - self.half[0],
            self.mid[1] - self.half[1],
            self.mid[2] - self.half[2],
        ]
    }

    #[must_use]
    pub fn maxs(self) -> [f32; 3] {
        [
            self.mid[0] + self.half[0],
            self.mid[1] + self.half[1],
            self.mid[2] + self.half[2],
        ]
    }

    pub const CLEARED: Self = Self {
        mid: [0.0; 3],
        half: [-131072.0; 3],
    };

    #[must_use]
    pub fn is_cleared(self) -> bool {
        self.mid == Self::CLEARED.mid && self.half == Self::CLEARED.half
    }

    #[must_use]
    pub fn negative_half_axis(self) -> Option<usize> {
        (0..3).find(|&axis| self.half[axis] < 0.0)
    }
}

/// `local` moved to `origin` and turned by `axis`, as the box that holds the turned box.
#[must_use]
pub fn bounds_from_origin_axis(origin: [f32; 3], axis: [[f32; 3]; 3], local: Bounds) -> Bounds {
    let m = local.mid();
    let h = local.half();
    Bounds::from_mid_half(
        [
            origin[0] + m[0] * axis[0][0] + m[1] * axis[1][0] + m[2] * axis[2][0],
            origin[1] + m[0] * axis[0][1] + m[1] * axis[1][1] + m[2] * axis[2][1],
            origin[2] + m[0] * axis[0][2] + m[1] * axis[1][2] + m[2] * axis[2][2],
        ],
        [
            h[0] * libm::fabsf(axis[0][0]) + h[1] * libm::fabsf(axis[1][0]) + h[2] * libm::fabsf(axis[2][0]),
            h[0] * libm::fabsf(axis[0][1]) + h[1] * libm::fabsf(axis[1][1]) + h[2] * libm::fabsf(axis[2][1]),
            h[0] * libm::fabsf(axis[0][2]) + h[1] * libm::fabsf(axis[1][2]) + h[2] * libm::fabsf(axis[2][2]),
        ],
    )
}

/// Culled when any plane has the whole box at or behind it (nearest-corner test).
#[must_use]
pub fn bounds_culled(bounds: Bounds, planes: &[[f32; 4]]) -> bool {
    box_culled(planes, bounds.mins(), bounds.maxs())
}

/// Size of BO2's GfxAabbTree; `children_offset` counts bytes of these.
pub const AABB_TREE_SIZE: usize = 40;

/// One BO2 GfxAabbTree node.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AabbTree {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub child_count: u16,
    pub surface_count: u16,
    /// First entry of the node's surfaces in `sorted_surf_index`.
    pub start_surf_index: u16,
    pub smodel_index_count: u16,
    /// BO2 holds a pointer; here the first entry in the cell's static-model index list.
    pub smodel_index_start: u32,
    /// Bytes from this node to its first child; children are consecutive nodes.
    pub children_offset: i32,
}

impl AabbTree {
    /// The node's box. A node with nothing under it holds inverted extremes (mins
    /// +FLT_MAX, maxs -FLT_MAX), whose half overflows to -inf; it is the cleared box,
    /// which culls by every plane as the inverted one does. One inverted axis is enough.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        if (0..3).any(|k| self.mins[k] > self.maxs[k]) {
            Bounds::CLEARED
        } else {
            Bounds::from_mins_maxs(self.mins, self.maxs)
        }
    }

    /// Index of the first child of the node at `index`, if it has children.
    #[must_use]
    pub fn first_child(&self, index: usize) -> Option<usize> {
        if self.child_count == 0 || self.children_offset <= 0 {
            return None;
        }
        let offset = self.children_offset as usize;
        (offset % AABB_TREE_SIZE == 0).then(|| index + offset / AABB_TREE_SIZE)
    }
}

/// One cell's tree and the world arrays it indexes.
#[derive(Clone, Copy, Debug)]
pub struct AabbTreeCull<'a> {
    pub nodes: &'a [AabbTree],
    pub smodel_indexes: &'a [u16],
    pub sorted_surf_index: &'a [u16],
    pub surfaces_bounds: &'a [Bounds],
    pub smodel_bounds: &'a [Bounds],
}

/// One view's visibility bytes (BO2 surfaceVisData / smodelVisData); 1 = visible.
pub struct DpvsVisData<'a> {
    pub surface_vis: &'a mut [u8],
    pub smodel_vis: &'a mut [u8],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AabbCullStats {
    pub nodes_visited: u32,
    pub nodes_rejected: u32,
    /// Nodes with every plane dropped: all their surfaces and models taken untested.
    pub nodes_accepted_whole: u32,
    pub planes_dropped: u32,
    pub surfaces_admitted: u32,
    pub surfaces_rejected_by_bounds: u32,
    /// Admitted with no bound to test (no planes left, or no bound for the surface).
    pub surfaces_admitted_unbounded: u32,
    pub smodels_admitted: u32,
    pub smodels_rejected_by_bounds: u32,
    pub smodels_admitted_unbounded: u32,
    /// Planes past BO2's 16 that were left out.
    pub clip_planes_overflow: u32,
}

/// R_AddCellStaticSurfacesInFrustum: marks what of one cell's tree lies inside `clip_planes`.
/// An empty tree adds nothing; planes past 16 are left out.
pub fn add_aabb_tree_surfaces_in_frustum(
    tree: &AabbTreeCull<'_>,
    clip_planes: &[[f32; 4]],
    out: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    if tree.nodes.is_empty() {
        return;
    }
    let count = clip_planes.len().min(MAX_CLIP_PLANES);
    stats.clip_planes_overflow += (clip_planes.len() - count) as u32;
    add_node(tree, 0, &clip_planes[..count], out, stats);
}

fn add_node(
    tree: &AabbTreeCull<'_>,
    index: usize,
    clip_planes: &[[f32; 4]],
    out: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    let Some(node) = tree.nodes.get(index) else {
        return;
    };
    stats.nodes_visited += 1;

    let mut kept = [[0.0f32; 4]; MAX_CLIP_PLANES];
    let mut kept_n = 0usize;
    for &plane in clip_planes {
        if box_culled_by(plane, node.mins, node.maxs) {
            stats.nodes_rejected += 1;
            return;
        }
        // The far corner behind the plane: the plane still cuts the node.
        let far: [f32; 3] =
            core::array::from_fn(|i| if plane[i] > 0.0 { node.mins[i] } else { node.maxs[i] });
        if plane_dist(plane, far) < 0.0 {
            kept[kept_n] = plane;
            kept_n += 1;
        } else {
            stats.planes_dropped += 1;
        }
    }
    let kept = &kept[..kept_n];

    if kept.is_empty() {
        stats.nodes_accepted_whole += 1;
        admit_surfaces(tree, node, &[], out, stats);
        admit_smodels(tree, node, &[], out, stats);
        return;
    }
    if node.child_count != 0 {
        if let Some(first) = node.first_child(index) {
            for c in 0..usize::from(node.child_count) {
                add_node(tree, first + c, kept, out, stats);
            }
        }
        return;
    }
    admit_surfaces(tree, node, kept, out, stats);
    admit_smodels(tree, node, kept, out, stats);
}

fn admit_surfaces(
    tree: &AabbTreeCull<'_>,
    node: &AabbTree,
    clip_planes: &[[f32; 4]],
    out: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    let start = usize::from(node.start_surf_index);
    for i in 0..usize::from(node.surface_count) {
        let Some(&surf) = tree.sorted_surf_index.get(start + i) else {
            break;
        };
        let surf = usize::from(surf);
        let Some(slot) = out.surface_vis.get_mut(surf) else {
            continue;
        };
        if *slot != 0 {
            continue;
        }
        match tree.surfaces_bounds.get(surf) {
            Some(&bounds) if !clip_planes.is_empty() => {
                if bounds_culled(bounds, clip_planes) {
                    stats.surfaces_rejected_by_bounds += 1;
                    continue;
                }
                stats.surfaces_admitted += 1;
            }
            Some(_) => stats.surfaces_admitted += 1,
            None => stats.surfaces_admitted_unbounded += 1,
        }
        *slot = 1;
    }
}

fn admit_smodels(
    tree: &AabbTreeCull<'_>,
    node: &AabbTree,
    clip_planes: &[[f32; 4]],
    out: &mut DpvsVisData<'_>,
    stats: &mut AabbCullStats,
) {
    let start = node.smodel_index_start as usize;
    for i in 0..usize::from(node.smodel_index_count) {
        let Some(&smodel) = tree.smodel_indexes.get(start + i) else {
            break;
        };
        let smodel = usize::from(smodel);
        let Some(slot) = out.smodel_vis.get_mut(smodel) else {
            continue;
        };
        if *slot != 0 {
            continue;
        }
        match tree.smodel_bounds.get(smodel) {
            Some(&bounds) if !clip_planes.is_empty() => {
                if bounds_culled(bounds, clip_planes) {
                    stats.smodels_rejected_by_bounds += 1;
                    continue;
                }
                stats.smodels_admitted += 1;
            }
            Some(_) => stats.smodels_admitted += 1,
            None => stats.smodels_admitted_unbounded += 1,
        }
        *slot = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(mins: [f32; 3], maxs: [f32; 3], start: u16, count: u16) -> AabbTree {
        AabbTree {
            mins,
            maxs,
            surface_count: count,
            start_surf_index: start,
            ..AabbTree::default()
        }
    }

    /// Root over two leaves at x 0..10 and x 20..30; each leaf lists its two surfaces in
    /// swapped order so the sortedSurfIndex remap shows.
    fn two_leaf_tree() -> ([AabbTree; 3], [u16; 4], [Bounds; 4]) {
        let root = AabbTree {
            mins: [0.0; 3],
            maxs: [30.0, 10.0, 10.0],
            child_count: 2,
            surface_count: 4,
            children_offset: AABB_TREE_SIZE as i32,
            ..AabbTree::default()
        };
        let nodes = [
            root,
            leaf([0.0; 3], [10.0, 10.0, 10.0], 0, 2),
            leaf([20.0, 0.0, 0.0], [30.0, 10.0, 10.0], 2, 2),
        ];
        let sorted = [1, 0, 3, 2];
        let bounds = [
            Bounds::from_mins_maxs([0.0; 3], [5.0, 1.0, 1.0]),
            Bounds::from_mins_maxs([5.0, 0.0, 0.0], [10.0, 1.0, 1.0]),
            Bounds::from_mins_maxs([20.0, 0.0, 0.0], [25.0, 1.0, 1.0]),
            Bounds::from_mins_maxs([25.0, 0.0, 0.0], [30.0, 1.0, 1.0]),
        ];
        (nodes, sorted, bounds)
    }

    fn run(planes: &[[f32; 4]]) -> ([u8; 4], AabbCullStats) {
        let (nodes, sorted, bounds) = two_leaf_tree();
        let tree = AabbTreeCull {
            nodes: &nodes,
            smodel_indexes: &[],
            sorted_surf_index: &sorted,
            surfaces_bounds: &bounds,
            smodel_bounds: &[],
        };
        let mut surface_vis = [0u8; 4];
        let mut smodel_vis = [0u8; 0];
        let mut stats = AabbCullStats::default();
        add_aabb_tree_surfaces_in_frustum(
            &tree,
            planes,
            &mut DpvsVisData {
                surface_vis: &mut surface_vis,
                smodel_vis: &mut smodel_vis,
            },
            &mut stats,
        );
        (surface_vis, stats)
    }

    #[test]
    fn no_planes_takes_the_root_whole() {
        let (vis, stats) = run(&[]);
        assert_eq!(vis, [1, 1, 1, 1]);
        assert_eq!(stats.nodes_visited, 1);
        assert_eq!(stats.nodes_accepted_whole, 1);
    }

    #[test]
    fn plane_rejects_one_leaf_and_drops_for_the_other() {
        // Keeps x > 15: the root straddles, leaf 1 is behind, leaf 2 is wholly in front.
        let (vis, stats) = run(&[[1.0, 0.0, 0.0, -15.0]]);
        assert_eq!(vis, [0, 0, 1, 1]);
        assert_eq!(stats.nodes_visited, 3);
        assert_eq!(stats.nodes_rejected, 1);
        assert_eq!(stats.nodes_accepted_whole, 1);
    }

    #[test]
    fn leaf_tests_each_surface_bound() {
        // Keeps x > 22: leaf 2 straddles, so surfaces 2 (20..25) and 3 (25..30) are tested.
        let (vis, stats) = run(&[[1.0, 0.0, 0.0, -22.0]]);
        assert_eq!(vis, [0, 0, 1, 1]);
        assert_eq!(stats.surfaces_admitted, 2);
        // Keeps x > 26: surface 2 (20..25) is behind.
        let (vis, stats) = run(&[[1.0, 0.0, 0.0, -26.0]]);
        assert_eq!(vis, [0, 0, 0, 1]);
        assert_eq!(stats.surfaces_rejected_by_bounds, 1);
    }

    #[test]
    fn box_touching_a_plane_is_rejected() {
        // Keeps x > 30: the root's nearest corner sits on the plane.
        let (vis, stats) = run(&[[1.0, 0.0, 0.0, -30.0]]);
        assert_eq!(vis, [0; 4]);
        assert_eq!(stats.nodes_rejected, 1);
    }

    #[test]
    fn children_offset_is_bytes_of_40() {
        let node = AabbTree {
            child_count: 2,
            children_offset: 80,
            ..AabbTree::default()
        };
        assert_eq!(node.first_child(3), Some(5));
        let odd = AabbTree {
            children_offset: 44,
            ..node
        };
        assert_eq!(odd.first_child(0), None);
    }

    #[test]
    fn planes_past_16_are_left_out() {
        let planes = [[0.0, 0.0, 0.0, 1.0]; 18];
        let (vis, stats) = run(&planes);
        assert_eq!(vis, [1; 4]);
        assert_eq!(stats.clip_planes_overflow, 2);
    }

    #[test]
    fn smodels_follow_the_node_index_list() {
        let nodes = [AabbTree {
            mins: [0.0; 3],
            maxs: [10.0; 3],
            smodel_index_count: 2,
            smodel_index_start: 1,
            ..AabbTree::default()
        }];
        let smodel_bounds = [
            Bounds::from_mins_maxs([0.0; 3], [1.0; 3]),
            Bounds::from_mins_maxs([8.0; 3], [9.0; 3]),
        ];
        let tree = AabbTreeCull {
            nodes: &nodes,
            smodel_indexes: &[9, 0, 1],
            sorted_surf_index: &[],
            surfaces_bounds: &[],
            smodel_bounds: &smodel_bounds,
        };
        let mut smodel_vis = [0u8; 2];
        let mut stats = AabbCullStats::default();
        add_aabb_tree_surfaces_in_frustum(
            &tree,
            &[[1.0, 0.0, 0.0, -5.0]],
            &mut DpvsVisData {
                surface_vis: &mut [],
                smodel_vis: &mut smodel_vis,
            },
            &mut stats,
        );
        assert_eq!(smodel_vis, [0, 1]);
        assert_eq!(stats.smodels_rejected_by_bounds, 1);
    }

    #[test]
    fn empty_node_is_the_cleared_box() {
        let empty = leaf([f32::MAX; 3], [-f32::MAX; 3], 0, 0);
        assert!(empty.bounds().is_cleared());
        assert!(leaf([0.0, 2.0, 0.0], [1.0; 3], 0, 0).bounds().is_cleared());
        let flat = leaf([0.0; 3], [1.0, 0.0, 1.0], 0, 0);
        assert_eq!(flat.bounds(), Bounds::from_mins_maxs([0.0; 3], [1.0, 0.0, 1.0]));
    }

    #[test]
    fn bounds_from_origin_axis_turns_the_box() {
        let local = Bounds::from_mins_maxs([0.0; 3], [2.0, 4.0, 6.0]);
        let axis = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let b = bounds_from_origin_axis([10.0, 0.0, 0.0], axis, local);
        assert_eq!(b.mins(), [6.0, 0.0, 0.0]);
        assert_eq!(b.maxs(), [10.0, 2.0, 6.0]);
    }
}
