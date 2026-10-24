//! Where an effect's random numbers come from.
//!
//! There is no live generator. Every element reads from one fixed table of
//! 508 floats: its seed picks a row (modulo [`FX_RAND_TABLE_MOD`]) and each
//! thing being randomised has its own channel, an offset added to that row.
//! The same seed therefore always gives the same effect.

use crate::random_table::FX_RANDOM_TABLE;
use crate::runtime::pool::FX_RAND_TABLE_MOD;

/// How many floats the table holds.
pub const FX_RANDOM_TABLE_FLOATS: usize = 508;

// Channels, grouped by what they randomise.
pub(super) const FX_RAND_CH_ANG_VEL_PITCH: u32 = 3;
pub(super) const FX_RAND_CH_ANG_VEL_YAW: u32 = 4;
pub(super) const FX_RAND_CH_ANG_VEL_ROLL: u32 = 5;
pub(super) const FX_RAND_CH_SPAWN_ORIGIN_X: u32 = 6;
pub(super) const FX_RAND_CH_SPAWN_ORIGIN_Y: u32 = 7;
pub(super) const FX_RAND_CH_SPAWN_ORIGIN_Z: u32 = 8;
pub(super) const FX_RAND_CH_SPAWN_OFFSET_YAW: u32 = 9;
pub(super) const FX_RAND_CH_SPAWN_OFFSET_HEIGHT: u32 = 10;
pub(super) const FX_RAND_CH_SPAWN_OFFSET_RADIUS: u32 = 11;
pub(super) const FX_RAND_CH_SPAWN_ANGLES_PITCH: u32 = 12;
pub(super) const FX_RAND_CH_SPAWN_ANGLES_YAW: u32 = 13;
pub(super) const FX_RAND_CH_SPAWN_ANGLES_ROLL: u32 = 14;
pub(super) const FX_RAND_CH_GRAVITY: u32 = 15;
pub(super) const FX_RAND_CH_REFLECTION: u32 = 16;
/// Life span.
pub const FX_RAND_CH_LIFE: u32 = 17;
/// Spawn delay.
pub const FX_RAND_CH_DELAY: u32 = 18;
/// How many parts a one-shot spawns.
pub const FX_RAND_CH_ONESHOT_COUNT: u32 = 19;
pub(super) const FX_RAND_CH_EMIT_DIST: u32 = 20;
const FX_RAND_CH_VISUAL: u32 = 21;
pub(super) const FX_RAND_CH_ATLAS: u32 = 22;
/// Colour.
pub const FX_RAND_CH_COLOR: u32 = 23;
pub(super) const FX_RAND_CH_INITIAL_ROTATION: u32 = 24;
pub(super) const FX_RAND_CH_ROTATION_DELTA: u32 = 25;
/// First size channel.
pub const FX_RAND_CH_SIZE0: u32 = 26;
/// Scale.
pub const FX_RAND_CH_SCALE: u32 = 28;

/// Multiplier and increment of the linear generator that turns an effect's
/// start time into its seed.
const SEED_MUL: u32 = 0x343fd;
const SEED_ADD: u32 = 0x269ec3;

/// The table position for a seed and channel.
fn table_index(seed: u32, channel: u32) -> usize {
    (seed % FX_RAND_TABLE_MOD + channel) as usize
}

/// The float in `[0, 1)` for this seed and channel.
pub fn random_table_f32(seed: u32, channel: u32) -> f32 {
    FX_RANDOM_TABLE[table_index(seed, channel)]
}

/// The low 16 bits of the table entry for this seed and channel.
pub fn random_table_u16(seed: u32, channel: u32) -> u16 {
    FX_RANDOM_TABLE[table_index(seed, channel)].to_bits() as u16
}

/// Which of an element's visuals it shows. One visual, or none, is always
/// index 0; with several the choice is a 16-bit fraction of the count.
pub fn elem_visual_index(visual_count: u8, random_seed: u32) -> usize {
    let count = u32::from(visual_count);
    if count <= 1 {
        return 0;
    }
    let fraction = u32::from(random_table_u16(random_seed, FX_RAND_CH_VISUAL));
    (count.wrapping_mul(fraction) >> 16) as usize
}

/// The seed an effect gets from the time it began.
pub fn effect_random_seed_from_msec(msec_begin: i32) -> u16 {
    let scrambled = (msec_begin as u32)
        .wrapping_mul(SEED_MUL)
        .wrapping_add(SEED_ADD);
    (((scrambled >> 17).wrapping_mul(FX_RAND_TABLE_MOD)) >> 15) as u16
}

/// The seed one element of an effect uses: the effect's seed shifted by the
/// element's place in spawn order and by when it began.
pub fn elem_random_seed(effect_seed: u16, sequence: u8, msec_begin: i32) -> u32 {
    let shifted = u32::from(effect_seed)
        .wrapping_add(u32::from(sequence).wrapping_mul(0x128))
        .wrapping_add(msec_begin as u32);
    shifted % FX_RAND_TABLE_MOD
}

/// The seed a trail uses. The sequence is signed here, and wraps.
pub fn trail_random_seed(effect_seed: u16, sequence: i8) -> u32 {
    let shift = i32::from(sequence).wrapping_mul(0x128) as u32;
    u32::from(effect_seed).wrapping_add(shift) % FX_RAND_TABLE_MOD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_the_documented_length() {
        assert_eq!(FX_RANDOM_TABLE.len(), FX_RANDOM_TABLE_FLOATS);
    }

    #[test]
    fn the_highest_channel_on_the_last_row_stays_in_the_table() {
        let last_row = FX_RAND_TABLE_MOD - 1;
        assert!(table_index(last_row, FX_RAND_CH_SCALE) < FX_RANDOM_TABLE_FLOATS);
    }

    #[test]
    fn a_seed_wraps_around_the_table_rows() {
        assert_eq!(
            random_table_f32(3, FX_RAND_CH_LIFE),
            random_table_f32(3 + FX_RAND_TABLE_MOD, FX_RAND_CH_LIFE)
        );
    }

    #[test]
    fn one_visual_is_always_the_first() {
        assert_eq!(elem_visual_index(0, 77), 0);
        assert_eq!(elem_visual_index(1, 77), 0);
    }

    #[test]
    fn several_visuals_pick_inside_the_count() {
        for seed in 0..FX_RAND_TABLE_MOD {
            assert!(elem_visual_index(5, seed) < 5);
        }
    }

    #[test]
    fn element_seeds_stay_below_the_row_count() {
        for sequence in [0u8, 1, 200, 255] {
            assert!(elem_random_seed(0x1de, sequence, i32::MAX) < FX_RAND_TABLE_MOD);
        }
        assert!(trail_random_seed(0x1de, -128) < FX_RAND_TABLE_MOD);
    }

    #[test]
    fn effect_seed_for_time_zero_is_from_the_increment_alone() {
        let expected = (((SEED_ADD >> 17) * FX_RAND_TABLE_MOD) >> 15) as u16;
        assert_eq!(effect_random_seed_from_msec(0), expected);
    }
}
