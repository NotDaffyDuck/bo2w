//! Black Ops II client input. Keys go down and up; their bound commands hold game buttons,
//! latch a stance, queue weapon switches and run the timed holds (stance press to prone or
//! dive, switch-weapon key to the inventory weapon, use double tap, fire jam). Each frame the
//! buttons, the key move and the turn are sampled into one user command.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod buttons;
mod client;
mod held;
mod look;
mod bind_ids;
mod intent_cmd;
pub mod gun_cycle;

#[cfg(test)]
mod tests;

/// Key numbers a client tracks (the board, mouse and pad buttons).
pub const KEYBOARD_SLOTS: usize = 256;

pub use buttons::HeldKeyLatchBank;
pub use client::{PlayerIntentFeed, Holds, input_cmd, key_event, lower_key_held, set_ads};
pub use held::{HeldKeyLatch, KeyPressRecord};
pub use look::{
    DEGREES_TO_WORD16, LookTurnRequest, FAST_TURN_KEY_RATIO, KEY_TILT_RATE,
    KEY_TURN_RATE, adjust_angles, scale_pointer_motion, pointer_turn_units,
};
pub use bind_ids::{
    HOLD_ID_END, BIND_COMMAND_TABLE, INVENTORY_COMMAND, SCRIPTED_KEY_SLOT,
    WEAPNEXT_INVENTORY_DOWN, action_number_for, action_label, action_labels, release_action_number,
};
pub use intent_cmd::{
    IntentBuildRequest, StrideBasis, PITCH_STEP_MAX, stick_to_thrust, clamp_char, build_input_command,
    pad_move_stretched, read_move_buttons,
};
