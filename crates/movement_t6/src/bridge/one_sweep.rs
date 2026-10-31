//! What one frame of player movement takes in and hands back. Black Ops II's
//! frame itself lives in `crate::pmove`.

use crate::bridge::{
    ScopeBlendArgs, ScopeWishArgs, AirborneStepArgs, LungeGunTimings, TickLocals, DashArgs,
    LookRotationLimit, FootedStepArgs,

};

/// A box sweep through the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepAsk {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub collide_filter: u32,
}

/// The player's box for this frame and what it collides with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullExtents {
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    pub collide_filter: u32,
}

/// What a frame leaves behind.
#[derive(Clone, Copy, Debug)]
pub struct MotionTickOutcome {
    pub pml: TickLocals,
    pub bounds: HullExtents,
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
pub struct OneTickMotionArgs {
    pub walk: FootedStepArgs,
    pub air: AirborneStepArgs,
    pub bounds: HullExtents,
    pub look_rotation: LookRotationLimit,
    pub sprint: DashArgs,
    pub ads_intent: ScopeWishArgs,
    pub ads_frac: ScopeBlendArgs,
    pub melee_charge: LungeGunTimings,
    pub knife_reach: f32,
    pub last_tick_buttons: u32,
    /// bo2zm: last frame's forward and side keys (BO2's oldcmd), for the
    /// prone aim hold.
    pub old_move: [i8; 2],
    /// The held weapon refuses to go prone.
    pub gun_forbids_lying: bool,
}
