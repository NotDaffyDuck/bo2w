//! Black Ops II's killcam. The server replays its own past: script sets a dead client's
//! archivetime and killcam entity, the server builds that client's snapshots from the archived
//! frame that far back, and the client switches to the killcam view while the playerstate's
//! killcam flag is non-zero (killcam_begin and killcam_end fire on the edges).
//!
//! What differs from MW2: a 2 MiB archive buffer (MW2 32 MiB), 12 killcam entity types with
//! per-type camera tuning, and a 300 ms blend between camera entities with a capsule trace.
//! The killcam's timings and its grenade and entity weapon lists are BO2's own compiled
//! _killcam.gsc and _globallogic_player.gsc, which the scripts run themselves.
#![forbid(unsafe_code)]

pub mod snap_history;
pub mod kill_view;

pub use snap_history::{
    HISTORY_RING_BYTES, HISTORY_FRAMES, TICKS_PER_SECOND, TICK_MS, SNAP_TIME_SLOTS,
    frame_still_archived, replay_frame,
};
pub use kill_view::{BLEND_MS, CamTuning, ExplosiveCam, FollowCam, KillcamEntityType, blend_view};
