//! Checks that the net plugin runs the right systems for each runtime role, in the order the
//! schedule tables promise. Each check returns a message saying what drifted.

mod order;
mod probe;
mod roles;
mod worker;

pub use order::{host_stage_sequence_check, receiver_directory_sequence, directory_sizes};
pub use roles::role;
pub use worker::{worker_command_layout, worker_command_inside_display};

use frame::ProcessDuty;

/// Run every check. The first that fails decides the message.
pub fn verify_all() -> Result<(), String> {
    host_stage_sequence_check()?;
    receiver_directory_sequence()?;
    directory_sizes()?;
    worker_command_layout()?;
    worker_command_inside_display()?;
    // (role, runs the authority half, runs the client half)
    for (which, authority, client) in [
        (ProcessDuty::Listen, true, true),
        (ProcessDuty::Dedicated, true, false),
        (ProcessDuty::Client, false, true),
        (ProcessDuty::Replay, false, true),
    ] {
        role(which, authority, client)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_check_passes() {
        assert_eq!(super::verify_all(), Ok(()));
    }
}
