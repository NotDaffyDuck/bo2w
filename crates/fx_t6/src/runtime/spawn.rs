//! When an effect's elements spawn: the one-shot count, the looping
//! schedule and the status a new effect starts with.

use crate::runtime::life::sample_life_span_msec;
use crate::runtime::status::FX_STATUS_HAS_PENDING_LOOP_ELEMS;

/// The element type of a trail.
pub const FX_ELEM_TYPE_TRAIL: u8 = 3;

/// An update window wider than this (in milliseconds) is trimmed before
/// looping elements are caught up.
const CATCHUP_WINDOW_MSEC: i32 = 0x80;
/// The status a new effect starts with: one reference and the high flag the
/// effect system sets on creation.
const NEW_EFFECT_STATUS: u32 = 0x4000_0001;

/// How many one-shot elements to spawn: a whole number from `base ..= base +
/// amplitude` picked by the 16-bit random `rand16`.
pub fn sample_oneshot_spawn_count(base: i32, amplitude: i32, rand16: u16) -> i32 {
    sample_life_span_msec(base, amplitude, rand16)
}

/// One looping spawn that is due: when, and which one in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxLoopingSpawn {
    pub msec: i32,
    pub sequence: i32,
}

/// The looping spawns that fall inside an update window, in order.
#[derive(Clone, Debug)]
pub struct FxLoopingSpawnSchedule {
    msec_when_played: i32,
    msec_update_end: i32,
    interval_msec: i32,
    max_count: i32,
    next_sequence: i32,
}

impl Iterator for FxLoopingSpawnSchedule {
    type Item = FxLoopingSpawn;

    fn next(&mut self) -> Option<FxLoopingSpawn> {
        if self.interval_msec <= 0 || self.next_sequence >= self.max_count {
            return None;
        }
        let msec = self
            .msec_when_played
            .wrapping_add(self.interval_msec.wrapping_mul(self.next_sequence));
        if msec > self.msec_update_end {
            return None;
        }
        let due = FxLoopingSpawn {
            msec,
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1);
        Some(due)
    }
}

/// Where to start catching up looping elements. An update window wider than
/// 128 ms is trimmed to the last `elem_duration_msec + 1` of it, since
/// anything older would already be dead; an open-ended window is not
/// trimmed.
pub fn looping_catchup_begin(
    msec_update_begin: i32,
    msec_update_end: i32,
    elem_duration_msec: i32,
) -> i32 {
    if msec_update_end == i32::MAX {
        return msec_update_begin;
    }
    let window = msec_update_end.wrapping_sub(msec_update_begin);
    if window <= CATCHUP_WINDOW_MSEC {
        return msec_update_begin;
    }
    let keep = elem_duration_msec.wrapping_add(1);
    if keep < window {
        msec_update_begin.wrapping_add(window.wrapping_sub(keep))
    } else {
        msec_update_begin
    }
}

/// The looping spawns due in an update: one every `interval_msec` from
/// `msec_when_played`, numbered from zero and stopping before `max_count`.
/// The run starts at the first spawn after the window's start (the spawn
/// numbered zero happens when the effect is played) and ends at
/// `msec_update_end`. An interval of zero or less spawns nothing.
pub fn looping_spawn_schedule(
    msec_when_played: i32,
    msec_update_begin: i32,
    msec_update_end: i32,
    interval_msec: i32,
    max_count: i32,
) -> FxLoopingSpawnSchedule {
    let next_sequence = if interval_msec <= 0 {
        max_count
    } else {
        msec_update_begin.wrapping_sub(msec_when_played) / interval_msec + 1
    };
    FxLoopingSpawnSchedule {
        msec_when_played,
        msec_update_end,
        interval_msec,
        max_count,
        next_sequence,
    }
}

/// The status word of a new effect. One that loops also holds a reference for
/// its pending loop and sets the pending-loop bit.
pub const fn spawn_effect_status(msec_looping_life: i32) -> u32 {
    let looping = if msec_looping_life != 0 {
        FX_STATUS_HAS_PENDING_LOOP_ELEMS + 1
    } else {
        0
    };
    NEW_EFFECT_STATUS.wrapping_add(looping)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due(schedule: FxLoopingSpawnSchedule) -> Vec<(i32, i32)> {
        schedule.map(|spawn| (spawn.msec, spawn.sequence)).collect()
    }

    #[test]
    fn a_one_shot_count_runs_from_base_to_base_plus_amplitude() {
        assert_eq!(sample_oneshot_spawn_count(3, 4, 0), 3);
        assert_eq!(sample_oneshot_spawn_count(3, 4, 0xffff), 7);
    }

    #[test]
    fn looping_spawns_come_every_interval_from_when_played() {
        let got = due(looping_spawn_schedule(1000, 1000, 1250, 100, 10));
        assert_eq!(got, [(1100, 1), (1200, 2)]);
    }

    #[test]
    fn spawns_before_the_window_are_skipped() {
        // The window starts at 1150, so sequence 1 (at 1100) is behind it.
        let got = due(looping_spawn_schedule(1000, 1150, 1350, 100, 10));
        assert_eq!(got, [(1200, 2), (1300, 3)]);
    }

    #[test]
    fn the_run_stops_at_the_max_count() {
        let got = due(looping_spawn_schedule(0, 0, 10_000, 100, 4));
        assert_eq!(got, [(100, 1), (200, 2), (300, 3)]);
    }

    #[test]
    fn no_interval_means_no_spawns() {
        assert!(due(looping_spawn_schedule(0, 0, 1000, 0, 5)).is_empty());
        assert!(due(looping_spawn_schedule(0, 0, 1000, -4, 5)).is_empty());
    }

    #[test]
    fn a_short_window_is_not_trimmed() {
        assert_eq!(looping_catchup_begin(1000, 1128, 10), 1000);
        assert_eq!(looping_catchup_begin(1000, 1010, 10), 1000);
    }

    #[test]
    fn a_long_window_keeps_only_the_element_duration() {
        // 500 ms window, elements last 99 ms: keep the last 100.
        assert_eq!(looping_catchup_begin(1000, 1500, 99), 1400);
    }

    #[test]
    fn a_window_inside_the_duration_is_not_trimmed() {
        assert_eq!(looping_catchup_begin(1000, 1200, 500), 1000);
    }

    #[test]
    fn an_open_ended_window_is_not_trimmed() {
        assert_eq!(looping_catchup_begin(1000, i32::MAX, 5), 1000);
    }

    #[test]
    fn a_new_effect_starts_with_one_reference() {
        assert_eq!(spawn_effect_status(0), 0x4000_0001);
    }

    #[test]
    fn a_looping_effect_also_holds_the_pending_loop() {
        assert_eq!(spawn_effect_status(5000), 0x4000_8002);
    }
}
