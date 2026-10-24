//! BO2's effect random numbers: 30 channels into one shared float table.

use crate::random_table::FX_RANDOM_TABLE;

pub const FX_RANDOM_TABLE_FLOATS: usize = 508;

/// Seeds run 0..479; the 29 entries after them let seed + channel wrap.
pub const FX_RAND_TABLE_MOD: u32 = 0x1df;

/// Table lookups at or past this index are an assertion in BO2.
pub const FX_RAND_INDEX_LIMIT: u32 = 0x1fc;

pub const FX_RAND_CHANNELS: u32 = 30;

pub const FX_RAND_VELOCITY_X: u32 = 0;
pub const FX_RAND_VELOCITY_Y: u32 = 1;
pub const FX_RAND_VELOCITY_Z: u32 = 2;
pub const FX_RAND_ANG_VEL_PITCH: u32 = 3;
pub const FX_RAND_ANG_VEL_YAW: u32 = 4;
pub const FX_RAND_ANG_VEL_ROLL: u32 = 5;
pub const FX_RAND_ORIGIN_X: u32 = 6;
pub const FX_RAND_ORIGIN_Y: u32 = 7;
pub const FX_RAND_ORIGIN_Z: u32 = 8;
pub const FX_RAND_OFFSET_YAW: u32 = 9;
pub const FX_RAND_OFFSET_HEIGHT: u32 = 10;
pub const FX_RAND_OFFSET_RADIUS: u32 = 11;
pub const FX_RAND_ANGLES_PITCH: u32 = 12;
pub const FX_RAND_ANGLES_YAW: u32 = 13;
pub const FX_RAND_ANGLES_ROLL: u32 = 14;
pub const FX_RAND_GRAVITY: u32 = 15;
pub const FX_RAND_REFLECTION: u32 = 16;
pub const FX_RAND_LIFE_SPAN: u32 = 17;
pub const FX_RAND_SPAWN_DELAY: u32 = 18;
pub const FX_RAND_SPAWN_COUNT: u32 = 19;
pub const FX_RAND_EMIT_DIST: u32 = 20;
pub const FX_RAND_VISUAL: u32 = 21;
pub const FX_RAND_TILE_START: u32 = 22;
pub const FX_RAND_COLOR: u32 = 23;
pub const FX_RAND_ROTATION: u32 = 24;
pub const FX_RAND_ROTATION_DELTA: u32 = 25;
pub const FX_RAND_SIZE0: u32 = 26;
pub const FX_RAND_SIZE1: u32 = 27;
pub const FX_RAND_SCALE: u32 = 28;
pub const FX_RAND_CLOUD_DENSITY: u32 = 29;

/// None where BO2 asserts (index at or past 0x1fc).
#[inline]
pub fn table_index(seed: u32, channel: u32) -> Option<usize> {
    let idx = seed.checked_add(channel)?;
    (idx < FX_RAND_INDEX_LIMIT).then_some(idx as usize)
}

#[inline]
fn entry(idx: usize) -> [u8; 4] {
    FX_RANDOM_TABLE[idx].to_le_bytes()
}

#[inline]
pub fn random_f32(seed: u32, channel: u32) -> Option<f32> {
    table_index(seed, channel).map(|i| f32::from_le_bytes(entry(i)))
}

/// The low 16 bits of the entry, which integer ranges use.
#[inline]
pub fn random_u16(seed: u32, channel: u32) -> Option<u16> {
    table_index(seed, channel).map(|i| {
        let b = entry(i);
        u16::from_le_bytes([b[0], b[1]])
    })
}

/// Table value times amplitude, plus base.
#[inline]
pub fn sample_float(base: f32, amplitude: f32, seed: u32, channel: u32) -> Option<f32> {
    random_f32(seed, channel).map(|r| r * amplitude + base)
}

/// (low16 times (amplitude + 1)) >> 16, plus base. Life span, delay and
/// spawn count use this.
#[inline]
pub fn sample_int(base: i32, amplitude: i32, seed: u32, channel: u32) -> Option<i32> {
    random_u16(seed, channel).map(|lo| {
        let scaled = (i64::from(lo) * (i64::from(amplitude) + 1)) >> 16;
        base.wrapping_add(scaled as i32)
    })
}

#[inline]
pub fn elem_seed(effect_seed: u16, sequence: u8, msec_begin: i32) -> u32 {
    u32::from(sequence)
        .wrapping_mul(0x128)
        .wrapping_add(u32::from(effect_seed))
        .wrapping_add(msec_begin as u32)
        % FX_RAND_TABLE_MOD
}

/// The seed of an effect that affects gameplay: the 0x343fd / 0x269ec3
/// generator stepped once from the spawn msec, bits 17 up, scaled to 0..478.
#[inline]
pub fn effect_seed_from_msec(msec_begin: i32) -> u16 {
    let x = (msec_begin as u32)
        .wrapping_mul(0x343fd)
        .wrapping_add(0x269ec3);
    ((((x >> 17) & 0x7fff) * FX_RAND_TABLE_MOD) >> 15) as u16
}

/// Any other effect's seed: a C library rand (0..=0x7fff) scaled to 0..478.
#[inline]
pub fn effect_seed_from_rand(rand: u16) -> u16 {
    (((u32::from(rand) & 0x7fff) * FX_RAND_TABLE_MOD) >> 15) as u16
}

/// Life span of an elem with a non-zero maxWindStrength: from the base life
/// towards lifespanAtMaxWind by the wind fraction.
// BO2 unknown: how the result is rounded to whole msec (see facts/QUESTIONS.md).
#[inline]
pub fn life_span_with_wind(base_life: f32, life_at_max_wind: f32, wind_fraction: f32) -> f32 {
    base_life + (life_at_max_wind - base_life) * wind_fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_every_seed_and_channel() {
        assert_eq!(FX_RANDOM_TABLE.len(), FX_RANDOM_TABLE_FLOATS);
        assert_eq!(FX_RAND_INDEX_LIMIT as usize, FX_RANDOM_TABLE_FLOATS);
        assert!(random_f32(FX_RAND_TABLE_MOD - 1, FX_RAND_CHANNELS - 1).is_some());
        assert!(random_f32(FX_RAND_INDEX_LIMIT, 0).is_none());
        assert!(random_f32(u32::MAX, 1).is_none());
    }

    #[test]
    fn samples() {
        let r = random_f32(7, FX_RAND_GRAVITY).unwrap();
        assert_eq!(
            sample_float(2.0, 3.0, 7, FX_RAND_GRAVITY),
            Some(r * 3.0 + 2.0)
        );
        assert_eq!(sample_int(100, 0, 7, FX_RAND_LIFE_SPAN), Some(100));
        let lo = i32::from(random_u16(7, FX_RAND_LIFE_SPAN).unwrap());
        assert_eq!(
            sample_int(100, 499, 7, FX_RAND_LIFE_SPAN),
            Some(100 + ((lo * 500) >> 16))
        );
    }

    #[test]
    fn seeds_stay_in_range() {
        for msec in [0, 1, 50, 123_456, -9, i32::MAX] {
            assert!(u32::from(effect_seed_from_msec(msec)) < FX_RAND_TABLE_MOD);
        }
        assert_eq!(effect_seed_from_rand(0), 0);
        assert_eq!(effect_seed_from_rand(0x7fff), 478);
        assert_eq!(elem_seed(5, 1, 10), 0x128 + 5 + 10);
        // 2 * 0x128 + 100 + 0 = 692, past 479
        assert_eq!(elem_seed(100, 2, 0), 692 - 479);
        assert!(elem_seed(478, 255, -1) < FX_RAND_TABLE_MOD);
    }

    #[test]
    fn wind_life() {
        assert_eq!(life_span_with_wind(1000.0, 500.0, 0.0), 1000.0);
        assert_eq!(life_span_with_wind(1000.0, 500.0, 1.0), 500.0);
        assert_eq!(life_span_with_wind(1000.0, 500.0, 0.5), 750.0);
    }
}
