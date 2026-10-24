//! The weapon animation slots and the rules for playing them, which
//! `weapon_t6` owns.

pub use weapon_t6::asset_slots::{
    ACTION_GOAL_TIME_SECS, ACTIVE_GOAL_WEIGHT, IDLE_INTERRUPT_GOAL_TIME_SECS, INACTIVE_GOAL_WEIGHT,
    WEAPON_ANIM_SLOTS, playback_rate, slot_uses_native_rate, weap_anim_extra,
};
