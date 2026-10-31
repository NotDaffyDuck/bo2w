//! One live part of an effect, and how pool slots map to the 16-bit handles
//! that link the parts together.

use fx_t6::runtime::{
    PARTICLE_NEVER_SETTLED, PARTICLE_BANK_LIMIT, PARTICLE_LIVE_RECORD_BYTES, FacingBirthKnobs,
    elem_handle_from_ptr_delta,
};

/// Ends a handle chain; no slot maps to it.
pub const NO_PARTICLE_TICKET: u16 = 0xffff;

#[derive(Clone, Debug)]
pub struct ParticleBinSlot {
    pub is_taken: bool,
    pub definition_slot: u8,
    pub part_kind: u8,
    pub bits: i32,
    pub look_variant_total: u8,
    pub nth: u8,
    /// Where it came to rest along its life, or `PARTICLE_NEVER_SETTLED`.
    pub settled_share: u8,
    pub emission_carry: u8,
    /// Neighbours in the owning effect's list.
    pub following_particle_ticket: u16,
    pub preceding_particle_ticket: u16,
    pub window_open_ms: i32,
    pub lifespan_window_ms: i32,
    pub launch_speed_vec: [f32; 3],
    pub at_point: [f32; 3],
    pub birth_point: [[f32; 2]; 3],
    pub birth_ring_reach: [f32; 2],
    pub birth_lift: [f32; 2],
    pub parent_emitter_slot: u16,
    pub category_slot: u8,
    pub paint_rank: u8,
}

impl Default for ParticleBinSlot {
    fn default() -> Self {
        Self {
            is_taken: false,
            definition_slot: 0,
            part_kind: 0,
            bits: 0,
            look_variant_total: 0,
            nth: 0,
            settled_share: PARTICLE_NEVER_SETTLED,
            emission_carry: 0,
            following_particle_ticket: NO_PARTICLE_TICKET,
            preceding_particle_ticket: NO_PARTICLE_TICKET,
            window_open_ms: 0,
            lifespan_window_ms: 0,
            launch_speed_vec: [0.0; 3],
            at_point: [0.0; 3],
            birth_point: [[0.0; 2]; 3],
            birth_ring_reach: [0.0; 2],
            birth_lift: [0.0; 2],
            parent_emitter_slot: 0,
            category_slot: 0,
            paint_rank: 0,
        }
    }
}

impl ParticleBinSlot {
    /// The spawn-time numbers that place this part inside its effect's frame.
    #[inline]
    pub fn spawn_facing(&self, seed: u32) -> FacingBirthKnobs {
        FacingBirthKnobs {
            birth_point: self.birth_point,
            birth_ring_reach: self.birth_ring_reach,
            birth_lift: self.birth_lift,
            seed,
        }
    }
}

/// Handles count 4-byte units from the pool start.
#[inline]
pub fn particle_ticket_of_bin(slot: u32) -> u16 {
    elem_handle_from_ptr_delta(slot.wrapping_mul(PARTICLE_LIVE_RECORD_BYTES as u32))
}

/// The pool slot a handle names, or `None` for the end marker, a handle that
/// does not sit on a slot boundary, or one past the pool.
#[inline]
pub fn particle_bin_of_ticket(handle: u16) -> Option<usize> {
    let stride = PARTICLE_LIVE_RECORD_BYTES as u32;
    let offset = u32::from(handle) * 4;
    let on_slot = handle != NO_PARTICLE_TICKET && offset % stride == 0;
    on_slot
        .then(|| (offset / stride) as usize)
        .filter(|&slot| slot < PARTICLE_BANK_LIMIT as usize)
}

/// Draw class by part type for types 0..=9; later types are class 1.
const CLASS_BY_TYPE: [Option<usize>; 10] = [
    Some(0), Some(0), Some(0), Some(0), // sprites and the like
    Some(2), Some(2),
    Some(1), Some(1), Some(1),
    None, // type 9 is never drawn by class
];

/// Which of the three draw classes a part type belongs to: types 0 to 3 are
/// class 0, types 4 and 5 are class 2, the rest class 1. Type 9 has none.
pub fn particle_category_of_shape(elem_type: u8) -> Option<usize> {
    CLASS_BY_TYPE.get(usize::from(elem_type)).copied().unwrap_or(Some(1))
}
