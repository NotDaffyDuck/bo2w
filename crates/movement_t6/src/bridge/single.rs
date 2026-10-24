//! What one frame of player movement takes in and hands back. Black Ops II's
//! frame itself lives in `crate::pmove`.

use crate::bridge::{
    AdsFracContext, AdsIntentContext, AirMoveContext, MeleeChargeWeaponDelays, Pml, SprintContext,
    ViewAngleClamp, WalkMoveContext,
};

/// A box sweep through the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundTraceInput {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub tracemask: u32,
}

/// The player's box for this frame and what it collides with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveBounds {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub tracemask: u32,
}

/// What a frame leaves behind.
#[derive(Clone, Copy, Debug)]
pub struct PmoveResult {
    pub pml: Pml,
    pub bounds: MoveBounds,
    /// A stance event to raise, if the stance changed this frame.
    pub stance_event: Option<u8>,
    /// The torso animation restarts.
    pub reset_torso: bool,
    /// bo2zm: how far a stair step carried the player up or down this frame
    /// (Black Ops II's pmove_t viewChange); the camera eases over it.
    pub view_change: f32,
}

/// Every setting a frame of movement reads.
#[derive(Clone, Copy, Debug)]
pub struct PmoveSingleContext {
    pub walk: WalkMoveContext,
    pub air: AirMoveContext,
    pub bounds: MoveBounds,
    pub view_angles: ViewAngleClamp,
    pub sprint: SprintContext,
    pub ads_intent: AdsIntentContext,
    pub ads_frac: AdsFracContext,
    pub melee_charge: MeleeChargeWeaponDelays,
    pub player_melee_range: f32,
    pub old_buttons: u32,
    /// bo2zm: last frame's forward and side keys (BO2's oldcmd), for the
    /// prone aim hold.
    pub old_move: [i8; 2],
    /// The held weapon refuses to go prone.
    pub weapon_blocks_prone: bool,
}
