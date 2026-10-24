//! Nudging a player out of a wall they ended up inside.

use trace_t6::Trace;

use crate::bridge::{CollisionBackend, GroundTraceInput};

/// The 26 directions to try, one unit each: every non-zero step on the three
/// axes, in the order the engine tries them (up first, then the sides, then
/// down, then the corners).
pub const BG_CORRECT_SOLID_DELTAS: [[f32; 3]; 26] = [
    [0.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [0.0, -1.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
    [-1.0, 0.0, 0.0],
    [0.0, -1.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 0.0, -1.0],
    [0.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0],
    [-1.0, -1.0, 0.0],
    [1.0, -1.0, 0.0],
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [-1.0, -1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 1.0, -1.0],
];

/// How far the settling trace drops from a freed spot to find the floor.
const SETTLE_DROP: f32 = 0.25;

/// Where the player ended up, and the downward trace that placed them.
#[derive(Clone, Debug, PartialEq)]
pub struct CorrectSolidOutcome {
    pub origin: [f32; 3],
    pub trace: Trace,
}

/// Try each one-unit nudge in turn and take the first that leaves the box out
/// of solid, then let it settle onto the floor a quarter unit below. `None`
/// when every nudge is still stuck.
pub fn correct_solid<C: CollisionBackend + ?Sized>(
    origin: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
    tracemask: u32,
    collision: &C,
) -> Option<CorrectSolidOutcome> {
    BG_CORRECT_SOLID_DELTAS.iter().find_map(|delta| {
        let point = [
            origin[0] + delta[0],
            origin[1] + delta[1],
            origin[2] + delta[2],
        ];
        let sweep = |start, end| GroundTraceInput {
            start,
            end,
            mins,
            maxs,
            tracemask,
        };

        if collision.trace(sweep(point, point)).startsolid {
            return None;
        }
        let settled_z = point[2] - SETTLE_DROP;
        let down = collision.trace(sweep(point, [point[0], point[1], settled_z]));
        if down.startsolid {
            return None;
        }
        Some(CorrectSolidOutcome {
            origin: [
                point[0],
                point[1],
                point[2] + (settled_z - point[2]) * down.fraction,
            ],
            trace: down,
        })
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::cell::RefCell;
    use std::vec;
    use std::vec::Vec;

    /// Solid wherever the box start sits inside `solid_below` or the origin's
    /// own spot; reports how many traces were asked.
    struct Pocket {
        stuck_at: Vec<[f32; 3]>,
        asked: RefCell<usize>,
    }

    impl CollisionBackend for Pocket {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            *self.asked.borrow_mut() += 1;
            Trace {
                startsolid: self.stuck_at.contains(&input.start),
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    #[test]
    fn the_first_free_nudge_wins_and_settles_a_quarter_unit_down() {
        let origin = [10.0, 20.0, 30.0];
        let world = Pocket {
            stuck_at: vec![[10.0, 20.0, 31.0]],
            asked: RefCell::new(0),
        };
        let outcome = correct_solid(origin, [-15.0; 3], [15.0; 3], 1, &world).unwrap();
        // Up is stuck, so the first free spot is the (-1, 0, +1) corner.
        assert_eq!(outcome.origin, [9.0, 20.0, 31.0 - SETTLE_DROP]);
    }

    #[test]
    fn a_player_stuck_in_every_direction_is_left_alone() {
        let origin = [0.0; 3];
        let stuck_at = BG_CORRECT_SOLID_DELTAS.iter().copied().collect();
        let world = Pocket {
            stuck_at,
            asked: RefCell::new(0),
        };
        assert_eq!(correct_solid(origin, [-1.0; 3], [1.0; 3], 1, &world), None);
        assert_eq!(*world.asked.borrow(), 26);
    }

    #[test]
    fn the_nudge_table_has_every_direction_once() {
        let mut seen = BG_CORRECT_SOLID_DELTAS.to_vec();
        seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
        seen.dedup();
        assert_eq!(seen.len(), 26);
        assert!(!seen.contains(&[0.0; 3]));
    }
}
