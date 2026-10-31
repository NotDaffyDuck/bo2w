//! Point traces against the static models: BO2's `CM_PointTraceStaticModels`
//! and `CM_PointTraceStaticModelsComplete` walk the world sectors along the
//! line, and each static model it touches takes the line into its own
//! space and traces its collision triangles (`CM_TraceStaticModel`,
//! `XModelTraceLine`). Only point traces reach the static models.

use crate::{StaticModel, World, angles_to_axis, intersect_extents_aabb, unit_and_length};
use trace_t6::{ENTITYNUM_WORLD, Trace, SweepBoxSize, TraceHitType, trace_set_hit_id};

/// How far before a triangle a trace stops.
pub const XMODEL_TRACE_SKIN: f32 = 0.125;
/// How far outside a triangle's edges a hit still counts.
pub const XMODEL_TRI_EPSILON: f32 = 0.001;

/// `XModelCollTri_s`: the triangle's plane, then two edge planes that read
/// 0 to 1 across it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModelHitTriangle {
    pub plane: [f32; 4],
    pub surface_tangent: [f32; 4],
    pub surface_bitangent: [f32; 4],
}

/// `XModelCollSurf_s`: a group of collision triangles, its box, its bone,
/// its contents and its surface flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XModelCollSurf<'a> {
    pub triangle_list: &'a [ModelHitTriangle],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub joint_position: i32,
    pub contents: u32,
    pub face_property_bits: u32,
}

/// The parts of `XModel` its line trace reads: no collision when
/// `collision_detail_tier` is below 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XModelColl<'a> {
    pub collision_detail_tier: i32,
    pub surfs: &'a [XModelCollSurf<'a>],
}

impl XModelColl<'_> {
    /// `XModelTraceLine`: trace `start`..`end`, in the model's own space,
    /// through the triangles of the surfaces with `contentmask`, keeping
    /// the nearest hit in `trace`; the bone of the last surface it hit,
    /// or -1.
    pub fn trace_line(&self, tr_out: &mut Trace, from_pt: [f32; 3], to_pt: [f32; 3], contentmask: u32) -> i32 {
        if self.collision_detail_tier < 0 {
            return -1;
        }
        let ext = SweepBoxSize::new(from_pt, to_pt);
        let delta = [to_pt[0] - from_pt[0], to_pt[1] - from_pt[1], to_pt[2] - from_pt[2]];
        let mut hit = -1;
        for surf in self.surfs {
            if surf.contents & contentmask == 0 || !intersect_extents_aabb(&ext, surf.mins, surf.maxs, tr_out.fraction) {
                continue;
            }
            for tri in surf.triangle_list {
                let p = tri.plane;
                let d_end = p[0] * to_pt[0] + p[1] * to_pt[1] + p[2] * to_pt[2] - p[3];
                if !(d_end < 0.0) {
                    continue;
                }
                let d_start = plane_dist(p, from_pt);
                if !(0.0 < d_start) {
                    continue;
                }
                let span = d_start - d_end;
                let skinned = (d_start - XMODEL_TRACE_SKIN) / span;
                let frac = if skinned < 0.0 { 0.0 } else { skinned };
                if !(frac < tr_out.fraction) {
                    continue;
                }
                let t = d_start / span;
                let at = [
                    delta[0] * t + from_pt[0],
                    delta[1] * t + from_pt[1],
                    delta[2] * t + from_pt[2],
                ];
                let s = plane_dist(tri.surface_tangent, at);
                if !(-XMODEL_TRI_EPSILON..=1.0 + XMODEL_TRI_EPSILON).contains(&s) {
                    continue;
                }
                let u = plane_dist(tri.surface_bitangent, at);
                if !(-XMODEL_TRI_EPSILON <= u && u + s <= 1.0 + XMODEL_TRI_EPSILON) {
                    continue;
                }
                hit = surf.joint_position;
                take_surface_hit(tr_out, frac, p, surf.face_property_bits, surf.contents);
            }
        }
        hit
    }
}

/// A nearer hit on a model surface: the trace now ends there, outside any
/// solid, with the surface's flags and contents.
fn take_surface_hit(trace: &mut Trace, frac_at: f32, face_pl: [f32; 4], surf_ix: u32, solid_bits: u32) {
    trace.starts_inside = false;
    trace.fully_embedded = false;
    trace.normal = [face_pl[0], face_pl[1], face_pl[2]];
    trace.fraction = frac_at;
    trace.cflags = solid_bits;
    trace.sflags = surf_ix;
}

/// A point's distance past a plane, summed in BO2's order.
fn plane_dist(p: [f32; 4], v: [f32; 3]) -> f32 {
    p[1] * v[1] + p[0] * v[0] + p[2] * v[2] - p[3]
}

/// `CM_InitStaticModel`'s inverse axis: the transpose of the model's axis
/// over its scale, which takes a world offset into the model's space.
#[must_use]
pub fn reverse_scaled_basis(angles: [f32; 3], scale: f32) -> [[f32; 3]; 3] {
    let axis = angles_to_axis(angles);
    let shrink = scale.recip();
    core::array::from_fn(|row| core::array::from_fn(|col| axis[col][row] * shrink))
}

impl StaticModel<'_> {
    /// A world point in the model's own space.
    fn local_point(&self, p: [f32; 3]) -> [f32; 3] {
        let a = self.reverse_scaled_basis;
        let d = [p[0] - self.origin[0], p[1] - self.origin[1], p[2] - self.origin[2]];
        [
            a[0][0] * d[0] + a[1][0] * d[1] + a[2][0] * d[2],
            a[0][1] * d[0] + a[1][1] * d[1] + a[2][1] * d[2],
            a[0][2] * d[0] + a[1][2] * d[1] + a[2][2] * d[2],
        ]
    }

    fn trace_line(&self, hit: &mut Trace, from_pt: [f32; 3], to_pt: [f32; 3], contentmask: u32) -> i32 {
        self.xmodel
            .map_or(-1, |m| m.trace_line(hit, self.local_point(from_pt), self.local_point(to_pt), contentmask))
    }

    /// `CM_TraceStaticModel`: trace `start`..`end` through this model
    /// (number `index`); a hit names it, counts as the world's and turns
    /// the normal back into the world.
    pub fn trace(&self, index: u32, trace: &mut Trace, from_pt: [f32; 3], to_pt: [f32; 3], contentmask: u32) {
        if self.trace_line(trace, from_pt, to_pt, contentmask) < 0 {
            return;
        }
        trace.static_model = Some(index);
        trace_set_hit_id(trace, TraceHitType::Entity, ENTITYNUM_WORLD);
        let a = self.reverse_scaled_basis;
        let n = trace.normal;
        let world = [
            n[0] * a[0][0] + n[1] * a[0][1] + n[2] * a[0][2],
            n[1] * a[1][1] + a[1][0] * n[0] + a[1][2] * n[2],
            n[1] * a[2][1] + a[2][0] * n[0] + a[2][2] * n[2],
        ];
        trace.normal = unit_and_length(world).unit;
    }

    /// `CM_TraceStaticModelComplete`: whether `start`..`end` gets through
    /// this model.
    #[must_use]
    pub fn trace_complete(&self, from_pt: [f32; 3], to_pt: [f32; 3], contentmask: u32) -> bool {
        let mut trace = Trace::new();
        self.trace_line(&mut trace, from_pt, to_pt, contentmask) < 0
    }
}

/// Whether the sector walks skip a static model: BO2 tests the mask's
/// lowest bit against "has no contents".
fn skips(contentmask: u32, contents: u32) -> bool {
    contentmask & u32::from(contents == 0) != 0
}

fn lerp4(a: [f32; 4], b: [f32; 4], amt: f32) -> [f32; 4] {
    [
        (b[0] - a[0]) * amt + a[0],
        (b[1] - a[1]) * amt + a[1],
        (b[2] - a[2]) * amt + a[2],
        (b[3] - a[3]) * amt + a[3],
    ]
}

impl World {
    /// `CM_PointTraceStaticModels`: trace the point `start`..`end` through
    /// the static models with `contentmask`, keeping a nearer hit than the
    /// one `trace` holds.
    pub fn point_trace_static_models(
        &self,
        models: &[StaticModel<'_>],
        hit: &mut Trace,
        from_pt: [f32; 3],
        end: [f32; 3],
        want_bits: u32,
    ) {
        let ext = SweepBoxSize::new(from_pt, end);
        let p1 = [ext.from[0], ext.from[1], ext.from[2], 0.0];
        let p2 = [ext.to[0], ext.to[1], ext.to[2], hit.fraction];
        self.point_trace_static_models_r(models, &ext, want_bits, 1, p1, p2, hit);
    }

    /// `CM_PointTraceStaticModels_r`: the models filed in `node` and its
    /// children along `p1`..`p2`, whose fourth part is the fraction.
    #[allow(clippy::too_many_arguments)]
    fn point_trace_static_models_r(
        &self,
        models: &[StaticModel<'_>],
        ext: &SweepBoxSize,
        want_bits: u32,
        mut node: usize,
        mut seg_a: [f32; 4],
        p2: [f32; 4],
        hit: &mut Trace,
    ) {
        while want_bits & self.sectors[node].contents_static_models != 0 {
            let s = &self.sectors[node];
            let mut m = s.prop_set;
            while m != 0 {
                let i = m as usize - 1;
                let model = &models[i];
                if !skips(want_bits, model.contents)
                    && intersect_extents_aabb(ext, model.absmin, model.absmax, hit.fraction)
                {
                    model.trace(i as u32, hit, ext.from, ext.to, want_bits);
                }
                m = model.next_model_in_world_sector;
            }
            let axis = s.axis as usize;
            let t2 = p2[axis] - s.dist;
            let t1 = seg_a[axis] - s.dist;
            if t2 * t1 < 0.0 {
                if hit.fraction <= seg_a[3] {
                    return;
                }
                let mid = lerp4(seg_a, p2, t1 / (t1 - t2));
                let side = usize::from(0.0 <= t2);
                self.point_trace_static_models_r(models, ext, want_bits, s.children[side] as usize, seg_a, mid, hit);
                seg_a = mid;
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
        from_pt: [f32; 3],
        end: [f32; 3],
        want_bits: u32,
    ) -> bool {
        let ext = SweepBoxSize::new(from_pt, end);
        self.point_trace_static_models_complete_r(models, &ext, want_bits, 1, ext.from, ext.to)
    }

    /// `CM_PointTraceStaticModelsComplete_r`.
    fn point_trace_static_models_complete_r(
        &self,
        models: &[StaticModel<'_>],
        ext: &SweepBoxSize,
        want_bits: u32,
        first_sector: usize,
        from: [f32; 3],
        to: [f32; 3],
    ) -> bool {
        let (mut node, mut p1, p2) = (first_sector, from, to);
        while want_bits & self.sectors[node].contents_static_models != 0 {
            let s = &self.sectors[node];
            // Links hold one more than the model's number; 0 ends the list.
            let filed = core::iter::successors(usize::from(s.prop_set).checked_sub(1), |&num| {
                usize::from(models[num].next_model_in_world_sector).checked_sub(1)
            });
            let clear = filed.map(|num| &models[num]).all(|model| {
                skips(want_bits, model.contents)
                    || !intersect_extents_aabb(ext, model.absmin, model.absmax, 1.0)
                    || model.trace_complete(ext.from, ext.to, want_bits)
            });
            if !clear {
                return false;
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
                if !self.point_trace_static_models_complete_r(models, ext, want_bits, s.children[side] as usize, p1, mid)
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
