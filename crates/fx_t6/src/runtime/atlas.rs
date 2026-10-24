//! Picking the cell of a sprite atlas that an element shows.
//!
//! An element's atlas record is eight bytes:
//! `[behaviour, start cell, frames per second, loop count, column bits,
//! row bits, entry count (16 bits, little endian)]`.

use crate::runtime::random::{FX_RAND_CH_ATLAS, random_table_u16};

/// How many bytes an atlas record takes.
const RECORD_LEN: usize = 8;

// Bits of the behaviour byte.
const START_KIND_MASK: u8 = 3;
const START_AT_RANDOM_CELL: u8 = 1;
const PLAY_ACROSS_LIFE: u8 = 4;
const HOLD_LAST_CELL: u8 = 8;

/// Where one cell of an atlas sits in texture space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxSpriteAtlasUv {
    /// Left edge.
    pub s0: f32,
    /// Width.
    pub ds: f32,
    /// Top edge.
    pub t0: f32,
    /// Height.
    pub dt: f32,
    /// How many cells the atlas has.
    pub entry_count: u16,
    /// Which cell this is.
    pub atlas_index: u16,
}

impl FxSpriteAtlasUv {
    /// The whole texture, for an element with no atlas.
    pub const FULL: Self = Self {
        s0: 0.0,
        ds: 1.0,
        t0: 0.0,
        dt: 1.0,
        entry_count: 1,
        atlas_index: 0,
    };

    /// The cell's corners as `[s, t]` pairs, going round from bottom-left.
    pub const fn corners(self) -> [[f32; 2]; 4] {
        let right = self.s0 + self.ds;
        let bottom = self.t0 + self.dt;
        [
            [self.s0, bottom],
            [self.s0, self.t0],
            [right, self.t0],
            [right, bottom],
        ]
    }
}

/// The cell an element shows right now.
///
/// `atlas` is the element's record, `random_seed` the element's seed,
/// `sequence` the element's spawn number, `msec_elapsed` how long it has
/// lived and `norm_time` how far through its life that is (0 to 1).
pub fn sprite_atlas_uv(
    atlas: &[u8],
    random_seed: u32,
    sequence: u8,
    msec_elapsed: i32,
    norm_time: f32,
) -> FxSpriteAtlasUv {
    let Some(
        &[
            behavior,
            start_cell,
            fps,
            loop_count,
            col_bits,
            row_bits,
            count_lo,
            count_hi,
        ],
    ) = atlas.first_chunk::<RECORD_LEN>()
    else {
        return FxSpriteAtlasUv::FULL;
    };
    let entry_count = i32::from(i16::from_le_bytes([count_lo, count_hi]));
    if entry_count <= 1 {
        return FxSpriteAtlasUv::FULL;
    }

    let mut cell = match behavior & START_KIND_MASK {
        0 => i32::from(start_cell),
        START_AT_RANDOM_CELL => {
            let roll = u32::from(random_table_u16(random_seed, FX_RAND_CH_ATLAS));
            (roll.wrapping_mul(entry_count as u32) >> 16) as i32
        }
        _ => i32::from(sequence) & (entry_count - 1),
    };

    if behavior & PLAY_ACROSS_LIFE != 0 {
        cell = cell.wrapping_add((entry_count as f32 * norm_time) as i32);
    } else if fps != 0 {
        let frames_x1000 = (msec_elapsed as u32).wrapping_mul(u32::from(fps)) as i32;
        cell = cell.wrapping_add(frames_x1000 / 1000);
    }

    if behavior & HOLD_LAST_CELL != 0 && cell >= i32::from(loop_count).wrapping_mul(entry_count) {
        cell = entry_count - 1;
    }

    let wrapped = (cell as u32) & (entry_count as u32).wrapping_sub(1);
    atlas_cell(wrapped, col_bits, row_bits, entry_count as u16)
}

/// Cell number `index` of an atlas laid out `1 << col_bits` across and
/// `1 << row_bits` down.
fn atlas_cell(index: u32, col_bits: u8, row_bits: u8, entry_count: u16) -> FxSpriteAtlasUv {
    let col_bits = col_bits & 0x1f;
    let row_bits = row_bits & 0x1f;
    let cols = 1u32 << col_bits;
    let rows = 1u32 << row_bits;
    let ds = 1.0 / cols as f32;
    let dt = 1.0 / rows as f32;
    let col = index & cols.wrapping_sub(1);
    let row = index >> col_bits;
    FxSpriteAtlasUv {
        s0: col as f32 * ds,
        ds,
        t0: row as f32 * dt,
        dt,
        entry_count,
        atlas_index: index as u16,
    }
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
        assert_eq!(sprite_atlas_uv(&[], 0, 0, 0, 0.0), FxSpriteAtlasUv::FULL);
        assert_eq!(
            sprite_atlas_uv(&[0, 0, 0, 0, 0, 0, 1, 0], 0, 0, 0, 0.0),
            FxSpriteAtlasUv::FULL
        );
        assert_eq!(
            sprite_atlas_uv(&[0, 0, 0, 0, 0, 0, 0xff, 0xff], 0, 0, 0, 0.0),
            FxSpriteAtlasUv::FULL
        );
    }

    #[test]
    fn a_cell_has_its_place_in_the_grid() {
        let uv = atlas_cell(3, 1, 1, 4);
        assert_eq!((uv.s0, uv.ds, uv.t0, uv.dt), (0.5, 0.5, 0.5, 0.5));
        assert_eq!(uv.atlas_index, 3);
        assert_eq!(uv.entry_count, 4);
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
        let at = |msec| sprite_atlas_uv(&rec, 0, 0, msec, 0.0).atlas_index;
        assert_eq!(at(0), 0);
        assert_eq!(at(100), 1);
        assert_eq!(at(250), 2);
        assert_eq!(at(450), 0, "four cells wrap round");
    }

    #[test]
    fn a_fixed_start_cell_is_the_first_cell_shown() {
        let rec = record(0, 3, 0, 0);
        assert_eq!(sprite_atlas_uv(&rec, 0, 0, 5000, 0.0).atlas_index, 3);
    }

    #[test]
    fn the_sequence_picks_the_start_cell_for_a_sequenced_atlas() {
        let rec = record(2, 0, 0, 0);
        assert_eq!(sprite_atlas_uv(&rec, 0, 5, 0, 0.0).atlas_index, 1);
    }

    #[test]
    fn a_random_start_stays_inside_the_atlas() {
        let rec = record(1, 0, 0, 0);
        for seed in 0..64 {
            assert!(sprite_atlas_uv(&rec, seed, 0, 0, 0.0).atlas_index < 4);
        }
    }

    #[test]
    fn playing_across_life_reaches_the_end_with_the_element() {
        let rec = record(4, 0, 0, 0);
        let at = |t| sprite_atlas_uv(&rec, 0, 0, 0, t).atlas_index;
        assert_eq!(at(0.0), 0);
        assert_eq!(at(0.5), 2);
        assert_eq!(at(0.99), 3);
    }

    #[test]
    fn holding_the_last_cell_stops_after_the_loops_are_used_up() {
        let hold = record(8, 0, 255, 1);
        let wrap = record(0, 0, 255, 1);
        // 4 ms at 255 fps is one full run of the four cells.
        assert_eq!(sprite_atlas_uv(&hold, 0, 0, 20, 0.0).atlas_index, 3);
        assert_eq!(sprite_atlas_uv(&wrap, 0, 0, 20, 0.0).atlas_index, 1);
    }
}
