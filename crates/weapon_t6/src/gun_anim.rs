//! MW2's weapon anim event numbers, still written by [`crate::combat_facts::spawn_weapon_hand`].
//!
//! TODO(fact): MW2 numbers. The first-person view reads BO2's own anim (`weap_anim_t6`) through
//! [`crate::asset_slots`]; the old field this feeds goes with the MW2 player state.

pub const GUN_CLIP_REPLAY_FLAG: u32 = 0x200;

pub mod gun_clip_cue {
    pub const LIFT: u32 = 0xb;
}

pub fn begin_gun_clip(anim_bits: &mut i32, cue: u32) {
    let prev = *anim_bits as u32;
    *anim_bits = ((!prev & GUN_CLIP_REPLAY_FLAG) | cue) as i32;
}
