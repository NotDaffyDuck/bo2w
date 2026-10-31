//! Finding the frame some milliseconds back from the newest.

use sim::Tick;

use super::*;

/// What a look-back found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryQueryResult {
    /// How far back the found frame really is. When nothing was found, the request itself if it
    /// was under a millisecond, otherwise 0.
    pub reached_millis: i32,
    pub found_tick: Option<Tick>,
}

impl HistoryQueryResult {
    pub fn empty_display(&self) -> bool {
        self.found_tick.is_none()
    }

    fn nothing(reached_millis: i32) -> Self {
        Self {
            reached_millis,
            found_tick: None,
        }
    }
}

impl SnapshotVault {
    /// Ticks the archive spans, counted from tick 0 to the newest: one past the newest tick.
    fn span_end(&self) -> Option<i64> {
        self.kept.back().map(|newest| i64::from(newest.at_tick.0) + 1)
    }

    /// Milliseconds from the oldest frame to the end of the newest.
    pub fn span_ms(&self) -> i32 {
        let (Some(end), Some(oldest)) = (self.span_end(), self.kept.front()) else {
            return 0;
        };
        ((end - i64::from(oldest.at_tick.0)) * i64::from(VAULT_STEP_MS)) as i32
    }

    /// The first held frame at or after `asked_millis` back from the end of the newest frame
    /// (rounded down to whole ticks, and no further back than the oldest frame).
    pub fn seek_back(&self, asked_millis: i32) -> HistoryQueryResult {
        if asked_millis < 1 {
            return HistoryQueryResult::nothing(asked_millis);
        }
        let (Some(end), Some(oldest)) = (self.span_end(), self.kept.front()) else {
            return HistoryQueryResult::nothing(0);
        };
        let ticks_back = i64::from(asked_millis) / i64::from(VAULT_STEP_MS);
        let wanted = (end - ticks_back).max(i64::from(oldest.at_tick.0));
        match self.kept.iter().find(|frame| i64::from(frame.at_tick.0) >= wanted) {
            Some(found) => HistoryQueryResult {
                reached_millis: ((end - i64::from(found.at_tick.0)) * i64::from(VAULT_STEP_MS)) as i32,
                found_tick: Some(found.at_tick),
            },
            None => HistoryQueryResult::nothing(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::StoredHistoryFrame;
    use super::*;
    use sim::Snapshot;

    /// Frames at the given ticks.
    fn archive_of(ticks: &[u32]) -> SnapshotVault {
        let mut archive = SnapshotVault::new();
        for stamp in ticks {
            archive.append(StoredHistoryFrame {
                at_tick: Tick(*stamp),
                state: Snapshot::unpublished(Tick(*stamp)),
                no_player_state: Vec::new(),
            });
        }
        archive
    }

    #[test]
    fn an_empty_archive_has_no_duration_and_nothing_to_show() {
        let archive = SnapshotVault::new();
        assert_eq!(archive.span_ms(), 0);
        let found = archive.seek_back(500);
        assert!(found.empty_display());
        assert_eq!(found.reached_millis, 0);
    }

    #[test]
    fn duration_runs_from_the_oldest_frame_to_the_end_of_the_newest() {
        assert_eq!(archive_of(&[10]).span_ms(), VAULT_STEP_MS);
        assert_eq!(archive_of(&[10, 11, 14]).span_ms(), 5 * VAULT_STEP_MS);
    }

    #[test]
    fn a_request_under_a_millisecond_finds_nothing_and_echoes_itself() {
        let archive = archive_of(&[1, 2, 3]);
        for asked in [0, -7] {
            let found = archive.seek_back(asked);
            assert_eq!((found.found_tick, found.reached_millis), (None, asked));
        }
    }

    #[test]
    fn a_lookup_counts_whole_ticks_back_from_the_end_of_the_newest_frame() {
        let archive = archive_of(&[0, 1, 2, 3, 4]);
        // One tick back from the end (tick 5) is tick 4; the span ends at 5, so it is
        // one tick-length ago.
        let one = archive.seek_back(VAULT_STEP_MS);
        assert_eq!(one.found_tick, Some(Tick(4)));
        assert_eq!(one.reached_millis, VAULT_STEP_MS);
        let three = archive.seek_back(3 * VAULT_STEP_MS);
        assert_eq!(three.found_tick, Some(Tick(2)));
        assert_eq!(three.reached_millis, 3 * VAULT_STEP_MS);
        // Under one tick rounds down to zero ticks back, which is past the newest frame.
        let none = archive.seek_back(VAULT_STEP_MS - 1);
        assert_eq!((none.found_tick, none.reached_millis), (None, 0));
    }

    #[test]
    fn asking_further_back_than_held_gives_the_oldest_frame() {
        let archive = archive_of(&[10, 11, 12]);
        let found = archive.seek_back(1_000_000);
        assert_eq!(found.found_tick, Some(Tick(10)));
        assert_eq!(found.reached_millis, 3 * VAULT_STEP_MS);
    }

    #[test]
    fn a_gap_in_ticks_gives_the_next_frame_forward() {
        let archive = archive_of(&[10, 11, 20]);
        // Wanted tick 18 isn't held; the first frame at or after it is 20.
        let found = archive.seek_back(3 * VAULT_STEP_MS);
        assert_eq!(found.found_tick, Some(Tick(20)));
        assert_eq!(found.reached_millis, VAULT_STEP_MS);
    }
}
