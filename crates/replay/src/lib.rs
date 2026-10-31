//! Demos: saving a match to disk and showing it again.
//!
//! On disk a demo is a short header naming the match, then every frame the
//! server sent, one per 50 ms tick. Showing it hands those frames to the
//! client the way a live connection would, so nothing downstream needs to
//! know it is watching a demo.
//!
//! - `names`, `identity`, `error`: demo locations, the header, failures.
//! - `file`: reading and writing the header and the framed stream.
//! - `demo`: whole matches, recorded ([`Recording`]) and shown ([`Playback`]).
//! - `clip`: the rolling window of recent ticks and saved clip folders.
//! - `plugin`: Bevy systems that record, fill the window and pace a showing.
//! - `ulid`: sortable ids for clip folders.

mod error;
mod identity;
mod names;

mod file;
mod demo;

mod clip;
mod plugin;
mod ulid;

#[cfg(test)]
mod testdir;
#[cfg(test)]
mod testing;

pub use error::*;
pub use identity::*;
pub use names::*;

pub use file::*;
pub use demo::*;

pub use clip::*;
pub use plugin::*;
pub use ulid::*;
