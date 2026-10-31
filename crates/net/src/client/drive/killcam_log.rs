//! The kill-replay trace. With `BO2MP_KILLCAMLOG=1` each presented frame of a replay prints
//! where the replay clock sits between two snapshots, and any body that moves further than
//! [`JUMP_UNITS`] between two frames prints a line of its own. Grep the log for "killcamlog:".

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::host_tick::peer_mail::HOST_STEP_MILLIS;
use crate::client::remote_buffer::{STANDIN_LAG_MILLIS, OtherPlayerSmoother};

/// A body that moves further than this between two presented frames is reported.
const JUMP_UNITS: f32 = 40.0;

pub fn is_switched_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_KILLCAMLOG").is_some_and(|value| value != "0"))
}

/// What changed about the replay since the frame before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Steady,
    Began,
    Ended,
}

/// Where the replay clock sits between its two snapshots, 0 at the left one and 1 at the right.
fn clock_position(effective_ms: i32, left_ms: Option<i32>, right_ms: Option<i32>) -> Option<f32> {
    match (left_ms, right_ms) {
        (Some(lo), Some(hi)) if hi > lo => {
            Some(((effective_ms - lo) as f32 / (hi - lo) as f32).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt()
}

#[derive(Default)]
pub struct KillcamLog {
    replaying: bool,
    watching_jumps: bool,
    frame: u32,
    last_seen: HashMap<i32, [f32; 3]>,
}

impl KillcamLog {
    /// Moves on to the next presented frame and says whether the replay began or ended in it.
    fn roll_forward(&mut self, replaying: bool) -> Edge {
        let edge = match (self.replaying, replaying) {
            (false, true) => Edge::Began,
            (true, false) => Edge::Ended,
            _ => Edge::Steady,
        };
        if edge != Edge::Steady {
            self.frame = 0;
        }
        if replaying {
            self.frame += 1;
        }
        // The frame that ends the replay is still checked for jumps.
        self.watching_jumps = replaying || self.replaying;
        self.replaying = replaying;
        edge
    }

    pub fn note_frame(&mut self, replaying: bool, clock_ms: i32, body_ms: i32, proxy: &OtherPlayerSmoother) {
        match self.roll_forward(replaying) {
            Edge::Began => diag::info!(WireLink, "killcamlog: replay begins clock={clock_ms}"),
            Edge::Ended => diag::info!(WireLink, "killcamlog: replay ends clock={clock_ms}"),
            Edge::Steady => {}
        }
        if !replaying {
            return;
        }
        let tick_ms = |snapshot: std::sync::Arc<sim::Snapshot>| snapshot.tick.0 as i32 * HOST_STEP_MILLIS;
        let lo = proxy.blended_frame_at(clock_ms).map(tick_ms);
        let hi = proxy.snapshot_after(clock_ms).map(tick_ms);
        let effective = clock_ms.saturating_sub(STANDIN_LAG_MILLIS);
        let alpha = clock_position(effective, lo, hi);
        diag::info!(
            WireLink,
            "killcamlog: frame={} replay_t={clock_ms} effective={effective} body_t={body_ms} left={lo:?} right={hi:?} alpha={alpha:?}",
            self.frame
        );
    }

    /// Remembers where a body was drawn. During a replay, a move of more than [`JUMP_UNITS`]
    /// comes back as the distance and the place it came from.
    fn track(&mut self, ent_no: i32, pos: [f32; 3]) -> Option<(f32, [f32; 3])> {
        let was = self.last_seen.insert(ent_no, pos)?;
        if !self.watching_jumps {
            return None;
        }
        let moved = distance(pos, was);
        (moved > JUMP_UNITS).then_some((moved, was))
    }

    pub fn note_body(&mut self, ent_no: i32, client_num: i32, e_type: u8, pos: [f32; 3], via_proxy: bool) {
        let Some((moved, was)) = self.track(ent_no, pos) else {
            return;
        };
        diag::info!(
            WireLink,
            "killcamlog: jump frame={} ent={ent_no} client={client_num} e_type={e_type} d={moved:.1} from=({:.0} {:.0} {:.0}) to=({:.0} {:.0} {:.0}) via={}",
            self.frame,
            was[0],
            was[1],
            was[2],
            pos[0],
            pos[1],
            pos[2],
            if via_proxy { "proxy_pose" } else { "settle_pose" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_sits_between_two_snapshots() {
        assert_eq!(clock_position(150, Some(100), Some(200)), Some(0.5));
        assert_eq!(clock_position(100, Some(100), Some(200)), Some(0.0));
    }

    #[test]
    fn a_clock_outside_its_snapshots_is_held_at_the_nearer_end() {
        assert_eq!(clock_position(50, Some(100), Some(200)), Some(0.0));
        assert_eq!(clock_position(500, Some(100), Some(200)), Some(1.0));
    }

    #[test]
    fn without_two_ordered_snapshots_there_is_no_position() {
        assert_eq!(clock_position(150, None, Some(200)), None);
        assert_eq!(clock_position(150, Some(100), None), None);
        assert_eq!(clock_position(150, Some(200), Some(200)), None);
        assert_eq!(clock_position(150, Some(300), Some(200)), None);
    }

    #[test]
    fn the_distance_is_straight_line() {
        assert_eq!(distance([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), 0.0);
        assert_eq!(distance([0.0, 0.0, 0.0], [3.0, 4.0, 12.0]), 13.0);
    }

    #[test]
    fn the_replay_edges_are_reported_once() {
        let mut log = KillcamLog::default();
        assert_eq!(log.roll_forward(false), Edge::Steady);
        assert_eq!(log.roll_forward(true), Edge::Began);
        assert_eq!(log.roll_forward(true), Edge::Steady);
        assert_eq!(log.roll_forward(false), Edge::Ended);
        assert_eq!(log.roll_forward(false), Edge::Steady);
    }

    #[test]
    fn replay_frames_are_counted_from_one_each_time() {
        let mut log = KillcamLog::default();
        for _ in 0..3 {
            log.roll_forward(true);
        }
        assert_eq!(log.frame, 3);
        log.roll_forward(false);
        assert_eq!(log.frame, 0);
        log.roll_forward(true);
        assert_eq!(log.frame, 1);
    }

    #[test]
    fn a_body_that_leaps_during_a_replay_is_reported_with_where_it_was() {
        let mut log = KillcamLog::default();
        log.roll_forward(true);
        assert_eq!(log.track(7, [0.0, 0.0, 0.0]), None);
        assert_eq!(log.track(7, [0.0, 0.0, 41.0]), Some((41.0, [0.0, 0.0, 0.0])));
        // The jump is measured from where it was last seen, not from where it started.
        assert_eq!(log.track(7, [0.0, 0.0, 50.0]), None);
    }

    #[test]
    fn a_move_of_exactly_the_limit_is_not_a_jump() {
        let mut log = KillcamLog::default();
        log.roll_forward(true);
        log.track(1, [0.0; 3]);
        assert_eq!(log.track(1, [JUMP_UNITS, 0.0, 0.0]), None);
    }

    #[test]
    fn jumps_are_ignored_outside_a_replay_but_the_last_frame_of_one_still_counts() {
        let mut log = KillcamLog::default();
        log.roll_forward(false);
        log.track(1, [0.0; 3]);
        assert_eq!(log.track(1, [500.0, 0.0, 0.0]), None);
        log.roll_forward(true);
        log.roll_forward(false);
        assert!(log.track(1, [0.0; 3]).is_some());
        log.roll_forward(false);
        assert_eq!(log.track(1, [900.0, 0.0, 0.0]), None);
    }

    #[test]
    fn bodies_are_followed_separately() {
        let mut log = KillcamLog::default();
        log.roll_forward(true);
        log.track(1, [0.0; 3]);
        assert_eq!(log.track(2, [500.0, 0.0, 0.0]), None);
    }
}
