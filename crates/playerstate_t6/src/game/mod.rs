//! bo2mp bridge: the live game's player state, kept here while its
//! fields move to BO2's `playerState_s` ([`crate::PlayerState`]).
//! Fields BO2 has already use BO2's shapes (sprint, mantle, the event
//! ring); the rest go after the MW2 crates are gone.

mod archive;
mod chrome;
mod playerstate;
mod seat;
mod third_person;
mod usercmd;

pub use chrome::{KillCamMode, killcam_lerp_deadline_ms};
pub use playerstate::{
    AnimPair, ENTITYNUM_NONE, PERK_COLDBLOODED, PERK_HEARTBREAKER, PERK_PISTOLDEATH, PERK_QUIETER,
    PERK_SCAVENGER, PlayerState, eflags, get_viewmodel_weapon_index, mantle_flags, other_flags,
    pm_flags, weap_flags,
};
pub use seat::{HITSCAN_KILL_CAM_ENTITY, SeatFocus, apply_killcam_seat, rebase_archived_timers};
pub use third_person::{
    CG_CAMERA_PULLBACK_BOX_HALF, CG_CAMERA_PULLBACK_CLIPMASK, CG_THIRD_PERSON_RANGE_DEFAULT,
    GENTITY_SPAWN_BASE, LINK_FLAGS_FORCE_THIRD_PERSON,
    MAX_CLIENT_CORPSES, OffsetThirdPersonViewInputs, PLAYER_CORPSE_ENTITY_BASE, PM_TYPE_DEAD,
    PM_TYPE_DEAD_LINKED, PM_TYPE_INTERMISSION, PM_TYPE_LAST_STAND, PM_TYPE_NORMAL_LINKED,
    PM_TYPE_SPECTATOR, ThirdPersonViewInputs, is_third_person_view,
    look_at_killer_yaw, offset_third_person_view, viewweapon_frame_runs,
};
pub use usercmd::{UserCmd, buttons, last_input};
