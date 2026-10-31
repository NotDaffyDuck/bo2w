//! The kill cam: a dead player watches their death again through the killer's archived records.
//!
//! The server keeps a short history of every player's record. When a kill cam plays, each archived copy is
//! moved forward in time to "now", then turned into a seat for the watcher.

use super::flag_bits::{entity_flag_bits, secondary_state_bits};
use super::record::{NO_ENTITY_SLOT, AvatarState};

/// How the kill cam frames the kill: from the killer's eyes, or from what did the killing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DeathReplayStyle {
    /// Through the killer's own eyes.
    Mode0 = 0,
    HelicopterView = 1,
    AirstrikeView = 2,
    GuidedMissileView = 3,
    AltMissileView = 4,
    RocketView = 5,
    TurretView = 6,
    AntiTankView = 7,
    RemoteDeviceView = 8,
}

impl DeathReplayStyle {
    /// How long the camera glides to a new target, in milliseconds. A turret's camera swings slower.
    const fn crossfade_millis(self) -> i32 {
        match self {
            Self::TurretView => 900,
            _ => 300,
        }
    }
}

/// When a camera glide that starts now should end: zero (no glide) unless the camera switched target this frame,
/// and never on the frame the kill cam opens. Clamps instead of wrapping near the end of the clock.
pub fn killcam_blend_until_ms(on_enter_frame: bool, new_target: bool, mode: DeathReplayStyle, clock_millis: i32) -> i32 {
    if new_target && !on_enter_frame { clock_millis.saturating_add(mode.crossfade_millis()) } else { 0 }
}

/// True when a running kill cam films from something other than the killer's eyes, which needs the camera behind.
pub(super) fn filmed_from_outside(replaying_final_death: bool, mode: DeathReplayStyle) -> bool {
    replaying_final_death && !matches!(mode, DeathReplayStyle::Mode0)
}

/// The entity a gun kill's cam follows: none, so the camera stays with the killer.
pub const HITSCAN_KILL_CAM_ENTITY: i32 = NO_ENTITY_SLOT;

/// What a kill-cam seat looks at: the entity carrying the camera, the entity it turns towards, and the killer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpectatorTarget {
    pub death_replay_target: i32,
    pub death_replay_focus: i32,
    pub death_replay_player: i32,
}

impl SpectatorTarget {
    /// A gun kill: no camera entity, nothing to turn towards, just the killer.
    pub const fn hitscan(killer: i32) -> Self {
        Self {
            death_replay_focus: NO_ENTITY_SLOT,
            death_replay_target: HITSCAN_KILL_CAM_ENTITY,
            death_replay_player: killer,
        }
    }
}

/// Moves an archived record's clocks forward by `delta_ms` so it plays as if it were now. Clocks still at zero mean
/// "not running" and stay at zero; the gap itself always moves. All of it wraps like the game clock does.
pub fn rebase_archived_timers(
    ps: &mut AvatarState,
    delta_ms: i32,
) {
    let shift = |clock: &mut i32| {
        if *clock != 0 {
            *clock = clock.wrapping_add(delta_ms);
        }
    };
    shift(&mut ps.last_cmd_ms);
    shift(&mut ps.move_timer);
    shift(&mut ps.hop_start_ms);
    shift(&mut ps.eye_blend_clock);
    shift(&mut ps.stun_start);
    ps.time_slice = delta_ms.wrapping_add(ps.time_slice);
}

/// Builds the record the watcher sees for one kill-cam frame: the killer's archived copy, keeping the watcher's own
/// preserved entity bit, marked as a seat watched from behind instead of a live player, aimed at `focus`.
pub fn apply_killcam_seat(viewer: &AvatarState, archived: &AvatarState, focus: SpectatorTarget) -> AvatarState {
    let keep = entity_flag_bits::KILLCAM_PRESERVED;
    AvatarState {
        e_flags: (viewer.e_flags & keep) | (archived.e_flags & !keep),
        secondary_state_bits: secondary_state_bits::DEAD_KILLCAM_TPV | (archived.secondary_state_bits & !secondary_state_bits::PLAYER),
        death_replay_target: focus.death_replay_target,
        death_replay_focus: focus.death_replay_focus,
        death_replay_player: focus.death_replay_player,
        ..*archived
    }
}
