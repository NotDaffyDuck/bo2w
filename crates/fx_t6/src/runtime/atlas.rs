//! Picking the cell of a sprite atlas that an element shows.
//!
//! An element's atlas record is eight bytes:
//! `[behaviour, start cell, frames per second, loop count, column bits,
//! row bits, entry count (16 bits, little endian)]`.

use crate::runtime::dice::DICE_LANE_SHEET;
use crate::runtime::dice::random_table_u16;

/// How many bytes an atlas record takes.
const RECORD_LEN: usize = 8;

// Bits of the behaviour byte.
const START_KIND_MASK: u8 = 3;
const START_AT_RANDOM_CELL: u8 = 1;
const PLAY_ACROSS_LIFE: u8 = 4;
const HOLD_LAST_CELL: u8 = 8;

/// Where one cell of an atlas sits in texture space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BillboardSheetCell {
    /// Left edge.
    pub s0: f32,
    /// Width.
    pub ds: f32,
    /// Top edge.
    pub t0: f32,
    /// Height.
    pub dt: f32,
    /// How many cells the atlas has.
    pub entry_tally: u16,
    /// Which cell this is.
    pub sheet_cell: u16,
}

impl BillboardSheetCell {
    /// The whole texture, for an element with no atlas.
    pub const FULL: BillboardSheetCell = Self {
        s0: 0.0,
        ds: 1.0,
        t0: 0.0,
        dt: 1.0,
        entry_tally: 1,
        sheet_cell: 0,
    };

    /// The cell's corners as `[s, t]` pairs, going round from bottom-left.
    pub const fn corners(self) -> [[f32; 2]; 4] {
        let (left, top) = (self.s0, self.t0);
        let right = left + self.ds;
        let bottom = top + self.dt;
        [
            [left, bottom],
            [left, top],
            [right, top],
            [right, bottom],
        ]
    }
}

/// The cell an element shows right now.
///
/// `sheet` is the element's record, `dice_start` the element's seed,
/// `seq_no` the element's spawn number, `ms_gone_by` how long it has
/// lived and `life_frac` how far through its life that is (0 to 1).
pub fn sprite_atlas_uv(
    sheet: &[u8],
    dice_start: u32,
    seq_no: u8,
    ms_gone_by: i32,
    life_frac: f32,
) -> BillboardSheetCell {
    // No record, or a single cell: the whole texture.
    AtlasRecord::parse(sheet)
        .filter(|record| record.entries > 1)
        .map_or(BillboardSheetCell::FULL, |record| record.cell_now(dice_start, seq_no, ms_gone_by, life_frac))
}

impl AtlasRecord {
    fn cell_now(&self, dice_start: u32, seq_no: u8, ms_gone_by: i32, norm_time: f32) -> BillboardSheetCell {
        let played = self.first_cell(dice_start, seq_no)
            .wrapping_add(self.cells_played(ms_gone_by, norm_time));
        // A holding atlas stops on its last cell once its loops are played.
        let parked = self.holds_last && played >= self.loops_cells();
        let shown = if parked { self.entries - 1 } else { played };
        let index = (shown as u32) & (self.entries as u32).wrapping_sub(1);
        atlas_cell(index, self.column_flags, self.line_flags, self.entries as u16)
    }
}

/// The eight record bytes of an atlas, picked apart.
struct AtlasRecord {
    behavior: u8,
    start_cell: u8,
    fps: u8,
    loop_count: u8,
    column_flags: u8,
    line_flags: u8,
    entries: i32,
    holds_last: bool,
}

impl AtlasRecord {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let raw = bytes.first_chunk::<RECORD_LEN>()?;
        Some(AtlasRecord {
            behavior: raw[0],
            start_cell: raw[1],
            fps: raw[2],
            loop_count: raw[3],
            column_flags: raw[4],
            line_flags: raw[5],
            entries: i32::from(i16::from_le_bytes([raw[6], raw[7]])),
            holds_last: raw[0] & HOLD_LAST_CELL != 0,
        })
    }

    /// The cell the element starts on, before it has played at all.
    fn first_cell(&self, dice_start: u32, sequence: u8) -> i32 {
        match self.behavior & START_KIND_MASK {
            0 => i32::from(self.start_cell),
            START_AT_RANDOM_CELL => {
                let roll = random_table_u16(dice_start, DICE_LANE_SHEET) as u32;
                (roll.wrapping_mul(self.entries as u32) >> 16) as i32
            }
            _ => i32::from(sequence) & (self.entries - 1),
        }
    }

    /// How many cells it has moved on: spread over its life, or at a fixed rate.
    fn cells_played(&self, ms_gone_by: i32, norm_time: f32) -> i32 {
        if self.behavior & PLAY_ACROSS_LIFE != 0 {
            (self.entries as f32 * norm_time) as i32
        } else if self.fps == 0 {
            0
        } else {
            let thousandths = (ms_gone_by as u32).wrapping_mul(u32::from(self.fps)) as i32;
            thousandths / 1000
        }
    }

    /// The cell count after which a holding atlas stops on its last cell.
    fn loops_cells(&self) -> i32 {
        i32::from(self.loop_count).wrapping_mul(self.entries)
    }
}

/// Cell number `index` of an atlas laid out `1 << column_flags` across and
/// `1 << line_flags` down.
fn atlas_cell(index: u32, column_flags: u8, line_flags: u8, entry_tally: u16) -> BillboardSheetCell {
    let (column_flags, line_flags) = (column_flags & 0x1f, line_flags & 0x1f);
    // Start and size along one axis for cell `n` of `1 << bits`.
    let span = |n: u32, bits: u8| {
        let size = 1.0 / (1u32 << bits) as f32;
        (n as f32 * size, size)
    };
    let (s0, ds) = span(index & ((1u32 << column_flags).wrapping_sub(1)), column_flags);
    let (t0, dt) = span(index >> column_flags, line_flags);
    BillboardSheetCell { s0, ds, t0, dt, entry_tally, sheet_cell: index as u16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 atlas of four cells with the given behaviour, start, rate and
    /// loop count.
    fn record(behavior: u8, start: u8, fps: u8, loops: u8) -> [u8; 8] {
        [behavior, start, fps, loops, 1, 1, 4, 0]
    }

    #[test]
    fn a_missing_or_single_cell_atlas_shows_the_whole_texture() {
        assert_eq!(sprite_atlas_uv(&[], 0, 0, 0, 0.0), BillboardSheetCell::FULL);
        assert_eq!(
            sprite_atlas_uv(&[0, 0, 0, 0, 0, 0, 1, 0], 0, 0, 0, 0.0),
            BillboardSheetCell::FULL
        );
        assert_eq!(
            sprite_atlas_uv(&[0, 0, 0, 0, 0, 0, 0xff, 0xff], 0, 0, 0, 0.0),
            BillboardSheetCell::FULL
        );
    }

    #[test]
    fn a_cell_has_its_place_in_the_grid() {
        let uv = atlas_cell(3, 1, 1, 4);
        assert_eq!((uv.s0, uv.ds, uv.t0, uv.dt), (0.5, 0.5, 0.5, 0.5));
        assert_eq!(uv.sheet_cell, 3);
        assert_eq!(uv.entry_tally, 4);
    }

    #[test]
    fn corners_go_round_from_the_bottom_left() {
        let uv = atlas_cell(1, 1, 1, 4);
        assert_eq!(
            uv.corners(),
            [[0.5, 0.5], [0.5, 0.0], [1.0, 0.0], [1.0, 0.5]]
        );
    }

    #[test]
    fn frame_rate_steps_through_the_cells() {
        let rec = record(0, 0, 10, 0);
        let at = |msec| sprite_atlas_uv(&rec, 0, 0, msec, 0.0).sheet_cell;
        assert_eq!(at(0), 0);
        assert_eq!(at(100), 1);
        assert_eq!(at(250), 2);
        assert_eq!(at(450), 0, "four cells wrap round");
    }

    #[test]
    fn a_fixed_start_cell_is_the_first_cell_shown() {
        let rec = record(0, 3, 0, 0);
        assert_eq!(sprite_atlas_uv(&rec, 0, 0, 5000, 0.0).sheet_cell, 3);
    }

    #[test]
    fn the_sequence_picks_the_start_cell_for_a_sequenced_atlas() {
        let rec = record(2, 0, 0, 0);
        assert_eq!(sprite_atlas_uv(&rec, 0, 5, 0, 0.0).sheet_cell, 1);
    }

    #[test]
    fn a_random_start_stays_inside_the_atlas() {
        let rec = record(1, 0, 0, 0);
        for seed in 0..64 {
            assert!(sprite_atlas_uv(&rec, seed, 0, 0, 0.0).sheet_cell < 4);
        }
    }

    #[test]
    fn playing_across_life_reaches_the_end_with_the_element() {
        let rec = record(4, 0, 0, 0);
        let at = |t| sprite_atlas_uv(&rec, 0, 0, 0, t).sheet_cell;
        assert_eq!(at(0.0), 0);
        assert_eq!(at(0.5), 2);
        assert_eq!(at(0.99), 3);
    }

    #[test]
    fn holding_the_last_cell_stops_after_the_loops_are_used_up() {
        let hold = record(8, 0, 255, 1);
        let wrap = record(0, 0, 255, 1);
        // 4 ms at 255 fps is one full run of the four cells.
        assert_eq!(sprite_atlas_uv(&hold, 0, 0, 20, 0.0).sheet_cell, 3);
        assert_eq!(sprite_atlas_uv(&wrap, 0, 0, 20, 0.0).sheet_cell, 1);
    }
}
