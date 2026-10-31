//! BO2's sight traces: `CM_BoxSightTrace` and `CM_SightTracePoint`, which
//! only ask whether anything is in the way. They walk the same trees as the
//! traces but stop at the first thing that blocks, and say what it was: a
//! brush's number plus one, the map's brush count plus one plus the terrain
//! box tree's number for terrain, -1 for a temporary model, 0 for clear.

use trace_t6::{Brush, SURFACE_CLIP_SLACK, Trace, SweepBoxSize, TraceWork};

use crate::temp::{BoxCapsule, TempModels};
use crate::tree::{hi_lo, intersect_extents_aabb, split};
use crate::{CAPSULE_MODEL_HANDLE, CheckCount, ClipMap, Leaf, TEMP_BRUSH_MODEL_HANDLE, dot, mesh, sub};

/// `col_prim_t`: one of the things `CM_SightTracePoint` looks at instead of
/// the map tree when it is handed a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SightPrim {
    /// A terrain box tree, by its number in the map.
    AabbTree(usize),
    /// A brush, by its number in the map.
    Brush(usize),
}

impl ClipMap<'_> {
    /// `CM_BoxSightTrace`: is the box `mins`..`maxs` blocked on its way
    /// from `start` to `end` through `model` (0 for the world)? On the
    /// world, brush `brush_hint - 1` (the one that blocked last time) is
    /// tried first.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn box_sight_trace(
        &self,
        check: &mut CheckCount<'_>,
        brush_hint: i32,
        from_pt: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        model: u32,
        solid_bits: u32,
    ) -> i32 {
        let tw = TraceWork::new(from_pt, end, mins, maxs, solid_bits);
        let mut tr_out = Trace {
            fraction: 1.0,
            ..Trace::default()
        };
        check.global = check.global.wrapping_add(1);
        match model {
            0 => {
                if let Some(brush) = self.hint_brush(brush_hint) {
                    let hit = sight_through_brush(&tw, brush, brush_hint - 1, &mut tr_out);
                    if hit != 0 {
                        return hit;
                    }
                }
                sight_through_tree(self, check, &tw, 0, tw.extents.from, tw.extents.to, &mut tr_out)
            }
            CAPSULE_MODEL_HANDLE => {
                if self.temp.box_contents & solid_bits != 0 {
                    sight_capsule_through_capsule(&tw, &self.temp, tr_out.fraction)
                } else {
                    0
                }
            }
            TEMP_BRUSH_MODEL_HANDLE => sight_through_temp_brush(&tw, &self.temp, &mut tr_out),
            _ => self
                .solid_model(model)
                .map_or(0, |cm| sight_through_leaf(self, check, &tw, &cm.leaf, &mut tr_out)),
        }
    }

    /// `CM_SightTracePoint`: is the line from `start` to `end` blocked by
    /// anything with `mask`? Brush `brush_hint - 1` is tried first, then
    /// `prims` if there are any, else the world.
    #[must_use]
    pub fn sight_trace_point(
        &self,
        check: &mut CheckCount<'_>,
        brush_hint: i32,
        from_pt: [f32; 3],
        end: [f32; 3],
        mask: u32,
        prims: &[SightPrim],
    ) -> i32 {
        if let Some(brush) = self.hint_brush(brush_hint)
            && point_through_brush(&SweepBoxSize::new(from_pt, end), brush)
        {
            return brush_hint;
        }
        let tw = TraceWork::new(from_pt, end, [0.0; 3], [0.0; 3], mask);
        let mut trace = Trace {
            fraction: 1.0,
            ..Trace::default()
        };
        check.global = check.global.wrapping_add(1);
        if prims.is_empty() {
            return sight_through_tree(self, check, &tw, 0, tw.extents.from, tw.extents.to, &mut trace);
        }
        // The first primitive that blocks names the result: a brush by its
        // number plus one, any mesh tree as one past the last brush.
        let mesh_blocked = self.brushes.len() as i32 + 1;
        let mut blocks = |prim: &SightPrim| match *prim {
            SightPrim::Brush(i) => {
                let brush = &self.brushes[i];
                let tested = brush.contents & mask != 0;
                Some(if tested { sight_through_brush(&tw, brush, i as i32, &mut trace) } else { 0 })
                    .filter(|&hit| hit != 0)
            }
            SightPrim::AabbTree(i) => {
                let node = &self.aabb_trees[i];
                let wanted = self.materials[node.material_index as usize].pack_presence_bits & mask != 0;
                let touched = (0..3).all(|k| {
                    tw.bounds[0][k] <= node.half_size[k] + node.origin[k]
                        && node.origin[k] - node.half_size[k] <= tw.bounds[1][k]
                });
                if !(wanted && touched) {
                    return None;
                }
                mesh::trace_through_aabb_tree(self, check, &tw, i, &mut trace);
                (trace.fraction < 1.0).then_some(mesh_blocked)
            }
        };
        prims.iter().find_map(|prim| blocks(prim)).unwrap_or(0)
    }

    /// The brush a sight trace's hint names, if it is one of the map's.
    fn hint_brush(&self, brush_hint: i32) -> Option<&Brush<'_>> {
        if brush_hint < 1 {
            return None;
        }
        self.brushes.get((brush_hint - 1) as usize)
    }
}

fn blend3(a: [f32; 3], b: [f32; 3], amt: f32) -> [f32; 3] {
    [
        (b[0] - a[0]) * amt + a[0],
        (b[1] - a[1]) * amt + a[1],
        (b[2] - a[2]) * amt + a[2],
    ]
}

/// `CM_SightTraceThroughTree`: the map tree from node `num` with the
/// segment `p1`..`p2`, nearer half first.
fn sight_through_tree(
    map: &ClipMap<'_>,
    check: &mut CheckCount<'_>,
    tw: &TraceWork,
    start: i32,
    from: [f32; 3],
    p2: [f32; 3],
    tr_out: &mut Trace,
) -> i32 {
    let (mut num, mut p1) = (start, from);
    loop {
        // Node numbers below zero name leaves, counted down from -1.
        let Ok(at) = usize::try_from(num) else {
            return sight_through_leaf(map, check, tw, &map.leafs[(-1 - num) as usize], tr_out);
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
            (dot(plane.normal, p1) - plane.dist, dot(plane.normal, p2) - plane.dist, offset)
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
        let (side, frac, far_side_ratio) = split(t1, t2, offset);
        let hit = sight_through_tree(map, check, tw, i32::from(node.children[side]), p1, blend3(p1, p2, frac), tr_out);
        if hit != 0 {
            return hit;
        }
        p1 = blend3(p1, p2, far_side_ratio);
        num = i32::from(node.children[side ^ 1]);
    }
}

/// `CM_SightTraceThroughLeaf`: the leaf's brushes, then its terrain.
fn sight_through_leaf(map: &ClipMap<'_>, check: &mut CheckCount<'_>, tw: &TraceWork, leaf: &Leaf, trace: &mut Trace) -> i32 {
    if tw.contents & leaf.solid_content_flags != 0 {
        let hit = sight_through_leaf_brush_node(map, tw, leaf, trace);
        if hit != 0 {
            return hit;
        }
    }
    if tw.contents & leaf.terrain_contents != 0 {
        let first = leaf.first_coll_aabb as usize;
        for i in 0..leaf.collision_box_total as usize {
            mesh::trace_through_aabb_tree(map, check, tw, first + i, trace);
            if trace.fraction != 1.0 {
                return (map.brushes.len() + 1 + first + i) as i32;
            }
        }
    }
    0
}

/// `CM_SightTraceThroughLeafBrushNode`.
fn sight_through_leaf_brush_node(map: &ClipMap<'_>, tw: &TraceWork, leaf: &Leaf, trace: &mut Trace) -> i32 {
    let mut absmins = [0.0; 3];
    let mut absmaxs = [0.0; 3];
    for i in 0..3 {
        absmins[i] = leaf.mins[i] - tw.size[i];
        absmaxs[i] = leaf.maxs[i] + tw.size[i];
    }
    if !intersect_extents_aabb(&tw.extents, absmins, absmaxs, 1.0) {
        return 0;
    }
    sight_through_leaf_brush_node_r(map, tw, leaf.brush_leaf_link as usize, tw.extents.from, tw.extents.to, trace)
}

/// `CM_SightTraceThroughLeafBrushNode_r`.
fn sight_through_leaf_brush_node_r(
    map: &ClipMap<'_>,
    tw: &TraceWork,
    first: usize,
    from: [f32; 3],
    p2: [f32; 3],
    tr_out: &mut Trace,
) -> i32 {
    let (mut idx, mut p1) = (first, from);
    if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
        return 0;
    }
    loop {
        let node = &map.leaf_brush_nodes[idx];
        if node.leaf_solid_total > 0 {
            for &b in node.brushes {
                let brush = &map.brushes[b as usize];
                if tw.contents & brush.contents != 0 {
                    let hit = sight_through_brush(tw, brush, i32::from(b), tr_out);
                    if hit != 0 {
                        return hit;
                    }
                }
            }
            return 0;
        }
        if node.leaf_solid_total < 0 {
            let hit = sight_through_leaf_brush_node_r(map, tw, idx + 1, p1, p2, tr_out);
            if hit != 0 {
                return hit;
            }
        }
        let axis = node.axis as usize;
        let t1 = p1[axis] - node.dist;
        let t2 = p2[axis] - node.dist;
        let offset = (tw.size[axis] + SURFACE_CLIP_SLACK) - node.range;
        let (hi, lo) = hi_lo(t1, t2);
        let step = if !(lo < offset) {
            if hi <= -offset {
                return 0;
            }
            node.child_offset[0]
        } else if !(-offset < hi) {
            node.child_offset[1]
        } else {
            let (side, frac, far_side_ratio) = split(t1, t2, offset);
            let near = idx + node.child_offset[side] as usize;
            let hit = sight_through_leaf_brush_node_r(map, tw, near, p1, blend3(p1, p2, frac), tr_out);
            if hit != 0 {
                return hit;
            }
            p1 = blend3(p1, p2, far_side_ratio);
            node.child_offset[side ^ 1]
        };
        idx += step as usize;
        if tw.contents & map.leaf_brush_nodes[idx].contents == 0 {
            return 0;
        }
    }
}

/// `CM_SightTraceThroughBrush`: does the sweep cross `brush` (number
/// `num`)? If so `num + 1`, with the trace cut where it goes in.
pub(crate) fn sight_through_brush(tw: &TraceWork, brush: &Brush<'_>, num: i32, trace: &mut Trace) -> i32 {
    let ext = &tw.extents;
    let mut enter = 0.0;
    let mut leave = 1.0;
    for (b, sign) in [(brush.lower, -1.0), (brush.upper, 1.0)] {
        for (i, bi) in b.iter().enumerate() {
            let d1 = (ext.from[i] - bi) * sign - tw.radius_offset[i];
            let d2 = (ext.to[i] - bi) * sign - tw.radius_offset[i];
            if !cut_axial(d1, d2, ext.inv_delta[i] * d1 * sign, &mut enter, &mut leave) {
                return 0;
            }
        }
    }
    for side in brush.sides {
        let plane = &side.plane;
        let dist = (tw.offset_z * plane.normal[2]).abs() + plane.plane_offset + tw.radius;
        let d1 = dot(plane.normal, ext.from) - dist;
        let d2 = dot(plane.normal, ext.to) - dist;
        if !cut_side(d1, d2, &mut enter, &mut leave) {
            return 0;
        }
    }
    trace.cflags = brush.contents;
    trace.fraction = enter;
    trace.sflags = 0;
    num + 1
}

/// `CM_SightTracePointThroughBrush`: does the line cross `brush`?
fn point_through_brush(ext: &SweepBoxSize, brush: &Brush<'_>) -> bool {
    let mut enter = 0.0;
    let mut leave = 1.0;
    for (b, sign) in [(brush.lower, -1.0), (brush.upper, 1.0)] {
        for (i, bi) in b.iter().enumerate() {
            let d1 = (ext.from[i] - bi) * sign;
            let d2 = (ext.to[i] - bi) * sign;
            if !cut_axial(d1, d2, ext.inv_delta[i] * d1 * sign, &mut enter, &mut leave) {
                return false;
            }
        }
    }
    for side in brush.sides {
        let plane = &side.plane;
        let d1 = dot(plane.normal, ext.from) - plane.plane_offset;
        let d2 = dot(plane.normal, ext.to) - plane.plane_offset;
        if !cut_side(d1, d2, &mut enter, &mut leave) {
            return false;
        }
    }
    true
}

/// One face of a brush's box, the start `d1` and end `d2` past it and the
/// fraction `f` where the line crosses it: narrow `enter`..`leave`, false
/// once nothing is left.
fn cut_axial(d1: f32, d2: f32, f: f32, enter: &mut f32, leave: &mut f32) -> bool {
    match (d1 <= 0.0, 0.0 < d2) {
        // Inside the face at both ends: it cuts nothing.
        (true, false) => true,
        // Outside at both ends: the line misses the brush.
        (false, true) => false,
        // Going out through it: the exit may come sooner.
        (true, true) => {
            let open = !(f <= *enter);
            if open && f - *leave < 0.0 {
                *leave = f;
            }
            open
        }
        // Coming in through it: the entry may come later.
        (false, false) => {
            let open = !(*leave <= f);
            if open && *enter - f < 0.0 {
                *enter = f;
            }
            open
        }
    }
}

/// One side plane of a brush, as `cut_axial`.
fn cut_side(d1: f32, d2: f32, enter: &mut f32, leave: &mut f32) -> bool {
    let inside_at_start = d1 <= 0.0;
    let outside_at_end = 0.0 < d2;
    if !inside_at_start && outside_at_end {
        return false;
    }
    let travel = d1 - d2;
    let bound = if inside_at_start { &mut *leave } else { &mut *enter };
    // Leaving moves the exit in; entering moves the entry on.
    let tighter = if inside_at_start { outside_at_end && travel * *bound < d1 } else { travel * *bound < d1 };
    if tighter {
        *bound = d1 / travel;
        return *enter < *leave;
    }
    true
}

/// `CM_SightTraceThroughTempBrush`: -1 if any of the temporary brushes
/// blocks.
fn sight_through_temp_brush(tw: &TraceWork, temp: &TempModels<'_>, trace: &mut Trace) -> i32 {
    for brush in temp.brushes {
        if brush.contents & tw.contents != 0 && sight_through_brush(tw, brush, 1, trace) != 0 {
            return -1;
        }
    }
    0
}

/// `CM_SightTraceCapsuleThroughCapsule`: -1 if the box's capsule blocks
/// the trace's: the end spheres that can meet first, then the sides, then
/// the end spheres the sides let past.
fn sight_capsule_through_capsule(tw: &TraceWork, temp: &TempModels<'_>, fraction: f32) -> i32 {
    let m = &temp.box_model;
    for i in 0..3 {
        if m.maxs[i] + 1.0 < tw.bounds[0][i] {
            return 0;
        }
    }
    for i in 0..3 {
        if tw.bounds[1][i] < m.mins[i] - 1.0 {
            return 0;
        }
    }
    let s = tw.extents.from;
    let e = tw.extents.to;
    let start_top = [s[0], s[1], tw.offset_z + s[2]];
    let from_foot = [s[0], s[1], s[2] - tw.offset_z];
    let b = BoxCapsule::new(m);
    let (box_top, box_bottom) = (b.top(), b.bottom());
    // The trace's top sphere against the box's bottom one, and its bottom
    // sphere against the box's top one.
    let from_below = || sight_sphere_blocked(tw, start_top, box_bottom, b.radius, fraction);
    let from_above = || sight_sphere_blocked(tw, from_foot, box_top, b.radius, fraction);

    if from_foot[2] <= box_top[2] {
        if start_top[2] < box_bottom[2] {
            if from_below() {
                return -1;
            }
            if tw.delta[2] <= 0.0 {
                return 0;
            }
        }
    } else {
        if from_above() {
            return -1;
        }
        if 0.0 <= tw.delta[2] {
            return 0;
        }
    }
    if sight_cylinder_blocked(tw, &b, fraction) {
        return -1;
    }
    let blocked = if e[2] - tw.offset_z <= box_top[2] {
        if box_bottom[2] <= tw.offset_z + e[2] || start_top[2] < box_bottom[2] {
            return 0;
        }
        from_below()
    } else {
        if box_top[2] < from_foot[2] {
            return 0;
        }
        from_above()
    };
    if blocked { -1 } else { 0 }
}

/// `CM_SightTraceSphereThroughSphere`: does the trace's end sphere at
/// `from` start inside, or run into, a sphere of `radius` at `centre`
/// before `fraction`?
fn sight_sphere_blocked(tw: &TraceWork, from: [f32; 3], centre: [f32; 3], reach_r: f32, fraction: f32) -> bool {
    let d = sub(from, centre);
    let d_sq = dot(d, d);
    let reach = tw.radius + reach_r;
    let c = d_sq - reach * reach;
    if c <= 0.0 {
        return true;
    }
    let b = dot(tw.delta, d);
    if b < 0.0 {
        let disc = b * b - tw.delta_len_sq * c;
        if 0.0 <= disc && (-b - libm::sqrtf(disc)) / tw.delta_len_sq + (b * 0.125) / libm::sqrtf(d_sq) < fraction {
            return true;
        }
    }
    false
}

/// `CM_SightTraceCylinderThroughCylinder`: do the sides of the trace's
/// capsule start against, or run into, the sides of the box's before
/// `fraction`?
fn sight_cylinder_blocked(tw: &TraceWork, b: &BoxCapsule, fraction: f32) -> bool {
    let s = tw.extents.from;
    let step_along_x = s[0] - b.centre[0];
    let dy = s[1] - b.centre[1];
    let d_sq = dy * dy + step_along_x * step_along_x;
    let reach = tw.radius + b.radius;
    let c = d_sq - reach * reach;
    let total_height = (tw.size[2] - tw.radius) + b.half_height;
    if !(0.0 < c) {
        // The sides already overlap: only the heights can keep them apart.
        return (s[2] - b.centre[2]).abs() <= total_height;
    }
    // Apart at the start: they must close in, meet, and meet in time.
    let bb = tw.delta[1] * dy + tw.delta[0] * step_along_x;
    let a = tw.delta[1] * tw.delta[1] + tw.delta[0] * tw.delta[0];
    let disc = bb * bb - a * c;
    if 0.0 <= bb || disc < 0.0 {
        return false;
    }
    let epsilon = (bb * 0.125) / libm::sqrtf(d_sq);
    let frac = (-bb - libm::sqrtf(disc)) / a + epsilon;
    !(fraction <= frac) && ((frac - epsilon) * tw.delta[2] + s[2] - b.centre[2]).abs() <= total_height
}
