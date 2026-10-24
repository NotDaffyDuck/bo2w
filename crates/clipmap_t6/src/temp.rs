//! BO2's temporary models: the box `CM_TempBoxModel` sets up, which traces
//! as a capsule against the trace's capsule, and the brush list
//! `CM_TempBrushModel` hands over.

use trace_t6::{Brush, Trace, TraceWork, test_box_in_brush, trace_through_brush};

use crate::{CAPSULE_MODEL_HANDLE, CModel, ClipMap, Leaf, TEMP_BRUSH_MODEL_HANDLE, dot, normalize_to, sub};

/// The trace thread's temporary models: `box_model` with its `box_brush`'s
/// contents, and the brushes of its geom list (BO2 skips the geoms that
/// have none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempModels<'a> {
    pub box_model: CModel,
    pub box_contents: u32,
    pub brushes: &'a [Brush<'a>],
}

impl Default for TempModels<'_> {
    /// As `CM_InitBoxHull` leaves them: a box that every content hits, in
    /// an empty leaf. BO2 gives that leaf a brush node of its own holding
    /// the box brush; the point contents never reach it, as the leaf's box
    /// is empty, so here it has none.
    fn default() -> Self {
        Self {
            box_model: CModel {
                leaf: Leaf {
                    mins: [f32::MAX; 3],
                    maxs: [-f32::MAX; 3],
                    brush_contents: u32::MAX,
                    terrain_contents: 0,
                    ..Leaf::default()
                },
                ..CModel::default()
            },
            box_contents: u32::MAX,
            brushes: &[],
        }
    }
}

impl<'a> ClipMap<'a> {
    /// `CM_TempBoxModel`: the box `mins`..`maxs` with `contents`, as a
    /// model handle a trace can be given.
    pub fn temp_box_model(&mut self, mins: [f32; 3], maxs: [f32; 3], contents: u32) -> u32 {
        self.temp.box_model.mins = mins;
        self.temp.box_model.maxs = maxs;
        self.temp.box_contents = contents;
        CAPSULE_MODEL_HANDLE
    }

    /// `CM_TempBrushModel`: these brushes, as a model handle a trace can be
    /// given.
    pub fn temp_brush_model(&mut self, brushes: &'a [Brush<'a>]) -> u32 {
        self.temp.brushes = brushes;
        TEMP_BRUSH_MODEL_HANDLE
    }
}

/// The temporary box as BO2 traces it: a capsule standing in its middle,
/// as wide as the box's half width or half height, whichever is less.
pub(crate) struct BoxCapsule {
    pub(crate) centre: [f32; 3],
    pub(crate) radius: f32,
    /// From the middle to each end sphere's centre.
    pub(crate) half_height: f32,
}

impl BoxCapsule {
    pub(crate) fn new(m: &CModel) -> Self {
        let centre = [
            (m.mins[0] + m.maxs[0]) * 0.5,
            (m.mins[1] + m.maxs[1]) * 0.5,
            (m.mins[2] + m.maxs[2]) * 0.5,
        ];
        let half_width = m.maxs[0] - centre[0];
        let half_height = m.maxs[2] - centre[2];
        let radius = if half_height < half_width {
            half_height
        } else {
            half_width
        };
        Self {
            centre,
            radius,
            half_height: half_height - radius,
        }
    }

    pub(crate) fn top(&self) -> [f32; 3] {
        [self.centre[0], self.centre[1], self.centre[2] + self.half_height]
    }

    pub(crate) fn bottom(&self) -> [f32; 3] {
        [self.centre[0], self.centre[1], self.centre[2] - self.half_height]
    }
}

/// `CM_TestCapsuleInCapsule`: is the trace's capsule, standing still,
/// inside the box's capsule? If so the trace is all solid.
pub(crate) fn test_capsule_in_capsule(tw: &TraceWork, temp: &TempModels<'_>, trace: &mut Trace) {
    let s = tw.extents.start;
    let b = BoxCapsule::new(&temp.box_model);
    let top = tw.offset_z + s[2];
    let bottom = s[2] - tw.offset_z;
    let reach = tw.radius + b.radius;
    let reach_sq = reach * reach;
    let dx = b.centre[0] - s[0];
    let dy = b.centre[1] - s[1];
    let apart = |dz: f32| reach_sq <= dy * dy + dx * dx + dz * dz;
    let (box_top, box_bottom) = (b.top()[2], b.bottom()[2]);
    if apart(box_top - top) && apart(box_top - bottom) && apart(box_bottom - top) && apart(box_bottom - bottom) {
        // No end spheres touch: the capsules meet only if their sides do.
        let total_half_height = tw.size[2] + b.half_height - tw.radius;
        if total_half_height < (s[2] - b.centre[2]).abs() {
            return;
        }
        if reach_sq <= dy * dy + dx * dx {
            return;
        }
    }
    trace.allsolid = true;
    trace.startsolid = true;
    trace.fraction = 0.0;
    trace.sflags = 0;
}

/// `CM_TraceCapsuleThroughCapsule`: sweep the trace's capsule against the
/// box's: the end spheres that can meet first, then the sides, then the
/// end spheres the sides let past.
pub(crate) fn trace_capsule_through_capsule(tw: &TraceWork, temp: &TempModels<'_>, trace: &mut Trace) {
    let m = &temp.box_model;
    for i in 0..3 {
        if m.maxs[i] + 1.0 < tw.bounds[0][i] {
            return;
        }
    }
    for i in 0..3 {
        if tw.bounds[1][i] < m.mins[i] - 1.0 {
            return;
        }
    }
    let s = tw.extents.start;
    let e = tw.extents.end;
    let start_top = [s[0], s[1], tw.offset_z + s[2]];
    let start_bottom = [s[0], s[1], s[2] - tw.offset_z];
    let end_top = [e[0], e[1], tw.offset_z + e[2]];
    let end_bottom = [e[0], e[1], e[2] - tw.offset_z];
    let b = BoxCapsule::new(m);
    let (box_top, box_bottom) = (b.top(), b.bottom());
    let contents = temp.box_contents;
    // The trace's top sphere against the box's bottom one, and its bottom
    // sphere against the box's top one.
    let from_below = |trace: &mut Trace| sphere_through_sphere(tw, contents, start_top, end_top, box_bottom, b.radius, trace);
    let from_above =
        |trace: &mut Trace| sphere_through_sphere(tw, contents, start_bottom, end_bottom, box_top, b.radius, trace);

    let sides = if start_bottom[2] <= box_top[2] {
        if box_bottom[2] <= start_top[2] {
            true
        } else {
            if from_below(trace) {
                return;
            }
            0.0 < tw.delta[2]
        }
    } else {
        if from_above(trace) {
            return;
        }
        tw.delta[2] < 0.0
    };
    if !sides || cylinder_through_cylinder(tw, contents, &b, trace) {
        return;
    }
    if end_bottom[2] <= box_top[2] {
        if box_bottom[2] <= end_top[2] || start_top[2] < box_bottom[2] {
            return;
        }
        from_below(trace);
    } else {
        if box_top[2] < start_bottom[2] {
            return;
        }
        from_above(trace);
    }
}

/// `CM_TraceSphereThroughSphere`: one of the trace's end spheres, moving
/// from `from` to `to`, against a sphere of `radius` at `centre`. True if
/// it hit (or started inside).
fn sphere_through_sphere(
    tw: &TraceWork,
    contents: u32,
    from: [f32; 3],
    to: [f32; 3],
    centre: [f32; 3],
    radius: f32,
    trace: &mut Trace,
) -> bool {
    let d = sub(from, centre);
    let reach = tw.radius + radius;
    let reach_sq = reach * reach;
    let d_sq = dot(d, d);
    let c = d_sq - reach_sq;
    if c <= 0.0 {
        trace.fraction = 0.0;
        trace.startsolid = true;
        trace.walkable = false;
        trace.normal = normalize_to(d).0;
        trace.cflags = contents;
        trace.sflags = 0;
        let q = sub(to, centre);
        if dot(q, q) <= reach_sq {
            trace.allsolid = true;
        }
        return true;
    }
    let b = dot(tw.delta, d);
    if b < 0.0 {
        let a = tw.delta_len_sq;
        let disc = b * b - a * c;
        if 0.0 <= disc {
            let (normal, len) = normalize_to(d);
            let frac = (-b - libm::sqrtf(disc)) / a + (len * 0.125) / b;
            if frac < trace.fraction {
                trace.fraction = if frac < 0.0 { 0.0 } else { frac };
                trace.normal = normal;
                trace.cflags = contents;
                trace.walkable = false;
                trace.sflags = 0;
                return true;
            }
        }
    }
    false
}

/// `CM_TraceCylinderThroughCylinder`: the sides of the trace's capsule
/// against the sides of the box's. True if it hit (or started inside).
fn cylinder_through_cylinder(tw: &TraceWork, contents: u32, b: &BoxCapsule, trace: &mut Trace) -> bool {
    let s = tw.extents.start;
    let dx = s[0] - b.centre[0];
    let dy = s[1] - b.centre[1];
    let d_sq = dx * dx + dy * dy;
    let reach = tw.radius + b.radius;
    let c = d_sq - reach * reach;
    let total_height = (tw.size[2] - tw.radius) + b.half_height;
    if 0.0 < c {
        let bb = tw.delta[0] * dx + tw.delta[1] * dy;
        if bb < 0.0 {
            let a = tw.delta[1] * tw.delta[1] + tw.delta[0] * tw.delta[0];
            let disc = bb * bb - a * c;
            if 0.0 <= disc {
                let (normal, len) = normalize_to([dx, dy, 0.0]);
                let epsilon = (len * 0.125) / bb;
                let frac = (-bb - libm::sqrtf(disc)) / a + epsilon;
                if frac < trace.fraction && ((frac - epsilon) * tw.delta[2] + s[2] - b.centre[2]).abs() <= total_height {
                    trace.fraction = if frac < 0.0 { 0.0 } else { frac };
                    trace.normal = normal;
                    trace.cflags = contents;
                    trace.sflags = 0;
                    trace.walkable = false;
                    return true;
                }
            }
        }
    } else if (s[2] - b.centre[2]).abs() <= total_height {
        trace.fraction = 0.0;
        trace.startsolid = true;
        trace.walkable = false;
        trace.normal = normalize_to([dx, dy, 0.0]).0;
        trace.cflags = contents;
        trace.sflags = 0;
        if (tw.extents.end[2] - b.centre[2]).abs() <= total_height {
            trace.allsolid = true;
        }
        return true;
    }
    false
}

/// `CM_TestInTempBrush`: the standing capsule against each brush of the
/// list it could hit, until one holds it.
pub(crate) fn test_in_temp_brush(tw: &TraceWork, temp: &TempModels<'_>, trace: &mut Trace) {
    for brush in temp.brushes {
        if brush.contents & tw.contents != 0 {
            test_box_in_brush(tw, brush, trace);
            if trace.allsolid {
                return;
            }
        }
    }
}

/// `CM_TraceThroughTempBrush`: the sweep through each brush of the list it
/// could hit, until one stops it at once.
pub(crate) fn trace_through_temp_brush(tw: &TraceWork, temp: &TempModels<'_>, trace: &mut Trace) {
    if trace.fraction == 0.0 {
        return;
    }
    for brush in temp.brushes {
        if brush.contents & tw.contents != 0 {
            trace_through_brush(tw, brush, trace);
            if trace.fraction == 0.0 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckCount, point_contents};

    const SOLID: u32 = 1;
    /// A player's box: 15 wide each way, 72 tall.
    const MINS: [f32; 3] = [-15.0, -15.0, 0.0];
    const MAXS: [f32; 3] = [15.0, 15.0, 72.0];

    /// The box handed to the traces: 16 wide each way, 72 tall, so its
    /// capsule has radius 16 and end spheres 20 above and below its middle.
    fn boxed() -> (ClipMap<'static>, u32) {
        let mut map = ClipMap::default();
        let handle = map.temp_box_model([-16.0, -16.0, 0.0], [16.0, 16.0, 72.0], SOLID);
        (map, handle)
    }

    fn sweep(map: &ClipMap<'_>, start: [f32; 3], end: [f32; 3], model: u32, contents: u32) -> Trace {
        let mut check = CheckCount::new(&mut []);
        map.box_trace(&mut check, start, end, MINS, MAXS, model, contents)
    }

    fn cube(mins: [f32; 3], maxs: [f32; 3], contents: u32) -> Brush<'static> {
        Brush {
            mins,
            maxs,
            contents,
            sides: &[],
            axial_cflags: [[contents; 3]; 2],
            axial_sflags: [[0; 3]; 2],
        }
    }

    #[test]
    fn the_temp_handles_name_the_box_model() {
        let (map, handle) = boxed();
        assert_eq!(handle, CAPSULE_MODEL_HANDLE);
        let m = map.brush_model(CAPSULE_MODEL_HANDLE).unwrap();
        assert_eq!((m.mins, m.maxs), ([-16.0, -16.0, 0.0], [16.0, 16.0, 72.0]));
        assert_eq!(map.brush_model(TEMP_BRUSH_MODEL_HANDLE), Some(m));
        assert_eq!(map.brush_model(1), None);
        // The box model's leaf stays empty, so no point is inside it.
        assert_eq!(point_contents(&map, [0.0, 0.0, 36.0], CAPSULE_MODEL_HANDLE), 0);
        let fresh = TempModels::default();
        assert_eq!((fresh.box_contents, fresh.box_model.leaf.brush_contents), (u32::MAX, u32::MAX));
    }

    #[test]
    fn a_capsule_standing_in_the_temp_box_is_solid() {
        let (map, handle) = boxed();
        let t = sweep(&map, [4.0, 0.0, 0.0], [4.0, 0.0, 0.0], handle, SOLID);
        assert!(t.allsolid && t.startsolid);
        assert_eq!((t.fraction, t.sflags), (0.0, 0));
    }

    #[test]
    fn a_capsule_beside_the_temp_box_is_clear() {
        let (map, handle) = boxed();
        // 31 apart is touching: just past it the sides miss.
        let t = sweep(&map, [31.5, 0.0, 0.0], [31.5, 0.0, 0.0], handle, SOLID);
        assert!(!t.startsolid && t.fraction == 1.0);
        let t = sweep(&map, [30.5, 0.0, 0.0], [30.5, 0.0, 0.0], handle, SOLID);
        assert!(t.allsolid);
        // Well above it, the end spheres and the sides both miss.
        let t = sweep(&map, [0.0, 0.0, 200.0], [0.0, 0.0, 200.0], handle, SOLID);
        assert!(!t.startsolid);
    }

    #[test]
    fn the_temp_box_only_stops_its_own_contents() {
        let (map, handle) = boxed();
        let t = sweep(&map, [0.0; 3], [0.0; 3], handle, 2);
        assert!(!t.startsolid);
        let t = sweep(&map, [-100.0, 0.0, 0.0], [100.0, 0.0, 0.0], handle, 2);
        assert_eq!(t.fraction, 1.0);
    }

    #[test]
    fn walking_into_the_temp_box_stops_at_its_side() {
        let (map, handle) = boxed();
        let t = sweep(&map, [-100.0, 0.0, 0.0], [100.0, 0.0, 0.0], handle, SOLID);
        // Touching at 31 apart, an eighth short: x = -31.125.
        assert!((t.fraction - 0.344_375).abs() < 1e-5, "{}", t.fraction);
        assert_eq!((t.normal, t.cflags, t.sflags), ([-1.0, 0.0, 0.0], SOLID, 0));
        assert!(!t.walkable && !t.startsolid);
    }

    #[test]
    fn walking_past_the_temp_box_misses_it() {
        let (map, handle) = boxed();
        let t = sweep(&map, [-100.0, 40.0, 0.0], [100.0, 40.0, 0.0], handle, SOLID);
        assert_eq!(t.fraction, 1.0);
    }

    #[test]
    fn dropping_onto_the_temp_box_lands_on_its_top_sphere() {
        let (map, handle) = boxed();
        let t = sweep(&map, [0.0, 0.0, 200.0], [0.0, 0.0, 0.0], handle, SOLID);
        // The bottom sphere (z 215 down to 15) meets the box's top one
        // (z 56) at 31 apart, an eighth short.
        assert!((t.fraction - 0.639_375).abs() < 1e-5, "{}", t.fraction);
        assert!((t.normal[2] - 1.0).abs() < 1e-6 && t.normal[0] == 0.0);
        assert!(t.walkable);
    }

    #[test]
    fn rising_into_the_temp_box_hits_its_bottom_sphere() {
        let mut map = ClipMap::default();
        let handle = map.temp_box_model([-16.0, -16.0, 200.0], [16.0, 16.0, 272.0], SOLID);
        let t = sweep(&map, [0.0; 3], [0.0, 0.0, 200.0], handle, SOLID);
        assert!(t.fraction < 1.0 && !t.startsolid);
        assert!((t.normal[2] + 1.0).abs() < 1e-6 && !t.walkable);
    }

    #[test]
    fn a_temp_brush_list_stops_traces_of_its_contents() {
        let brushes = [cube([-64.0, -64.0, 0.0], [-48.0, 64.0, 64.0], 2), cube([-16.0, -16.0, 0.0], [16.0, 16.0, 64.0], SOLID)];
        let mut map = ClipMap::default();
        let handle = map.temp_brush_model(&brushes);
        assert_eq!(handle, TEMP_BRUSH_MODEL_HANDLE);
        let mut check = CheckCount::new(&mut []);
        let t = map.box_trace(&mut check, [-100.0, 0.0, 32.0], [100.0, 0.0, 32.0], [0.0; 3], [0.0; 3], handle, SOLID);
        assert!(t.fraction < 0.5 && 0.4 < t.fraction, "{}", t.fraction);
        assert_eq!((t.normal, t.cflags), ([-1.0, 0.0, 0.0], SOLID));
        let t = map.box_trace(&mut check, [0.0, 0.0, 32.0], [0.0, 0.0, 32.0], [0.0; 3], [0.0; 3], handle, SOLID);
        assert!(t.allsolid && t.startsolid);
        let t = map.box_trace(&mut check, [-56.0, 0.0, 32.0], [-56.0, 0.0, 32.0], [0.0; 3], [0.0; 3], handle, SOLID);
        assert!(!t.startsolid);
    }
}
