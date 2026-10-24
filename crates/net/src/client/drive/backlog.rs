//! Giving up on a host that has stopped answering. A single quiet second is a hitch and only
//! makes the client re-adopt the host's state; enough of them in a row, or an action the host
//! never settled, fails the session.

use bevy::prelude::*;

use super::outbox::PendingClientSends;
use super::stores::ClientPredictionState;
use crate::authority::inbox::ClientActionInbox;
use crate::authority::runtime::AuthorityInputGate;
use crate::client::frame_clock::FrameClock;
use crate::client::realtime::ClientRealtime;

/// How long the host may acknowledge nothing before that counts as one stall. Both ways of
/// measuring it, the age of the oldest command still waiting and the command time that has piled
/// up unacknowledged, are wall-clock spans, so one long frame can fill either on its own.
const STALL_MS: i32 = 1000;

/// How many stalls the link may take before the session is failed. A hitch costs one and is
/// recovered from; a host that has really gone away keeps earning them until they run out.
const STALLS_BEFORE_FAIL: u32 =
    (master_protocol::SESSION_IDLE.as_millis() / STALL_MS as u128) as u32 - 1;

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
    fn note(&mut self, now_ms: i32, acks: u64) -> Option<u32> {
        if acks != self.acks_seen {
            *self = Self { run: 0, acks_seen: acks, marked_at: Some(now_ms) };
            return None;
        }
        if self.marked_at.is_some_and(|at| now_ms.saturating_sub(at) < STALL_MS) {
            return None;
        }
        self.marked_at = Some(now_ms);
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
    for time in times {
        span = span.saturating_add(time.saturating_sub(before).max(0));
        before = time;
    }
    span
}

pub fn enforce_client_work_limits(
    pending: Res<PendingClientSends>,
    mut prediction: ResMut<ClientPredictionState>,
    cg: Res<FrameClock>,
    cls: Res<ClientRealtime>,
    mut signon: ResMut<crate::SignonState>,
    bridge: Option<Res<crate::MasterBridge>>,
    mut gate: ResMut<AuthorityInputGate>,
    actions: Res<ClientActionInbox>,
    mut stalls: Local<BacklogStalls>,
) {
    if signon.phase.is_failed() {
        gate.local_cmds_enabled = false;
        return;
    }
    let waiting_since = pending.iter().next().map(|(_, cmd, _)| cmd.server_time);
    let piled_up = span_of(prediction.0.history().iter().map(|row| row.cmd.server_time));
    let stalled = waiting_since.is_some_and(|since| cg.time().saturating_sub(since) > STALL_MS)
        || piled_up > STALL_MS;
    let stall = if stalled {
        let acks = pending.acked_through().map_or(0, |seq| u64::from(seq.0));
        stalls.note(cls.realtime(), acks)
    } else {
        stalls.forget();
        None
    };

    let reason = match judge(actions.timed_out(), stall) {
        Verdict::Carry => return,
        Verdict::Resync { stall } => {
            diag::warn!(
                Net,
                "input backlog: no authority ack in {STALL_MS} ms \
                 (stall {stall}/{STALLS_BEFORE_FAIL}) — re-adopting"
            );
            prediction.0.force_resync();
            return;
        }
        Verdict::Fail(reason) => reason,
    };

    gate.local_cmds_enabled = false;
    prediction.0.disarm();
    let match_key = crate::signon::live_match_key(bridge.as_deref());
    if let Some(bridge) = bridge {
        bridge.fail(reason);
    }
    signon.set_phase(crate::SignonPhase::Failed(crate::SignonFailReason::Transport {
        source: reason.to_owned(),
        stage: crate::FailStage::Transport,
        match_key,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_threshold_follows_the_session_idle_time() {
        let idle_seconds = master_protocol::SESSION_IDLE.as_secs() as u32;
        assert_eq!(STALLS_BEFORE_FAIL, idle_seconds - 1);
        assert!(STALLS_BEFORE_FAIL >= 1);
    }

    #[test]
    fn a_stall_needs_a_full_stall_length_after_the_last_news() {
        let mut stalls = BacklogStalls::default();
        // The count starts at zero acks, so a first call with none is already "no news".
        assert_eq!(stalls.note(0, 0), Some(1));
        assert_eq!(stalls.note(10, 5), None);
        assert_eq!(stalls.note(500, 5), None);
        assert_eq!(stalls.note(1010, 5), Some(1));
    }

    #[test]
    fn at_most_one_stall_is_counted_per_stall_length() {
        let mut stalls = BacklogStalls::default();
        assert_eq!(stalls.note(100, 0), Some(1));
        assert_eq!(stalls.note(600, 0), None);
        assert_eq!(stalls.note(1099, 0), None);
        assert_eq!(stalls.note(1100, 0), Some(2));
        assert_eq!(stalls.note(2100, 0), Some(3));
    }

    #[test]
    fn an_acknowledgement_starts_the_count_over() {
        let mut stalls = BacklogStalls::default();
        stalls.note(0, 0);
        stalls.note(1000, 0);
        assert_eq!(stalls.note(1500, 7), None);
        assert_eq!(stalls.note(2500, 7), Some(1));
    }

    #[test]
    fn forgetting_clears_everything() {
        let mut stalls = BacklogStalls::default();
        stalls.note(0, 0);
        stalls.note(1000, 0);
        stalls.forget();
        assert_eq!(stalls.note(1100, 0), Some(1));
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
        app.init_resource::<PendingClientSends>()
            .init_resource::<ClientPredictionState>()
            .init_resource::<FrameClock>()
            .init_resource::<ClientRealtime>()
            .init_resource::<crate::SignonState>()
            .insert_resource(AuthorityInputGate { local_cmds_enabled: true })
            .init_resource::<ClientActionInbox>()
            .add_systems(Update, enforce_client_work_limits);
        if failed {
            app.world_mut().resource_mut::<crate::SignonState>().set_phase(
                crate::SignonPhase::Failed(crate::SignonFailReason::Transport {
                    source: "gone".to_owned(),
                    stage: crate::FailStage::Transport,
                    match_key: crate::signon::live_match_key(None),
                }),
            );
        }
        app
    }

    #[test]
    fn a_failed_signon_switches_local_commands_off() {
        let mut app = app_with_failed_signon(true);
        app.update();
        assert!(!app.world().resource::<AuthorityInputGate>().local_cmds_enabled);
    }

    #[test]
    fn a_quiet_link_with_nothing_queued_leaves_commands_on() {
        let mut app = app_with_failed_signon(false);
        app.update();
        assert!(app.world().resource::<AuthorityInputGate>().local_cmds_enabled);
        assert!(!app.world().resource::<crate::SignonState>().phase.is_failed());
    }
}
