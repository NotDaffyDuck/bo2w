//! Leaf lookups and point contents: `CM_PointLeafnum_r`,
//! `CM_BoxLeafnums(_r)`, `BoxOnPlaneSide`, `CM_PositionTest`,
//! `CM_PointContents`, `CM_TransformedPointContents` and `AnglesToAxis`.

use trace_t6::{Trace, TraceWork};

use crate::{PlaneEquation, CheckCount, ClipMap, Leaf, POSITION_TEST_MAX_LEAFS, dot, mesh, tree};

impl ClipMap<'_> {
    /// `CM_PointLeafnum_r` from the top of the tree: the leaf `p` is in.
    #[must_use]
    pub fn leaf_holding(&self, p: [f32; 3]) -> &Leaf {
        let mut num: i32 = 0;
        while num >= 0 {
            let node = &self.nodes[num as usize];
            let plane = &node.plane;
            let d = if plane.kind < 3 {
                p[plane.kind as usize]
            } else {
                dot(plane.normal, p)
            };
            num = i32::from(if 0.0 <= d - plane.dist {
                node.children[0]
            } else {
                node.children[1]
            });
        }
        &self.leafs[(-1 - num) as usize]
    }
}

/// `BoxOnPlaneSide`: 1 when the box is all in front, 2 when all behind,
/// 3 when it crosses.
#[must_use]
pub fn box_on_plane_side(mins: [f32; 3], hi_box: [f32; 3], plane: &PlaneEquation) -> u8 {
    let mut dist1 = 0.0;
    let mut separation_sq = 0.0;
    for i in 0..3 {
        let (near, far) = if plane.signbits & (1 << i) != 0 {
            (hi_box[i], mins[i])
        } else {
            (mins[i], hi_box[i])
        };
        dist1 += plane.normal[i] * far;
        separation_sq += plane.normal[i] * near;
    }
    u8::from(dist1 >= plane.dist) | (u8::from(separation_sq < plane.dist) << 1)
}

/// `leafList_s`: the leafs a box touches.
#[derive(Debug)]
pub struct LeafList<'b> {
    pub list: &'b mut [u16],
    pub count: usize,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    /// The last leaf stored that is in a cluster, or -1.
    pub last_leaf: i32,
    pub overflowed: bool,
}

impl<'b> LeafList<'b> {
    #[must_use]
    pub fn new(list: &'b mut [u16], mins: [f32; 3], hi_box: [f32; 3]) -> Self {
        Self {
            list,
            count: 0,
            mins,
            maxs: hi_box,
            last_leaf: -1,
            overflowed: false,
        }
    }

    /// The leafs stored.
    #[must_use]
    pub fn leafs(&self) -> &[u16] {
        &self.list[..self.count]
    }

    /// `CM_StoreLeafs`.
    fn store(&mut self, map: &ClipMap<'_>, num: i32) {
        let leaf = -1 - num;
        if map.leafs[leaf as usize].cluster != -1 {
            self.last_leaf = leaf;
        }
        if self.count >= self.list.len() {
            self.overflowed = true;
            return;
        }
        self.list[self.count] = leaf as u16;
        self.count += 1;
    }
}

/// `CM_BoxLeafnums_r` from node `num` (0 for the whole map).
pub fn box_leafnums(map: &ClipMap<'_>, ll: &mut LeafList<'_>, mut num: i32) {
    while num >= 0 {
        let node = &map.nodes[num as usize];
        let side = box_on_plane_side(ll.mins, ll.maxs, &node.plane);
        num = i32::from(if side == 1 {
            node.children[0]
        } else {
            if side != 2 {
                box_leafnums(map, ll, i32::from(node.children[0]));
            }
            node.children[1]
        });
    }
    ll.store(map, num);
}

/// `CM_PositionTest`: the world leafs the box touches, brushes then terrain.
pub(crate) fn position_test(map: &ClipMap<'_>, check: &mut CheckCount<'_>, tw: &TraceWork, trace: &mut Trace) {
    if trace.fully_embedded {
        return;
    }
    let s = tw.extents.from;
    let mut mins = [0.0; 3];
    let mut maxs = [0.0; 3];
    for i in 0..3 {
        mins[i] = (s[i] - tw.size[i]) - 1.0;
        maxs[i] = s[i] + tw.size[i] + 1.0;
    }
    let mut buf = [0u16; POSITION_TEST_MAX_LEAFS];
    let mut ll = LeafList::new(&mut buf, mins, maxs);
    box_leafnums(map, &mut ll, 0);
    for &l in ll.leafs() {
        if trace.fully_embedded {
            return;
        }
        let leaf = &map.leafs[l as usize];
        if (tw.contents & leaf.solid_content_flags == 0 || !tree::test_in_leaf_brush_node(map, tw, leaf, trace))
            && tw.contents & leaf.terrain_contents != 0
        {
            mesh::mesh_test_in_leaf(map, check, tw, leaf, trace);
        }
    }
}

/// `CM_PointContents`: the contents of the brushes of `model` (0 for the
/// world) that hold `p`.
#[must_use]
pub fn point_contents(map: &ClipMap<'_>, p: [f32; 3], model: u32) -> u32 {
    let leaf = if model == 0 {
        map.leaf_holding(p)
    } else {
        match map.solid_model(model) {
            Some(cm) => &cm.leaf,
            None => return 0,
        }
    };
    if leaf.brush_leaf_link == 0 {
        return 0;
    }
    #[allow(clippy::needless_range_loop)] // one index walks the leaf box and the point
    for i in 0..3 {
        if !(leaf.mins[i] < p[i] && p[i] < leaf.maxs[i]) {
            return 0;
        }
    }
    point_contents_leaf_brush_node_r(map, p, leaf.brush_leaf_link as usize)
}

/// `CM_PointContentsLeafBrushNode_r`.
fn point_contents_leaf_brush_node_r(map: &ClipMap<'_>, p: [f32; 3], first: usize) -> u32 {
    let holds = |b: &u16| {
        let brush = &map.brushes[usize::from(*b)];
        let in_box = (0..3).all(|i| brush.lower[i] <= p[i] && p[i] <= brush.upper[i]);
        let behind_sides = || brush.sides.iter().all(|side| dot(side.plane.normal, p) <= side.plane.plane_offset);
        (in_box && behind_sides()).then_some(brush.contents)
    };
    let mut found = 0;
    let mut at = first;
    loop {
        let node = &map.leaf_brush_nodes[at];
        // A node with brushes ends the walk; a negative count also has a
        // subtree right after it, then the walk goes on down one side.
        match node.leaf_solid_total {
            listed if listed > 0 => return node.brushes.iter().filter_map(holds).fold(found, |all, c| all | c),
            split if split < 0 => found |= point_contents_leaf_brush_node_r(map, p, at + 1),
            _ => {}
        }
        let below = p[node.axis as usize] <= node.dist;
        at += node.child_offset[usize::from(below)] as usize;
    }
}

/// `CM_TransformedPointContents`: point contents of a model placed at
/// `origin` turned by `angles` (pitch, yaw, roll in degrees).
#[must_use]
pub fn transformed_point_contents(map: &ClipMap<'_>, p: [f32; 3], model: u32, base_at: [f32; 3], rot_deg: [f32; 3]) -> u32 {
    let mut local = [p[0] - base_at[0], p[1] - base_at[1], p[2] - base_at[2]];
    if rot_deg != [0.0; 3] {
        let axis = angles_to_axis(rot_deg);
        local = [dot(axis[0], local), dot(axis[1], local), dot(axis[2], local)];
    }
    point_contents(map, local, model)
}

/// `AnglesToAxis`: forward, left and up for pitch, yaw and roll in degrees.
#[must_use]
pub fn angles_to_axis(rot_deg: [f32; 3]) -> [[f32; 3]; 3] {
    const RADIANS_PER_ARC_DEGREE: f32 = 0.017_453_292;
    let (sy, cy) = libm::sincosf(rot_deg[1] * RADIANS_PER_ARC_DEGREE);
    let (sp, codepoint) = libm::sincosf(rot_deg[0] * RADIANS_PER_ARC_DEGREE);
    let (r_sin, cr) = libm::sincosf(rot_deg[2] * RADIANS_PER_ARC_DEGREE);
    [
        [codepoint * cy, codepoint * sy, -sp],
        [r_sin * sp * cy - cr * sy, r_sin * sp * sy + cr * cy, r_sin * codepoint],
        [cr * sp * cy + r_sin * sy, cr * sp * sy - r_sin * cy, cr * codepoint],
    ]
}
