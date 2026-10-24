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
mod names;
mod usercmd;
pub mod weapon_select;

#[cfg(test)]
mod tests;

/// Key numbers a client tracks (the board, mouse and pad buttons).
pub const KEY_COUNT: usize = 256;

pub use buttons::KbuttonSet;
pub use client::{ClientInput, Holds, input_cmd, key_event, lower_key_held, set_ads};
pub use held::{Kbutton, KeyState};
pub use look::{
    ANGLE2SHORT, AdjustAnglesInput, CL_ANGLESPEEDKEY_DEFAULT, CL_PITCHSPEED_DEFAULT,
    CL_YAWSPEED_DEFAULT, adjust_angles, apply_mouse_sensitivity, mouse_move_angles,
};
pub use names::{
    HOLD_PAIR_LIMIT, INPUT_COMMAND_NAMES, INVENTORY_COMMAND, SCRIPT_KEYNUM,
    WEAPNEXT_INVENTORY_DOWN, command_id_lookup, command_name, command_names, key_up_command_id,
};
pub use usercmd::{
    CreateCmdInput, MoveAxes, PITCH_STEP_MAX, axis_to_move, clamp_char, create_cmd,
    pad_move_stretched, sample_move,
};
