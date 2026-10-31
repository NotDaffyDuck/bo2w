//! The server's snapshot archive the killcam replays from.

/// The server runs at 20 frames a second (asserted as 1000 / 50).
pub const TICKS_PER_SECOND: i64 = 20;
pub const TICK_MS: i64 = 1000 / TICKS_PER_SECOND;

/// The archive frame table: 1200 slots of (start offset, size), 60 s at 20 Hz.
pub const HISTORY_FRAMES: usize = 1200;

/// ARCHIVED_SNAPSHOT_BUFFER_SIZE: the archive's bytes also bound how far back it reaches.
pub const HISTORY_RING_BYTES: usize = 0x200000;

/// The snapshot time cache.
pub const SNAP_TIME_SLOTS: usize = 16;

/// The archived frame a client with `archive_time_ms` replays (archivetime is set by script
/// in seconds and kept in milliseconds).
#[must_use]
pub fn replay_frame(current_frame: i64, archive_time_ms: i64) -> i64 {
    current_frame - archive_time_ms * TICKS_PER_SECOND / 1000
}

/// Whether `frame` is still in the frame table; the oldest is `next_frame - 1200`. When it is
/// gone (or its bytes were overwritten) the client's archivetime resets to 0 and it plays live.
#[must_use]
pub fn frame_still_archived(next_frame: i64, frame: i64) -> bool {
    frame < next_frame && frame >= next_frame - HISTORY_FRAMES as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_seconds_back_is_a_hundred_frames() {
        assert_eq!(replay_frame(5000, 5000), 4900);
        assert_eq!(replay_frame(5000, 0), 5000);
    }

    #[test]
    fn the_table_holds_a_minute() {
        assert!(frame_still_archived(5000, 3800));
        assert!(!frame_still_archived(5000, 3799));
        assert!(!frame_still_archived(5000, 5000));
        assert_eq!(HISTORY_FRAMES as i64 * TICK_MS, 60_000);
    }
}
