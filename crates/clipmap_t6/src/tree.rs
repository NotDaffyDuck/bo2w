//! The map tree and the leaf brush node tree: `CM_TraceThroughTree`,
//! `CM_TraceThroughLeaf`, `CM_TraceThroughLeafBrushNode(_r)` and
//! `CM_TestInLeafBrushNode(_r)`.

use trace_t6::{SURFACE_CLIP_SLACK, Trace, SweepBoxSize, TraceWork, test_box_in_brush, trace_through_brush};

use crate::{CheckCount, ClipMap, Leaf, mesh};

/// A split closer to flat than this goes all to the near side.
const SPLIT_EPSILON: f32 = 4.768_371_6e-7;

fn dot4(n: [f32; 3], p: [f32; 4]) -> f32 {
    n[0] * p[0] + n[1] * p[1] + n[2] * p[2]
}

fn lerp4(a: [f32; 4], b: [f32; 4], amt: f32) -> [f32; 4] {
    [
        (b[0] - a[0]) * amt + a[0],
        (b[1] - a[1]) * amt + a[1],
        (b[2] - a[2]) * amt + a[2],
        (b[3] - a[3]) * amt + a[3],
    ]
}

/// BO2's larger and smaller of two plane distances, ties as it breaks them.
pub(crate) fn hi_lo(t1: f32, t2: f32) -> (f32, f32) {
    let hi = if 0.0 <= t1 - t2 { t1 } else { t2 };
    let lo = if 0.0 <= t2 - t1 { t1 } else { t2 };
    (hi, lo)
}

/// Where a segment that crosses a split is cut: the side its start is on,
/// the fraction the near half reaches and where the far half starts.
pub(crate) fn split(t1: f32, t2: f32, offset: f32) -> (usize, f32, f32) {
    let d = t2 - t1;
    if d.abs() <= SPLIT_EPSILON {
        // Too flat to cut: the near half takes it all.
        return (0, 1.0, 0.0);
    }
    let t = if 0.0 <= d { -t1 } else { t1 };
    let stow_kind = 1.0 / d.abs();
    let (near, far) = ((t + offset) * stow_kind, (t - offset) * stow_kind);
    // The near half never reaches past the end, the far half never starts
    // before the start.
    let reach = if 1.0 - near < 0.0 { 1.0 } else { near };
    let resume = if far < 0.0 { 0.0 } else { far };
    (usize::from(0.0 <= d), reach, resume)
}

/// `CM_TraceThroughTree`: walk the map tree from node `num` with the
/// segment `p1`..`p2` (each with its trace fraction as the fourth value).
#[allow(clippy::too_many_arguments)]
pub(crate) fn trace_through_tree(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    start: i32,
    from: [f32; 4],
    p2: [f32; 4],
    hit: &mut Trace,
) {
    let (mut num, mut p1) = (start, from);
    loop {
        // Node numbers below zero name leaves, counted down from -1.
        let Ok(at) = usize::try_from(num) else {
            return trace_through_leaf(map, check, tw, &map.leafs[(-1 - num) as usize], hit);
        };
        let node = &map.nodes[at];
        let plane = &node.plane;
        let (t1, t2, offset) = if plane.kind < 3 {
            let k = plane.kind as usize;
            (p1[k] - plane.dist, p2[k] - plane.dist, tw.size[k] + SURFACE_CLIP_SLACK)
        } else {
            let offset = if tw.is_point {
                SURFACE_CLIP_SLACK
            } else {
                tw.bounding_radius + SURFACE_CLIP_SLACK
            };
            (dot4(plane.normal, p1) - plane.dist, dot4(plane.normal, p2) - plane.dist, offset)
        };
        let (hi, lo) = hi_lo(t1, t2);
        if !(lo < offset) {
            num = i32::from(node.children[0]);
            continue;
        }
        if !(-offset < hi) {
            num = i32::from(node.children[1]);
            continue;
        }
        if hit.fraction <= p1[3] {
            return;
        }
        let (side, frac, far_side_ratio) = split(t1, t2, offset);
        let mid = lerp4(p1, p2, frac);
        trace_through_tree(map, check, tw, i32::from(node.children[side]), p1, mid, hit);
        p1 = lerp4(p1, p2, far_side_ratio);
        num = i32::from(node.children[side ^ 1]);
    }
}

/// `CM_TraceThroughLeaf`: the leaf's brushes, then its terrain.
pub(crate) fn trace_through_leaf(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    leaf: &Leaf,
    trace_out: &mut Trace,
) {
    if trace_out.fraction == 0.0 {
        return;
    }
    if tw.contents & leaf.solid_content_flags != 0 && trace_through_leaf_brush_node(map, tw, leaf, trace_out) {
        return;
    }
    if tw.contents & leaf.terrain_contents == 0 {
        return;
    }
    let first = leaf.first_coll_aabb as usize;
    for i in 0..leaf.collision_box_total as usize {
        if trace_out.fraction == 0.0 {
            return;
        }
        mesh::trace_through_aabb_tree(map, check, tw, first + i, trace_out);
    }
}

/// `CM_TraceThroughLeafBrushNode`: true once the trace is stopped dead.
fn trace_through_leaf_brush_node(map: &ClipMap<'_>, tw: &TraceWork, leaf: &Leaf, hit: &mut Trace) -> bool {
    let mut absmins = [0.0; 3];
    let mut absmaxs = [0.0; 3];
    for i in 0..3 {
        absmins[i] = leaf.mins[i] - tw.size[i];
        absmaxs[i] = leaf.maxs[i] + tw.size[i];
    }
    if !intersect_extents_aabb(&tw.extents, absmins, absmaxs, hit.fraction) {
        return false;
    }
    let s = tw.extents.from;
    let e = tw.extents.to;
    let p1 = [s[0], s[1], s[2], 0.0];
    let p2 = [e[0], e[1], e[2], hit.fraction];
    trace_through_leaf_brush_node_r(map, tw, leaf.brush_leaf_link as usize, p1, p2, hit);
    hit.fraction == 0.0
}

/// `CM_TraceThroughLeafBrushNode_r`.
fn trace_through_leaf_brush_node_r(
    map: &ClipMap<'_>,
    tw: &TraceWork,
    mut idx: usize,
    mut seg_a: [f32; 4],
    p2: [f32; 4],
    hit: &mut Trace,
) {
    if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_solid_total > 0 {
            for &b in node.brushes {
                let brush = &map.brushes[b as usize];
                if tw.contents & brush.contents != 0 {
                    trace_through_brush(tw, brush, hit);
                }
            }
            return;
        }
        if node.leaf_solid_total < 0 {
            trace_through_leaf_brush_node_r(map, tw, idx + 1, seg_a, p2, hit);
        }
        let axis = node.axis as usize;
        let t1 = seg_a[axis] - node.dist;
        let t2 = p2[axis] - node.dist;
        let offset = (tw.size[axis] + SURFACE_CLIP_SLACK) - node.range;
        let (hi, lo) = hi_lo(t1, t2);
        let step = if !(lo < offset) {
            if hi <= -offset {
                return;
            }
            node.child_offset[0]
        } else if !(-offset < hi) {
            node.child_offset[1]
        } else {
            if hit.fraction <= seg_a[3] {
                return;
            }
            let (side, frac, far_side_ratio) = split(t1, t2, offset);
            let mid = lerp4(seg_a, p2, frac);
            trace_through_leaf_brush_node_r(map, tw, idx + node.child_offset[side] as usize, seg_a, mid, hit);
            seg_a = lerp4(seg_a, p2, far_side_ratio);
            node.child_offset[side ^ 1]
        };
        idx += step as usize;
        if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
            return;
        }
    }
}

/// `CM_TraceThroughAabbTree`'s and the leaf brush node's box test
/// (`IntersectExtentsAabb`): does the segment, cut at `frac`, touch the box?
#[must_use]
pub fn intersect_extents_aabb(ext: &SweepBoxSize, mins: [f32; 3], hi_box: [f32; 3], frac: f32) -> bool {
    let mut tmin = 0.0;
    let mut tmax = frac;
    for (b, sign) in [(mins, -1.0), (hi_box, 1.0)] {
        #[allow(clippy::needless_range_loop)] // one index walks the segment ends and the box
        for i in 0..3 {
            let s = (ext.from[i] - b[i]) * sign;
            let e = (ext.to[i] - b[i]) * sign;
            if s <= 0.0 {
                if e > 0.0 {
                    let t = ext.inv_delta[i] * s * sign;
                    if t <= tmin {
                        return false;
                    }
                    if t < tmax {
                        tmax = t;
                    }
                }
            } else {
                if e > 0.0 {
                    return false;
                }
                let t = ext.inv_delta[i] * s * sign;
                if tmax <= t {
                    return false;
                }
                if tmin < t {
                    tmin = t;
                }
            }
        }
    }
    true
}

/// `CM_TestInLeafBrushNode`: true when the box ends up all in solid.
pub(crate) fn test_in_leaf_brush_node(map: &ClipMap<'_>, tw: &TraceWork, leaf: &Leaf, trace: &mut Trace) -> bool {
    for i in 0..3 {
        if tw.bounds[1][i] <= leaf.mins[i] || leaf.maxs[i] <= tw.bounds[0][i] {
            return false;
        }
    }
    test_in_leaf_brush_node_r(map, tw, leaf.brush_leaf_link as usize, trace);
    trace.fully_embedded
}

/// `CM_TestInLeafBrushNode_r`.
fn test_in_leaf_brush_node_r(map: &ClipMap<'_>, tw: &TraceWork, mut idx: usize, trace: &mut Trace) {
    if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_solid_total > 0 {
            for &b in node.brushes {
                let brush = &map.brushes[b as usize];
                if tw.contents & brush.contents != 0 {
                    test_box_in_brush(tw, brush, trace);
                    if trace.fully_embedded {
                        return;
                    }
                }
            }
            return;
        }
        if node.leaf_solid_total < 0 {
            test_in_leaf_brush_node_r(map, tw, idx + 1, trace);
            if trace.fully_embedded {
                return;
            }
        }
        let axis = node.axis as usize;
        let step = if tw.bounds[0][axis] <= node.dist {
            if node.dist <= tw.bounds[1][axis] {
                test_in_leaf_brush_node_r(map, tw, idx + node.child_offset[0] as usize, trace);
                if trace.fully_embedded {
                    return;
                }
            }
            node.child_offset[1]
        } else {
            node.child_offset[0]
        };
        idx += step as usize;
        if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
            return;
        }
    }
}

/// bo2mp: the brushes of `contents` whose leaf brush nodes the box
/// `mins..maxs` reaches, from every world leaf it touches, each once and in
/// first-reached order, appended to `out`. The same walk as
/// `CM_TestInLeafBrushNode_r`, gathering instead of testing.
pub fn box_brushes(map: &ClipMap<'_>, mins: [f32; 3], hi_box: [f32; 3], solid_bits: u32, out: &mut alloc::vec::Vec<u16>) {
    if map.nodes.is_empty() {
        return;
    }
    let mut leafs = [0u16; 1024];
    let mut ll = crate::LeafList::new(&mut leafs, mins, hi_box);
    crate::box_leafnums(map, &mut ll, 0);
    let from_pt = out.len();
    for &l in ll.leafs() {
        let leaf = &map.leafs[usize::from(l)];
        if leaf.solid_content_flags & solid_bits == 0 {
            continue;
        }
        if (0..3).any(|i| hi_box[i] <= leaf.mins[i] || leaf.maxs[i] <= mins[i]) {
            continue;
        }
        box_brushes_r(map, [mins, hi_box], solid_bits, leaf.brush_leaf_link as usize, from_pt, out);
    }
}

fn box_brushes_r(map: &ClipMap<'_>, bounds: [[f32; 3]; 2], solid_bits: u32, mut idx: usize, from_pt: usize, out: &mut alloc::vec::Vec<u16>) {
    if solid_bits & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_solid_total > 0 {
            for &b in node.brushes {
                if solid_bits & map.brushes[usize::from(b)].contents != 0 && !out[from_pt..].contains(&b) {
                    out.push(b);
                }
            }
            return;
        }
        if node.leaf_solid_total < 0 {
            box_brushes_r(map, bounds, solid_bits, idx + 1, from_pt, out);
        }
        let axis = node.axis as usize;
        let step = if bounds[0][axis] <= node.dist {
            if node.dist <= bounds[1][axis] {
                box_brushes_r(map, bounds, solid_bits, idx + node.child_offset[0] as usize, from_pt, out);
            }
            node.child_offset[1]
        } else {
            node.child_offset[0]
        };
        idx += step as usize;
        if solid_bits & map.leaf_brush_nodes[idx].contents == 0 {
            return;
        }
    }
}
