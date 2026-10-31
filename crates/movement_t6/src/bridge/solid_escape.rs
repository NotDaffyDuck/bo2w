//! Freeing a player whose box has ended up inside solid geometry.

use trace_t6::Trace;
use crate::bridge::{SweepAsk, SolidProbe};

/// Horizontal steps along a single axis: west, south, east, north.
const SIDE_STEPS: [[f32; 2]; 4] = [[-1.0, 0.0], [0.0, -1.0], [1.0, 0.0], [0.0, 1.0]];
/// Horizontal diagonal steps, going round from south-west.
const DIAGONAL_STEPS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
/// Height of each layer, highest first.
const LAYER_HEIGHTS: [f32; 3] = [1.0, 0.0, -1.0];

/// Lay the one-unit escape steps out in the order they are tried. For each
/// layer from top to bottom: straight up or down (the middle layer has no
/// such step), then the four sides. After all three layers, the four
/// diagonals of each layer from top to bottom.
const fn escape_order() -> [[f32; 3]; 26] {
    let mut steps = [[0.0; 3]; 26];
    let mut next = 0;
    let mut layer = 0;
    while layer < LAYER_HEIGHTS.len() {
        let height = LAYER_HEIGHTS[layer];
        if layer != 1 {
            steps[next] = [0.0, 0.0, height];
            next += 1;
        }
        let mut side = 0;
        while side < SIDE_STEPS.len() {
            steps[next] = [SIDE_STEPS[side][0], SIDE_STEPS[side][1], height];
            next += 1;
            side += 1;
        }
        layer += 1;
    }
    layer = 0;
    while layer < LAYER_HEIGHTS.len() {
        let mut corner = 0;
        while corner < DIAGONAL_STEPS.len() {
            let [x, y] = DIAGONAL_STEPS[corner];
            steps[next] = [x, y, LAYER_HEIGHTS[layer]];
            next += 1;
            corner += 1;
        }
        layer += 1;
    }
    steps
}

/// Every one-unit step to a neighbouring cell, in the order an unstick tries
/// them.
pub const UNSTICK_NUDGES: [[f32; 3]; 26] = escape_order();

/// How far below a freed spot to look for the floor to rest on.
const SETTLE_DROP: f32 = 0.25;

/// Where the player ended up, and the downward trace that placed them.
#[derive(Clone, Debug, PartialEq)]
pub struct UnstickVerdict {
    pub origin: [f32; 3],
    pub hit: Trace,
}

/// Take the first escape step whose spot is clear and from which a short
/// drop is clear too, and rest the box where that drop stops. `None` when no
/// step frees the box.
pub fn correct_solid<C: SolidProbe + ?Sized>(
    at_pos: [f32; 3],
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    collide_filter: u32,
    solids: &C,
) -> Option<UnstickVerdict> {
    let sweep = |from_pt: [f32; 3], to_pt: [f32; 3]| {
        solids.trace(SweepAsk {
            start: from_pt,
            end: to_pt,
            lo_corner: box_lo,
            hi_corner: box_hi,
            collide_filter,
        })
    };
    let try_step = |step: &[f32; 3]| -> Option<UnstickVerdict> {
        let [x, y, z] = core::array::from_fn(|axis| at_pos[axis] + step[axis]);
        let spot = [x, y, z];
        if sweep(spot, spot).starts_inside {
            return None;
        }
        let floor_z = z - SETTLE_DROP;
        let drop = sweep(spot, [x, y, floor_z]);
        if drop.starts_inside {
            return None;
        }
        let rest_z = z + (floor_z - z) * drop.fraction;
        Some(UnstickVerdict {
            origin: [x, y, rest_z],
            hit: drop,
        })
    };
    UNSTICK_NUDGES.iter().find_map(try_step)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::cell::Cell;
    use std::vec::Vec;

    /// A world that is solid only at listed box starts, counting its traces.
    struct StuckSpots {
        solid: Vec<[f32; 3]>,
        traces: Cell<usize>,
    }

    impl StuckSpots {
        fn new(solid: Vec<[f32; 3]>) -> Self {
            Self {
                solid,
                traces: Cell::new(0),
            }
        }
    }

    impl SolidProbe for StuckSpots {
        fn trace(&self, input: SweepAsk) -> Trace {
            self.traces.set(self.traces.get() + 1);
            let mut met = Trace::new();
            met.starts_inside = self.solid.contains(&input.start);
            met
        }
    }

    #[test]
    fn steps_run_up_sides_down_then_diagonals() {
        let n = UNSTICK_NUDGES;
        assert_eq!(n[0], [0.0, 0.0, 1.0]);
        assert_eq!(n[1], [-1.0, 0.0, 1.0]);
        assert_eq!(n[4], [0.0, 1.0, 1.0]);
        assert_eq!(n[5], [-1.0, 0.0, 0.0]);
        assert_eq!(n[9], [0.0, 0.0, -1.0]);
        assert_eq!(n[13], [0.0, 1.0, -1.0]);
        assert_eq!(n[14], [-1.0, -1.0, 1.0]);
        assert_eq!(n[19], [1.0, -1.0, 0.0]);
        assert_eq!(n[25], [-1.0, 1.0, -1.0]);
    }

    #[test]
    fn every_neighbour_appears_exactly_once() {
        let mut steps = UNSTICK_NUDGES.to_vec();
        steps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        steps.dedup();
        assert_eq!(steps.len(), 26);
        assert!(!steps.contains(&[0.0; 3]));
    }

    #[test]
    fn first_clear_step_is_taken_and_dropped_onto_the_floor() {
        let world = StuckSpots::new(std::vec![[10.0, 20.0, 31.0]]);
        let freed = correct_solid([10.0, 20.0, 30.0], [-15.0; 3], [15.0; 3], 1, &world).unwrap();
        // Straight up is blocked, so the west step on the top layer wins.
        assert_eq!(freed.origin, [9.0, 20.0, 31.0 - SETTLE_DROP]);
    }

    #[test]
    fn boxed_in_on_all_sides_gives_up_after_one_trace_each() {
        let world = StuckSpots::new(UNSTICK_NUDGES.to_vec());
        assert_eq!(
            correct_solid([0.0; 3], [-1.0; 3], [1.0; 3], 1, &world),
            None
        );
        assert_eq!(world.traces.get(), 26);
    }
}
