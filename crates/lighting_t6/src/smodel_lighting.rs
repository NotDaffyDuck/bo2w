//! Shared numbers and bit tricks for lighting static models.
//!
//! A static model that needs lit texels gets an "entry" in the lighting
//! atlas. Its draw instance stores the entry plus one as a *handle*, so that
//! handle 0 can mean "none".

/// Bit 0 of a static model's flags: neither the sun nor spot lights cast a
/// shadow from it.
pub const STATIC_MODEL_FLAG_NO_CAST_SHADOW: u8 = 0x1;

/// A draw instance with this flag set has its lighting patched in later
/// rather than sampled on the spot.
const SMODEL_LIGHTING_DEFER_FLAG: u8 = 0x20;

/// Static models cache one index per level of detail.
pub const SMC_CACHE_INDEX_LODS: usize = 4;

/// Entry 0 is held back after the light walk, so assignment starts at 1.
pub(crate) const SMODEL_LIGHTING_RESERVED_ENTRY0_AFTER_WALK: u32 = 1;

/// How many frames an entry must sit unused before it may be taken over.
pub(crate) const SMODEL_LIGHTING_FREEABLE_AGE_FRAMES: i32 = 4;

/// Warning index for "more lit static models are visible than entries".
pub const SMODEL_LIGHTING_WARN_TOO_MUCH: u32 = 0xd;

/// Warning index for a dynamic model that could not get a lighting slot.
pub const MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED: u32 = 4;

/// The handle that stands for an atlas entry.
pub const fn model_lighting_handle_from_entry(entry: u16) -> u16 {
    entry.wrapping_add(1)
}

/// The atlas entry behind a handle; `None` for the empty handle.
pub(crate) const fn handle_entry(handle: u16) -> Option<u16> {
    if handle == 0 { None } else { Some(handle - 1) }
}

/// Whether a draw instance's flags ask for lighting to be patched in later.
pub(crate) const fn defers_lighting(flags: u8) -> bool {
    flags & SMODEL_LIGHTING_DEFER_FLAG != 0
}

/// A static model's place in the dirty bitmap: the word and the bit. Model 0
/// is the top bit of word 0.
pub(crate) const fn dirty_bit(smodel_index: u32) -> (usize, u32) {
    (
        (smodel_index >> 5) as usize,
        0x8000_0000 >> (smodel_index & 31),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_is_the_entry_plus_one() {
        assert_eq!(model_lighting_handle_from_entry(0), 1);
        assert_eq!(model_lighting_handle_from_entry(41), 42);
        assert_eq!(model_lighting_handle_from_entry(u16::MAX), 0);
    }

    #[test]
    fn the_empty_handle_has_no_entry() {
        assert_eq!(handle_entry(0), None);
        assert_eq!(handle_entry(1), Some(0));
        assert_eq!(handle_entry(u16::MAX), Some(u16::MAX - 1));
    }

    #[test]
    fn handles_and_entries_convert_both_ways() {
        for entry in [0u16, 1, 7, 4000, u16::MAX - 1] {
            assert_eq!(
                handle_entry(model_lighting_handle_from_entry(entry)),
                Some(entry)
            );
        }
    }

    #[test]
    fn only_the_defer_flag_defers_lighting() {
        assert!(defers_lighting(0x20));
        assert!(defers_lighting(0xff));
        assert!(!defers_lighting(0xdf));
        assert!(!defers_lighting(STATIC_MODEL_FLAG_NO_CAST_SHADOW));
    }

    #[test]
    fn dirty_bits_run_from_the_top_of_each_word() {
        assert_eq!(dirty_bit(0), (0, 0x8000_0000));
        assert_eq!(dirty_bit(1), (0, 0x4000_0000));
        assert_eq!(dirty_bit(31), (0, 1));
        assert_eq!(dirty_bit(32), (1, 0x8000_0000));
        assert_eq!(dirty_bit(100), (3, 0x0800_0000));
    }

    #[test]
    fn the_fixed_numbers_are_what_the_callers_expect() {
        assert_eq!(SMC_CACHE_INDEX_LODS, 4);
        assert_eq!(SMODEL_LIGHTING_WARN_TOO_MUCH, 0xd);
        assert_eq!(MODEL_LIGHTING_WARN_CACHE_ALLOC_FAILED, 4);
        assert_eq!(SMODEL_LIGHTING_RESERVED_ENTRY0_AFTER_WALK, 1);
        assert_eq!(SMODEL_LIGHTING_FREEABLE_AGE_FRAMES, 4);
    }
}
