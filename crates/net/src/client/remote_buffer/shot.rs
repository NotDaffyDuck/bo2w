//! Which snapshots a delayed shot was resolved against.

use sim::{ShotRecordOrigin, ShotRecordGrade};

use super::{OtherPlayerSmoother, lagged_clock};

impl OtherPlayerSmoother {
    /// Describe the pair of snapshots a sample at `draw_clock_millis` falls between, for shot
    /// records. Outside the buffer the nearest end is reported as held (before the oldest) or
    /// starved (after the newest); with an empty buffer, the default record.
    pub fn shot_bracket(&self, draw_clock_millis: i32) -> ShotRecordOrigin {
        let effective = lagged_clock(draw_clock_millis);
        let (Some(eldest), Some(newest)) = (self.history.front(), self.history.back()) else {
            return ShotRecordOrigin::default();
        };
        let pinned = |tick, quality| ShotRecordOrigin {
            left: tick,
            right: tick,
            opacity: 0.0,
            quality,
        };
        if effective < eldest.clock_stamp_ms {
            return pinned(eldest.frame.tick, ShotRecordGrade::Held);
        }
        if effective > newest.clock_stamp_ms {
            return pinned(newest.frame.tick, ShotRecordGrade::RanDry);
        }
        let (Some(earlier), Some(later)) = self.bracket(effective) else {
            return ShotRecordOrigin::default();
        };
        if earlier.clock_stamp_ms == later.clock_stamp_ms {
            return ShotRecordOrigin {
                left: earlier.frame.tick,
                right: later.frame.tick,
                opacity: 0.0,
                quality: ShotRecordGrade::Exact,
            };
        }
        let span = (later.clock_stamp_ms - earlier.clock_stamp_ms) as f32;
        ShotRecordOrigin {
            left: earlier.frame.tick,
            right: later.frame.tick,
            opacity: ((effective - earlier.clock_stamp_ms) as f32 / span).clamp(0.0, 1.0),
            quality: ShotRecordGrade::SmoothedBetweenFrames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_tick::peer_mail::HOST_STEP_MILLIS;
    use crate::client::remote_buffer::STANDIN_LAG_MILLIS;

    fn proxy_with(ticks: &[u32]) -> OtherPlayerSmoother {
        let mut proxy = OtherPlayerSmoother::new();
        for &tick in ticks {
            let mut snapshot = sim::Snapshot::unpublished(sim::Tick(0));
            snapshot.tick = sim::Tick(tick);
            proxy.take_copy(&snapshot);
        }
        proxy
    }

    fn render_time_for(tick_ms: i32) -> i32 {
        tick_ms + STANDIN_LAG_MILLIS
    }

    #[test]
    fn an_empty_buffer_gives_the_default_record() {
        let sample = OtherPlayerSmoother::new().shot_bracket(5000);
        assert_eq!(sample, ShotRecordOrigin::default());
    }

    #[test]
    fn before_the_oldest_is_held_and_after_the_newest_is_starved() {
        let proxy = proxy_with(&[10, 20]);
        assert_eq!(proxy.shot_bracket(0).quality, ShotRecordGrade::Held);
        let late = proxy.shot_bracket(render_time_for(30 * HOST_STEP_MILLIS));
        assert_eq!(late.quality, ShotRecordGrade::RanDry);
        assert_eq!(late.left, sim::Tick(20));
        assert_eq!(late.right, sim::Tick(20));
    }

    #[test]
    fn on_a_snapshot_is_exact_and_between_two_is_interpolated() {
        let proxy = proxy_with(&[10, 20]);
        let exact = proxy.shot_bracket(render_time_for(10 * HOST_STEP_MILLIS));
        assert_eq!(exact.quality, ShotRecordGrade::Exact);
        let between = proxy.shot_bracket(render_time_for(15 * HOST_STEP_MILLIS));
        assert_eq!(between.quality, ShotRecordGrade::SmoothedBetweenFrames);
        assert_eq!((between.left, between.right), (sim::Tick(10), sim::Tick(20)));
        assert!((between.opacity - 0.5).abs() < 1e-6);
    }
}
