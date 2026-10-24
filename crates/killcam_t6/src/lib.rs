//! Black Ops II's killcam. The server replays its own past: script sets a dead client's
//! archivetime and killcam entity, the server builds that client's snapshots from the archived
//! frame that far back, and the client switches to the killcam view while the playerstate's
//! killcam flag is non-zero (killcam_begin and killcam_end fire on the edges).
//!
//! What differs from MW2: a 2 MiB archive buffer (MW2 32 MiB), 12 killcam entity types with
//! per-type camera tuning, a 300 ms blend between camera entities with a capsule trace, and
//! BO2's own grenade and entity weapon lists. The script timings live in _killcam.gsc's
//! bytecode, which was not decoded; `script` keeps MW2's values until they are.
#![forbid(unsafe_code)]

pub mod archive;
pub mod camera;
pub mod script;

pub use archive::{
    ARCHIVE_BUFFER_BYTES, ARCHIVE_FRAMES, SERVER_FPS, SERVER_FRAME_MS, SNAPSHOT_TIME_CACHE,
    frame_still_archived, replay_frame,
};
pub use camera::{BLEND_MS, CamTuning, ExplosiveCam, FollowCam, KillcamEntityType, blend_view};
