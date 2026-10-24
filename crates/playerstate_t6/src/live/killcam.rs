//! The kill cam: a dead player watches their death again through the killer's archived records.
//!
//! The server keeps a short history of every player's record. When a kill cam plays, each archived copy is
//! moved forward in time to "now", then turned into a seat for the watcher.

use super::flag_bits::{eflags, other_flags};
use super::record::{ENTITYNUM_NONE, PlayerState};

/// How the kill cam frames the kill: from the killer's eyes, or from what did the killing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KillCamMode {
    /// Through the killer's own eyes.
    Mode0 = 0,
    Mode1Heli = 1,
    Mode2Airstrike = 2,
    Mode3Missile = 3,
    Mode4MissileAlt = 4,
    Mode5Rocket = 5,
    Mode6Turret = 6,
    Mode7Javelin = 7,
    Mode8Remote = 8,
}

impl KillCamMode {
    /// How long the camera glides to a new target, in milliseconds. A turret's camera swings slower.
    const fn blend_ms(self) -> i32 {
        match self {
            Self::Mode6Turret => 900,
            _ => 300,
        }
    }
}

/// When a camera glide that starts now should end: zero (no glide) unless the camera switched target this frame,
/// and never on the frame the kill cam opens. Clamps instead of wrapping near the end of the clock.
pub fn killcam_lerp_deadline_ms(on_enter_frame: bool, focus_changed: bool, mode: KillCamMode, now_ms: i32) -> i32 {
    if focus_changed && !on_enter_frame { now_ms.saturating_add(mode.blend_ms()) } else { 0 }
}

/// True when a running kill cam films from something other than the killer's eyes, which needs the camera behind.
pub(super) fn filmed_from_outside(in_killcam: bool, mode: KillCamMode) -> bool {
    in_killcam && !matches!(mode, KillCamMode::Mode0)
}

/// The entity a gun kill's cam follows: none, so the camera stays with the killer.
pub const HITSCAN_KILL_CAM_ENTITY: i32 = ENTITYNUM_NONE;

/// What a kill-cam seat looks at: the entity carrying the camera, the entity it turns towards, and the killer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatFocus {
    pub kill_cam_entity: i32,
    pub kill_cam_look_at_entity: i32,
    pub kill_cam_client_num: i32,
}

impl SeatFocus {
    /// A gun kill: no camera entity, nothing to turn towards, just the killer.
    pub const fn hitscan(attacker_client_num: i32) -> Self {
        Self {
            kill_cam_client_num: attacker_client_num,
            kill_cam_entity: HITSCAN_KILL_CAM_ENTITY,
            kill_cam_look_at_entity: ENTITYNUM_NONE,
        }
    }
}

/// Moves an archived record's clocks forward by `delta_ms` so it plays as if it were now. Clocks still at zero mean
/// "not running" and stay at zero; the gap itself always moves. All of it wraps like the game clock does.
pub fn rebase_archived_timers(ps: &mut PlayerState, delta_ms: i32) {
    let shift = |clock: &mut i32| {
        if *clock != 0 {
            *clock = clock.wrapping_add(delta_ms);
        }
    };
    shift(&mut ps.command_time);
    shift(&mut ps.pm_time);
    shift(&mut ps.jump_time);
    shift(&mut ps.view_height_lerp_time);
    shift(&mut ps.shellshock_time);
    ps.delta_time = ps.delta_time.wrapping_add(delta_ms);
}

/// Builds the record the watcher sees for one kill-cam frame: the killer's archived copy, keeping the watcher's own
/// preserved entity bit, marked as a seat watched from behind instead of a live player, aimed at `focus`.
pub fn apply_killcam_seat(viewer: &PlayerState, archived: &PlayerState, focus: SeatFocus) -> PlayerState {
    let keep = eflags::KILLCAM_PRESERVED;
    PlayerState {
        e_flags: (viewer.e_flags & keep) | (archived.e_flags & !keep),
        other_flags: other_flags::DEAD_KILLCAM_TPV | (archived.other_flags & !other_flags::PLAYER),
        kill_cam_entity: focus.kill_cam_entity,
        kill_cam_look_at_entity: focus.kill_cam_look_at_entity,
        kill_cam_client_num: focus.kill_cam_client_num,
        ..*archived
    }
}
