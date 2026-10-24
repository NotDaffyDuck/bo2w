//! One BO2 brush (`cbrush_t`) and the two things a trace does with it: sweep
//! through it (`CM_TraceThroughBrush`) and test whether it starts inside it
//! (`CM_TestBoxInBrush`).
//!
//! A BO2 brush keeps its six box faces as bounds (`mins`, `maxs`), each with
//! its own flags, and only its sloped faces as planes (`sides`). The sweep
//! tries the box faces first, then the sloped ones.

use crate::{SURFACE_CLIP_EPSILON, Trace, TraceWork};

/// A brush's box faces: low x, y, z, then high x, y, z.
pub const AXIAL_SIDES: usize = 6;

/// BO2's `cplane_s`, less the lookup bytes it caches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub dist: f32,
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
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub contents: u32,
    pub sides: &'a [BrushSide],
    pub axial_cflags: [[u32; 3]; 2],
    pub axial_sflags: [[u32; 3]; 2],
}

/// The face a sweep hit first.
#[derive(Clone, Copy)]
struct Lead {
    normal: [f32; 3],
    sflags: u32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How far a sloped face sits out from its plane for this capsule.
fn side_dist(tw: &TraceWork, plane: &Plane) -> f32 {
    (plane.normal[2] * tw.offset_z).abs() + plane.dist + tw.radius
}

/// `CM_TraceThroughBrush`: sweep the capsule through one brush. A hit that
/// is closer than `trace.fraction` replaces it.
///
/// A trace that starts inside the brush gets `startsolid` (and `allsolid` if
/// it never gets out), with the flags of the brush's top face.
pub fn trace_through_brush(tw: &TraceWork, brush: &Brush<'_>, trace: &mut Trace) {
    let start = tw.extents.start;
    let end = tw.extents.end;
    let mut enter_frac = 0.0_f32;
    let mut leave_frac = trace.fraction;
    let mut all_inside = true;
    let mut lead: Option<Lead> = None;

    for (face, (bound, sign)) in [(brush.mins, -1.0_f32), (brush.maxs, 1.0_f32)]
        .into_iter()
        .enumerate()
    {
        for axis in 0..3 {
            let d1 = (start[axis] - bound[axis]) * sign - tw.radius_offset[axis];
            let d2 = (end[axis] - bound[axis]) * sign - tw.radius_offset[axis];
            if d1 <= 0.0 {
                if d2 > 0.0 {
                    let f = tw.extents.inv_delta[axis] * d1 * sign;
                    if f <= enter_frac {
                        return;
                    }
                    all_inside = false;
                    if f < leave_frac {
                        leave_frac = f;
                    }
                }
                continue;
            }
            let front = if d1 > SURFACE_CLIP_EPSILON {
                SURFACE_CLIP_EPSILON
            } else {
                d1
            };
            if front <= d2 {
                return;
            }
            let f = (d1 - SURFACE_CLIP_EPSILON) * tw.extents.inv_delta[axis] * sign;
            if leave_frac <= f {
                return;
            }
            if d2 > 0.0 {
                all_inside = false;
            }
            if enter_frac < f || lead.is_none() {
                if enter_frac < f {
                    enter_frac = f;
                }
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                lead = Some(Lead {
                    normal,
                    sflags: brush.axial_sflags[face][axis],
                });
            }
        }
    }

    for side in brush.sides {
        let dist = side_dist(tw, &side.plane);
        let d1 = dot(side.plane.normal, start) - dist;
        let d2 = dot(side.plane.normal, end) - dist;
        if d1 <= 0.0 {
            if d2 > 0.0 {
                let delta = d1 - d2;
                if delta * leave_frac < d1 {
                    leave_frac = d1 / delta;
                    if leave_frac <= enter_frac {
                        return;
                    }
                }
                all_inside = false;
            }
            continue;
        }
        let front = if d1 > SURFACE_CLIP_EPSILON {
            SURFACE_CLIP_EPSILON
        } else {
            d1
        };
        if front <= d2 {
            return;
        }
        if d2 > 0.0 {
            all_inside = false;
        }
        let delta = d1 - d2;
        if d1 - SURFACE_CLIP_EPSILON <= delta * enter_frac {
            if lead.is_none() {
                lead = Some(Lead {
                    normal: side.plane.normal,
                    sflags: side.sflags,
                });
            }
        } else {
            enter_frac = (d1 - SURFACE_CLIP_EPSILON) / delta;
            lead = Some(Lead {
                normal: side.plane.normal,
                sflags: side.sflags,
            });
            if leave_frac <= enter_frac {
                return;
            }
        }
    }

    trace.cflags = brush.contents;
    match lead {
        None => {
            trace.startsolid = true;
            if all_inside {
                trace.allsolid = true;
                trace.fraction = 0.0;
            }
            trace.sflags = brush.axial_sflags[1][2];
            trace.cflags = brush.axial_cflags[1][2];
        }
        Some(lead) => {
            trace.fraction = enter_frac;
            trace.normal = lead.normal;
            trace.sflags = lead.sflags;
            trace.walkable = false;
        }
    }
}

/// `CM_TestBoxInBrush`: is the capsule, standing still, inside this brush?
/// If so the trace is all solid.
pub fn test_box_in_brush(tw: &TraceWork, brush: &Brush<'_>, trace: &mut Trace) {
    for axis in 0..3 {
        if !(tw.bounds[0][axis] < brush.maxs[axis] && brush.mins[axis] < tw.bounds[1][axis]) {
            return;
        }
    }
    let start = tw.extents.start;
    for side in brush.sides {
        if dot(side.plane.normal, start) - side_dist(tw, &side.plane) > 0.0 {
            return;
        }
    }
    trace.startsolid = true;
    trace.allsolid = true;
    trace.fraction = 0.0;
    trace.sflags = 0;
    trace.cflags = brush.contents;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_brushes;

    const SOLID: u32 = 1;

    fn cube<'a>(mins: [f32; 3], maxs: [f32; 3], sides: &'a [BrushSide]) -> Brush<'a> {
        Brush {
            mins,
            maxs,
            contents: SOLID,
            sides,
            axial_cflags: [[SOLID; 3]; 2],
            axial_sflags: [[0x10, 0x20, 0x30], [0x40, 0x50, 0x60]],
        }
    }

    fn sweep(brush: &Brush<'_>, start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) -> Trace {
        let mut t = Trace::new();
        trace_brushes([brush], start, end, mins, maxs, SOLID, &mut t);
        t
    }

    #[test]
    fn point_hits_a_wall_short_by_the_epsilon() {
        let b = cube([64.0, -32.0, -32.0], [128.0, 32.0, 32.0], &[]);
        let t = sweep(&b, [0.0; 3], [128.0, 0.0, 0.0], [0.0; 3], [0.0; 3]);
        assert!((t.fraction - (64.0 - 0.125) / 128.0).abs() < 1e-6);
        assert_eq!(t.normal, [-1.0, 0.0, 0.0]);
        assert_eq!(t.sflags, 0x10);
        assert_eq!(t.cflags, SOLID);
        assert!(!t.startsolid);
    }

    #[test]
    fn player_lands_on_a_floor() {
        let b = cube([-256.0, -256.0, -64.0], [256.0, 256.0, 0.0], &[]);
        let t = sweep(&b, [0.0, 0.0, 40.0], [0.0, 0.0, -40.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!((t.end_pos([0.0, 0.0, 40.0], [0.0, 0.0, -40.0])[2] - 0.125).abs() < 1e-4);
        assert_eq!(t.normal, [0.0, 0.0, 1.0]);
        assert_eq!(t.sflags, 0x60);
    }

    #[test]
    fn starting_inside_takes_the_top_face_flags() {
        let b = cube([-64.0; 3], [64.0; 3], &[]);
        let t = sweep(&b, [0.0; 3], [8.0, 0.0, 0.0], [0.0; 3], [0.0; 3]);
        assert!(t.startsolid && t.allsolid);
        assert_eq!(t.fraction, 0.0);
        assert_eq!(t.sflags, 0x60);
    }

    #[test]
    fn a_miss_keeps_the_old_flags() {
        let b = cube([64.0; 3], [128.0; 3], &[]);
        let mut t = Trace::new();
        t.sflags = 0x1234;
        trace_brushes([&b], [0.0; 3], [-8.0, 0.0, 0.0], [0.0; 3], [0.0; 3], SOLID, &mut t);
        assert_eq!(t.fraction, 1.0);
        assert_eq!(t.sflags, 0x1234);
    }

    #[test]
    fn position_test_inside_and_outside() {
        let b = cube([-64.0; 3], [64.0; 3], &[]);
        let inside = sweep(&b, [60.0, 0.0, 0.0], [60.0, 0.0, 0.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!(inside.allsolid && inside.startsolid && inside.fraction == 0.0);
        let outside = sweep(&b, [80.0, 0.0, 0.0], [80.0, 0.0, 0.0], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
        assert!(!outside.startsolid && outside.fraction == 1.0);
    }

    #[test]
    fn a_ramp_side_is_hit_before_the_box() {
        // A 45 degree ramp cut off the top of a box: x + z <= 32.
        let n = core::f32::consts::FRAC_1_SQRT_2;
        let ramp = [BrushSide {
            plane: Plane {
                normal: [n, 0.0, n],
                dist: 32.0 * n,
            },
            cflags: SOLID,
            sflags: 0x77,
        }];
        let b = cube([-64.0, -64.0, -64.0], [64.0, 64.0, 64.0], &ramp);
        let t = sweep(&b, [0.0, 0.0, 128.0], [0.0, 0.0, -128.0], [0.0; 3], [0.0; 3]);
        assert_eq!(t.normal, [n, 0.0, n]);
        assert_eq!(t.sflags, 0x77);
        let z = t.end_pos([0.0, 0.0, 128.0], [0.0, 0.0, -128.0])[2];
        assert!((z - (32.0 + 0.125 / n)).abs() < 1e-3, "{z}");
    }
}
