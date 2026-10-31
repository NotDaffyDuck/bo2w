//! Which grid cell a point falls in, how much each of the cell's eight corners
//! counts, and which corner's strongest light the point takes.
//!
//! Corners are numbered by bits: bit 2 steps along the row axis, bit 1 along
//! the column axis, bit 0 up.

use crate::run_rows::RunQuad;

/// One axis of the grid: how a world coordinate becomes a cell on it.
#[derive(Clone, Copy)]
struct GridAxis {
    /// log2 of the cell size in inches.
    cell_shift: u32,
    /// Cells per inch, for the position inside a cell.
    cells_per_inch: f64,
}

/// Sideways cells are 32 inches, upward ones 64.
const SIDEWAYS: GridAxis = GridAxis { cell_shift: 5, cells_per_inch: 1.0 / 32.0 };
const UPWARD: GridAxis = GridAxis { cell_shift: 6, cells_per_inch: 1.0 / 64.0 };

/// Inches the world is shifted by so every cell index is positive, as an
/// integer for the cell and as a float for the position inside it.
const GRID_SHIFT_INCHES: i32 = 1 << 17;
const GRID_SHIFT_INCHES_F64: f64 = 131072.0;

/// Under this weight a corner is left out.
const MIN_CORNER_WEIGHT: f32 = 0.001;

/// 2^32: added to a negative cell index to read it as the unsigned number it
/// is stored as.
const TWO_POW_32: f32 = 4_294_967_296.0;

impl GridAxis {
    fn cell(self, coord: f32) -> i32 {
        let inch = libm::floorf(coord) as i32;
        inch.wrapping_add(GRID_SHIFT_INCHES) >> self.cell_shift
    }

    /// How far into `cell` the coordinate sits, 0 to 1.
    fn lerp(self, coord: f32, cell: i32) -> f32 {
        let shifted = f64::from(coord) + GRID_SHIFT_INCHES_F64;
        let start = f64::from(cell_as_unsigned_f32(cell));
        (shifted * self.cells_per_inch - start) as f32
    }
}

/// A grid cell, in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IlluminationVoxelRef {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl IlluminationVoxelRef {
    pub(crate) const fn axes(self) -> [i32; 3] {
        let IlluminationVoxelRef { x, y, z } = self;
        [x, y, z]
    }
}

/// The cell holding a world position.
pub fn cell_under(pos: [f32; 3]) -> IlluminationVoxelRef {
    let [x, y, z] = pos;
    IlluminationVoxelRef { x: SIDEWAYS.cell(x), y: SIDEWAYS.cell(y), z: UPWARD.cell(z) }
}

/// A cell index as the float the grid math uses: negative indices read as
/// their unsigned value.
pub(crate) fn cell_as_unsigned_f32(cell: i32) -> f32 {
    let wrap = if cell.is_negative() { TWO_POW_32 } else { 0.0 };
    cell as f32 + wrap
}

/// How far along its cell a position is on the row axis, the column axis and
/// up, each 0 to 1.
pub fn axis_mix(
    pos: [f32; 3],
    cell: IlluminationVoxelRef,
    line_direction: u32,
    column_direction: u32,
) -> [f32; 3] {
    let at = cell.axes();
    let sideways = |axis: u32| SIDEWAYS.lerp(pos[axis as usize], at[axis as usize]);
    [sideways(line_direction), sideways(column_direction), UPWARD.lerp(pos[2], cell.z)]
}

/// The weight of each corner for the given position along the cell. A
/// corner's weight is (column share * up share) * row share, each share the
/// lerp on its high side and one less the lerp on its low side.
pub fn corner_mix(lerp: [f32; 3]) -> [f32; 8] {
    let share = |axis: usize, high: bool| if high { lerp[axis] } else { 1.0 - lerp[axis] };
    std::array::from_fn(|corner| {
        let bit = |n: usize| corner >> n & 1 == 1;
        share(1, bit(1)) * share(2, bit(0)) * share(0, bit(2))
    })
}

/// Whether a corner counts for enough to keep.
pub const fn corner_keeps_weight(weight: f32) -> bool {
    MIN_CORNER_WEIGHT <= weight
}

/// The first primary-light index in the sun's band at the top of the byte
/// range; indices from it up are the sun's.
pub const fn sun_band_light(sun_leads_primaries: u32) -> u32 {
    (u8::MAX as u32 + 1).wrapping_sub(sun_leads_primaries)
}

/// Whether bit `corner` of an entry's trace bits asks for a sight check.
pub const fn corner_wants_trace(wants_ray_check: u8, corner: u32) -> bool {
    (wants_ray_check >> (corner % 8)) & 1 == 1
}

/// Which colour set a lookup that found no corners falls back to: the second
/// unless either row was cleared.
pub const fn fallback_cell_entry(low_cleared: bool, high_cleared: bool) -> u32 {
    !(low_cleared || high_cleared) as u32
}

/// The two rows a lookup reads, as cells: the one holding the point and its
/// neighbour on the row axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPickRunLengthSpan {
    pub first: [i32; 3],
    pub neighbor_cell: [i32; 3],
}

pub fn packed_row_cells(
    cell: IlluminationVoxelRef,
    line_direction: u32,
) -> Option<GridPickRunLengthSpan> {
    let base = cell.axes();
    let step = line_direction as usize;
    (line_direction < 3).then(|| GridPickRunLengthSpan {
        first: base,
        neighbor_cell: std::array::from_fn(|k| if k == step { base[k].wrapping_add(1) } else { base[k] }),
    })
}

/// The two rows' entry quads as one list of the cell's eight corners.
pub fn grid_pick_combine_cell_quartets(
    low: RunQuad,
    high: RunQuad,
) -> [Option<u32>; 8] {
    std::array::from_fn(|corner| if corner < 4 { low.quad_cells[corner] } else { high.quad_cells[corner - 4] })
}

/// What the pick needs to know about one corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridCornerChoice {
    /// The corner's strongest light, if the corner has an entry.
    pub dominant_light: Option<u8>,
    pub wants_ray_check: u8,
    pub share: f32,
    /// The sight check, if it was asked for, found the point visible.
    pub valid_when_traced: bool,
}

/// The light taken and the corners still counted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridDominantLightChoice {
    pub dominant_light: u8,
    pub entry_in_use: [bool; 8],
}

/// Whether `candidate` should replace `current`. An unset current always
/// yields; an unset candidate never wins; a sun current yields to anything;
/// otherwise a non-sun candidate needs the larger weight.
fn candidate_wins(
    held: u8,
    held_share: f32,
    rival: u8,
    rival_share: f32,
    sun_floor: u32,
) -> bool {
    let is_daylight = |light: u8| u32::from(light) >= sun_floor;
    match (held, rival) {
        (0, _) => true,
        (_, 0) => false,
        _ => is_daylight(held) || (!is_daylight(rival) && rival_share > held_share),
    }
}

/// The pick so far: the light held and its weight, and whether a corner has
/// passed its sight check yet.
struct Choice {
    light: u8,
    share: f32,
    seen_pass: bool,
}

/// Goes through the eight corners and settles on one light. A corner whose
/// sight check failed only counts while no corner has passed; once one has,
/// failed corners drop out, and the first pass drops the corners before it.
pub fn grid_choose_main_light_from_corners(
    corners: &[GridCornerChoice; 8],
    sun_floor: u32,
) -> GridDominantLightChoice {
    let mut choice = Choice { light: 0, share: 0.0, seen_pass: false };
    let mut entry_in_use = [false; 8];

    for (index, corner) in (0u32..).zip(corners) {
        let slot = index as usize;
        let light = match corner.dominant_light {
            Some(light) if corner_keeps_weight(corner.share) => light,
            _ => continue,
        };
        let passes = corner.valid_when_traced || !corner_wants_trace(corner.wants_ray_check, index);
        if passes && !choice.seen_pass {
            // The first pass: it takes over, and every corner before it drops out.
            choice = Choice { light, share: corner.share, seen_pass: true };
            entry_in_use = [false; 8];
            entry_in_use[slot] = true;
            continue;
        }
        if !passes && choice.seen_pass {
            continue;
        }
        entry_in_use[slot] = true;
        if candidate_wins(choice.light, choice.share, light, corner.share, sun_floor) {
            (choice.light, choice.share) = (light, corner.share);
        }
    }

    GridDominantLightChoice { dominant_light: choice.light, entry_in_use }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(primary: Option<u8>, weight: f32) -> GridCornerChoice {
        GridCornerChoice { dominant_light: primary, wants_ray_check: 0, share: weight, valid_when_traced: false }
    }

    fn traced(primary: u8, share: f32, wants_ray_check: u8) -> GridCornerChoice {
        GridCornerChoice { wants_ray_check, ..corner(Some(primary), share) }
    }

    fn pick(list: &[GridCornerChoice]) -> GridDominantLightChoice {
        let mut all = [corner(None, 0.0); 8];
        all[..list.len()].copy_from_slice(list);
        grid_choose_main_light_from_corners(&all, 254)
    }

    fn alive(first: &[bool]) -> [bool; 8] {
        let mut all = [false; 8];
        all[..first.len()].copy_from_slice(first);
        all
    }

    #[test]
    fn the_world_origin_is_in_the_middle_of_the_grid() {
        assert_eq!(cell_under([0.0; 3]), IlluminationVoxelRef { x: 4096, y: 4096, z: 2048 });
    }

    #[test]
    fn a_position_just_below_zero_is_in_the_cell_before() {
        assert_eq!(cell_under([-1.0, 31.0, -1.0]), IlluminationVoxelRef { x: 4095, y: 4096, z: 2047 });
    }

    #[test]
    fn lerps_run_from_zero_to_one_across_a_cell() {
        let pos = [16.0, 0.0, 32.0];
        let cell = cell_under(pos);
        assert_eq!(axis_mix(pos, cell, 0, 1), [0.5, 0.0, 0.5]);
        assert_eq!(axis_mix([0.0, 16.0, 0.0], cell, 1, 0)[0], 0.5);
    }

    #[test]
    fn a_negative_cell_reads_as_an_unsigned_number() {
        assert_eq!([-1, 5].map(cell_as_unsigned_f32), [4294967296.0, 5.0]);
    }

    #[test]
    fn corner_weights_follow_the_lerps() {
        assert_eq!(corner_mix([0.0; 3])[0], 1.0);
        assert_eq!(corner_mix([1.0; 3])[7], 1.0);
        assert_eq!(corner_mix([0.5; 3]), [0.125; 8]);
        let one_hot = [([1.0, 0.0, 0.0], 4), ([0.0, 1.0, 0.0], 2), ([0.0, 0.0, 1.0], 1)];
        for (lerp, corner) in one_hot {
            assert_eq!(corner_mix(lerp)[corner], 1.0, "{lerp:?}");
        }
    }

    #[test]
    fn tiny_weights_are_dropped() {
        assert!(corner_keeps_weight(0.001));
        assert!(!corner_keeps_weight(0.0009));
    }

    #[test]
    fn the_sun_band_counts_back_from_the_top_of_the_byte_range() {
        assert_eq!([2, 0].map(sun_band_light), [254, 256]);
    }

    #[test]
    fn trace_bits_are_read_per_corner() {
        assert!(corner_wants_trace(0b100, 2));
        assert!(!corner_wants_trace(0b100, 1));
        assert!(corner_wants_trace(0x80, 7));
    }

    #[test]
    fn the_default_entry_is_zero_when_either_row_was_cleared() {
        let got = [(false, false), (true, false), (false, true)].map(|(a, b)| fallback_cell_entry(a, b));
        assert_eq!(got, [1, 0, 0]);
    }

    #[test]
    fn the_adjacent_row_steps_along_the_row_axis() {
        let cell = IlluminationVoxelRef { x: 1, y: 2, z: 3 };
        let rows = packed_row_cells(cell, 1).unwrap();
        assert_eq!((rows.first, rows.neighbor_cell), ([1, 2, 3], [1, 3, 3]));
        assert_eq!(packed_row_cells(cell, 3), None);
    }

    #[test]
    fn merging_quads_puts_the_low_row_first() {
        let quad = |corners| RunQuad { quad_cells: corners, empty_run_requires_trace: false };
        let merged = grid_pick_combine_cell_quartets(quad([Some(0), Some(4), None, Some(12)]), quad([Some(16), None, None, None]));
        assert_eq!(merged, [Some(0), Some(4), None, Some(12), Some(16), None, None, None]);
    }

    #[test]
    fn with_no_primaries_nothing_is_picked_or_alive() {
        let picked = pick(&[]);
        assert_eq!((picked.dominant_light, picked.entry_in_use), (0, [false; 8]));
    }

    #[test]
    fn the_heavier_light_wins() {
        let picked = pick(&[corner(Some(3), 0.2), corner(Some(5), 0.4)]);
        assert_eq!((picked.dominant_light, picked.entry_in_use), (5, alive(&[true, true])));
    }

    #[test]
    fn a_light_too_faint_to_keep_is_dropped() {
        let picked = pick(&[corner(Some(3), 0.0005), corner(Some(5), 0.4)]);
        assert_eq!(picked.dominant_light, 5);
        assert!(!picked.entry_in_use[0]);
    }

    #[test]
    fn a_passing_corner_drops_the_failed_ones_before_it() {
        let picked = pick(&[traced(4, 0.5, 1), corner(Some(6), 0.3)]);
        assert_eq!((picked.dominant_light, picked.entry_in_use), (6, alive(&[false, true])));
    }

    #[test]
    fn a_failed_corner_after_a_pass_is_dropped() {
        let picked = pick(&[corner(Some(6), 0.3), traced(4, 0.9, 2)]);
        assert_eq!(picked.dominant_light, 6);
        assert!(!picked.entry_in_use[1]);
    }

    #[test]
    fn a_failed_corner_alone_still_supplies_a_light() {
        let picked = pick(&[traced(4, 0.5, 1)]);
        assert_eq!(picked.dominant_light, 4);
        assert!(picked.entry_in_use[0]);
    }

    #[test]
    fn a_sun_light_gives_way_to_the_next_corner() {
        assert_eq!(pick(&[corner(Some(255), 0.6), corner(Some(3), 0.1)]).dominant_light, 3);
    }

    #[test]
    fn an_unset_light_never_replaces_a_set_one() {
        assert!(candidate_wins(0, 0.0, 3, 0.1, 254));
        assert!(!candidate_wins(3, 0.9, 0, 0.1, 254));
        assert!(!candidate_wins(3, 0.9, 4, 0.1, 254));
        assert!(!candidate_wins(3, 0.1, 255, 0.9, 254));
    }
}
