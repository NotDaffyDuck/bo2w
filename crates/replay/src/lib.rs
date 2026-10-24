//! Recording a match and playing it back.
//!
//! A recording is the stream of frames the server sends, one per 50 ms tick,
//! behind a small header that says which match it belongs to. Playing one
//! back feeds those frames to the client exactly as a connection would, so
//! the client cannot tell a demo from a live match.
//!
//! * `names`, `identity` and `error`: where demos live, what a header says,
//!   and what can go wrong.
//! * `file`: the on-disk format (header, then length-prefixed frames).
//! * `session`: [`Recording`] and [`Playback`], whole matches in and out.
//! * `clip`: the ring of recent ticks and the folder a saved clip goes in.
//! * `plugin`: the Bevy systems that feed a recording, fill the ring and pace
//!   a playback.
//! * `ulid`: the ids clips are named by.

mod clip;
mod error;
mod file;
mod identity;
mod names;
mod plugin;
mod session;
mod ulid;

#[cfg(test)]
mod testdir;
#[cfg(test)]
mod testing;

pub use clip::{
    CLIP_BYTE_BUDGET, CLIP_DEMO_FILE, CLIP_DUMP_FILE, CLIP_LATEST, CLIP_MANIFEST_FILE, CLIP_MS,
    CLIP_TICK_MS, CLIP_TICKS, ClipRing, clip_demo_path, clip_dir, clip_manifest, clips_dir,
    create_clip_dir, existing_clip_demo, resolve_playback_path, rewrite_latest_symlink,
};
pub use error::ReplayError;
pub use file::{FileTransport, MAGIC, RecordReader, RecordWriter};
pub use identity::{MatchRecordIdentity, ZONE_FIELD_LEN};
pub use names::{auto_demo_name, demo_path, demo_stem, next_free_demo_name, sanitize_demo_name};
pub use plugin::{
    PendingReplayArm, ReplayDiagnostics, ReplayPlayback, ReplayPlugin, ReplaySession,
};
pub use session::{Playback, PlayedTick, Recording};
pub use ulid::{UlidError, new_ulid};
