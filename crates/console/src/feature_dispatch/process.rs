//! Ending the process: the exit code, and leaving the master server first.

use std::{
    io::Write,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use net::{MasterBridge, MasterBridgeState};

/// How long a quit waits for the master to confirm that this client left.
const LEAVE_WAIT: Duration = Duration::from_millis(250);

/// How often the wait looks again.
const LEAVE_POLL: Duration = Duration::from_millis(5);

fn code_of(exit: &AppExit) -> i32 {
    match exit {
        AppExit::Success => 0,
        AppExit::Error(code) => i32::from(code.get()),
    }
}

/// Stops the process the moment the app has been asked to exit, with the
/// code it asked for. The last request of a frame decides.
pub(crate) fn exit_process(mut exits: MessageReader<AppExit>, bridge: Option<Res<MasterBridge>>) {
    let Some(code) = exits.read().last().map(code_of) else {
        return;
    };
    if let Some(bridge) = bridge {
        leave_master(&bridge);
    }
    diag::lifecycle_boundary("process_exit", &format!(" code={code}"));
    diag::flush();
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

/// Tells the master this client is going, if it is in a room, and gives it a
/// moment to answer.
fn leave_master(bridge: &MasterBridge) {
    let in_a_room = matches!(
        bridge.state(),
        MasterBridgeState::Hosting { .. }
            | MasterBridgeState::Joining { .. }
            | MasterBridgeState::Joined { .. }
    );
    if !in_a_room {
        return;
    }
    bridge.leave();
    let deadline = Instant::now() + LEAVE_WAIT;
    while Instant::now() < deadline {
        if bridge.state().is_terminal() {
            return;
        }
        std::thread::sleep(LEAVE_POLL);
    }
    diag::warn!(
        Console,
        "quit: the master had not confirmed the leave after {} ms",
        LEAVE_WAIT.as_millis()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_zero_and_an_error_keeps_its_code() {
        assert_eq!(code_of(&AppExit::Success), 0);
        assert_eq!(code_of(&AppExit::from_code(3)), 3);
        assert_eq!(code_of(&AppExit::from_code(255)), 255);
    }
}
