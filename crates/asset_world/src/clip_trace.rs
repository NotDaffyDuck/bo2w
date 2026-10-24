//! Box sweeps through a map's collision.

use std::cell::RefCell;

use crate::clip_flags::t5_surface_flags;
use crate::clip_tables::ClipCollision;

/// Where a swept box ended up.
#[derive(Clone, Copy, Debug)]
pub struct ClipSweepHit {
    /// How much of the move got through, 0 to 1.
    pub fraction: f32,
    pub normal: [f32; 3],
    pub endpos: [f32; 3],
    pub startsolid: bool,
    pub allsolid: bool,
    pub surface_flags: u32,
}

/// The visit marks a clip map's trace keeps so it looks at a partition only
/// once per trace. They outlive a trace and are shared by every trace on the
/// thread that goes through the same map.
#[derive(Default)]
struct VisitMarks {
    /// The address of the partition table the marks were sized for.
    table: usize,
    counter: u16,
    seen: Vec<u16>,
}

impl VisitMarks {
    fn fit_to(&mut self, table: usize, len: usize) {
        if self.table != table || self.seen.len() != len {
            self.table = table;
            self.counter = 0;
            self.seen.clear();
            self.seen.resize(len, 0);
        }
    }
}

thread_local! {
    static VISIT_MARKS: RefCell<VisitMarks> = RefCell::default();
}

fn lerp(from: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * t)
}

impl ClipCollision {
    /// Sweeps a box through the map's world. A collision with no clip map
    /// stops nothing: the whole move gets through.
    fn trace_world(
        &self,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> trace_t6::Trace {
        let Some(map) = self.t6 else {
            return trace_t6::Trace {
                fraction: 1.0,
                ..trace_t6::Trace::default()
            };
        };
        VISIT_MARKS.with_borrow_mut(|marks| {
            marks.fit_to(map.partitions.as_ptr() as usize, map.partitions.len());
            let mut check = clipmap_t6::CheckCount {
                global: marks.counter,
                partitions: &mut marks.seen,
            };
            let trace = map.box_trace(&mut check, start, end, mins, maxs, 0, mask);
            let counter = check.global;
            marks.counter = counter;
            trace
        })
    }

    /// Whether a point can see along the line to another: nothing solid
    /// under `mask` between them, and the start not inside something.
    pub fn box_sight_clear(&self, start: [f32; 3], end: [f32; 3], mask: u32) -> bool {
        let trace = self.trace_world(start, end, [0.0; 3], [0.0; 3], mask);
        trace.fraction >= 1.0 && !trace.startsolid
    }

    /// Moves a box from `start` toward `end` and reports where it stops.
    pub fn sweep_box(
        &self,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    ) -> ClipSweepHit {
        let trace = self.trace_world(start, end, mins, maxs, mask);
        ClipSweepHit {
            fraction: trace.fraction,
            normal: trace.normal,
            endpos: lerp(start, end, trace.fraction),
            startsolid: trace.startsolid,
            allsolid: trace.allsolid,
            surface_flags: t5_surface_flags(trace.sflags),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_runs_from_one_end_to_the_other() {
        let (a, b) = ([0.0, 10.0, -4.0], [4.0, 10.0, 4.0]);
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 1.0), b);
        assert_eq!(lerp(a, b, 0.5), [2.0, 10.0, 0.0]);
    }

    #[test]
    fn a_map_with_no_clip_map_lets_every_move_through() {
        let map = ClipCollision::default();
        let (start, end) = ([0.0; 3], [100.0, 0.0, 50.0]);
        let hit = map.sweep_box(start, end, [-8.0; 3], [8.0; 3], u32::MAX);
        assert_eq!(hit.fraction, 1.0);
        assert_eq!(hit.endpos, end);
        assert!(!hit.startsolid && !hit.allsolid);
        assert!(map.box_sight_clear(start, end, u32::MAX));
    }

    #[test]
    fn marks_are_resized_only_when_the_table_changes() {
        let (first, second) = (0x1000, 0x2000);
        let mut marks = VisitMarks::default();

        marks.fit_to(first, 4);
        assert_eq!(marks.seen.len(), 4);
        marks.counter = 9;
        marks.seen[2] = 9;

        marks.fit_to(first, 4);
        assert_eq!((marks.counter, marks.seen[2]), (9, 9));

        marks.fit_to(second, 6);
        assert_eq!((marks.counter, marks.seen.len(), marks.seen[2]), (0, 6, 0));
    }
}
