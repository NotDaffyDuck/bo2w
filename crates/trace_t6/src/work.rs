//! BO2's trace setup (`traceWork_t`, `CM_CalcTraceExtents`, `CM_Trace`).
//!
//! A box trace in BO2 is a capsule: the box is moved so its middle is the
//! origin, then swept as a capsule whose radius is the smaller of the box's
//! half width and half height. `offset_z` is how far the capsule's two
//! sphere centres sit above and below its middle.

use crate::{Brush, Trace, test_box_in_brush, trace_through_brush};

/// How far a sweep stops short of a surface.
pub const SURFACE_CLIP_EPSILON: f32 = 0.125;

/// BO2's `TraceExtents`: the swept segment and one over its length per axis
/// (`1 / (start - end)`, or 0 on an axis it does not move along).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraceExtents {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub inv_delta: [f32; 3],
}

impl TraceExtents {
    /// `CM_CalcTraceExtents`.
    #[must_use]
    pub fn new(start: [f32; 3], end: [f32; 3]) -> Self {
        let mut inv_delta = [0.0; 3];
        for (i, inv) in inv_delta.iter_mut().enumerate() {
            let d = start[i] - end[i];
            *inv = if d == 0.0 { 0.0 } else { 1.0 / d };
        }
        Self {
            start,
            end,
            inv_delta,
        }
    }
}

/// BO2's `traceWork_t` (less its thread scratch).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraceWork {
    pub delta: [f32; 3],
    pub midpoint: [f32; 3],
    pub half_delta: [f32; 3],
    pub half_delta_abs: [f32; 3],
    /// Half the box, per axis.
    pub size: [f32; 3],
    /// What the capsule touches on its way: `bounds[0]` low, `bounds[1]` high.
    pub bounds: [[f32; 3]; 2],
    /// How far the capsule reaches along each axis from its middle.
    pub radius_offset: [f32; 3],
    pub extents: TraceExtents,
    pub delta_len: f32,
    pub delta_len_sq: f32,
    pub contents: u32,
    pub is_point: bool,
    /// The sweep's bounds are small next to the box, so BO2 culls by the
    /// bounds alone.
    pub axial_cull_only: bool,
    pub radius: f32,
    pub offset_z: f32,
    pub bounding_radius: f32,
}

impl TraceWork {
    /// The setup `CM_Trace` does for a box from `start` to `end`.
    #[must_use]
    pub fn new(start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3], contents: u32) -> Self {
        let mut tw = Self {
            contents,
            ..Self::default()
        };
        let mut s = [0.0; 3];
        let mut e = [0.0; 3];
        for i in 0..3 {
            let offset = (mins[i] + maxs[i]) * 0.5;
            tw.size[i] = maxs[i] - offset;
            s[i] = start[i] + offset;
            e[i] = end[i] + offset;
            tw.midpoint[i] = (s[i] + e[i]) * 0.5;
            tw.delta[i] = e[i] - s[i];
            tw.half_delta[i] = tw.delta[i] * 0.5;
            tw.half_delta_abs[i] = tw.half_delta[i].abs();
        }
        tw.extents = TraceExtents::new(s, e);
        tw.delta_len_sq = dot(tw.delta, tw.delta);
        tw.delta_len = libm::sqrtf(tw.delta_len_sq);
        tw.radius = if tw.size[2] < tw.size[0] {
            tw.size[2]
        } else {
            tw.size[0]
        };
        tw.bounding_radius = libm::sqrtf(dot(tw.size, tw.size));
        tw.offset_z = tw.size[2] - tw.radius;
        tw.radius_offset = [tw.radius, tw.radius, tw.offset_z + tw.radius];
        for i in 0..3 {
            let (lo, hi) = if s[i] < e[i] { (s[i], e[i]) } else { (e[i], s[i]) };
            let reach = if i == 2 {
                tw.offset_z + tw.radius
            } else {
                tw.radius
            };
            tw.bounds[0][i] = lo - reach;
            tw.bounds[1][i] = hi + reach;
        }
        let volume = (tw.bounds[1][0] - tw.bounds[0][0])
            * (tw.bounds[1][1] - tw.bounds[0][1])
            * (tw.bounds[1][2] - tw.bounds[0][2]);
        tw.axial_cull_only =
            volume < tw.size[0] * tw.size[1] * tw.size[2] * 16.0 * tw.delta_len;
        tw.is_point = tw.size[0] + tw.size[1] + tw.size[2] == 0.0;
        tw
    }

    /// The trace does not move: BO2 runs a position test instead of a sweep.
    #[must_use]
    pub fn is_position_test(&self) -> bool {
        self.extents.start == self.extents.end
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `CM_Trace` over a plain list of brushes (the clip map crate walks the map
/// tree to find them). `trace` carries the best hit so far in and out, as in
/// BO2: a brush only counts if it is hit before `trace.fraction`.
///
/// Like BO2, the surface flags are cleared while tracing and put back if no
/// brush was hit closer than the trace already was.
pub fn trace_brushes<'a, I>(
    brushes: I,
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    contents: u32,
    trace: &mut Trace,
) where
    I: IntoIterator<Item = &'a Brush<'a>>,
{
    let mut tw = TraceWork::new(start, end, mins, maxs, contents);
    let old_sflags = trace.sflags;
    let old_fraction = trace.fraction;
    trace.sflags = u32::MAX;
    let position = tw.is_position_test();
    if position {
        tw.is_point = false;
    }
    for brush in brushes {
        if brush.contents & contents == 0 {
            continue;
        }
        if position {
            test_box_in_brush(&tw, brush, trace);
        } else {
            trace_through_brush(&tw, brush, trace);
        }
        if trace.allsolid {
            break;
        }
    }
    if trace.fraction >= old_fraction {
        trace.sflags = old_sflags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_box_is_a_capsule() {
        // A standing player: 15 wide, 70 tall.
        let tw = TraceWork::new(
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [-15.0, -15.0, 0.0],
            [15.0, 15.0, 70.0],
            1,
        );
        assert_eq!(tw.size, [15.0, 15.0, 35.0]);
        assert_eq!(tw.radius, 15.0);
        assert_eq!(tw.offset_z, 20.0);
        assert_eq!(tw.radius_offset, [15.0, 15.0, 35.0]);
        assert_eq!(tw.extents.start, [0.0, 0.0, 35.0]);
        assert_eq!(tw.extents.inv_delta, [-0.1, 0.0, 0.0]);
        assert_eq!(tw.bounds, [[-15.0, -15.0, 0.0], [25.0, 15.0, 70.0]]);
        assert_eq!(tw.delta_len, 10.0);
        assert!(!tw.is_point);
    }

    #[test]
    fn flat_box_radius_is_its_half_height() {
        let tw = TraceWork::new([0.0; 3], [0.0; 3], [-8.0, -8.0, -2.0], [8.0, 8.0, 2.0], 1);
        assert_eq!(tw.radius, 2.0);
        assert_eq!(tw.offset_z, 0.0);
        assert!(tw.is_position_test());
    }

    #[test]
    fn a_point_trace() {
        let tw = TraceWork::new([0.0; 3], [0.0, 0.0, -100.0], [0.0; 3], [0.0; 3], 1);
        assert!(tw.is_point);
        assert_eq!(tw.radius, 0.0);
        assert_eq!(tw.bounds, [[0.0, 0.0, -100.0], [0.0, 0.0, 0.0]]);
    }
}
