//! The sizes of the effect pools and the handle arithmetic that goes with
//! them.
//!
//! A handle is a byte offset into its pool divided by four, held in 16 bits.

/// Bytes in one effect slot.
pub const FX_EFFECT_SLOT_SIZE: usize = 0x90;
/// How many effects can be alive at once.
pub const FX_EFFECT_POOL_CAPACITY: u32 = 0x400;
/// Bytes in one element's runtime record.
pub const FX_ELEM_RUNTIME_STRIDE: usize = 0x28;
/// How many elements can be alive at once. Black Ops II's own limit, so a
/// full pool refuses new elements the way the game does. A handle holds the
/// byte offset over four in 16 bits, which allows at most 6,553 slots of this
/// size.
pub const FX_ELEM_POOL_CAPACITY: u32 = crate::pool::FX_ELEM_LIMIT;
/// Bytes in one trail's runtime record.
pub const FX_TRAIL_RUNTIME_STRIDE: usize = 0x10;
/// How many trails can be alive at once.
pub const FX_TRAIL_POOL_CAPACITY: u32 = 0x80;
/// Bytes in one trail element's runtime record.
pub const FX_TRAIL_ELEM_RUNTIME_STRIDE: usize = 0x20;
/// How many trail elements can be alive at once.
pub const FX_TRAIL_ELEM_POOL_CAPACITY: u32 = 0x800;
/// How many handles the effect handle ring holds.
pub const FX_EFFECT_HANDLE_RING_SIZE: u32 = 0x400;
/// The modulus the random table index wraps at.
pub const FX_RAND_TABLE_MOD: u32 = 0x1df;
/// The bolt value that means a played effect is not attached to anything.
pub const FX_PLAY_BOLT_NONE: u32 = 0x7ff;
/// The entity number of the world.
pub const FX_ENTITYNUM_WORLD: u32 = entity_t6::ENTITYNUM_WORLD as u32;
/// The bolt value that means a spawned effect is not attached to anything.
pub const FX_SPAWN_BOLT_NONE: u32 = 0xfff;
/// How many spot lights the effects may light at once.
pub const FX_SPOT_LIGHT_LIMIT: i32 = 1;
/// The warning raised when the element pool is full.
pub const FX_WARN_ELEM_LIMIT: u32 = 0x31;

const fn handle_of(byte_offset: u32) -> u16 {
    (byte_offset >> 2) as u16
}

/// The handle of effect slot `slot`.
pub const fn effect_handle_for_slot(slot: u32) -> u16 {
    handle_of(slot.wrapping_mul(FX_EFFECT_SLOT_SIZE as u32))
}

/// The handle of the element `byte_delta` bytes into the element pool.
pub const fn elem_handle_from_ptr_delta(byte_delta: u32) -> u16 {
    handle_of(byte_delta)
}

/// The handle of trail slot `slot`.
pub const fn trail_handle_for_slot(slot: u32) -> u16 {
    handle_of(slot.wrapping_mul(FX_TRAIL_RUNTIME_STRIDE as u32))
}

/// The handle of trail element slot `slot`.
pub const fn trail_elem_handle_for_slot(slot: u32) -> u16 {
    handle_of(slot.wrapping_mul(FX_TRAIL_ELEM_RUNTIME_STRIDE as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_handles_are_the_byte_offset_over_four() {
        assert_eq!(effect_handle_for_slot(0), 0);
        assert_eq!(effect_handle_for_slot(1), 0x24);
        assert_eq!(trail_handle_for_slot(3), 12);
        assert_eq!(trail_elem_handle_for_slot(2), 16);
        assert_eq!(elem_handle_from_ptr_delta(0x28), 10);
    }

    #[test]
    fn a_handle_that_overflows_sixteen_bits_wraps() {
        assert_eq!(elem_handle_from_ptr_delta(0x4_0000), 0);
        assert_eq!(elem_handle_from_ptr_delta(0x4_0004), 1);
    }

    #[test]
    fn the_element_pool_fits_the_sixteen_bit_handle() {
        let furthest = FX_ELEM_POOL_CAPACITY as usize * FX_ELEM_RUNTIME_STRIDE / 4;
        assert!(furthest <= usize::from(u16::MAX));
    }

    #[test]
    fn the_effect_ring_is_a_power_of_two_the_size_of_the_pool() {
        assert!(FX_EFFECT_HANDLE_RING_SIZE.is_power_of_two());
        assert_eq!(FX_EFFECT_HANDLE_RING_SIZE, FX_EFFECT_POOL_CAPACITY);
    }
}
