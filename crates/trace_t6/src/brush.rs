//! One BO2 brush (`cbrush_t`) and the two things a trace does with it: sweep
//! through it (`CM_TraceThroughBrush`) and test whether it starts inside it
//! (`CM_TestBoxInBrush`).
//!
//! A BO2 brush keeps its six box faces as bounds (`mins`, `maxs`), each with
//! its own flags, and only its sloped faces as planes (`sides`). The sweep
//! tries the box faces first, then the sloped ones.

use crate::capsule::{SURFACE_CLIP_SLACK, TraceWork};
use crate::result::TraceResult;

/// A brush's box faces: low x, y, z, then high x, y, z.
pub const AXIAL_SIDES: usize = 6;

/// BO2's `cplane_s`, less the lookup bytes it caches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub plane_offset: f32,
}

/// BO2's `cbrushside_t`: a sloped face and its flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrushSide {
    pub plane: Plane,
    pub cflags: u32,
    pub sflags: u32,
}

/// BO2's `cbrush_t`. `axial_*flags[0]` are the low faces, `[1]` the high
/// faces, each per axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Brush<'a> {
    pub lower: [f32; 3],
    pub upper: [f32; 3],
    pub contents: u32,
    pub sides: &'a [BrushSide],
    pub axial_cflags: [[u32; 3]; 2],
    pub axial_sflags: [[u32; 3]; 2],
}

/// The face a sweep hit first: its outward normal and its surface flags.
#[derive(Clone, Copy)]
struct Lead {
    looks: [f32; 3],
    sflags: u32,
}

impl Plane {
    /// Signed height of `at` over this face once it is pushed out by the
    /// capsule's size.
    fn height_above(&self, tw: &TraceWork, at: [f32; 3]) -> f32 {
        let pushed_out = (self.normal[2] * tw.offset_z).abs() + self.plane_offset + tw.radius;
        self.normal[0] * at[0] + self.normal[1] * at[1] + self.normal[2] * at[2] - pushed_out
    }
}

/// True when a sweep that starts `d1` out from a face (`d1 > 0`) ends no
/// nearer to it than the clip lip, so it can never touch the brush.
fn never_reaches(d1: f32, d2: f32) -> bool {
    let lip = if d1 > SURFACE_CLIP_SLACK { SURFACE_CLIP_SLACK } else { d1 };
    d2 >= lip
}

/// The part of the sweep that lies inside every face looked at so far.
/// A face that leaves nothing of it makes the sweep miss (`None`).
struct Window {
    enter: f32,
    leave: f32,
    never_out: bool,
    lead: Option<Lead>,
}

impl Window {
    /// A box face. `scale` is the inverse sweep length on the face's axis,
    /// signed by which way the face points.
    fn box_face(&mut self, d1: f32, d2: f32, scale: f32, face: Lead) -> Option<()> {
        if d1 > 0.0 {
            if never_reaches(d1, d2) {
                return None;
            }
            let at = (d1 - SURFACE_CLIP_SLACK) * scale;
            if self.leave <= at {
                return None;
            }
            self.never_out &= !(d2 > 0.0);
            if self.enter < at {
                self.enter = at;
                self.lead = Some(face);
            } else {
                self.lead.get_or_insert(face);
            }
            return Some(());
        }
        if d2 > 0.0 {
            let at = d1 * scale;
            if at <= self.enter {
                return None;
            }
            self.never_out = false;
            if at < self.leave {
                self.leave = at;
            }
        }
        Some(())
    }

    /// A sloped face.
    fn sloped_face(&mut self, d1: f32, d2: f32, face: Lead) -> Option<()> {
        let span = d1 - d2;
        if d1 > 0.0 {
            if never_reaches(d1, d2) {
                return None;
            }
            self.never_out &= !(d2 > 0.0);
            let reach = d1 - SURFACE_CLIP_SLACK;
            if reach <= span * self.enter {
                self.lead.get_or_insert(face);
                return Some(());
            }
            self.enter = reach / span;
            self.lead = Some(face);
            if self.leave <= self.enter {
                return None;
            }
            return Some(());
        }
        if d2 > 0.0 {
            if span * self.leave < d1 {
                self.leave = d1 / span;
                if self.leave <= self.enter {
                    return None;
                }
            }
            self.never_out = false;
        }
        Some(())
    }
}

/// Clip the sweep against every face of `brush`: box faces low then high,
/// x to z, then the sloped ones. `None` when the sweep misses the brush.
fn clip(tw: &TraceWork, brush: &Brush<'_>, limit: f32) -> Option<Window> {
    let start = tw.extents.from;
    let end = tw.extents.to;
    let mut window = Window { enter: 0.0, leave: limit, never_out: true, lead: None };

    for slot in 0..AXIAL_SIDES {
        let (high, axis) = (slot / 3, slot % 3);
        let (bound, sign) = if high == 0 { (brush.lower[axis], -1.0_f32) } else { (brush.upper[axis], 1.0_f32) };
        let pad = tw.radius_offset[axis];
        let d1 = sign * (start[axis] - bound) - pad;
        let d2 = sign * (end[axis] - bound) - pad;
        let mut facing = [0.0_f32; 3];
        facing[axis] = sign;
        let face = Lead { looks: facing, sflags: brush.axial_sflags[high][axis] };
        window.box_face(d1, d2, sign * tw.extents.inv_delta[axis], face)?;
    }

    for side in brush.sides {
        let face = Lead { looks: side.plane.normal, sflags: side.sflags };
        window.sloped_face(side.plane.height_above(tw, start), side.plane.height_above(tw, end), face)?;
    }
    Some(window)
}

/// `CM_TraceThroughBrush`: sweep the capsule through one brush. A hit that
/// is closer than `trace.fraction` replaces it.
///
/// A trace that starts inside the brush gets `startsolid` (and `fully_embedded` if
/// it never gets out), with the flags of the brush's top face.
pub fn trace_through_brush(tw: &TraceWork, brush: &Brush<'_>, trace: &mut TraceResult) {
    let Some(window) = clip(tw, brush, trace.fraction) else {
        return;
    };
    let Some(lead) = window.lead else {
        trace.starts_inside = true;
        if window.never_out {
            trace.fully_embedded = true;
            trace.fraction = 0.0;
        }
        trace.sflags = brush.axial_sflags[1][2];
        trace.cflags = brush.axial_cflags[1][2];
        return;
    };
    trace.fraction = window.enter;
    trace.normal = lead.looks;
    trace.sflags = lead.sflags;
    trace.cflags = brush.contents;
    trace.walkable = false;
}

/// `CM_TestBoxInBrush`: is the capsule, standing still, inside this brush?
/// If so the trace is all solid.
pub fn test_box_in_brush(tw: &TraceWork, brush: &Brush<'_>, trace: &mut TraceResult) {
    let overlaps = (0..3).all(|a| tw.bounds[0][a] < brush.upper[a] && brush.lower[a] < tw.bounds[1][a]);
    let start = tw.extents.from;
    if !overlaps || brush.sides.iter().any(|side| side.plane.height_above(tw, start) > 0.0) {
        return;
    }
    trace.starts_inside = true;
    trace.fully_embedded = true;
    trace.fraction = 0.0;
    trace.sflags = 0;
    trace.cflags = brush.contents;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_brushes;

    const SOLID: u32 = 1;

    fn cube<'a>(lo: [f32; 3], hi: [f32; 3], sides: &'a [BrushSide]) -> Brush<'a> {
        Brush {
            lower: lo,
            upper: hi,
            contents: SOLID,
            sides,
            axial_cflags: [[SOLID; 3]; 2],
            axial_sflags: [[0x10, 0x20, 0x30], [0x40, 0x50, 0x60]],
        }
    }

    fn sweep(brush: &Brush<'_>, from: [f32; 3], till: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> TraceResult {
        let mut found = TraceResult::new();
        trace_brushes([brush], from, till, lo, hi, SOLID, &mut found);
        found
    }

    #[test]
    fn point_hits_a_wall_short_by_the_epsilon() {
        let b = cube([64.0, -32.0, -32.0], [128.0, 32.0, 32.0], &[]);
        let t = sweep(&b, [0.0; 3], [128.0, 0.0, 0.0], [0.0; 3], [0.0; 3]);
        assert!((t.fraction - (64.0 - 0.125) / 128.0).abs() < 1e-6);
        assert_eq!(t.normal, [-1.0, 0.0, 0.0]);
        assert_eq!(t.sflags, 0x10);
        assert_eq!(t.cflags, SOLID);
        assert!(!t.starts_inside);
    }

    #[test]
    fn player_lands_on_a_floor() {
        let b = cube([-256.0, -256.0, -64.0], [256.0, 256.0, 0.0], &[]);
        let t = sweep(&b, [0.0, 0.0, 40.0], [0.0, 0.0, -40.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!((t.finish_location([0.0, 0.0, 40.0], [0.0, 0.0, -40.0])[2] - 0.125).abs() < 1e-4);
        assert_eq!(t.normal, [0.0, 0.0, 1.0]);
        assert_eq!(t.sflags, 0x60);
    }

    #[test]
    fn starting_inside_takes_the_top_face_flags() {
        let b = cube([-64.0; 3], [64.0; 3], &[]);
        let t = sweep(&b, [0.0; 3], [8.0, 0.0, 0.0], [0.0; 3], [0.0; 3]);
        assert!(t.starts_inside && t.fully_embedded);
        assert_eq!(t.fraction, 0.0);
        assert_eq!(t.sflags, 0x60);
    }

    #[test]
    fn a_miss_keeps_the_old_flags() {
        let b = cube([64.0; 3], [128.0; 3], &[]);
        let mut t = TraceResult::new();
        t.sflags = 0x1234;
        trace_brushes([&b], [0.0; 3], [-8.0, 0.0, 0.0], [0.0; 3], [0.0; 3], SOLID, &mut t);
        assert_eq!(t.fraction, 1.0);
        assert_eq!(t.sflags, 0x1234);
    }

    #[test]
    fn position_test_inside_and_outside() {
        let b = cube([-64.0; 3], [64.0; 3], &[]);
        let inside = sweep(&b, [60.0, 0.0, 0.0], [60.0, 0.0, 0.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!(inside.fully_embedded && inside.starts_inside && inside.fraction == 0.0);
        let outside = sweep(&b, [80.0, 0.0, 0.0], [80.0, 0.0, 0.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!(!outside.starts_inside && outside.fraction == 1.0);
    }

    #[test]
    fn a_ramp_side_is_hit_before_the_box() {
        // A 45 degree ramp cut off the top of a box: x + z <= 32.
        let n = core::f32::consts::FRAC_1_SQRT_2;
        let ramp = [BrushSide {
            plane: Plane {
                normal: [n, 0.0, n],
                plane_offset: 32.0 * n,
            },
            cflags: SOLID,
            sflags: 0x77,
        }];
        let b = cube([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0], &ramp);
        let t = sweep(&b, [0.0, 0.0, 128.0], [0.0, 0.0, -128.0], [0.0; 3], [0.0; 3]);
        assert_eq!(t.normal, [n, 0.0, n]);
        assert_eq!(t.sflags, 0x77);
        let z = t.finish_location([0.0, 0.0, 128.0], [0.0, 0.0, -128.0])[2];
        assert!((z - (32.0 + 0.125 / n)).abs() < 1e-3, "{z}");
    }
}
