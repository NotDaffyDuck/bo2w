//! Moving the clock forward each frame, pulling it toward the server's time.

use super::clock::{ADJUST_FAST, ADJUST_NUDGE, ADJUST_RESET, FrameClock};
use crate::HostInstant;

/// How far the clock may trail the latest server time and still count as extrapolating.
const EXTRAPOLATION_SLACK_MS: i32 = 8;

impl FrameClock {
    /// Advance a clock that must follow the server exactly, even backwards (a listen server).
    pub fn step_local_host_clock(&mut self, frame_secs: f32, live: bool, authority_clock_ms: Option<HostInstant>) {
        self.drift_toward_host(frame_secs, live, authority_clock_ms, false);
    }

    /// Advance a clock that never runs backwards (a remote client).
    pub fn advance_remote(&mut self, frame_secs: f32, live: bool, authority_clock_ms: Option<HostInstant>) {
        self.drift_toward_host(frame_secs, live, authority_clock_ms, true);
    }

    fn drift_toward_host(
        &mut self,
        frame_secs: f32,
        live: bool,
        authority_clock_ms: Option<HostInstant>,
        never_back: bool,
    ) {
        if !live {
            return;
        }
        self.latest_correction_note = 0;
        if !self.ticking {
            let ms = authority_clock_ms.map(HostInstant::as_millis).unwrap_or(0);
            self.now_ms = ms;
            self.prior_ms = ms;
            self.step_millis_elapsed = 0;
            self.subtick_millis = 0.0;
            self.previous_host_state = Some(ms);
            self.ticking = true;
            return;
        }

        // Whole milliseconds this frame; the remainder carries to the next.
        self.subtick_millis += frame_secs.max(0.0) * 1000.0;
        let whole_ms = self.subtick_millis as i32;
        self.subtick_millis -= whole_ms as f32;
        let mut next = self.now_ms.wrapping_add(whole_ms);
        self.predicted_ahead = self
            .previous_host_state
            .is_some_and(|server| next >= server.wrapping_sub(EXTRAPOLATION_SLACK_MS));

        // A server time we have not seen yet pulls the clock toward it.
        if let Some(server) = authority_clock_ms.map(HostInstant::as_millis)
            && self.previous_host_state != Some(server)
        {
            self.previous_host_state = Some(server);
            let offset = server.wrapping_sub(next);
            let gap = offset.unsigned_abs();
            if gap > Self::CLOCK_NUDGE_SNAP_MILLIS as u32 {
                self.subtick_millis = 0.0;
                self.wipe_count = self.wipe_count.saturating_add(1);
                self.latest_correction_note = ADJUST_RESET;
                // bo2zm: BO2's CL_AdjustTimeDelta RESET sets serverTime and oldServerTime to
                // the snapshot's time, the one place its clock steps back. Held ahead after a
                // host stall, every command stayed in the host's future.
                self.set_host_ms(server);
                return;
            } else if gap > Self::CLOCK_NUDGE_QUICK_MILLIS as u32 {
                next = next.wrapping_add(offset / 2);
                self.quick_count = self.quick_count.saturating_add(1);
                self.latest_correction_note = ADJUST_FAST;
            } else if gap > 0 {
                next = next.wrapping_add(offset.signum());
                self.adjust_count = self.adjust_count.saturating_add(1);
                self.latest_correction_note = ADJUST_NUDGE;
            }
        }

        let target = if never_back { next.max(self.now_ms) } else { next };
        self.set_host_ms(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 0.05; // exactly 50 ms after the *1000 and truncation

    fn at(ms: i32) -> Option<HostInstant> {
        Some(HostInstant::from_millis_value(ms))
    }

    /// A listen-style clock started at 1000 ms.
    fn started_at_1000() -> FrameClock {
        let mut clock = FrameClock::default();
        clock.step_local_host_clock(FRAME, true, at(1000));
        clock
    }

    #[test]
    fn an_inactive_clock_does_not_move() {
        let mut clock = FrameClock::default();
        clock.step_local_host_clock(FRAME, false, at(1000));
        assert_eq!(clock, FrameClock::default());
    }

    #[test]
    fn the_first_active_advance_takes_the_server_time_with_no_step() {
        let clock = started_at_1000();
        assert!(clock.started());
        assert_eq!((clock.time(), clock.prior_ms(), clock.step_millis_elapsed()), (1000, 1000, 0));
        assert_eq!(clock.subtick_millis(), 0.0);
    }

    #[test]
    fn the_first_advance_without_a_server_time_starts_at_zero() {
        let mut clock = FrameClock::default();
        clock.advance_remote(FRAME, true, None);
        assert!(clock.started());
        assert_eq!(clock.time(), 0);
    }

    #[test]
    fn with_no_new_server_time_the_clock_runs_on_its_own() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1000));
        assert_eq!((clock.time(), clock.step_millis_elapsed()), (1050, 50));
        assert_eq!(clock.latest_correction_note(), None);
        assert!(clock.predicted_ahead());
    }

    #[test]
    fn leftover_fractions_of_a_millisecond_carry_to_the_next_frame() {
        let mut clock = started_at_1000();
        let delta = 0.0078125; // 7.8125 ms, exact in binary
        clock.step_local_host_clock(delta, true, at(1000));
        assert_eq!(clock.step_millis_elapsed(), 7);
        assert_eq!(clock.subtick_millis(), 0.8125);
        clock.step_local_host_clock(delta, true, at(1000));
        assert_eq!(clock.step_millis_elapsed(), 8, "7.8125 + 0.8125 = 8.625");
        assert_eq!(clock.subtick_millis(), 0.625);
    }

    #[test]
    fn a_negative_frame_delta_counts_as_zero() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(-1.0, true, at(1000));
        assert_eq!((clock.time(), clock.step_millis_elapsed()), (1000, 0));
    }

    #[test]
    fn a_millisecond_of_skew_nudges_by_one() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1051)); // we'd be at 1050
        assert_eq!(clock.time(), 1051);
        assert_eq!(clock.latest_correction_note(), Some("nudge"));
        assert_eq!(clock.adjust_count, 1);
    }

    #[test]
    fn skew_over_a_hundred_milliseconds_closes_half_the_gap() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1250)); // we'd be at 1050: 200 ahead
        assert_eq!(clock.time(), 1150);
        assert_eq!(clock.latest_correction_note(), Some("fast"));
        assert_eq!(clock.quick_count, 1);
    }

    #[test]
    fn skew_of_exactly_a_hundred_is_still_a_nudge() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1150));
        assert_eq!(clock.time(), 1051);
        assert_eq!(clock.latest_correction_note(), Some("nudge"));
    }

    #[test]
    fn skew_over_a_thousand_snaps_to_the_server_and_drops_the_fraction() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(0.0078125, true, at(1000)); // leaves a fraction
        clock.step_local_host_clock(FRAME, true, at(2100));
        assert_eq!((clock.time(), clock.subtick_millis()), (2100, 0.0));
        assert_eq!(clock.latest_correction_note(), Some("reset"));
        assert_eq!(clock.wipe_count, 1);
    }

    #[test]
    fn the_same_server_time_twice_is_not_a_correction() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1010)); // we'd be at 1050: pulled back one
        assert_eq!((clock.time(), clock.latest_correction_note()), (1049, Some("nudge")));
        clock.step_local_host_clock(FRAME, true, at(1010));
        assert_eq!(clock.latest_correction_note(), None, "1010 was already seen");
    }

    #[test]
    fn a_reset_steps_a_listen_and_a_remote_clock_back_to_the_server() {
        let backwards = |remote: bool| {
            let mut clock = FrameClock::default();
            clock.step_local_host_clock(FRAME, true, at(2000));
            // Server jumps 1600 ms behind: a reset, BO2's one step back.
            if remote {
                clock.advance_remote(0.0, true, at(400));
            } else {
                clock.step_local_host_clock(0.0, true, at(400));
            }
            clock
        };
        let listen = backwards(false);
        assert_eq!((listen.time(), listen.step_millis_elapsed()), (400, 0));
        let remote = backwards(true);
        assert_eq!((remote.time(), remote.step_millis_elapsed()), (400, 0));
    }

    #[test]
    fn a_remote_clock_never_steps_back_on_a_nudge() {
        let mut remote = started_at_1000();
        remote.advance_remote(0.0, true, at(999)); // nudge would give 999
        assert_eq!(remote.time(), 1000);
        let mut listen = started_at_1000();
        listen.step_local_host_clock(0.0, true, at(999));
        assert_eq!(listen.time(), 999);
    }

    #[test]
    fn running_ahead_of_the_server_is_extrapolation_and_running_behind_is_not() {
        let mut clock = started_at_1000();
        clock.step_local_host_clock(FRAME, true, at(1000));
        assert!(clock.predicted_ahead());
        // Server announced 2000 earlier; we are a long way behind it.
        let mut behind = started_at_1000();
        behind.previous_host_state = Some(2000);
        behind.step_local_host_clock(FRAME, true, None);
        assert!(!behind.predicted_ahead());
    }
}
