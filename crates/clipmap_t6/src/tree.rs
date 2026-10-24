//! The map tree and the leaf brush node tree: `CM_TraceThroughTree`,
//! `CM_TraceThroughLeaf`, `CM_TraceThroughLeafBrushNode(_r)` and
//! `CM_TestInLeafBrushNode(_r)`.

use trace_t6::{SURFACE_CLIP_EPSILON, Trace, TraceExtents, TraceWork, test_box_in_brush, trace_through_brush};

use crate::{CheckCount, ClipMap, Leaf, mesh};

/// A split closer to flat than this goes all to the near side.
const SPLIT_EPSILON: f32 = 4.768_371_6e-7;

fn dot4(n: [f32; 3], p: [f32; 4]) -> f32 {
    n[0] * p[0] + n[1] * p[1] + n[2] * p[2]
}

fn lerp4(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    [
        (b[0] - a[0]) * f + a[0],
        (b[1] - a[1]) * f + a[1],
        (b[2] - a[2]) * f + a[2],
        (b[3] - a[3]) * f + a[3],
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
    let (side, mut frac, mut frac2) = if d.abs() <= SPLIT_EPSILON {
        (0, 1.0, 0.0)
    } else {
        let t = if 0.0 <= d { -t1 } else { t1 };
        let inv = 1.0 / d.abs();
        (usize::from(0.0 <= d), (t + offset) * inv, (t - offset) * inv)
    };
    if 1.0 - frac < 0.0 {
        frac = 1.0;
    }
    if frac2 < 0.0 {
        frac2 = 0.0;
    }
    (side, frac, frac2)
}

/// `CM_TraceThroughTree`: walk the map tree from node `num` with the
/// segment `p1`..`p2` (each with its trace fraction as the fourth value).
#[allow(clippy::too_many_arguments)]
pub(crate) fn trace_through_tree(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    mut num: i32,
    mut p1: [f32; 4],
    p2: [f32; 4],
    trace: &mut Trace,
) {
    loop {
        if num < 0 {
            trace_through_leaf(map, check, tw, &map.leafs[(-1 - num) as usize], trace);
            return;
        }
        let node = &map.nodes[num as usize];
        let plane = &node.plane;
        let (t1, t2, offset) = if plane.kind < 3 {
            let k = plane.kind as usize;
            (p1[k] - plane.dist, p2[k] - plane.dist, tw.size[k] + SURFACE_CLIP_EPSILON)
        } else {
            let offset = if tw.is_point {
                SURFACE_CLIP_EPSILON
            } else {
                tw.bounding_radius + SURFACE_CLIP_EPSILON
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
        if trace.fraction <= p1[3] {
            return;
        }
        let (side, frac, frac2) = split(t1, t2, offset);
        let mid = lerp4(p1, p2, frac);
        trace_through_tree(map, check, tw, i32::from(node.children[side]), p1, mid, trace);
        p1 = lerp4(p1, p2, frac2);
        num = i32::from(node.children[side ^ 1]);
    }
}

/// `CM_TraceThroughLeaf`: the leaf's brushes, then its terrain.
pub(crate) fn trace_through_leaf(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    leaf: &Leaf,
    trace: &mut Trace,
) {
    if trace.fraction == 0.0 {
        return;
    }
    if tw.contents & leaf.brush_contents != 0 && trace_through_leaf_brush_node(map, tw, leaf, trace) {
        return;
    }
    if tw.contents & leaf.terrain_contents == 0 {
        return;
    }
    let first = leaf.first_coll_aabb as usize;
    for i in 0..leaf.coll_aabb_count as usize {
        if trace.fraction == 0.0 {
            return;
        }
        mesh::trace_through_aabb_tree(map, check, tw, first + i, trace);
    }
}

/// `CM_TraceThroughLeafBrushNode`: true once the trace is stopped dead.
fn trace_through_leaf_brush_node(map: &ClipMap<'_>, tw: &TraceWork, leaf: &Leaf, trace: &mut Trace) -> bool {
    let mut absmins = [0.0; 3];
    let mut absmaxs = [0.0; 3];
    for i in 0..3 {
        absmins[i] = leaf.mins[i] - tw.size[i];
        absmaxs[i] = leaf.maxs[i] + tw.size[i];
    }
    if !intersect_extents_aabb(&tw.extents, absmins, absmaxs, trace.fraction) {
        return false;
    }
    let s = tw.extents.start;
    let e = tw.extents.end;
    let p1 = [s[0], s[1], s[2], 0.0];
    let p2 = [e[0], e[1], e[2], trace.fraction];
    trace_through_leaf_brush_node_r(map, tw, leaf.leaf_brush_node as usize, p1, p2, trace);
    trace.fraction == 0.0
}

/// `CM_TraceThroughLeafBrushNode_r`.
fn trace_through_leaf_brush_node_r(
    map: &ClipMap<'_>,
    tw: &TraceWork,
    mut idx: usize,
    mut p1: [f32; 4],
    p2: [f32; 4],
    trace: &mut Trace,
) {
    if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_brush_count > 0 {
            for &b in node.brushes {
                let brush = &map.brushes[b as usize];
                if tw.contents & brush.contents != 0 {
                    trace_through_brush(tw, brush, trace);
                }
            }
            return;
        }
        if node.leaf_brush_count < 0 {
            trace_through_leaf_brush_node_r(map, tw, idx + 1, p1, p2, trace);
        }
        let axis = node.axis as usize;
        let t1 = p1[axis] - node.dist;
        let t2 = p2[axis] - node.dist;
        let offset = (tw.size[axis] + SURFACE_CLIP_EPSILON) - node.range;
        let (hi, lo) = hi_lo(t1, t2);
        let step = if !(lo < offset) {
            if hi <= -offset {
                return;
            }
            node.child_offset[0]
        } else if !(-offset < hi) {
            node.child_offset[1]
        } else {
            if trace.fraction <= p1[3] {
                return;
            }
            let (side, frac, frac2) = split(t1, t2, offset);
            let mid = lerp4(p1, p2, frac);
            trace_through_leaf_brush_node_r(map, tw, idx + node.child_offset[side] as usize, p1, mid, trace);
            p1 = lerp4(p1, p2, frac2);
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
pub fn intersect_extents_aabb(ext: &TraceExtents, mins: [f32; 3], maxs: [f32; 3], frac: f32) -> bool {
    let mut tmin = 0.0;
    let mut tmax = frac;
    for (b, sign) in [(mins, -1.0), (maxs, 1.0)] {
        #[allow(clippy::needless_range_loop)] // one index walks the segment ends and the box
        for i in 0..3 {
            let s = (ext.start[i] - b[i]) * sign;
            let e = (ext.end[i] - b[i]) * sign;
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
    test_in_leaf_brush_node_r(map, tw, leaf.leaf_brush_node as usize, trace);
    trace.allsolid
}

/// `CM_TestInLeafBrushNode_r`.
fn test_in_leaf_brush_node_r(map: &ClipMap<'_>, tw: &TraceWork, mut idx: usize, trace: &mut Trace) {
    if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_brush_count > 0 {
            for &b in node.brushes {
                let brush = &map.brushes[b as usize];
                if tw.contents & brush.contents != 0 {
                    test_box_in_brush(tw, brush, trace);
                    if trace.allsolid {
                        return;
                    }
                }
            }
            return;
        }
        if node.leaf_brush_count < 0 {
            test_in_leaf_brush_node_r(map, tw, idx + 1, trace);
            if trace.allsolid {
                return;
            }
        }
        let axis = node.axis as usize;
        let step = if tw.bounds[0][axis] <= node.dist {
            if node.dist <= tw.bounds[1][axis] {
                test_in_leaf_brush_node_r(map, tw, idx + node.child_offset[0] as usize, trace);
                if trace.allsolid {
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
pub fn box_brushes(map: &ClipMap<'_>, mins: [f32; 3], maxs: [f32; 3], contents: u32, out: &mut alloc::vec::Vec<u16>) {
    if map.nodes.is_empty() {
        return;
    }
    let mut leafs = [0u16; 1024];
    let mut ll = crate::LeafList::new(&mut leafs, mins, maxs);
    crate::box_leafnums(map, &mut ll, 0);
    let start = out.len();
    for &l in ll.leafs() {
        let leaf = &map.leafs[usize::from(l)];
        if leaf.brush_contents & contents == 0 {
            continue;
        }
        if (0..3).any(|i| maxs[i] <= leaf.mins[i] || leaf.maxs[i] <= mins[i]) {
            continue;
        }
        box_brushes_r(map, [mins, maxs], contents, leaf.leaf_brush_node as usize, start, out);
    }
}

fn box_brushes_r(map: &ClipMap<'_>, bounds: [[f32; 3]; 2], contents: u32, mut idx: usize, start: usize, out: &mut alloc::vec::Vec<u16>) {
    if contents & map.leaf_brush_nodes[idx].contents == 0 {
        return;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_brush_count > 0 {
            for &b in node.brushes {
                if contents & map.brushes[usize::from(b)].contents != 0 && !out[start..].contains(&b) {
                    out.push(b);
                }
            }
            return;
        }
        if node.leaf_brush_count < 0 {
            box_brushes_r(map, bounds, contents, idx + 1, start, out);
        }
        let axis = node.axis as usize;
        let step = if bounds[0][axis] <= node.dist {
            if node.dist <= bounds[1][axis] {
                box_brushes_r(map, bounds, contents, idx + node.child_offset[0] as usize, start, out);
            }
            node.child_offset[1]
        } else {
            node.child_offset[0]
        };
        idx += step as usize;
        if contents & map.leaf_brush_nodes[idx].contents == 0 {
            return;
        }
    }
}
