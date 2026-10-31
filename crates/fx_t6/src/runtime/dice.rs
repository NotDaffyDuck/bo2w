//! Where an effect's random numbers come from.
//!
//! There is no live generator. Every element reads from one fixed table of
//! 508 floats: its seed picks a row (modulo [`DICE_TABLE_WRAP`]) and each
//! thing being randomised has its own channel, an offset added to that row.
//! The same seed therefore always gives the same effect.

use crate::random_table::FX_RANDOM_TABLE;
use crate::runtime::bank::DICE_TABLE_WRAP;

/// How many floats the table holds.
pub const DICE_TABLE_REALS: usize = 508;

// Channels, grouped by what they randomise.
pub(super) const DICE_LANE_SPIN_TILT: u32 = 3;
pub(super) const DICE_LANE_SPIN_TURN: u32 = 4;
pub(super) const DICE_LANE_SPIN_ROLL: u32 = 5;
pub(super) const DICE_LANE_BIRTH_ACROSS: u32 = 6;
pub(super) const DICE_LANE_BIRTH_SIDE: u32 = 7;
pub(super) const DICE_LANE_BIRTH_HEIGHT: u32 = 8;
pub(super) const DICE_LANE_RING_TURN: u32 = 9;
pub(super) const DICE_LANE_BIRTH_LIFT: u32 = 10;
pub(super) const DICE_LANE_RING_REACH: u32 = 11;
pub(super) const DICE_LANE_BIRTH_TILT: u32 = 12;
pub(super) const DICE_LANE_BIRTH_TURN: u32 = 13;
pub(super) const DICE_LANE_BIRTH_ROLL: u32 = 14;
pub(super) const DICE_LANE_FALL: u32 = 15;
pub(super) const DICE_LANE_BOUNCE: u32 = 16;
/// Life span.
pub const DICE_LANE_LIFESPAN: u32 = 17;
/// Spawn delay.
pub const DICE_LANE_WAIT: u32 = 18;
/// How many parts a one-shot spawns.
pub const DICE_LANE_BURST_TOTAL: u32 = 19;
pub(super) const DICE_LANE_EMIT_GAP: u32 = 20;
const DICE_LANE_LOOK_PICK: u32 = 21;
pub(super) const DICE_LANE_SHEET: u32 = 22;
/// Colour.
pub const DICE_LANE_TINT: u32 = 23;
pub(super) const DICE_LANE_FIRST_ROLL: u32 = 24;
pub(super) const DICE_LANE_ROLL_STEP: u32 = 25;
/// First size channel.
pub const DICE_LANE_FIRST_EXTENT: u32 = 26;
/// Scale.
pub const DICE_LANE_GROWTH: u32 = 28;

/// Multiplier and increment of the linear generator that turns an effect's
/// start time into its seed.
const SEED_MUL: u32 = 0x343fd;
const SEED_ADD: u32 = 0x269ec3;

/// The table position for a seed and channel.
fn lookup_slot(seed: u32, lane: u32) -> usize {
    (seed % DICE_TABLE_WRAP + lane) as usize
}

/// The float in `[0, 1)` for this seed and channel.
pub fn random_table_f32(seed: u32, lane: u32) -> f32 {
    FX_RANDOM_TABLE[lookup_slot(seed, lane)]
}

/// The low 16 bits of the table entry for this seed and channel.
pub fn random_table_u16(seed: u32, lane: u32) -> u16 {
    FX_RANDOM_TABLE[lookup_slot(seed, lane)].to_bits() as u16
}

/// Which of an element's visuals it shows. One visual, or none, is always
/// index 0; with several the choice is a 16-bit fraction of the count.
pub fn elem_visual_index(look_variant_total: u8, dice_start: u32) -> usize {
    let count = u32::from(look_variant_total);
    if count <= 1 {
        return 0;
    }
    let fraction = random_table_u16(dice_start, DICE_LANE_LOOK_PICK) as u32;
    (count.wrapping_mul(fraction) >> 16) as usize
}

/// The seed an effect gets from the time it began.
pub fn effect_random_seed_from_msec(window_open_ms: i32) -> u16 {
    let scrambled = (window_open_ms as u32)
        .wrapping_mul(SEED_MUL)
        .wrapping_add(SEED_ADD);
    (((scrambled >> 17).wrapping_mul(DICE_TABLE_WRAP)) >> 15) as u16
}

/// The seed one element of an effect uses: the effect's seed shifted by the
/// element's place in spawn order and by when it began.
pub fn elem_random_seed(emitter_dice_start: u16, seq_no: u8, window_open_ms: i32) -> u32 {
    let shifted = u32::from(emitter_dice_start)
        .wrapping_add(u32::from(seq_no).wrapping_mul(0x128))
        .wrapping_add(window_open_ms as u32);
    shifted % DICE_TABLE_WRAP
}

/// The seed a trail uses. The sequence is signed here, and wraps.
pub fn trail_random_seed(emitter_dice_start: u16, seq_no: i8) -> u32 {
    let shift = i32::from(seq_no).wrapping_mul(0x128) as u32;
    u32::from(emitter_dice_start).wrapping_add(shift) % DICE_TABLE_WRAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_the_documented_length() {
        assert_eq!(FX_RANDOM_TABLE.len(), DICE_TABLE_REALS);
    }

    #[test]
    fn the_highest_channel_on_the_last_row_stays_in_the_table() {
        let last_row = DICE_TABLE_WRAP - 1;
        assert!(lookup_slot(last_row, DICE_LANE_GROWTH) < DICE_TABLE_REALS);
    }

    #[test]
    fn a_seed_wraps_around_the_table_rows() {
        assert_eq!(
            random_table_f32(3, DICE_LANE_LIFESPAN),
            random_table_f32(3 + DICE_TABLE_WRAP, DICE_LANE_LIFESPAN)
        );
    }

    #[test]
    fn one_visual_is_always_the_first() {
        assert_eq!(elem_visual_index(0, 77), 0);
        assert_eq!(elem_visual_index(1, 77), 0);
    }

    #[test]
    fn several_visuals_pick_inside_the_count() {
        for seed in 0..DICE_TABLE_WRAP {
            assert!(elem_visual_index(5, seed) < 5);
        }
    }

    #[test]
    fn element_seeds_stay_below_the_row_count() {
        for sequence in [0u8, 1, 200, 255] {
            assert!(elem_random_seed(0x1de, sequence, i32::MAX) < DICE_TABLE_WRAP);
        }
        assert!(trail_random_seed(0x1de, -128) < DICE_TABLE_WRAP);
    }

    #[test]
    fn effect_seed_for_time_zero_is_from_the_increment_alone() {
        let expected = (((SEED_ADD >> 17) * DICE_TABLE_WRAP) >> 15) as u16;
        assert_eq!(effect_random_seed_from_msec(0), expected);
    }
}
