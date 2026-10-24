//! The state the running match keeps for each player between ticks, and the input a client sends each frame.
//!
//! This is the shape the simulation, the snapshot writer and the client view all share today. BO2's own
//! `playerState_s` lives at the crate root ([`crate::PlayerState`]); fields move over to it as the
//! remaining callers switch.
//!
//! - `record`: the per-player record, its blank value, and the entity numbers it refers to.
//! - `flag_bits`: the bit sets stored in that record, plus perk bits.
//! - `killcam`: replaying a victim's death through the killer's archived record.
//! - `chase_cam`: when the camera leaves the head, and where it goes.
//! - `command`: one frame of player input.

mod chase_cam;
mod command;
mod flag_bits;
mod killcam;
mod record;

pub use chase_cam::{
    CG_CAMERA_PULLBACK_BOX_HALF, CG_CAMERA_PULLBACK_CLIPMASK, CG_THIRD_PERSON_RANGE_DEFAULT, ChaseView,
    LINK_FLAGS_FORCE_THIRD_PERSON, OffsetThirdPersonViewInputs, ThirdPersonViewInputs, is_third_person_view,
    look_at_killer_yaw, offset_third_person_view, viewweapon_frame_runs,
};
pub use command::{UserCmd, buttons, last_input};
pub use flag_bits::{
    PERK_COLDBLOODED, PERK_HEARTBREAKER, PERK_PISTOLDEATH, PERK_QUIETER, PERK_SCAVENGER, eflags, mantle_flags,
    other_flags, pm_flags, weap_flags,
};
pub use killcam::{
    HITSCAN_KILL_CAM_ENTITY, KillCamMode, SeatFocus, apply_killcam_seat, killcam_lerp_deadline_ms,
    rebase_archived_timers,
};
pub use record::{
    AnimPair, ENTITYNUM_NONE, GENTITY_SPAWN_BASE, MAX_CLIENT_CORPSES, PLAYER_CORPSE_ENTITY_BASE, PM_TYPE_DEAD,
    PM_TYPE_DEAD_LINKED, PM_TYPE_INTERMISSION, PM_TYPE_LAST_STAND, PM_TYPE_NORMAL_LINKED, PM_TYPE_SPECTATOR,
    PlayerState, get_viewmodel_weapon_index,
};
