//! Pool sizes and hard limits of BO2's effects system.

pub const FX_EFFECT_LIMIT: u32 = 1024;

pub const FX_EFFECT_HANDLE_MASK: u32 = 0x3ff;

/// The effect ring starts warning about priority at this many live effects.
pub const FX_EFFECT_PRIORITY_WARNING_AT: u32 = 1014;

pub const FX_EFFECT_SLOT_SIZE: usize = 0x110;

pub const FX_ELEM_LIMIT: u32 = 2048;

/// 44 bytes of elem plus a 4-byte link.
pub const FX_ELEM_SIZE: usize = 48;

pub const FX_ELEM_LINK_SIZE: usize = 4;

/// End of the elem free list.
pub const FX_ELEM_FREE_END: u16 = 0x7ff;

/// Elem handles count in units of 12 bytes into the pool.
pub const FX_ELEM_HANDLE_UNIT: usize = 12;

/// The elem warning fires once more than this many elems are active.
pub const FX_ELEM_WARNING_ABOVE: u32 = 2018;

pub const FX_TRAIL_LIMIT: u32 = 128;

pub const FX_TRAIL_ELEM_LIMIT: u32 = 2048;

pub const FX_VIS_BLOCKER_BUFFERS: usize = 2;

pub const FX_VIS_BLOCKER_LIMIT: usize = 256;

pub const FX_UNIQUE_HANDLE_LIMIT: usize = 1024;

pub const FX_DEFERRED_RING: usize = 128;

/// A deferred spawn is refused once more than this many are queued.
pub const FX_DEFERRED_QUEUED_LIMIT: usize = 511;

/// Only one spot-light effect may be active at a time.
pub const FX_SPOT_LIGHT_LIMIT: u32 = 1;

pub const FX_MAX_VISUALS: usize = 32;

pub const FX_MAX_DECAL_VISUALS: usize = 16;

pub const FX_RESTART_LIST: usize = 32;

/// Looping spawns catch up at most this much backlog per update.
pub const FX_LOOPING_CATCH_UP_MS: i32 = 128;

pub const FX_TOTAL_BUFFER_BYTES: usize = 0x72d00;

pub const FX_SYSTEM_BYTES: usize = 0x380;

pub const FX_SYSTEM_SHARED_BYTES: usize = 0x860;

/// Effects between the first active and the first free handle of the ring.
#[inline]
pub fn effect_ring_distance(first_active: u32, first_free: u32) -> u32 {
    first_free.wrapping_sub(first_active)
}

/// A new effect fails to spawn once the ring holds 1024.
#[inline]
pub fn effect_ring_full(first_active: u32, first_free: u32) -> bool {
    effect_ring_distance(first_active, first_free) >= FX_EFFECT_LIMIT
}

#[inline]
pub fn effect_ring_priority_warning(first_active: u32, first_free: u32) -> bool {
    effect_ring_distance(first_active, first_free) >= FX_EFFECT_PRIORITY_WARNING_AT
}

#[inline]
pub fn effect_slot(handle: u32) -> usize {
    (handle & FX_EFFECT_HANDLE_MASK) as usize
}

#[inline]
pub fn elem_handle_from_index(index: usize) -> u32 {
    (index * FX_ELEM_SIZE / FX_ELEM_HANDLE_UNIT) as u32
}

#[inline]
pub fn elem_index_from_handle(handle: u32) -> usize {
    handle as usize * FX_ELEM_HANDLE_UNIT / FX_ELEM_SIZE
}

#[inline]
pub fn elem_warning(active_elems: u32) -> bool {
    active_elems > FX_ELEM_WARNING_ABOVE
}

#[inline]
pub fn deferred_spawn_refused(queued: usize) -> bool {
    queued > FX_DEFERRED_QUEUED_LIMIT
}

#[inline]
pub fn max_visuals(is_decal: bool) -> usize {
    if is_decal {
        FX_MAX_DECAL_VISUALS
    } else {
        FX_MAX_VISUALS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elem_pool_fits_its_handles() {
        assert_eq!(FX_ELEM_SIZE, 44 + FX_ELEM_LINK_SIZE);
        assert_eq!(elem_handle_from_index(1), 4);
        assert_eq!(elem_index_from_handle(elem_handle_from_index(2047)), 2047);
    }

    #[test]
    fn effect_ring_limits() {
        assert!(!effect_ring_full(10, 10 + 1023));
        assert!(effect_ring_full(10, 10 + 1024));
        assert!(effect_ring_full(u32::MAX - 5, 1024 - 6));
        assert!(!effect_ring_priority_warning(0, 1013));
        assert!(effect_ring_priority_warning(0, 1014));
        assert_eq!(effect_slot(0x400 + 7), 7);
    }

    #[test]
    fn warnings_and_refusals() {
        assert!(!elem_warning(2018));
        assert!(elem_warning(2019));
        assert!(!deferred_spawn_refused(511));
        assert!(deferred_spawn_refused(512));
        assert_eq!(max_visuals(true), 16);
        assert_eq!(max_visuals(false), 32);
    }
}
