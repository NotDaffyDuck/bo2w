//! The bits and counts packed into an effect's status word.

/// The low bits of the reference count, the part a unique effect is judged
/// on.
pub const FX_STATUS_REF_COUNT_LOW_MASK: u32 = 0x1fff;
/// The whole reference count, pending-loop bit included.
pub const FX_STATUS_REF_COUNT_MASK: u32 = 0xffff;
/// The effect still has looping elements waiting to spawn.
pub const FX_STATUS_HAS_PENDING_LOOP_ELEMS: u32 = 0x8000;
/// The count of effects this effect owns, in the bits above the reference
/// count.
pub const FX_STATUS_OWNED_EFFECTS_MASK: u32 = 0x03ff_0000;

/// Whether the effect's only remaining reference is its own, so it is done
/// once that goes.
pub const fn status_is_unique_done(status: u32) -> bool {
    status & FX_STATUS_REF_COUNT_LOW_MASK == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_reference_is_unique_and_done() {
        assert!(status_is_unique_done(1));
        assert!(status_is_unique_done(0x4000_0001));
    }

    #[test]
    fn other_counts_are_not() {
        assert!(!status_is_unique_done(0));
        assert!(!status_is_unique_done(2));
        assert!(!status_is_unique_done(0x4000_8002));
    }

    #[test]
    fn the_owned_effect_count_sits_above_the_reference_count() {
        assert_eq!(FX_STATUS_OWNED_EFFECTS_MASK & FX_STATUS_REF_COUNT_MASK, 0);
    }

    #[test]
    fn the_pending_loop_bit_is_outside_the_low_count() {
        assert_eq!(
            FX_STATUS_HAS_PENDING_LOOP_ELEMS & FX_STATUS_REF_COUNT_LOW_MASK,
            0
        );
        assert_eq!(
            FX_STATUS_REF_COUNT_MASK,
            FX_STATUS_REF_COUNT_LOW_MASK | 0xe000
        );
    }
}
