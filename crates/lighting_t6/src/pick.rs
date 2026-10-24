//! Which grid cell a point falls in, how much each of the cell's eight corners
//! counts, and which corner's strongest light the point takes.
//!
//! Corners are numbered by bits: bit 2 steps along the row axis, bit 1 along
//! the column axis, bit 0 up.

use crate::row_rle::LightGridEntryQuad;

/// Shifts the world so every cell index is positive.
const CELL_BIAS: i32 = 0x20000;
const WORLD_BIAS: f64 = -131072.0;
/// Cells per inch sideways (32-inch cells) and up (64-inch cells).
const XY_PER_INCH: f64 = 0.03125;
const Z_PER_INCH: f64 = 0.015625;
/// Under this weight a corner is left out.
const MIN_CORNER_WEIGHT: f32 = 0.001;
/// Added to a negative cell index to read it as the unsigned number it is
/// stored as.
const UNSIGNED_WRAP: f32 = 4294967296.0;

/// A grid cell, in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridCell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl LightGridCell {
    pub(crate) const fn axes(self) -> [i32; 3] {
        [self.x, self.y, self.z]
    }
}

/// The cell holding a world position.
pub fn light_grid_sample_cell(pos: [f32; 3]) -> LightGridCell {
    let cell =
        |coord: f32, shift: u32| (libm::floorf(coord) as i32).wrapping_add(CELL_BIAS) >> shift;
    LightGridCell {
        x: cell(pos[0], 5),
        y: cell(pos[1], 5),
        z: cell(pos[2], 6),
    }
}

/// A cell index as the float the grid math uses: negative indices read as
/// their unsigned value.
pub(crate) fn cell_as_unsigned_f32(cell: i32) -> f32 {
    let value = cell as f32;
    if cell < 0 {
        value + UNSIGNED_WRAP
    } else {
        value
    }
}

fn axis_lerp(coord: f32, cell: i32, per_inch: f64) -> f32 {
    let origin = cell_as_unsigned_f32(cell) as f64;
    (((coord as f64) - WORLD_BIAS) * per_inch - origin) as f32
}

/// How far along its cell a position is on the row axis, the column axis and
/// up, each 0 to 1.
pub fn light_grid_axis_lerps(
    pos: [f32; 3],
    cell: LightGridCell,
    row_axis: u32,
    col_axis: u32,
) -> [f32; 3] {
    let cells = cell.axes();
    let (row, col) = (row_axis as usize, col_axis as usize);
    [
        axis_lerp(pos[row], cells[row], XY_PER_INCH),
        axis_lerp(pos[col], cells[col], XY_PER_INCH),
        axis_lerp(pos[2], cell.z, Z_PER_INCH),
    ]
}

/// The weight of each corner for the given position along the cell.
pub fn light_grid_corner_weights(lerp: [f32; 3]) -> [f32; 8] {
    let [row_hi, col_hi, up_hi] = lerp;
    let (row_lo, col_lo, up_lo) = (1.0 - row_hi, 1.0 - col_hi, 1.0 - up_hi);
    let along = [
        col_lo * up_lo,
        col_lo * up_hi,
        col_hi * up_lo,
        col_hi * up_hi,
    ];
    [
        along[0] * row_lo,
        along[1] * row_lo,
        along[2] * row_lo,
        along[3] * row_lo,
        along[0] * row_hi,
        along[1] * row_hi,
        along[2] * row_hi,
        along[3] * row_hi,
    ]
}

/// Whether a corner counts for enough to keep.
pub const fn light_grid_corner_weight_keeps_entry(weight: f32) -> bool {
    weight >= MIN_CORNER_WEIGHT
}

/// The first primary-light index in the sun's band at the top of the byte
/// range; indices from it up are the sun's.
pub const fn light_grid_primary_light_sun_band(sun_primary_first: u32) -> u32 {
    0x100u32.wrapping_sub(sun_primary_first)
}

/// Whether bit `corner` of an entry's trace bits asks for a sight check.
pub const fn light_grid_corner_needs_trace(needs_trace: u8, corner: u32) -> bool {
    needs_trace & (1u8 << (corner & 7)) != 0
}

/// Which colour set a lookup that found no corners falls back to: the second
/// unless either row was cleared.
pub const fn light_grid_pick_default_grid_entry(low_cleared: bool, high_cleared: bool) -> u32 {
    if low_cleared || high_cleared { 0 } else { 1 }
}

/// The two rows a lookup reads, as cells: the one holding the point and its
/// neighbour on the row axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightGridPickRowRleCells {
    pub base: [i32; 3],
    pub adjacent: [i32; 3],
}

pub fn light_grid_pick_row_rle_cells(
    cell: LightGridCell,
    row_axis: u32,
) -> Option<LightGridPickRowRleCells> {
    let axis = usize::try_from(row_axis).ok().filter(|&axis| axis < 3)?;
    let base = cell.axes();
    let mut adjacent = base;
    adjacent[axis] = adjacent[axis].wrapping_add(1);
    Some(LightGridPickRowRleCells { base, adjacent })
}

/// The two rows' entry quads as one list of the cell's eight corners.
pub fn light_grid_pick_merge_entry_quads(
    low: LightGridEntryQuad,
    high: LightGridEntryQuad,
) -> [Option<u32>; 8] {
    let [a, b, c, d] = low.corners;
    let [e, f, g, h] = high.corners;
    [a, b, c, d, e, f, g, h]
}

/// What the pick needs to know about one corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGridPickCorner {
    /// The corner's strongest light, if the corner has an entry.
    pub primary_light: Option<u8>,
    pub needs_trace: u8,
    pub weight: f32,
    /// The sight check, if it was asked for, found the point visible.
    pub is_valid_if_traced: bool,
}

/// The light taken and the corners still counted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGridPickPrimary {
    pub primary_light: u8,
    pub entry_alive: [bool; 8],
}

/// Whether `candidate` should replace `current`. An unset current always
/// yields; an unset candidate never wins; a sun current yields to anything;
/// otherwise a non-sun candidate needs the larger weight.
fn candidate_wins(
    current: u8,
    current_weight: f32,
    candidate: u8,
    candidate_weight: f32,
    sun_band: u32,
) -> bool {
    if current == 0 {
        return true;
    }
    if candidate == 0 {
        return false;
    }
    u32::from(current) >= sun_band
        || (u32::from(candidate) < sun_band && current_weight < candidate_weight)
}

/// Goes through the eight corners and settles on one light. A corner whose
/// sight check failed only counts while no corner has passed; once one has,
/// failed corners drop out, and the first pass drops the corners before it.
pub fn light_grid_pick_primary_from_corners(
    corners: &[LightGridPickCorner; 8],
    sun_band: u32,
) -> LightGridPickPrimary {
    let mut primary_light = 0u8;
    let mut best_weight = 0.0f32;
    let mut passed = false;
    let mut entry_alive = [true; 8];

    for (index, corner) in corners.iter().enumerate() {
        let Some(light) = corner.primary_light else {
            entry_alive[index] = false;
            continue;
        };
        if !light_grid_corner_weight_keeps_entry(corner.weight) {
            entry_alive[index] = false;
            continue;
        }
        let failed = light_grid_corner_needs_trace(corner.needs_trace, index as u32)
            && !corner.is_valid_if_traced;
        match (failed, passed) {
            (false, false) => {
                primary_light = light;
                best_weight = corner.weight;
                passed = true;
                entry_alive[..index].fill(false);
            }
            (true, true) => entry_alive[index] = false,
            _ => {
                if candidate_wins(primary_light, best_weight, light, corner.weight, sun_band) {
                    primary_light = light;
                    best_weight = corner.weight;
                }
            }
        }
    }

    LightGridPickPrimary {
        primary_light,
        entry_alive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(primary: Option<u8>, weight: f32) -> LightGridPickCorner {
        LightGridPickCorner {
            primary_light: primary,
            needs_trace: 0,
            weight,
            is_valid_if_traced: false,
        }
    }

    fn corners(list: &[LightGridPickCorner]) -> [LightGridPickCorner; 8] {
        let mut all = [corner(None, 0.0); 8];
        all[..list.len()].copy_from_slice(list);
        all
    }

    #[test]
    fn the_world_origin_is_in_the_middle_of_the_grid() {
        let cell = light_grid_sample_cell([0.0, 0.0, 0.0]);
        assert_eq!(
            cell,
            LightGridCell {
                x: 4096,
                y: 4096,
                z: 2048
            }
        );
    }

    #[test]
    fn a_position_just_below_zero_is_in_the_cell_before() {
        let cell = light_grid_sample_cell([-1.0, 31.0, -1.0]);
        assert_eq!(
            cell,
            LightGridCell {
                x: 4095,
                y: 4096,
                z: 2047
            }
        );
    }

    #[test]
    fn lerps_run_from_zero_to_one_across_a_cell() {
        let pos = [16.0, 0.0, 32.0];
        let cell = light_grid_sample_cell(pos);
        assert_eq!(light_grid_axis_lerps(pos, cell, 0, 1), [0.5, 0.0, 0.5]);
        let swapped = light_grid_axis_lerps([0.0, 16.0, 0.0], cell, 1, 0);
        assert_eq!(swapped[0], 0.5);
    }

    #[test]
    fn a_negative_cell_reads_as_an_unsigned_number() {
        assert_eq!(cell_as_unsigned_f32(-1), 4294967296.0);
        assert_eq!(cell_as_unsigned_f32(5), 5.0);
    }

    #[test]
    fn corner_weights_follow_the_lerps() {
        assert_eq!(light_grid_corner_weights([0.0; 3])[0], 1.0);
        assert_eq!(light_grid_corner_weights([1.0; 3])[7], 1.0);
        assert_eq!(light_grid_corner_weights([0.5; 3]), [0.125; 8]);
        let w = light_grid_corner_weights([1.0, 0.0, 0.0]);
        assert_eq!(w[4], 1.0);
        let w = light_grid_corner_weights([0.0, 1.0, 0.0]);
        assert_eq!(w[2], 1.0);
        let w = light_grid_corner_weights([0.0, 0.0, 1.0]);
        assert_eq!(w[1], 1.0);
    }

    #[test]
    fn tiny_weights_are_dropped() {
        assert!(light_grid_corner_weight_keeps_entry(0.001));
        assert!(!light_grid_corner_weight_keeps_entry(0.0009));
    }

    #[test]
    fn the_sun_band_counts_back_from_the_top_of_the_byte_range() {
        assert_eq!(light_grid_primary_light_sun_band(2), 254);
        assert_eq!(light_grid_primary_light_sun_band(0), 256);
    }

    #[test]
    fn trace_bits_are_read_per_corner() {
        assert!(light_grid_corner_needs_trace(0b100, 2));
        assert!(!light_grid_corner_needs_trace(0b100, 1));
        assert!(light_grid_corner_needs_trace(0x80, 7));
    }

    #[test]
    fn the_default_entry_is_zero_when_either_row_was_cleared() {
        assert_eq!(light_grid_pick_default_grid_entry(false, false), 1);
        assert_eq!(light_grid_pick_default_grid_entry(true, false), 0);
        assert_eq!(light_grid_pick_default_grid_entry(false, true), 0);
    }

    #[test]
    fn the_adjacent_row_steps_along_the_row_axis() {
        let cell = LightGridCell { x: 1, y: 2, z: 3 };
        let rows = light_grid_pick_row_rle_cells(cell, 1).unwrap();
        assert_eq!(rows.base, [1, 2, 3]);
        assert_eq!(rows.adjacent, [1, 3, 3]);
        assert_eq!(light_grid_pick_row_rle_cells(cell, 3), None);
    }

    #[test]
    fn merging_quads_puts_the_low_row_first() {
        let low = LightGridEntryQuad {
            corners: [Some(0), Some(4), None, Some(12)],
            clear_needs_trace: false,
        };
        let high = LightGridEntryQuad {
            corners: [Some(16), None, None, None],
            clear_needs_trace: false,
        };
        assert_eq!(
            light_grid_pick_merge_entry_quads(low, high),
            [Some(0), Some(4), None, Some(12), Some(16), None, None, None]
        );
    }

    #[test]
    fn with_no_primaries_nothing_is_picked_or_alive() {
        let picked = light_grid_pick_primary_from_corners(&corners(&[]), 254);
        assert_eq!(picked.primary_light, 0);
        assert_eq!(picked.entry_alive, [false; 8]);
    }

    #[test]
    fn the_heavier_light_wins() {
        let list = [corner(Some(3), 0.2), corner(Some(5), 0.4)];
        let picked = light_grid_pick_primary_from_corners(&corners(&list), 254);
        assert_eq!(picked.primary_light, 5);
        assert_eq!(
            picked.entry_alive,
            [true, true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn a_light_too_faint_to_keep_is_dropped() {
        let list = [corner(Some(3), 0.0005), corner(Some(5), 0.4)];
        let picked = light_grid_pick_primary_from_corners(&corners(&list), 254);
        assert_eq!(picked.primary_light, 5);
        assert!(!picked.entry_alive[0]);
    }

    #[test]
    fn a_passing_corner_drops_the_failed_ones_before_it() {
        let mut failed = corner(Some(4), 0.5);
        failed.needs_trace = 1;
        let list = [failed, corner(Some(6), 0.3)];
        let picked = light_grid_pick_primary_from_corners(&corners(&list), 254);
        assert_eq!(picked.primary_light, 6);
        assert_eq!(
            picked.entry_alive,
            [false, true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn a_failed_corner_after_a_pass_is_dropped() {
        let mut failed = corner(Some(4), 0.9);
        failed.needs_trace = 2;
        let list = [corner(Some(6), 0.3), failed];
        let picked = light_grid_pick_primary_from_corners(&corners(&list), 254);
        assert_eq!(picked.primary_light, 6);
        assert!(!picked.entry_alive[1]);
    }

    #[test]
    fn a_failed_corner_alone_still_supplies_a_light() {
        let mut failed = corner(Some(4), 0.5);
        failed.needs_trace = 1;
        let picked = light_grid_pick_primary_from_corners(&corners(&[failed]), 254);
        assert_eq!(picked.primary_light, 4);
        assert!(picked.entry_alive[0]);
    }

    #[test]
    fn a_sun_light_gives_way_to_the_next_corner() {
        let list = [corner(Some(255), 0.6), corner(Some(3), 0.1)];
        let picked = light_grid_pick_primary_from_corners(&corners(&list), 254);
        assert_eq!(picked.primary_light, 3);
    }

    #[test]
    fn an_unset_light_never_replaces_a_set_one() {
        assert!(candidate_wins(0, 0.0, 3, 0.1, 254));
        assert!(!candidate_wins(3, 0.9, 0, 0.1, 254));
        assert!(!candidate_wins(3, 0.9, 4, 0.1, 254));
        assert!(!candidate_wins(3, 0.1, 255, 0.9, 254));
    }
}
