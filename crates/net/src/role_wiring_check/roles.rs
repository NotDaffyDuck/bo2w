//! Each runtime role runs its half of the schedule, and only that.

use bevy::prelude::*;
use frame::ProcessDuty;

use super::probe::{bare_app, probe_authority_sets, probe_client_sets, trace_of};
use crate::app_wiring::NetworkingPlugin;

/// Build the net plugin for `which`, run the schedules the role is expected to have, and check
/// that exactly the expected halves ran and the role resource was set.
pub fn role(which: ProcessDuty, require_host: bool, require_receiver: bool) -> Result<(), String> {
    let mut harness = bare_app();
    match which {
        ProcessDuty::Listen => harness.add_plugins(NetworkingPlugin::listen()),
        ProcessDuty::Dedicated => harness.add_plugins(NetworkingPlugin::dedicated()),
        ProcessDuty::Client => harness.add_plugins(NetworkingPlugin::client()),
        ProcessDuty::Replay => harness.add_plugins(NetworkingPlugin::replay()),
    };
    if require_host {
        probe_authority_sets(&mut harness);
        harness.world_mut().run_schedule(FixedUpdate);
    }
    if require_receiver {
        probe_client_sets(&mut harness);
        harness.world_mut().run_schedule(Update);
    }

    let trace = trace_of(&harness);
    if trace.has_prefix("A:") != require_host {
        return Err(format!(
            "{which:?}: authority execution mismatch (trace={:?})",
            trace.0
        ));
    }
    if trace.has_prefix("C:") != require_receiver {
        return Err(format!(
            "{which:?}: client execution mismatch (trace={:?})",
            trace.0
        ));
    }
    let got = *harness.world().resource::<ProcessDuty>();
    if got != which {
        return Err(format!("{which:?}: RuntimeRole resource is {got:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listen_host_runs_both_halves() {
        assert_eq!(role(ProcessDuty::Listen, true, true), Ok(()));
    }

    #[test]
    fn a_dedicated_server_runs_only_the_authority() {
        assert_eq!(role(ProcessDuty::Dedicated, true, false), Ok(()));
    }

    #[test]
    fn clients_and_replays_run_only_the_client() {
        assert_eq!(role(ProcessDuty::Client, false, true), Ok(()));
        assert_eq!(role(ProcessDuty::Replay, false, true), Ok(()));
    }
}
