//! The sight check behind a grid corner that asks to be traced: from the
//! sampled point to a spot just in front of the corner.

use crate::pick::{LightGridCell, cell_as_unsigned_f32};

/// What blocks a corner's sight line (solid and a window-style surface).
pub const LIGHT_GRID_SIGHT_CONTENT_MASK: u32 = 0x2001;

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
    cell: LightGridCell,
    corner: u32,
    row_axis: u32,
    col_axis: u32,
) -> Option<[f32; 3]> {
    let (row, col) = (row_axis as usize, col_axis as usize);
    if row > 2 || col > 2 {
        return None;
    }
    let mut pos = [
        cell_to_inches(cell.x, false),
        cell_to_inches(cell.y, false),
        cell_to_inches(cell.z, true),
    ];
    if corner & 4 != 0 {
        pos[row] += 32.0;
    }
    if corner & 2 != 0 {
        pos[col] += 32.0;
    }
    if corner & 1 != 0 {
        pos[2] += 64.0;
    }
    Some(pos)
}

/// The end of the sight line: the corner, moved a hair toward the sample.
fn nudged_toward(sample: [f32; 3], corner: [f32; 3]) -> [f32; 3] {
    let mut dir = [
        sample[0] - corner[0],
        sample[1] - corner[1],
        sample[2] - corner[2],
    ];
    let len_sq = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if len_sq > MIN_DIRECTION_LEN_SQ {
        let inverse = 1.0 / libm::sqrtf(len_sq);
        dir = [dir[0] * inverse, dir[1] * inverse, dir[2] * inverse];
    }
    [
        corner[0] + dir[0] * SIGHT_NUDGE,
        corner[1] + dir[1] * SIGHT_NUDGE,
        corner[2] + dir[2] * SIGHT_NUDGE,
    ]
}

/// Asks `sight_clear(from, to)` whether the sampled point can see `corner` of
/// `cell`. `None` when an axis index is out of range.
pub fn light_grid_isvalid_sample(
    cell: LightGridCell,
    corner: u32,
    row_axis: u32,
    col_axis: u32,
    sample_pos: [f32; 3],
    sight_clear: impl Fn([f32; 3], [f32; 3]) -> bool,
) -> Option<bool> {
    let corner_pos = corner_position(cell, corner, row_axis, col_axis)?;
    Some(sight_clear(
        sample_pos,
        nudged_toward(sample_pos, corner_pos),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ORIGIN_CELL: LightGridCell = LightGridCell {
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
            light_grid_isvalid_sample(ORIGIN_CELL, 0, 0, 1, [8.0, 0.0, 0.0], |from, to| {
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
        let result = light_grid_isvalid_sample(ORIGIN_CELL, 3, 0, 1, [1.0, 1.0, 1.0], |_, _| false);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn a_bad_axis_never_calls_the_check() {
        let result = light_grid_isvalid_sample(ORIGIN_CELL, 0, 5, 1, [0.0; 3], |_, _| {
            panic!("no line to check")
        });
        assert_eq!(result, None);
    }
}
