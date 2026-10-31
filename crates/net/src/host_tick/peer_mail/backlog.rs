//! The limits on how much unconsumed input one peer may hold, and the fault that names which
//! limit it broke.

/// Commands a client repeats in every packet, and the most one peer may keep queued.
pub const MOVE_RESEND_WINDOW: usize = crate::client::run_ahead::DEFAULT_MOVE_LOG_LIMIT;

pub const PEER_MOVE_BACKLOG_LIMIT: usize = MOVE_RESEND_WINDOW;

/// The most command time one peer may have queued ahead of the authority.
pub const MOVE_BACKLOG_SPAN_LIMIT: i32 = 1000;

/// A gap between two queued commands counts for at most this much command time.
pub(crate) const COMMAND_TIME_STEP_MS: i32 = 200;

/// How long the head of a queue may stay stuck before the peer is faulted.
pub const MAX_COMMAND_STALL_MS: i32 = 1000;

/// How far ahead of the authority clock a command may be stamped and still wait
/// for its tick: BO2's `ClientThink_real` clamps `serverTime` to `level.time + 200`.
pub const MAX_COMMAND_AHEAD_MS: i32 = 200;

/// A command due within BO2's window waits for its tick. One stamped further
/// ahead (sent while the host stalled) runs now, like BO2's clamp, instead of
/// blocking the queue until the no-progress kick.
pub(crate) fn command_waits(authority_clock_ms: i32, clock_millis: i32) -> bool {
    let ahead = authority_clock_ms.saturating_sub(clock_millis);
    ahead > 0 && ahead <= MAX_COMMAND_AHEAD_MS
}

/// The most commands one peer can have consumed in a single authority frame.
pub const PEER_INPUT_CAP_PER_STEP: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveQueueOverflow {
    BacklogLength { queued: usize },

    QueueDuration { buffered_span_millis: i32 },

    NoProgress { stalled_ms: i32 },
}

impl core::fmt::Display for MoveQueueOverflow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BacklogLength { queued } => write!(
                f,
                "InputBacklogExceeded: {queued} unique commands queued (max \
                 {PEER_MOVE_BACKLOG_LIMIT})"
            ),
            Self::QueueDuration { buffered_span_millis } => write!(
                f,
                "InputBacklogExceeded: {buffered_span_millis} ms of command time queued (max \
                 {MOVE_BACKLOG_SPAN_LIMIT})"
            ),
            Self::NoProgress { stalled_ms } => write!(
                f,
                "InputBacklogExceeded: command queue made no progress for {stalled_ms} ms (max \
                 {MAX_COMMAND_STALL_MS})"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_fault_names_its_limit() {
        let count = MoveQueueOverflow::BacklogLength { queued: 300 }.to_string();
        assert!(count.starts_with("InputBacklogExceeded: 300 unique commands queued"));
        assert!(count.contains(&PEER_MOVE_BACKLOG_LIMIT.to_string()));

        let duration = MoveQueueOverflow::QueueDuration { buffered_span_millis: 1500 }.to_string();
        assert!(duration.contains("1500 ms of command time queued"));
        assert!(duration.contains("max 1000"));

        let stall = MoveQueueOverflow::NoProgress { stalled_ms: 1200 }.to_string();
        assert!(stall.contains("no progress for 1200 ms"));
    }

    #[test]
    fn the_queue_limit_follows_the_redundancy_window() {
        assert_eq!(PEER_MOVE_BACKLOG_LIMIT, MOVE_RESEND_WINDOW);
    }
}
