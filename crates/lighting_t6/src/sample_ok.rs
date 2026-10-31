//! The sight check behind a grid corner that asks to be traced: from the
//! sampled point to a spot just in front of the corner.

use crate::corner_pick::{IlluminationVoxelRef, cell_as_unsigned_f32};

/// What blocks a corner's sight line (solid and a window-style surface).
pub const GRID_LINE_OF_SIGHT_CONTENT_BITS: u32 = 0x2001;

/// How far in front of the corner, toward the sampled point, the line ends.
const SIGHT_NUDGE: f32 = 0.01;
/// Direction lengths squared at or under this are left unnormalised.
const MIN_DIRECTION_LEN_SQ: f32 = 1e-24;
const WORLD_BIAS: f64 = 131072.0;

fn cell_to_inches(cell: i32, up: bool) -> f32 {
    let size = if up { 64.0 } else { 32.0 };
    (cell_as_unsigned_f32(cell) as f64 * size - WORLD_BIAS) as f32
}

/// The world position of one of a cell's eight corners.
fn corner_position(
    cell: IlluminationVoxelRef,
    corner: u32,
    line_direction: u32,
    column_direction: u32,
) -> Option<[f32; 3]> {
    let (row_no, col_no) = (line_direction as usize, column_direction as usize);
    if row_no > 2 || col_no > 2 {
        return None;
    }
    let mut where_at = [
        cell_to_inches(cell.x, false),
        cell_to_inches(cell.y, false),
        cell_to_inches(cell.z, true),
    ];
    if corner & 4 != 0 {
        where_at[row_no] += 32.0;
    }
    if corner & 2 != 0 {
        where_at[col_no] += 32.0;
    }
    if corner & 1 != 0 {
        where_at[2] += 64.0;
    }
    Some(where_at)
}

/// The end of the sight line: the corner, moved a hair toward the sample.
fn nudged_toward(sample: [f32; 3], corner: [f32; 3]) -> [f32; 3] {
    let mut aim = [
        sample[0] - corner[0],
        sample[1] - corner[1],
        sample[2] - corner[2],
    ];
    let magnitude_squared = aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2];
    if magnitude_squared > MIN_DIRECTION_LEN_SQ {
        let inverse = 1.0 / libm::sqrtf(magnitude_squared);
        aim = [aim[0] * inverse, aim[1] * inverse, aim[2] * inverse];
    }
    [
        corner[0] + aim[0] * SIGHT_NUDGE,
        corner[1] + aim[1] * SIGHT_NUDGE,
        corner[2] + aim[2] * SIGHT_NUDGE,
    ]
}

/// Asks `line_of_sight_open(from, to)` whether the sampled point can see `corner` of
/// `cell`. `None` when an axis index is out of range.
pub fn sample_counts(
    cell: IlluminationVoxelRef,
    corner: u32,
    line_direction: u32,
    column_direction: u32,
    probe_at: [f32; 3],
    line_of_sight_open: impl Fn([f32; 3], [f32; 3]) -> bool,
) -> Option<bool> {
    let corner_pos = corner_position(cell, corner, line_direction, column_direction)?;
    Some(line_of_sight_open(
        probe_at,
        nudged_toward(probe_at, corner_pos),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ORIGIN_CELL: IlluminationVoxelRef = IlluminationVoxelRef {
        x: 4096,
        y: 4096,
        z: 2048,
    };

    #[test]
    fn corner_zero_is_the_cell_origin() {
        assert_eq!(corner_position(ORIGIN_CELL, 0, 0, 1), Some([0.0, 0.0, 0.0]));
    }

    #[test]
    fn corner_bits_step_along_the_row_column_and_up() {
        assert_eq!(
            corner_position(ORIGIN_CELL, 4, 0, 1),
            Some([32.0, 0.0, 0.0])
        );
        assert_eq!(
            corner_position(ORIGIN_CELL, 2, 0, 1),
            Some([0.0, 32.0, 0.0])
        );
        assert_eq!(
            corner_position(ORIGIN_CELL, 1, 0, 1),
            Some([0.0, 0.0, 64.0])
        );
        assert_eq!(
            corner_position(ORIGIN_CELL, 7, 0, 1),
            Some([32.0, 32.0, 64.0])
        );
    }

    #[test]
    fn the_row_axis_decides_which_world_axis_the_row_bit_moves() {
        assert_eq!(
            corner_position(ORIGIN_CELL, 4, 1, 0),
            Some([0.0, 32.0, 0.0])
        );
        assert_eq!(
            corner_position(ORIGIN_CELL, 2, 1, 0),
            Some([32.0, 0.0, 0.0])
        );
    }

    #[test]
    fn an_axis_past_the_third_has_no_corner() {
        assert_eq!(corner_position(ORIGIN_CELL, 0, 3, 1), None);
        assert_eq!(corner_position(ORIGIN_CELL, 0, 0, 3), None);
    }

    #[test]
    fn the_target_sits_a_hair_in_front_of_the_corner() {
        let target = nudged_toward([10.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        assert!((target[0] - SIGHT_NUDGE).abs() < 1e-6);
        assert_eq!(target[1], 0.0);
        assert_eq!(target[2], 0.0);
    }

    #[test]
    fn a_sample_on_the_corner_leaves_the_target_on_it() {
        assert_eq!(
            nudged_toward([4.0, 5.0, 6.0], [4.0, 5.0, 6.0]),
            [4.0, 5.0, 6.0]
        );
    }

    #[test]
    fn the_sample_is_the_start_of_the_checked_line() {
        let seen = Cell::new(None);
        let result =
            sample_counts(ORIGIN_CELL, 0, 0, 1, [8.0, 0.0, 0.0], |from, to| {
                seen.set(Some((from, to)));
                true
            });
        assert_eq!(result, Some(true));
        let (from, to) = seen.get().unwrap();
        assert_eq!(from, [8.0, 0.0, 0.0]);
        assert!((to[0] - SIGHT_NUDGE).abs() < 1e-6);
    }

    #[test]
    fn a_blocked_line_reports_false() {
        let result = sample_counts(ORIGIN_CELL, 3, 0, 1, [1.0, 1.0, 1.0], |_, _| false);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn a_bad_axis_never_calls_the_check() {
        let result = sample_counts(ORIGIN_CELL, 0, 5, 1, [0.0; 3], |_, _| {
            panic!("no line to check")
        });
        assert_eq!(result, None);
    }
}
