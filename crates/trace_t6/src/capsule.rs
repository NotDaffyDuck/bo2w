//! BO2's trace setup (`traceWork_t`, `CM_CalcTraceExtents`, `CM_Trace`).
//!
//! A box trace in BO2 is a capsule: the box is moved so its middle is the
//! origin, then swept as a capsule whose radius is the smaller of the box's
//! half width and half height. `offset_z` is how far the capsule's two
//! sphere centres sit above and below its middle.

use crate::brush::{Brush, test_box_in_brush, trace_through_brush};
use crate::result::TraceResult;

/// How far a sweep stops short of a surface.
pub const SURFACE_CLIP_SLACK: f32 = 0.125;

/// BO2's `SweepBoxSize`: the swept segment and one over its length per axis
/// (`1 / (start - end)`, or 0 on an axis it does not move along).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SweepBoxSize {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub inv_delta: [f32; 3],
}

impl SweepBoxSize {
    /// `CM_CalcTraceExtents`.
    #[must_use]
    pub fn new(from: [f32; 3], till: [f32; 3]) -> Self {
        let inv_delta = core::array::from_fn(|axis| {
            let gap = from[axis] - till[axis];
            if gap == 0.0 { 0.0 } else { 1.0 / gap }
        });
        Self { from, to: till, inv_delta }
    }
}

/// BO2's `traceWork_t` (less its thread scratch).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraceWork {
    pub delta: [f32; 3],
    pub middle: [f32; 3],
    pub half_delta: [f32; 3],
    pub half_delta_abs: [f32; 3],
    /// Half the box, per axis.
    pub size: [f32; 3],
    /// What the capsule touches on its way: `bounds[0]` low, `bounds[1]` high.
    pub bounds: [[f32; 3]; 2],
    /// How far the capsule reaches along each axis from its middle.
    pub radius_offset: [f32; 3],
    pub extents: SweepBoxSize,
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
    /// The setup `CM_Trace` does for a box (`lo`..`hi`) moving from `from` to `till`.
    #[must_use]
    pub fn new(from: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3], content_mask: u32) -> Self {
        let mut tw = Self::default();
        tw.contents = content_mask;
        let mut s = [0.0; 3];
        let mut e = [0.0; 3];
        for i in 0..3 {
            let offset = (lo[i] + hi[i]) * 0.5;
            tw.size[i] = hi[i] - offset;
            s[i] = from[i] + offset;
            e[i] = till[i] + offset;
            tw.middle[i] = (s[i] + e[i]) * 0.5;
            tw.delta[i] = e[i] - s[i];
            tw.half_delta[i] = tw.delta[i] * 0.5;
            tw.half_delta_abs[i] = tw.half_delta[i].abs();
        }
        tw.extents = SweepBoxSize::new(s, e);
        tw.delta_len_sq = tw.delta.iter().fold(0.0, |sum, c| sum + c * c);
        tw.delta_len = libm::sqrtf(tw.delta_len_sq);
        tw.radius = if tw.size[2] < tw.size[0] {
            tw.size[2]
        } else {
            tw.size[0]
        };
        tw.bounding_radius = libm::sqrtf(tw.size.iter().fold(0.0, |sum, c| sum + c * c));
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
        self.extents.from == self.extents.to
    }
}

/// `CM_Trace` over a plain list of brushes (the clip map crate walks the map
/// tree to find them). `trace` carries the best hit so far in and out, as in
/// BO2: a brush only counts if it is hit before `best.fraction`.
///
/// Like BO2, the surface flags are cleared while tracing and put back if no
/// brush was hit closer than the trace already was.
pub fn trace_brushes<'a, I>(
    brushes: I,
    from: [f32; 3],
    to: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
    content_mask: u32,
    best: &mut TraceResult,
) where
    I: IntoIterator<Item = &'a Brush<'a>>,
{
    let mut tw = TraceWork::new(from, to, lo, hi, content_mask);
    let standing = tw.is_position_test();
    tw.is_point &= !standing;
    let pass: fn(&TraceWork, &Brush<'_>, &mut TraceResult) = if standing { test_box_in_brush } else { trace_through_brush };

    let (kept_sflags, kept_fraction) = (best.sflags, best.fraction);
    best.sflags = u32::MAX;
    // Stops at the first brush that leaves the trace all solid.
    let _ = brushes.into_iter().filter(|b| b.contents & content_mask != 0).any(|b| {
        pass(&tw, b, best);
        best.fully_embedded
    });
    if best.fraction >= kept_fraction {
        best.sflags = kept_sflags;
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
        assert_eq!(tw.extents.from, [0.0, 0.0, 35.0]);
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
