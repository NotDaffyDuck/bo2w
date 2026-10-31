//! Giving up on a host that has stopped answering. A single quiet second is a hitch and only
//! makes the client re-adopt the host's state; enough of them in a row, or an action the host
//! never settled, fails the session.

use bevy::prelude::*;

use super::outbox::QueuedOutgoingPlayerMessages;
use super::stores::ClientGuessState;
use crate::host_tick::peer_mail::PlayerActionIntake;
use crate::host_tick::tick_systems::HostInputValve;
use crate::client::frame_clock::FrameClock;
use crate::client::realtime::ClientRealtime;

/// How long the host may acknowledge nothing before that counts as one stall. Both ways of
/// measuring it, the age of the oldest command still waiting and the command time that has piled
/// up unacknowledged, are wall-clock spans, so one long frame can fill either on its own.
const STALL_MS: i32 = 1000;

/// How many stalls the link may take before the session is failed. A hitch costs one and is
/// recovered from; a host that has really gone away keeps earning them until they run out.
const STALLS_BEFORE_FAIL: u32 =
    (master_protocol::LINK_IDLE_LIMIT.as_millis() / STALL_MS as u128) as u32 - 1;

const ACTION_TIMEOUT_REASON: &str = "ActionOutcomeUnknown: authority outcome deadline exceeded";
const BACKLOG_REASON: &str = "InputBacklogExceeded";

/// The stalls counted since the host last acknowledged anything.
#[derive(Debug, Default)]
pub struct BacklogStalls {
    run: u32,
    acks_seen: u64,
    marked_at: Option<i32>,
}

impl BacklogStalls {
    /// Counts a stall: at most one per `STALL_MS`, and only while the acknowledgement count has
    /// stayed where it was. `None` means this moment was already dealt with.
    fn record_stall(&mut self, clock_millis: i32, receipts_batch: u64) -> Option<u32> {
        if receipts_batch != self.acks_seen {
            *self = Self { run: 0, acks_seen: receipts_batch, marked_at: Some(clock_millis) };
            return None;
        }
        if self.marked_at.is_some_and(|at| clock_millis.saturating_sub(at) < STALL_MS) {
            return None;
        }
        self.marked_at = Some(clock_millis);
        self.run = self.run.saturating_add(1);
        Some(self.run)
    }

    fn forget(&mut self) {
        *self = Self::default();
    }
}

/// What the check decided this frame.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Carry,
    Resync { stall: u32 },
    Fail(&'static str),
}

fn judge(actions_timed_out: bool, stall: Option<u32>) -> Verdict {
    if actions_timed_out {
        Verdict::Fail(ACTION_TIMEOUT_REASON)
    } else if let Some(stall) = stall {
        if stall >= STALLS_BEFORE_FAIL {
            Verdict::Fail(BACKLOG_REASON)
        } else {
            Verdict::Resync { stall }
        }
    } else {
        Verdict::Carry
    }
}

/// The span of command time between the first and last of `times`, counting only the steps
/// forward.
fn span_of(times: impl IntoIterator<Item = i32>) -> i32 {
    let mut times = times.into_iter();
    let Some(mut before) = times.next() else {
        return 0;
    };
    let mut span = 0i32;
    for tm in times {
        span = span.saturating_add(tm.saturating_sub(before).max(0));
        before = tm;
    }
    span
}

pub fn cap_receiver_workload(
    pending: Res<QueuedOutgoingPlayerMessages>,
    mut forecast: ResMut<ClientGuessState>,
    cg: Res<FrameClock>,
    cls: Res<ClientRealtime>,
    mut join_rec: ResMut<crate::JoinProgressRecord>,
    dir_link: Option<Res<crate::DirectoryLink>>,
    mut gate: ResMut<HostInputValve>,
    actions: Res<PlayerActionIntake>,
    mut stalls: Local<BacklogStalls>,
) {
    if join_rec.phase.has_failed() {
        gate.local_cmds_enabled = false;
        return;
    }
    let waiting_since = pending.iter().next().map(|(_, cmd, _)| cmd.authority_clock_ms);
    let piled_up = span_of(forecast.0.history().iter().map(|row| row.cmd.authority_clock_ms));
    let stalled = waiting_since.is_some_and(|since| cg.time().saturating_sub(since) > STALL_MS)
        || piled_up > STALL_MS;
    let stall = if stalled {
        let receipts_batch = pending.acked_through().map_or(0, |seq| u64::from(seq.0));
        stalls.record_stall(cls.now_ms(), receipts_batch)
    } else {
        stalls.forget();
        None
    };

    let why = match judge(actions.deadline_passed(), stall) {
        Verdict::Carry => return,
        Verdict::Resync { stall } => {
            diag::warn!(
                WireLink,
                "input backlog: no authority ack in {STALL_MS} ms \
                 (stall {stall}/{STALLS_BEFORE_FAIL}) — re-adopting"
            );
            forecast.0.force_resync();
            return;
        }
        Verdict::Fail(why) => why,
    };

    gate.local_cmds_enabled = false;
    forecast.0.stand_down();
    let game_session_key = crate::join_walk::current_round_key(dir_link.as_deref());
    if let Some(dir_link) = dir_link {
        dir_link.mark_failed(why);
    }
    join_rec.enter_phase(crate::JoinSequenceStep::Failed(crate::JoinFailureCause::Transport {
        source: why.to_owned(),
        stage: crate::AbortStep::Transport,
        game_session_key,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_threshold_follows_the_session_idle_time() {
        let idle_seconds = master_protocol::LINK_IDLE_LIMIT.as_secs() as u32;
        assert_eq!(STALLS_BEFORE_FAIL, idle_seconds - 1);
        assert!(STALLS_BEFORE_FAIL >= 1);
    }

    #[test]
    fn a_stall_needs_a_full_stall_length_after_the_last_news() {
        let mut stalls = BacklogStalls::default();
        // The count starts at zero receipts_batch, so a first call with none is already "no news".
        assert_eq!(stalls.record_stall(0, 0), Some(1));
        assert_eq!(stalls.record_stall(10, 5), None);
        assert_eq!(stalls.record_stall(500, 5), None);
        assert_eq!(stalls.record_stall(1010, 5), Some(1));
    }

    #[test]
    fn at_most_one_stall_is_counted_per_stall_length() {
        let mut stalls = BacklogStalls::default();
        assert_eq!(stalls.record_stall(100, 0), Some(1));
        assert_eq!(stalls.record_stall(600, 0), None);
        assert_eq!(stalls.record_stall(1099, 0), None);
        assert_eq!(stalls.record_stall(1100, 0), Some(2));
        assert_eq!(stalls.record_stall(2100, 0), Some(3));
    }

    #[test]
    fn an_acknowledgement_starts_the_count_over() {
        let mut stalls = BacklogStalls::default();
        stalls.record_stall(0, 0);
        stalls.record_stall(1000, 0);
        assert_eq!(stalls.record_stall(1500, 7), None);
        assert_eq!(stalls.record_stall(2500, 7), Some(1));
    }

    #[test]
    fn forgetting_clears_everything() {
        let mut stalls = BacklogStalls::default();
        stalls.record_stall(0, 0);
        stalls.record_stall(1000, 0);
        stalls.forget();
        assert_eq!(stalls.record_stall(1100, 0), Some(1));
    }

    #[test]
    fn a_timed_out_action_fails_the_session_whatever_the_stalls_say() {
        assert_eq!(judge(true, None), Verdict::Fail(ACTION_TIMEOUT_REASON));
        assert_eq!(judge(true, Some(1)), Verdict::Fail(ACTION_TIMEOUT_REASON));
    }

    #[test]
    fn early_stalls_resync_and_the_last_allowed_one_fails() {
        assert_eq!(judge(false, None), Verdict::Carry);
        assert_eq!(judge(false, Some(1)), Verdict::Resync { stall: 1 });
        assert_eq!(
            judge(false, Some(STALLS_BEFORE_FAIL - 1)),
            Verdict::Resync { stall: STALLS_BEFORE_FAIL - 1 }
        );
        assert_eq!(judge(false, Some(STALLS_BEFORE_FAIL)), Verdict::Fail(BACKLOG_REASON));
        assert_eq!(judge(false, Some(STALLS_BEFORE_FAIL + 4)), Verdict::Fail(BACKLOG_REASON));
    }

    #[test]
    fn the_piled_up_span_counts_forward_steps_only() {
        assert_eq!(span_of([]), 0);
        assert_eq!(span_of([500]), 0);
        assert_eq!(span_of([100, 150, 400]), 300);
        // A step back adds nothing and the next gap is measured from the lower stamp.
        assert_eq!(span_of([100, 300, 250, 400]), 200 + 150);
    }

    #[test]
    fn the_span_does_not_overflow() {
        assert_eq!(span_of([0, i32::MAX, 0, i32::MAX]), i32::MAX);
    }

    fn app_with_failed_signon(failed: bool) -> App {
        let mut app = App::new();
        app.init_resource::<QueuedOutgoingPlayerMessages>()
            .init_resource::<ClientGuessState>()
            .init_resource::<FrameClock>()
            .init_resource::<ClientRealtime>()
            .init_resource::<crate::JoinProgressRecord>()
            .insert_resource(HostInputValve { local_cmds_enabled: true })
            .init_resource::<PlayerActionIntake>()
            .add_systems(Update, cap_receiver_workload);
        if failed {
            app.world_mut().resource_mut::<crate::JoinProgressRecord>().enter_phase(
                crate::JoinSequenceStep::Failed(crate::JoinFailureCause::Transport {
                    source: "gone".to_owned(),
                    stage: crate::AbortStep::Transport,
                    game_session_key: crate::join_walk::current_round_key(None),
                }),
            );
        }
        app
    }

    #[test]
    fn a_failed_signon_switches_local_commands_off() {
        let mut app = app_with_failed_signon(true);
        app.update();
        assert!(!app.world().resource::<HostInputValve>().local_cmds_enabled);
    }

    #[test]
    fn a_quiet_link_with_nothing_queued_leaves_commands_on() {
        let mut app = app_with_failed_signon(false);
        app.update();
        assert!(app.world().resource::<HostInputValve>().local_cmds_enabled);
        assert!(!app.world().resource::<crate::JoinProgressRecord>().phase.has_failed());
    }
}
