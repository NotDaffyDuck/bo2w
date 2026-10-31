//! Where a correction starts replaying, and whether an arriving snapshot is news.

use sim::Tick;

/// The tick a snapshot covers and the first tick after it that must be replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorrectionCutoff {
    pub frame_host_step: Tick,
    pub playback_step: Tick,
}

impl CorrectionCutoff {
    pub const fn after(frame_host_step: Tick) -> Self {
        Self {
            frame_host_step,
            playback_step: Tick(frame_host_step.0.wrapping_add(1)),
        }
    }
}

/// How an incoming snapshot compares with the newest one already taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePosition {
    New,
    Duplicate,
    Stale,
}

pub const fn place_of_tick(last: Option<Tick>, incoming: Tick) -> FramePosition {
    match last {
        None => FramePosition::New,
        Some(last) if incoming.0 > last.0 => FramePosition::New,
        Some(last) if incoming.0 == last.0 => FramePosition::Duplicate,
        Some(_) => FramePosition::Stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_starts_one_tick_after_the_snapshot() {
        let boundary = CorrectionCutoff::after(Tick(10));
        assert_eq!((boundary.frame_host_step, boundary.playback_step), (Tick(10), Tick(11)));
    }

    #[test]
    fn the_replay_tick_wraps_at_the_end_of_the_counter() {
        assert_eq!(CorrectionCutoff::after(Tick(u32::MAX)).playback_step, Tick(0));
    }

    #[test]
    fn the_first_snapshot_is_always_new() {
        assert_eq!(place_of_tick(None, Tick(0)), FramePosition::New);
        assert_eq!(place_of_tick(None, Tick(u32::MAX)), FramePosition::New);
    }

    #[test]
    fn a_later_tick_is_new_the_same_tick_repeats_and_an_earlier_one_is_stale() {
        let last = Some(Tick(50));
        assert_eq!(place_of_tick(last, Tick(51)), FramePosition::New);
        assert_eq!(place_of_tick(last, Tick(50)), FramePosition::Duplicate);
        assert_eq!(place_of_tick(last, Tick(49)), FramePosition::Stale);
    }

    #[test]
    fn the_comparison_is_plain_not_wrapping() {
        assert_eq!(place_of_tick(Some(Tick(u32::MAX)), Tick(0)), FramePosition::Stale);
    }
}
