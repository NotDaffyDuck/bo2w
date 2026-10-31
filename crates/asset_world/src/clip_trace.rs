//! Box sweeps through a map's collision.

use std::cell::RefCell;

use crate::clip_flags::t5_surface_flags;
use crate::clip_tables::CollisionClipData;

/// Where a swept box ended up.
#[derive(Clone, Copy, Debug)]
pub struct SolidsTraceContact {
    /// How much of the move got through, 0 to 1.
    pub fraction: f32,
    pub normal: [f32; 3],
    pub finish_point: [f32; 3],
    pub began_inside: bool,
    pub fully_embedded: bool,
    pub face_bits: u32,
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
    fn fit_to(&mut self, table: usize, width: usize) {
        if self.table != table || self.seen.len() != width {
            self.table = table;
            self.counter = 0;
            self.seen.clear();
            self.seen.resize(width, 0);
        }
    }
}

std::thread_local! {
    static VISIT_MARKS: RefCell<VisitMarks> = RefCell::default();
}

fn lerp(from: [f32; 3], dest: [f32; 3], share: f32) -> [f32; 3] {
    std::array::from_fn(|axis| from[axis] + (dest[axis] - from[axis]) * share)
}

impl CollisionClipData {
    /// Sweeps a box through the map's world. A collision with no clip map
    /// stops nothing: the whole move gets through.
    fn world_trace(&self, from: [f32; 3], dest: [f32; 3], lo: [f32; 3], hi: [f32; 3], mask: u32) -> trace_t6::Trace {
        let Some(map) = self.t6 else {
            return trace_t6::Trace { fraction: 1.0, ..Default::default() };
        };
        let (table, len) = (map.partitions.as_ptr() as usize, map.partitions.len());
        VISIT_MARKS.with_borrow_mut(|marks| {
            marks.fit_to(table, len);
            let mut check = clipmap_t6::CheckCount { global: marks.counter, partitions: &mut marks.seen };
            let found = map.box_trace(&mut check, from, dest, lo, hi, 0, mask);
            marks.counter = check.global;
            found
        })
    }

    /// Whether a point can see along the line to another: nothing solid
    /// under `mask` between them, and the start not inside something.
    pub fn box_sight_clear(&self, eye: [f32; 3], aim: [f32; 3], solid_mask: u32) -> bool {
        let found = self.world_trace(eye, aim, [0.0; 3], [0.0; 3], solid_mask);
        !found.starts_inside && found.fraction >= 1.0
    }

    /// Moves a box from `start` toward `end` and reports where it stops.
    pub fn sweep_box(&self, from: [f32; 3], dest: [f32; 3], lo: [f32; 3], hi: [f32; 3], mask: u32) -> SolidsTraceContact {
        SolidsTraceContact::along(&self.world_trace(from, dest, lo, hi, mask), from, dest)
    }
}

impl SolidsTraceContact {
    /// The report for `trace` on a move from `from` to `to`.
    fn along(trace: &trace_t6::Trace, from: [f32; 3], dest: [f32; 3]) -> Self {
        let trace_t6::Trace { fraction, normal, starts_inside: startsolid, fully_embedded, sflags, .. } = *trace;
        Self {
            fraction,
            normal,
            finish_point: lerp(from, dest, fraction),
            began_inside: startsolid,
            fully_embedded,
            face_bits: t5_surface_flags(sflags),
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
        let map = CollisionClipData::default();
        let (start, end) = ([0.0; 3], [100.0, 0.0, 50.0]);
        let hit = map.sweep_box(start, end, [-8.0; 3], [8.0; 3], u32::MAX);
        assert_eq!(hit.fraction, 1.0);
        assert_eq!(hit.finish_point, end);
        assert!(!hit.began_inside && !hit.fully_embedded);
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
