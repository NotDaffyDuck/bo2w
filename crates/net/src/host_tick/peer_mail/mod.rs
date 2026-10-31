//! What the authority has received from its peers and not yet consumed: movement commands and
//! gameplay actions, plus the fixed-rate authority clock they are stamped against.

mod backlog;
mod clock;
mod commands;
mod requests;

pub use backlog::{
    MoveQueueOverflow, MAX_COMMAND_STALL_MS, PEER_INPUT_CAP_PER_STEP,
    MOVE_BACKLOG_SPAN_LIMIT, PEER_MOVE_BACKLOG_LIMIT, MOVE_RESEND_WINDOW,
};
pub(crate) use backlog::COMMAND_TIME_STEP_MS;
pub use clock::{HOST_TICK_FREQUENCY, HOST_STEP_MILLIS, HostTimeKeeper, HostInstant, execute_steady_host_loop};
pub use commands::{PlayerMoveIntake, CollectedMoves};
pub use requests::{ActionQueueRefusal, PlayerActionIntake, PLAYER_ACTION_QUEUE_LIMIT};
