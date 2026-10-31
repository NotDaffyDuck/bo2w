//! When an effect's elements spawn: the one-shot count, the looping
//! schedule and the status a new effect starts with.

use crate::runtime::lifespan::sample_life_span_msec;
use crate::runtime::state_word::EMITTER_REPEAT_QUEUED_BIT;

/// The element type of a trail.
pub const PARTICLE_SHAPE_STREAK: u8 = 3;

/// An update window wider than this (in milliseconds) is trimmed before
/// looping elements are caught up.
const CATCHUP_WINDOW_MSEC: i32 = 0x80;
/// The status a new effect starts with: one reference and the high flag the
/// effect system sets on creation.
const NEW_EFFECT_STATUS: u32 = 0x4000_0001;

/// How many one-shot elements to spawn: a whole number from `base ..= base +
/// amplitude` picked by the 16-bit random `roll16`.
pub fn sample_oneshot_spawn_count(base: i32, spread: i32, roll16: u16) -> i32 {
    sample_life_span_msec(base, spread, roll16)
}

/// One looping spawn that is due: when, and which one in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepeatBirth {
    pub due_ms: i32,
    pub sequence: i32,
}

/// The looping spawns that fall inside an update window, in order.
#[derive(Clone, Debug)]
pub struct RepeatBirthTimetable {
    /// Spawn `n` happens at `played_at + n * step`.
    played_at: i32,
    step: i32,
    /// Spawns after this time wait for a later update.
    until: i32,
    /// The spawn numbers still to come (empty when the step is not positive).
    left: core::ops::Range<i32>,
}

impl Iterator for RepeatBirthTimetable {
    type Item = RepeatBirth;

    fn next(&mut self) -> Option<RepeatBirth> {
        let sequence = self.left.start;
        let msec = self.step.wrapping_mul(sequence).wrapping_add(self.played_at);
        let due = self.left.contains(&sequence) && msec <= self.until;
        due.then(|| {
            self.left.start += 1;
            RepeatBirth { due_ms: msec, sequence }
        })
    }
}

/// Where to start catching up looping elements. An update window wider than
/// 128 ms is trimmed to the last `elem_duration_msec + 1` of it, since
/// anything older would already be dead; an open-ended window is not
/// trimmed.
pub fn looping_catchup_begin(
    refresh_window_open_ms: i32,
    refresh_window_close_ms: i32,
    elem_life_ms: i32,
) -> i32 {
    let lifetime = elem_life_ms.wrapping_add(1);
    let wide = refresh_window_close_ms.wrapping_sub(refresh_window_open_ms);
    let open_ended = refresh_window_close_ms == i32::MAX;
    if open_ended || wide <= CATCHUP_WINDOW_MSEC || lifetime >= wide {
        refresh_window_open_ms
    } else {
        // begin + (window - lifetime), i.e. one lifetime before the end.
        refresh_window_close_ms.wrapping_sub(lifetime)
    }
}

/// The looping spawns due in an update: one every `cadence_ms` from
/// `started_at_effect_ms`, numbered from zero and stopping before `cap`.
/// The run starts at the first spawn after the window's start (the spawn
/// numbered zero happens when the effect is played) and ends at
/// `refresh_window_close_ms`. An interval of zero or less spawns nothing.
pub fn looping_spawn_schedule(
    started_at_effect_ms: i32,
    refresh_window_open_ms: i32,
    refresh_window_close_ms: i32,
    cadence_ms: i32,
    cap: i32,
) -> RepeatBirthTimetable {
    let elapsed = refresh_window_open_ms.wrapping_sub(started_at_effect_ms);
    // The first spawn after the window opens; none at all without a step.
    let first = if cadence_ms > 0 { elapsed / cadence_ms + 1 } else { cap };
    RepeatBirthTimetable {
        played_at: started_at_effect_ms,
        step: cadence_ms,
        until: refresh_window_close_ms,
        left: first..cap,
    }
}

/// The status word of a new effect. One that loops also holds a reference for
/// its pending loop and sets the pending-loop bit.
pub const fn spawn_effect_status(loop_lifetime_millis: i32) -> u32 {
    match loop_lifetime_millis {
        0 => NEW_EFFECT_STATUS,
        _ => NEW_EFFECT_STATUS.wrapping_add(EMITTER_REPEAT_QUEUED_BIT + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due(schedule: RepeatBirthTimetable) -> Vec<(i32, i32)> {
        schedule.map(|spawn| (spawn.due_ms, spawn.sequence)).collect()
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
