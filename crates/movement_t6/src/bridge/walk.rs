//! Everything a walking move needs besides the player and the frame.

use crate::bridge::{AirborneStepArgs, Bo2Feel, Bo2Lunge, StrideScaleArgs, LeapKickoffArgs};

/// The tuning a walk step reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootedStepArgs {
    /// How the stick or keys scale into a speed.
    pub cmd_scale: StrideScaleArgs,
    /// The weapon's own walking-speed factor.
    pub gun_carry_pace: f32,
    /// The buttons held on the previous command.
    pub last_tick_buttons: u32,
    /// How a jump starts.
    pub jump: LeapKickoffArgs,
    /// How the air move behaves if the step turns into a fall.
    pub air: AirborneStepArgs,
    /// Black Ops II's perk-driven movement feel.
    pub feel: Bo2Feel,
    /// bo2mp: `Some` while the knife lunge runs (the T6 weapon brain); its
    /// friction replaces the ground's for as long as it lasts.
    pub lunge: Option<Bo2Lunge>,
}
