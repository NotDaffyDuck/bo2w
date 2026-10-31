//! The server's rolling record of recent snapshots, kept so a kill cam can replay what happened
//! some seconds ago.

mod frame;
mod lookup;
mod store;

pub use frame::StoredHistoryFrame;
pub use lookup::HistoryQueryResult;
pub use store::{HistoryStoreStats, SnapshotVault};

use crate::host_tick::peer_mail::HOST_STEP_MILLIS;

/// Milliseconds between archived frames: one per authority tick.
pub const VAULT_STEP_MS: i32 = HOST_STEP_MILLIS;

/// Client slots whose cached snapshot the archive is sized around.
pub const HISTORY_CACHED_RECEIVER_LIMIT: usize = 0x144;

/// Frames kept: 60 seconds at 20 Hz.
pub const HISTORY_FRAME_CAPACITY: usize = killcam_t6::HISTORY_FRAMES;

/// Bytes kept, by the estimate in [`StoredHistoryFrame::bytes`]. Our frames are our own encoding, so
/// this stays generous until their real size is measured.
pub const HISTORY_SIZE_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// How far back a lookup can reach, in frames.
pub const ARCHIVE_LOOKUP_WINDOW_TICKS: usize = HISTORY_FRAME_CAPACITY;

/// How many frames the archive can hold for `_receivers` clients; the frame limit does not depend
/// on the client count.
pub const fn archive_attainable_ticks(_receivers: usize) -> usize {
    HISTORY_FRAME_CAPACITY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_the_frame_limit_whatever_the_client_count() {
        assert_eq!(ARCHIVE_LOOKUP_WINDOW_TICKS, HISTORY_FRAME_CAPACITY);
        assert_eq!(archive_attainable_ticks(0), HISTORY_FRAME_CAPACITY);
        assert_eq!(archive_attainable_ticks(18), HISTORY_FRAME_CAPACITY);
    }

    #[test]
    fn sixty_seconds_of_frames() {
        assert_eq!(HISTORY_FRAME_CAPACITY as i32 * VAULT_STEP_MS, 60_000);
    }
}
