//! Resources the authority loop reads and writes between its fixed-update phases.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use sim::ClientId;

use crate::client::run_ahead::InputCommandNumber;

/// The simulation the authority steps each fixed tick.
#[derive(Resource)]
pub struct HostSimulationWorld(pub sim::SimWorld);

impl Default for HostSimulationWorld {
    fn default() -> Self {
        Self(sim::SimWorld::new())
    }
}

/// Lag-compensation sample_origin_tag for the shots each client sent, keyed by (client, command time).
#[derive(Resource, Debug, Default)]
pub struct PlayerShotOrigins(pub HashMap<(ClientId, i32), sim::ShotRecordOrigin>);

impl PlayerShotOrigins {
    pub fn record_origin(&mut self, who: ClientId, stamp: i32, record: sim::ShotRecordOrigin) {
        self.0.insert((who, stamp), record);
    }

    /// Forget every sample that belongs to `peer`.
    pub(super) fn forget_client(&mut self, peer: ClientId) {
        self.0.retain(|(owner, _), _| *owner != peer);
    }
}

/// Whether the listen-server's own player may contribute commands yet.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostInputValve {
    pub local_cmds_enabled: bool,
}

/// While set the authority does not tick (a level is loading).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostLoadingPause(pub bool);

impl HostLoadingPause {
    pub fn get(self) -> bool {
        self.0
    }
}

/// Input gathered for the tick that is about to be stepped.
#[derive(Resource, Default)]
pub struct StagedHostTickInput(pub Option<sim::StepInputs>);

/// Command acknowledgements owed to clients after this tick.
#[derive(Resource, Default)]
pub struct StagedReceipts(pub Vec<(ClientId, InputCommandNumber)>);

/// Result of the step phase, waiting for the snapshot phase to publish it.
#[derive(Resource, Default)]
pub struct StagedStepOutput(pub Option<HostStepOutput>);

/// The published result of the current tick; cleared at the start of the next one.
#[derive(Resource, Default)]
pub struct HostStepNumber(pub Option<HostStepOutput>);

/// Everything one authority step produced.
#[derive(Clone, Debug)]
pub struct HostStepOutput {
    pub input: sim::StepInputs,
    pub snapshot: sim::Snapshot,

    pub weapon_ids_by_script: Arc<[String]>,

    pub pending_last_kill: Option<(ClientId, ClientId)>,

    pub script_seats: Vec<(ClientId, sim::ScriptSeat)>,

    pub script_exit_level: bool,
}

impl HostStepOutput {
    /// Ids of every client present in this tick's snapshot.
    pub(super) fn roster(&self) -> Vec<ClientId> {
        self.snapshot.meta.clients.iter().map(|(pid, _)| *pid).collect()
    }
}

/// Last network problem, for the diagnostics overlay.
#[derive(Resource, Default)]
pub struct LinkDiagnosticsReport {
    pub line: Option<String>,
}

impl LinkDiagnosticsReport {
    pub(super) fn report(&mut self, line: String) {
        diag::info!(WireLink, "{line}");
        self.line = Some(line);
    }
}

/// Names of the phases that ran this tick, in order (used by schedule tests).
#[derive(Resource, Default, Debug, Clone)]
pub struct HostStageTrail(pub Vec<&'static str>);

impl HostStageTrail {
    /// Record `phase` when a trace resource is installed.
    pub(super) fn mark(trace: Option<ResMut<Self>>, phase: &'static str) {
        if let Some(mut trace) = trace {
            trace.0.push(phase);
        }
    }
}

/// Peers the authority must drop at the end of the tick, with the reason shown to them.
#[derive(Resource, Debug, Default)]
pub struct StagedLinkFailures(pub Vec<(ClientId, String)>);

/// Roster of the previous published tick, used to notice who left.
#[derive(Resource, Debug, Default)]
pub struct PreviousHostRoster(pub(super) Vec<ClientId>);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> sim::ShotRecordOrigin {
        sim::ShotRecordOrigin::default()
    }

    #[test]
    fn forgetting_a_client_keeps_the_others() {
        let mut samples = PlayerShotOrigins::default();
        samples.record_origin(ClientId(1), 100, sample());
        samples.record_origin(ClientId(1), 150, sample());
        samples.record_origin(ClientId(2), 100, sample());
        samples.forget_client(ClientId(1));
        assert_eq!(samples.0.len(), 1);
        assert!(samples.0.contains_key(&(ClientId(2), 100)));
    }

    #[test]
    fn a_later_sample_for_the_same_command_replaces_the_earlier_one() {
        let mut samples = PlayerShotOrigins::default();
        samples.record_origin(ClientId(3), 40, sample());
        samples.record_origin(ClientId(3), 40, sample());
        assert_eq!(samples.0.len(), 1);
    }

    #[test]
    fn load_hold_reports_its_flag() {
        assert!(HostLoadingPause(true).get());
        assert!(!HostLoadingPause::default().get());
    }

    #[test]
    fn trace_marks_only_when_installed() {
        let mut world = World::new();
        world.insert_resource(HostStageTrail::default());
        let mut state = bevy::ecs::system::SystemState::<Option<ResMut<HostStageTrail>>>::new(
            &mut world,
        );
        HostStageTrail::mark(state.get_mut(&mut world).unwrap(), "ClockStep");
        HostStageTrail::mark(None, "Ignored");
        assert_eq!(world.resource::<HostStageTrail>().0, vec!["ClockStep"]);
    }

    #[test]
    fn diagnostics_keep_the_latest_line() {
        let mut diagnostics = LinkDiagnosticsReport::default();
        diagnostics.report("first".to_owned());
        diagnostics.report("second".to_owned());
        assert_eq!(diagnostics.line.as_deref(), Some("second"));
    }
}
