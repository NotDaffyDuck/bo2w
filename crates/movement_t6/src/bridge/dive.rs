//! bo2zm: Black Ops II's dive to prone (the "dolphin dive"): the player
//! state flags it rides in, shared by the host and the local prediction.
//! The dive itself runs in Black Ops II's frame (`movement_t6`).

use playerstate_t6::game::PlayerState;

/// Black Ops II's dive bit (pm_flags 0x200000): set from take-off until the
/// slide ends. `jump_time` tells the two apart: 0 or above in the air, below
/// 0 sliding (Dtp_IsSliding).
pub const PMF_DIVE: u32 = 0x0020_0000;

/// A dive is under way: in the air or sliding.
pub fn active(ps: &PlayerState) -> bool {
    ps.pm_flags & PMF_DIVE != 0
}

/// BO2's Dtp_IsSliding: the landed dive slides.
pub fn sliding(ps: &PlayerState) -> bool {
    active(ps) && ps.jump_time < 0
}

/// In the air, diving (BG_CheckProne's dive test).
pub fn in_air(ps: &PlayerState) -> bool {
    active(ps) && ps.jump_time >= 0
}

/// BO2's `divetoprone` for scripts: from take-off until the slide ends.
/// PhD Flopper's blast and the dive notifies read this.
pub fn dive_to_prone(ps: &PlayerState) -> bool {
    active(ps)
}
