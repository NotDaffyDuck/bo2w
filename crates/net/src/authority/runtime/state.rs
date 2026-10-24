//! Resources the authority loop reads and writes between its fixed-update phases.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use sim::ClientId;

use crate::client::predict::CmdSeq;

/// The simulation the authority steps each fixed tick.
#[derive(Resource)]
pub struct AuthorityWorld(pub sim::SimWorld);

impl Default for AuthorityWorld {
    fn default() -> Self {
        Self(sim::SimWorld::new())
    }
}

/// Lag-compensation provenance for the shots each client sent, keyed by (client, command time).
#[derive(Resource, Debug, Default)]
pub struct ClientShotSamples(pub HashMap<(ClientId, i32), sim::ShotSampleProvenance>);

impl ClientShotSamples {
    pub fn note(&mut self, id: ClientId, command_time: i32, sample: sim::ShotSampleProvenance) {
        self.0.insert((id, command_time), sample);
    }

    /// Forget every sample that belongs to `client`.
    pub(super) fn forget_client(&mut self, client: ClientId) {
        self.0.retain(|(owner, _), _| *owner != client);
    }
}

/// Whether the listen-server's own player may contribute commands yet.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityInputGate {
    pub local_cmds_enabled: bool,
}

/// While set the authority does not tick (a level is loading).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityLoadHold(pub bool);

impl AuthorityLoadHold {
    pub fn get(self) -> bool {
        self.0
    }
}

/// Input gathered for the tick that is about to be stepped.
#[derive(Resource, Default)]
pub struct PendingAuthorityInput(pub Option<sim::TickInput>);

/// Command acknowledgements owed to clients after this tick.
#[derive(Resource, Default)]
pub struct PendingAcks(pub Vec<(ClientId, CmdSeq)>);

/// Result of the step phase, waiting for the snapshot phase to publish it.
#[derive(Resource, Default)]
pub struct PendingStepResult(pub Option<ServerTickData>);

/// The published result of the current tick; cleared at the start of the next one.
#[derive(Resource, Default)]
pub struct ServerTick(pub Option<ServerTickData>);

/// Everything one authority step produced.
#[derive(Clone, Debug)]
pub struct ServerTickData {
    pub input: sim::TickInput,
    pub snapshot: sim::Snapshot,

    pub weapon_script_names: Arc<[String]>,

    pub pending_final_kill: Option<(ClientId, ClientId)>,

    pub script_seats: Vec<(ClientId, sim::ScriptSeat)>,

    pub script_exit_level: bool,
}

impl ServerTickData {
    /// Ids of every client present in this tick's snapshot.
    pub(super) fn roster(&self) -> Vec<ClientId> {
        self.snapshot.meta.clients.iter().map(|(id, _)| *id).collect()
    }
}

/// Last network problem, for the diagnostics overlay.
#[derive(Resource, Default)]
pub struct NetDiagnostics {
    pub line: Option<String>,
}

impl NetDiagnostics {
    pub(super) fn report(&mut self, line: String) {
        diag::info!(Net, "{line}");
        self.line = Some(line);
    }
}

/// Names of the phases that ran this tick, in order (used by schedule tests).
#[derive(Resource, Default, Debug, Clone)]
pub struct AuthorityPhaseTrace(pub Vec<&'static str>);

impl AuthorityPhaseTrace {
    /// Record `phase` when a trace resource is installed.
    pub(super) fn mark(trace: Option<ResMut<Self>>, phase: &'static str) {
        if let Some(mut trace) = trace {
            trace.0.push(phase);
        }
    }
}

/// Peers the authority must drop at the end of the tick, with the reason shown to them.
#[derive(Resource, Debug, Default)]
pub struct PendingConnectionFaults(pub Vec<(ClientId, String)>);

/// Roster of the previous published tick, used to notice who left.
#[derive(Resource, Debug, Default)]
pub struct LastAuthorityRoster(pub(super) Vec<ClientId>);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> sim::ShotSampleProvenance {
        sim::ShotSampleProvenance::default()
    }

    #[test]
    fn forgetting_a_client_keeps_the_others() {
        let mut samples = ClientShotSamples::default();
        samples.note(ClientId(1), 100, sample());
        samples.note(ClientId(1), 150, sample());
        samples.note(ClientId(2), 100, sample());
        samples.forget_client(ClientId(1));
        assert_eq!(samples.0.len(), 1);
        assert!(samples.0.contains_key(&(ClientId(2), 100)));
    }

    #[test]
    fn a_later_sample_for_the_same_command_replaces_the_earlier_one() {
        let mut samples = ClientShotSamples::default();
        samples.note(ClientId(3), 40, sample());
        samples.note(ClientId(3), 40, sample());
        assert_eq!(samples.0.len(), 1);
    }

    #[test]
    fn load_hold_reports_its_flag() {
        assert!(AuthorityLoadHold(true).get());
        assert!(!AuthorityLoadHold::default().get());
    }

    #[test]
    fn trace_marks_only_when_installed() {
        let mut world = World::new();
        world.insert_resource(AuthorityPhaseTrace::default());
        let mut state = bevy::ecs::system::SystemState::<Option<ResMut<AuthorityPhaseTrace>>>::new(
            &mut world,
        );
        AuthorityPhaseTrace::mark(state.get_mut(&mut world).unwrap(), "Advance");
        AuthorityPhaseTrace::mark(None, "Ignored");
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Advance"]);
    }

    #[test]
    fn diagnostics_keep_the_latest_line() {
        let mut diagnostics = NetDiagnostics::default();
        diagnostics.report("first".to_owned());
        diagnostics.report("second".to_owned());
        assert_eq!(diagnostics.line.as_deref(), Some("second"));
    }
}
