//! MW2's weapon anim event numbers, still written by [`crate::combat_facts::spawn_weapon_hand`].
//!
//! TODO(fact): MW2 numbers. The first-person view reads BO2's own anim (`weap_anim_t6`) through
//! [`crate::asset_slots`]; the old field this feeds goes with the MW2 player state.

pub const WEAP_ANIM_RESTART_BIT: u32 = 0x200;

pub mod weap_anim_event {
    pub const RAISE: u32 = 0xb;
}

pub fn start_weapon_anim(weap_anim: &mut i32, event: u32) {
    let old = *weap_anim as u32;
    *weap_anim = ((!old & WEAP_ANIM_RESTART_BIT) | event) as i32;
}
