//! The terrain mesh: `CM_TraceThroughAabbTree(_r)`, the partition and
//! triangle sweeps (`CM_PointTraceThroughTriangle`,
//! `CM_TraceCapsuleThroughTriangle`, `CM_TraceSphereThroughEdge`,
//! `CM_TraceSphereThroughVertex`) and the position tests
//! (`CM_MeshTestInLeaf`, `CM_PositionTestInAabbTree_r`,
//! `CM_PositionTestCapsuleInTriangle`).

use trace_t6::{SURFACE_CLIP_EPSILON, Trace, TraceWork};

use crate::{AabbTree, CheckCount, ClipMap, Leaf, add, cross, dot, normalize_to, scale, sub};

/// Nodes `CM_TraceThroughAabbTree_r` keeps waiting, at most.
const AABB_STACK: usize = 128;

/// The box test both walks make on a mesh tree node: the swept box against
/// the node's box, on the three axes and (unless the sweep is short enough
/// to skip it) the three cross axes.
fn node_touched(tw: &TraceWork, node: &AabbTree) -> bool {
    let mut e = [0.0; 3];
    let mut d = [0.0; 3];
    for i in 0..3 {
        e[i] = node.half_size[i] + tw.size[i];
        d[i] = tw.midpoint[i] - node.origin[i];
        if d[i].abs() > tw.half_delta_abs[i] + e[i] {
            return false;
        }
    }
    if tw.axial_cull_only {
        return true;
    }
    let hd = tw.half_delta;
    let hda = tw.half_delta_abs;
    !(hda[1] * e[2] + hda[2] * e[1] < (hd[1] * d[2] - hd[2] * d[1]).abs()
        || hda[0] * e[2] + hda[2] * e[0] < (hd[2] * d[0] - hd[0] * d[2]).abs()
        || hda[1] * e[0] + hda[0] * e[1] < (hd[0] * d[1] - hd[1] * d[0]).abs())
}

/// `CM_TraceThroughAabbTree`: the mesh tree `index` of a leaf. A closer
/// hit takes the tree's material flags.
pub(crate) fn trace_through_aabb_tree(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    index: usize,
    trace: &mut Trace,
) {
    let material = &map.materials[map.aabb_trees[index].material_index as usize];
    if material.content_flags & tw.contents == 0 {
        return;
    }
    let old_fraction = trace.fraction;
    trace_through_aabb_tree_r(map, check, tw, index, trace);
    if trace.fraction < old_fraction {
        trace.sflags = material.surface_flags;
        trace.cflags = material.content_flags;
    }
}

/// `CM_TraceThroughAabbTree_r`: the tree's nodes in order, children first
/// to last, from a stack.
fn trace_through_aabb_tree_r(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    index: usize,
    trace: &mut Trace,
) {
    if !node_touched(tw, &map.aabb_trees[index]) {
        return;
    }
    let mut stack = [0usize; AABB_STACK];
    stack[0] = index;
    let mut len = 1;
    while len > 0 {
        len -= 1;
        let node = &map.aabb_trees[stack[len]];
        if node.child_count == 0 {
            aabb_tree_work(map, check, tw, node, trace);
            continue;
        }
        for c in (0..node.child_count as usize).rev() {
            let child = node.u as usize + c;
            if len < AABB_STACK && node_touched(tw, &map.aabb_trees[child]) {
                stack[len] = child;
                len += 1;
            }
        }
    }
}

/// `CM_TraceThroughAabbTree_r`'s work on a leaf node: the triangles of its
/// partition, once a trace.
fn aabb_tree_work(map: &ClipMap<'_>, check: &mut CheckCount<'_>, tw: &TraceWork, node: &AabbTree, trace: &mut Trace) {
    let p = node.u as usize;
    if !check.first_look(p) {
        return;
    }
    let part = &map.partitions[p];
    let old_fraction = trace.fraction;
    for t in 0..part.tri_count as usize {
        let tri = part.first_tri as usize + t;
        if tw.is_point {
            point_through_triangle(tw, map.tri_verts(tri), trace);
        } else {
            capsule_through_triangle(map, tw, tri, trace);
        }
    }
    if trace.fraction < old_fraction {
        trace.hit_partition = p as i32;
    }
}

/// `CM_PointTraceThroughTriangle`: a point sweep hits only the front.
fn point_through_triangle(tw: &TraceWork, [v0, v1, v2]: [[f32; 3]; 3], trace: &mut Trace) {
    let e1 = sub(v0, v1);
    let e2 = sub(v0, v2);
    let n = cross(e2, e1);
    let dn = dot(tw.delta, n);
    if !(dn < 0.0) {
        return;
    }
    let w = sub(v0, tw.extents.start);
    let d = dot(w, n);
    if !(d <= 0.0 && trace.fraction * dn < d) {
        return;
    }
    let c = cross(tw.delta, w);
    let f1 = dot(c, e1);
    if !(f1 <= 0.0 && dn <= f1) {
        return;
    }
    let f2 = dot(c, e2);
    if f2 >= 0.0 && dn <= f1 - f2 {
        trace.walkable = false;
        trace.normal = normalize_to(n).0;
        trace.fraction = d / dn;
    }
}

/// What `CM_TraceSphereThroughEdge` found.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EdgeHit {
    Miss,
    /// The sphere hit the edge; the trace has it.
    Hits,
    /// The sphere passes the edge's first or second end: try that vertex.
    MayHitV0,
    MayHitV1,
}

/// `CM_TraceCapsuleThroughTriangle`: the capsule's lower (or, under a
/// ceiling, upper) sphere against triangle `tri`'s face, then its edges
/// and corners.
fn capsule_through_triangle(map: &ClipMap<'_>, tw: &TraceWork, tri: usize, trace: &mut Trace) {
    let [v0, v1, v2] = map.tri_verts(tri);
    let e1 = sub(v0, v1);
    let e2 = sub(v0, v2);
    let n = cross(e2, e1);
    let dn = dot(tw.delta, n);
    if !(dn < 0.0) {
        return;
    }
    let (nn, len) = normalize_to(n);
    let oz = if nn[2] < 0.0 { -tw.offset_z } else { tw.offset_z };
    let start = tw.extents.start;
    let p = [start[0], start[1], start[2] - oz];
    let q = sub(sub(p, scale(nn, tw.radius)), v0);
    let dist = dot(q, nn);
    let (q, frac, startsolid) = if dist >= 0.0 {
        let frac = -((dist - SURFACE_CLIP_EPSILON) * len) / dn;
        if trace.fraction <= frac {
            return;
        }
        (q, frac, false)
    } else {
        let r = sub(p, v0);
        let dd = dot(r, nn);
        if tw.radius * tw.radius <= dd * dd {
            return;
        }
        (sub(r, scale(nn, dd)), 0.0, true)
    };
    let c = cross(tw.delta, q);
    let base = tri * 3;
    let mut hit_edge = false;
    let mut walkable = true;
    let mut vert = None;
    let f1 = dot(c, e1);
    if f1 < 0.0 {
        hit_edge = true;
        let edge_walkable = map.edge_is_walkable(base + 2);
        match sphere_through_edge(tw, trace, p, v0, e1) {
            EdgeHit::Hits => {
                trace.walkable = edge_walkable;
                return;
            }
            EdgeHit::MayHitV0 => {
                vert = Some(v0);
                walkable = edge_walkable;
            }
            EdgeHit::MayHitV1 => {
                vert = Some(v1);
                walkable = edge_walkable;
            }
            EdgeHit::Miss => {}
        }
    }
    let f2 = dot(c, e2);
    if f2 > 0.0 {
        hit_edge = true;
        let edge_walkable = map.edge_is_walkable(base + 1);
        match sphere_through_edge(tw, trace, p, v0, e2) {
            EdgeHit::Hits => {
                trace.walkable = edge_walkable;
                return;
            }
            EdgeHit::MayHitV0 => {
                vert = Some(v0);
                walkable &= edge_walkable;
            }
            EdgeHit::MayHitV1 => {
                vert = Some(v2);
                walkable &= edge_walkable;
            }
            EdgeHit::Miss => {}
        }
    }
    if dn <= f2 - f1 {
        if !hit_edge {
            trace.normal = nn;
            trace.walkable = false;
            trace.fraction = if frac <= 0.0 { 0.0 } else { frac };
            trace.startsolid = startsolid;
            return;
        }
    } else {
        let edge_walkable = map.edge_is_walkable(base);
        match sphere_through_edge(tw, trace, p, v1, sub(v1, v2)) {
            EdgeHit::Hits => {
                trace.walkable = edge_walkable;
                return;
            }
            EdgeHit::MayHitV0 => {
                vert = Some(v1);
                walkable &= edge_walkable;
            }
            EdgeHit::MayHitV1 => {
                vert = Some(v2);
                walkable &= edge_walkable;
            }
            EdgeHit::Miss => {}
        }
    }
    if let Some(v) = vert {
        sphere_through_vertex(tw, trace, p, v, walkable);
    }
}

/// `CM_TraceSphereThroughEdge`: the sphere from `p` along the sweep
/// against the edge from `v` along `-edge`.
fn sphere_through_edge(tw: &TraceWork, trace: &mut Trace, p: [f32; 3], v: [f32; 3], edge: [f32; 3]) -> EdgeHit {
    let sd = sub(p, v);
    let perp = cross(edge, tw.delta);
    let r = tw.radius + SURFACE_CLIP_EPSILON;
    let s = dot(perp, sd);
    let perp_sq = dot(perp, perp);
    let disc = r * r * perp_sq - s * s;
    if !(0.0 < disc) {
        return EdgeHit::Miss;
    }
    let edge_sq = dot(edge, edge);
    let inv = 1.0 / perp_sq;
    let half = libm::sqrtf(edge_sq * disc) * inv;
    let t = dot(cross(sd, edge), perp);
    if !(0.0 <= inv * t + half) {
        return EdgeHit::Miss;
    }
    let enter = inv * t - half;
    if !(enter < trace.fraction) {
        return EdgeHit::Miss;
    }
    if enter >= 0.0 {
        let hp = add(scale(tw.delta, enter), sd);
        let proj = -dot(edge, hp);
        if !(0.0 < proj) {
            return EdgeHit::MayHitV0;
        }
        if edge_sq <= proj {
            return EdgeHit::MayHitV1;
        }
        let n = add(scale(edge, proj / edge_sq), hp);
        trace.normal = scale(n, 1.0 / r);
        trace.fraction = enter;
        EdgeHit::Hits
    } else {
        let proj = -dot(edge, sd);
        if !(0.0 < proj) {
            return EdgeHit::MayHitV0;
        }
        if edge_sq <= proj {
            return EdgeHit::MayHitV1;
        }
        let n = add(scale(edge, proj / edge_sq), sd);
        if 0.0 <= dot(tw.delta, n) {
            return EdgeHit::Miss;
        }
        trace.normal = normalize_to(n).0;
        trace.fraction = 0.0;
        trace.startsolid = !((tw.radius * tw.radius * perp_sq - s * s) * edge_sq <= t * t);
        EdgeHit::Hits
    }
}

/// `CM_TraceSphereThroughVertex`.
fn sphere_through_vertex(tw: &TraceWork, trace: &mut Trace, p: [f32; 3], vert: [f32; 3], walkable: bool) {
    let sd = sub(p, vert);
    let b = dot(tw.delta, sd);
    if !(b < 0.0) {
        return;
    }
    let c0 = dot(sd, sd);
    let r = tw.radius + SURFACE_CLIP_EPSILON;
    let c = c0 - r * r;
    if 0.0 < c {
        let a = tw.delta_len_sq;
        let disc = b * b - a * c;
        if !(b * b * 0.001 <= disc) {
            return;
        }
        let frac = (-libm::sqrtf(disc) - b) / a;
        if frac < trace.fraction {
            let n = scale(add(scale(tw.delta, frac), sd), 1.0 / r);
            // One Newton step back towards unit length.
            trace.normal = scale(n, (3.0 - dot(n, n)) * 0.5);
            trace.walkable = walkable;
            trace.fraction = frac;
        }
    } else {
        trace.normal = scale(sd, 1.0 / libm::sqrtf(c0));
        trace.walkable = walkable;
        trace.fraction = 0.0;
        if c0 < tw.radius * tw.radius {
            trace.startsolid = true;
        }
    }
}

/// `CM_MeshTestInLeaf`: is the box inside a leaf's terrain?
pub(crate) fn mesh_test_in_leaf(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    leaf: &Leaf,
    trace: &mut Trace,
) {
    let first = leaf.first_coll_aabb as usize;
    for i in 0..leaf.coll_aabb_count as usize {
        let index = first + i;
        let material = &map.materials[map.aabb_trees[index].material_index as usize];
        if material.content_flags & tw.contents == 0 {
            continue;
        }
        position_test_in_aabb_tree_r(map, check, tw, index, trace);
        if trace.allsolid {
            trace.sflags = material.surface_flags;
            trace.cflags = material.content_flags;
            return;
        }
    }
}

/// `CM_PositionTestInAabbTree_r`.
fn position_test_in_aabb_tree_r(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    index: usize,
    trace: &mut Trace,
) {
    let node = &map.aabb_trees[index];
    if !node_touched(tw, node) {
        return;
    }
    if node.child_count == 0 {
        let p = node.u as usize;
        if !check.first_look(p) {
            return;
        }
        let part = &map.partitions[p];
        for t in 0..part.tri_count as usize {
            position_test_capsule_in_triangle(tw, map.tri_verts(part.first_tri as usize + t), trace);
            if trace.startsolid {
                return;
            }
        }
        return;
    }
    for c in 0..node.child_count as usize {
        position_test_in_aabb_tree_r(map, check, tw, node.u as usize + c, trace);
        if trace.startsolid {
            return;
        }
    }
}

/// `CM_PositionTestCapsuleInTriangle`.
fn position_test_capsule_in_triangle(tw: &TraceWork, [v0, v1, v2]: [[f32; 3]; 3], trace: &mut Trace) {
    let start = tw.extents.start;
    let radius_sq = tw.radius * tw.radius;
    let hit = if tw.offset_z == 0.0 {
        let pt = closest_point_on_tri(start, v0, sub(v1, v0), sub(v2, v0));
        let d = sub(pt, start);
        dot(d, d) < radius_sq
    } else {
        let oz = tw.offset_z;
        let a = [start[0], start[1], start[2] + oz];
        let b = [start[0], start[1], start[2] - oz];
        does_capsule_intersect_triangle(a, b, radius_sq, [v0, v1, v2])
    };
    if hit {
        trace.fraction = 0.0;
        trace.allsolid = true;
        trace.startsolid = true;
    }
}

/// `ClosestPointOnTri`: the point of the triangle `b + s*e0 + t*e1` (s, t
/// at least 0, s + t at most 1) nearest `p`, by BO2's region walk.
#[must_use]
pub fn closest_point_on_tri(p: [f32; 3], b: [f32; 3], e0: [f32; 3], e1: [f32; 3]) -> [f32; 3] {
    enum Then {
        Done,
        /// Clamp to the `e0` edge (t = 0).
        EdgeE0,
        /// Clamp to the `e1` edge (s = 0).
        EdgeE1,
        /// Clamp to the far edge (s + t = 1).
        Far,
    }
    let w = sub(b, p);
    let a = dot(e0, e0);
    let bb = dot(e0, e1);
    let c = dot(e1, e1);
    let d = dot(e0, w);
    let e = dot(e1, w);
    let det = a * c - bb * bb;
    let mut s = e * bb - d * c;
    let mut t = d * bb - a * e;
    let then = if det < s + t {
        if s >= 0.0 && t < 0.0 {
            s = 1.0;
            t = 0.0;
            if 0.0 < d + a {
                Then::EdgeE0
            } else if d + a <= e + bb {
                Then::Done
            } else {
                Then::Far
            }
        } else if s < 0.0 {
            s = 0.0;
            t = 1.0;
            if 0.0 < e + c {
                Then::EdgeE1
            } else if e + c <= d + bb {
                Then::Done
            } else {
                Then::Far
            }
        } else {
            Then::Far
        }
    } else if s >= 0.0 {
        if t >= 0.0 {
            let inv = 1.0 / det;
            s *= inv;
            t *= inv;
            Then::Done
        } else {
            Then::EdgeE0
        }
    } else if t < 0.0 {
        s = 0.0;
        t = 0.0;
        if d < 0.0 {
            Then::EdgeE0
        } else if 0.0 <= e {
            Then::Done
        } else {
            Then::EdgeE1
        }
    } else {
        Then::EdgeE1
    };
    match then {
        Then::Done => {}
        Then::EdgeE0 => {
            t = 0.0;
            s = if d < 0.0 {
                if -d < a { -(d / a) } else { 1.0 }
            } else {
                0.0
            };
        }
        Then::EdgeE1 => {
            s = 0.0;
            t = if e < 0.0 {
                if -e < c { -(e / c) } else { 1.0 }
            } else {
                0.0
            };
        }
        Then::Far => {
            t = 1.0;
            let numer = ((c - bb) + e) - d;
            if numer < 0.0 {
                s = 0.0;
            } else {
                let denom = (a - 2.0 * bb) + c;
                if numer < denom {
                    s = numer / denom;
                    t = 1.0 - s;
                } else {
                    s = 1.0;
                    t = 0.0;
                }
            }
        }
    }
    add(add(scale(e0, s), b), scale(e1, t))
}

/// `DoesCapsuleIntersectTriangle`: does the capsule whose sphere centres
/// run from `a` to `b` (radius squared `radius_sq`) touch the triangle?
#[must_use]
pub fn does_capsule_intersect_triangle(a: [f32; 3], b: [f32; 3], radius_sq: f32, [v0, v1, v2]: [[f32; 3]; 3]) -> bool {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let n = cross(e1, e2);
    let plane = f64::from(dot(n, v0));
    let da = f64::from(dot(n, a)) - plane;
    let db = f64::from(dot(n, b)) - plane;
    let aa = f64::from(dot(e1, e1));
    let bb = f64::from(dot(e2, e2));
    let ab = f64::from(dot(e1, e2));
    let det = bb * aa - ab * ab;
    // Is the point, as `v0 - point`, over the triangle?
    let over = |w: [f32; 3]| {
        let wb = f64::from(dot(w, e2));
        let wa = f64::from(dot(w, e1));
        let u = wb * ab - wa * bb;
        let v = wa * ab - wb * aa;
        u >= 0.0 && v >= 0.0 && u + v <= det
    };
    if da * db < 0.0 {
        let f = (da / (da - db)) as f32;
        let x = add(scale(sub(b, a), f), a);
        if over(sub(v0, x)) {
            return true;
        }
    }
    let reach = f64::from(dot(n, n) * radius_sq);
    let near_a = da * da < reach;
    let near_b = db * db < reach;
    if da * db >= 0.0 && !near_a && !near_b {
        return false;
    }
    if near_a && over(sub(v0, a)) {
        return true;
    }
    if near_b && over(sub(v0, b)) {
        return true;
    }
    let seg = sub(b, a);
    !(segment_distance_sq(v0, e1, a, seg) >= radius_sq
        && segment_distance_sq(v0, e2, a, seg) >= radius_sq
        && segment_distance_sq(v1, sub(v2, v1), a, seg) >= radius_sq)
}

/// `CM_DistanceSquaredBetweenSegments`: the squared distance between the
/// segments `p1 + s*d1` and `p2 + t*d2` (s, t in 0..1). BO2's exact sum is
/// not known, so this is the usual clamped closest-points form.
#[must_use]
pub fn segment_distance_sq(p1: [f32; 3], d1: [f32; 3], p2: [f32; 3], d2: [f32; 3]) -> f32 {
    let r = sub(p1, p2);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    let (s, t) = if a <= f32::EPSILON && e <= f32::EPSILON {
        (0.0, 0.0)
    } else if a <= f32::EPSILON {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = dot(d1, r);
        if e <= f32::EPSILON {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut s = if denom != 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    let d = sub(add(p1, scale(d1, s)), add(p2, scale(d2, t)));
    dot(d, d)
}
