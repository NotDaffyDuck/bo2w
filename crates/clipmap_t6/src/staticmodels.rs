//! Point traces against the static models: BO2's `CM_PointTraceStaticModels`
//! and `CM_PointTraceStaticModelsComplete` walk the world sectors along the
//! line, and each static model it touches takes the line into its own
//! space and traces its collision triangles (`CM_TraceStaticModel`,
//! `XModelTraceLine`). Only point traces reach the static models.

use crate::{StaticModel, World, angles_to_axis, intersect_extents_aabb, normalize_to};
use trace_t6::{ENTITYNUM_WORLD, Trace, TraceExtents, TraceHitType, trace_set_hit_id};

/// How far before a triangle a trace stops.
pub const XMODEL_TRACE_SKIN: f32 = 0.125;
/// How far outside a triangle's edges a hit still counts.
pub const XMODEL_TRI_EPSILON: f32 = 0.001;

/// `XModelCollTri_s`: the triangle's plane, then two edge planes that read
/// 0 to 1 across it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XModelCollTri {
    pub plane: [f32; 4],
    pub svec: [f32; 4],
    pub tvec: [f32; 4],
}

/// `XModelCollSurf_s`: a group of collision triangles, its box, its bone,
/// its contents and its surface flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XModelCollSurf<'a> {
    pub tris: &'a [XModelCollTri],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub bone_idx: i32,
    pub contents: u32,
    pub surf_flags: u32,
}

/// The parts of `XModel` its line trace reads: no collision when
/// `coll_lod` is below 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XModelColl<'a> {
    pub coll_lod: i32,
    pub surfs: &'a [XModelCollSurf<'a>],
}

impl XModelColl<'_> {
    /// `XModelTraceLine`: trace `start`..`end`, in the model's own space,
    /// through the triangles of the surfaces with `contentmask`, keeping
    /// the nearest hit in `trace`; the bone of the last surface it hit,
    /// or -1.
    pub fn trace_line(&self, trace: &mut Trace, start: [f32; 3], end: [f32; 3], contentmask: u32) -> i32 {
        if self.coll_lod < 0 {
            return -1;
        }
        let ext = TraceExtents::new(start, end);
        let delta = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let mut hit = -1;
        for surf in self.surfs {
            if surf.contents & contentmask == 0 || !intersect_extents_aabb(&ext, surf.mins, surf.maxs, trace.fraction) {
                continue;
            }
            for tri in surf.tris {
                let p = tri.plane;
                let d_end = p[0] * end[0] + p[1] * end[1] + p[2] * end[2] - p[3];
                if !(d_end < 0.0) {
                    continue;
                }
                let d_start = plane_dist(p, start);
                if !(0.0 < d_start) {
                    continue;
                }
                let span = d_start - d_end;
                let mut frac = (d_start - XMODEL_TRACE_SKIN) / span;
                if frac < 0.0 {
                    frac = 0.0;
                }
                if !(frac < trace.fraction) {
                    continue;
                }
                let t = d_start / span;
                let at = [
                    delta[0] * t + start[0],
                    delta[1] * t + start[1],
                    delta[2] * t + start[2],
                ];
                let s = plane_dist(tri.svec, at);
                if !(-XMODEL_TRI_EPSILON..=1.0 + XMODEL_TRI_EPSILON).contains(&s) {
                    continue;
                }
                let u = plane_dist(tri.tvec, at);
                if !(-XMODEL_TRI_EPSILON <= u && u + s <= 1.0 + XMODEL_TRI_EPSILON) {
                    continue;
                }
                hit = surf.bone_idx;
                trace.allsolid = false;
                trace.startsolid = false;
                trace.fraction = frac;
                trace.sflags = surf.surf_flags;
                trace.cflags = surf.contents;
                trace.normal = [p[0], p[1], p[2]];
            }
        }
        hit
    }
}

/// A point's distance past a plane, summed in BO2's order.
fn plane_dist(p: [f32; 4], v: [f32; 3]) -> f32 {
    p[1] * v[1] + p[0] * v[0] + p[2] * v[2] - p[3]
}

/// `CM_InitStaticModel`'s inverse axis: the transpose of the model's axis
/// over its scale, which takes a world offset into the model's space.
#[must_use]
pub fn inv_scaled_axis(angles: [f32; 3], scale: f32) -> [[f32; 3]; 3] {
    let axis = angles_to_axis(angles);
    let inv = 1.0 / scale;
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = axis[j][i] * inv;
        }
    }
    out
}

impl StaticModel<'_> {
    /// A world point in the model's own space.
    fn local_point(&self, p: [f32; 3]) -> [f32; 3] {
        let a = self.inv_scaled_axis;
        let d = [p[0] - self.origin[0], p[1] - self.origin[1], p[2] - self.origin[2]];
        [
            a[0][0] * d[0] + a[1][0] * d[1] + a[2][0] * d[2],
            a[0][1] * d[0] + a[1][1] * d[1] + a[2][1] * d[2],
            a[0][2] * d[0] + a[1][2] * d[1] + a[2][2] * d[2],
        ]
    }

    fn trace_line(&self, trace: &mut Trace, start: [f32; 3], end: [f32; 3], contentmask: u32) -> i32 {
        self.xmodel
            .map_or(-1, |m| m.trace_line(trace, self.local_point(start), self.local_point(end), contentmask))
    }

    /// `CM_TraceStaticModel`: trace `start`..`end` through this model
    /// (number `index`); a hit names it, counts as the world's and turns
    /// the normal back into the world.
    pub fn trace(&self, index: u32, trace: &mut Trace, start: [f32; 3], end: [f32; 3], contentmask: u32) {
        if self.trace_line(trace, start, end, contentmask) < 0 {
            return;
        }
        trace.static_model = Some(index);
        trace_set_hit_id(trace, TraceHitType::Entity, ENTITYNUM_WORLD);
        let a = self.inv_scaled_axis;
        let n = trace.normal;
        let world = [
            n[0] * a[0][0] + n[1] * a[0][1] + n[2] * a[0][2],
            n[1] * a[1][1] + a[1][0] * n[0] + a[1][2] * n[2],
            n[1] * a[2][1] + a[2][0] * n[0] + a[2][2] * n[2],
        ];
        trace.normal = normalize_to(world).0;
    }

    /// `CM_TraceStaticModelComplete`: whether `start`..`end` gets through
    /// this model.
    #[must_use]
    pub fn trace_complete(&self, start: [f32; 3], end: [f32; 3], contentmask: u32) -> bool {
        let mut trace = Trace::new();
        self.trace_line(&mut trace, start, end, contentmask) < 0
    }
}

/// Whether the sector walks skip a static model: BO2 tests the mask's
/// lowest bit against "has no contents".
fn skips(contentmask: u32, contents: u32) -> bool {
    contentmask & u32::from(contents == 0) != 0
}

fn lerp4(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    [
        (b[0] - a[0]) * f + a[0],
        (b[1] - a[1]) * f + a[1],
        (b[2] - a[2]) * f + a[2],
        (b[3] - a[3]) * f + a[3],
    ]
}

impl World {
    /// `CM_PointTraceStaticModels`: trace the point `start`..`end` through
    /// the static models with `contentmask`, keeping a nearer hit than the
    /// one `trace` holds.
    pub fn point_trace_static_models(
        &self,
        models: &[StaticModel<'_>],
        trace: &mut Trace,
        start: [f32; 3],
        end: [f32; 3],
        contentmask: u32,
    ) {
        let ext = TraceExtents::new(start, end);
        let p1 = [ext.start[0], ext.start[1], ext.start[2], 0.0];
        let p2 = [ext.end[0], ext.end[1], ext.end[2], trace.fraction];
        self.point_trace_static_models_r(models, &ext, contentmask, 1, p1, p2, trace);
    }

    /// `CM_PointTraceStaticModels_r`: the models filed in `node` and its
    /// children along `p1`..`p2`, whose fourth part is the fraction.
    #[allow(clippy::too_many_arguments)]
    fn point_trace_static_models_r(
        &self,
        models: &[StaticModel<'_>],
        ext: &TraceExtents,
        contentmask: u32,
        mut node: usize,
        mut p1: [f32; 4],
        p2: [f32; 4],
        trace: &mut Trace,
    ) {
        while contentmask & self.sectors[node].contents_static_models != 0 {
            let s = &self.sectors[node];
            let mut m = s.static_models;
            while m != 0 {
                let i = m as usize - 1;
                let model = &models[i];
                if !skips(contentmask, model.contents)
                    && intersect_extents_aabb(ext, model.absmin, model.absmax, trace.fraction)
                {
                    model.trace(i as u32, trace, ext.start, ext.end, contentmask);
                }
                m = model.next_model_in_world_sector;
            }
            let axis = s.axis as usize;
            let t2 = p2[axis] - s.dist;
            let t1 = p1[axis] - s.dist;
            if t2 * t1 < 0.0 {
                if trace.fraction <= p1[3] {
                    return;
                }
                let mid = lerp4(p1, p2, t1 / (t1 - t2));
                let side = usize::from(0.0 <= t2);
                self.point_trace_static_models_r(models, ext, contentmask, s.children[side] as usize, p1, mid, trace);
                p1 = mid;
                node = s.children[1 - side] as usize;
            } else {
                let near = if t2 - t1 < 0.0 { t2 } else { t1 };
                node = s.children[usize::from(near < 0.0)] as usize;
            }
        }
    }

    /// `CM_PointTraceStaticModelsComplete`: whether the point `start`..`end`
    /// gets through every static model with `contentmask`.
    #[must_use]
    pub fn point_trace_static_models_complete(
        &self,
        models: &[StaticModel<'_>],
        start: [f32; 3],
        end: [f32; 3],
        contentmask: u32,
    ) -> bool {
        let ext = TraceExtents::new(start, end);
        self.point_trace_static_models_complete_r(models, &ext, contentmask, 1, ext.start, ext.end)
    }

    /// `CM_PointTraceStaticModelsComplete_r`.
    fn point_trace_static_models_complete_r(
        &self,
        models: &[StaticModel<'_>],
        ext: &TraceExtents,
        contentmask: u32,
        mut node: usize,
        mut p1: [f32; 3],
        p2: [f32; 3],
    ) -> bool {
        while contentmask & self.sectors[node].contents_static_models != 0 {
            let s = &self.sectors[node];
            let mut m = s.static_models;
            while m != 0 {
                let model = &models[m as usize - 1];
                if !skips(contentmask, model.contents)
                    && intersect_extents_aabb(ext, model.absmin, model.absmax, 1.0)
                    && !model.trace_complete(ext.start, ext.end, contentmask)
                {
                    return false;
                }
                m = model.next_model_in_world_sector;
            }
            let axis = s.axis as usize;
            let t2 = p2[axis] - s.dist;
            let t1 = p1[axis] - s.dist;
            if t2 * t1 < 0.0 {
                let f = t1 / (t1 - t2);
                let mid = [
                    (p2[0] - p1[0]) * f + p1[0],
                    (p2[1] - p1[1]) * f + p1[1],
                    (p2[2] - p1[2]) * f + p1[2],
                ];
                let side = usize::from(0.0 <= t2);
                if !self.point_trace_static_models_complete_r(models, ext, contentmask, s.children[side] as usize, p1, mid)
                {
                    return false;
                }
                p1 = mid;
                node = s.children[1 - side] as usize;
            } else {
                let near = if t2 - t1 < 0.0 { t2 } else { t1 };
                node = s.children[usize::from(near < 0.0)] as usize;
            }
        }
        true
    }
}
