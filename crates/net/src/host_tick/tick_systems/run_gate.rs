//! The authority's run gate, the match-end wipe of its per-match state, and peer retirement.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{GamePaused, GameSessionClosed, ProcessDuty};
use sim::ClientId;

use super::loop_resources::{
    HostInputValve, HostLoadingPause, HostSimulationWorld, PlayerShotOrigins,
    PreviousHostRoster, StagedReceipts, StagedHostTickInput, StagedLinkFailures,
    StagedStepOutput, HostStepNumber,
};
use crate::host_tick::peer_mail::{HostTimeKeeper, PlayerActionIntake, PlayerMoveIntake};
use crate::net_links::snapshot_history::SnapshotVault;
use crate::net_links::listen_link::LocalHostTickPipe;
use crate::net_links::relay::HostUdpSwitchboard;
use crate::{ActionRequestCounter, PlayerActionBook, GuaranteedEventRouter};

/// Run condition for every authority system.
pub fn host_due_for_step(
    role: Res<ProcessDuty>,
    world: Option<Res<HostSimulationWorld>>,
    hold: Option<Res<HostLoadingPause>>,
    paused: Option<Res<GamePaused>>,
) -> bool {
    if !role.hosts_simulation() {
        return false;
    }
    // No clip geometry means no level has been loaded into the sim yet.
    let Some(sim_world) = world else { return false };
    if sim_world.0.solid_brush_total() == 0 {
        return false;
    }
    if hold.map_or(false, |h| h.0) {
        return false;
    }
    // The pause menu freezes a listen (solo) server only; dedicated servers keep going.
    let menu_open = paused.map_or(false, |p| p.0);
    !(menu_open && *role == ProcessDuty::Listen)
}

/// The per-client bookkeeping that belongs to one match and starts over with the next.
#[derive(SystemParam)]
pub struct ActionBatchScope<'w> {
    commands: ResMut<'w, PlayerMoveIntake>,
    action_queue: ResMut<'w, PlayerActionIntake>,
    action_ledger: ResMut<'w, PlayerActionBook>,
    reliable_events: ResMut<'w, GuaranteedEventRouter>,
    ids: ResMut<'w, ActionRequestCounter>,
    shot_samples: ResMut<'w, PlayerShotOrigins>,
    previous_roster: ResMut<'w, PreviousHostRoster>,
}

impl ActionBatchScope<'_> {
    /// Forgets every client's commands, actions, reliable events and shot samples.
    pub(super) fn open_new_scope(&mut self) {
        self.previous_roster.0.clear();
        self.shot_samples.0.clear();
        self.ids.restart();
        self.reliable_events.clear();
        self.action_ledger.begin_timed_region();
        self.action_queue.clear();
        self.commands.clear();
    }
}

/// What travels between the phases of one authority tick; dropped when a match is torn down.
#[derive(SystemParam)]
pub(super) struct TickHandoff<'w> {
    gate: ResMut<'w, HostInputValve>,
    staged_input: ResMut<'w, StagedHostTickInput>,
    unsent_acks: ResMut<'w, StagedReceipts>,
    step_result: ResMut<'w, StagedStepOutput>,
    latest_step_seen: ResMut<'w, HostStepNumber>,
}

impl TickHandoff<'_> {
    fn discard_all(&mut self) {
        self.latest_step_seen.0.take();
        self.step_result.0.take();
        self.unsent_acks.0.clear();
        self.staged_input.0.take();
        std::mem::take(&mut *self.gate);
    }
}

/// The match-owned pieces that only exist while a level or a listen session does.
#[derive(SystemParam)]
pub(super) struct MatchOwned<'w> {
    sim: Option<ResMut<'w, HostSimulationWorld>>,
    loopback: Option<ResMut<'w, LocalHostTickPipe>>,
    archive: Option<ResMut<'w, SnapshotVault>>,
}

impl MatchOwned<'_> {
    fn shut_down(&mut self) {
        if let Some(sim) = &mut self.sim {
            sim.0.end_game_session();
        }
        if let Some(link) = &mut self.loopback {
            link.empty_out();
        }
        if let Some(frames) = &mut self.archive {
            frames.clear();
        }
    }

    /// Reports what is still alive after the shutdown to the perf counters.
    fn report_hold(&self) {
        let (running, movers) = match &self.sim {
            Some(sim) => (sim.0.is_running(), sim.0.scripted_mover_total() as i64),
            None => (false, 0),
        };
        let queued = self.loopback.as_ref().map(|link| link.queued_len() as i64).unwrap_or(0);
        perf::sim_hold(running as i64, movers, queued);
    }
}

pub(super) fn reset_on_match_torn_down(
    mut torn: MessageReader<GameSessionClosed>,
    mut clock: ResMut<HostTimeKeeper>,
    mut scope: ActionBatchScope,
    mut handoff: TickHandoff,
    mut owned: MatchOwned,
) {
    let closed_count = torn.read().count();
    if closed_count > 0 {
        std::mem::take(&mut *clock);
        scope.open_new_scope();
        handoff.discard_all();
        owned.shut_down();
        owned.report_hold();
    }
}

pub(super) fn inject_link_impairments(
    mut pending: ResMut<StagedLinkFailures>,
    mut sim: ResMut<HostSimulationWorld>,
    mut relay: Option<ResMut<HostUdpSwitchboard>>,
) {
    let faults = std::mem::take(&mut pending.0);
    for (peer, why) in faults {
        if let Some(relay) = &mut relay {
            relay.drop_seat(peer, &why);
        }
        sim.0.drop_player_record(peer);
    }
}

/// Every queue that may still hold entries for a single peer.
#[derive(SystemParam)]
pub(super) struct PeerQueues<'w> {
    relay: Option<ResMut<'w, HostUdpSwitchboard>>,
    staged_input: ResMut<'w, StagedHostTickInput>,
    unsent_acks: ResMut<'w, StagedReceipts>,
    commands: ResMut<'w, PlayerMoveIntake>,
    action_queue: ResMut<'w, PlayerActionIntake>,
    action_ledger: ResMut<'w, PlayerActionBook>,
    reliable_events: ResMut<'w, GuaranteedEventRouter>,
    shot_samples: ResMut<'w, PlayerShotOrigins>,
}

impl PeerQueues<'_> {
    fn forget(&mut self, peer: ClientId) {
        if let Some(relay) = &mut self.relay {
            relay.drop_player_record(peer);
        }
        if let Some(staged) = &mut self.staged_input.0 {
            staged.cmds.retain(|entry| entry.0 != peer);
            staged.actions.retain(|entry| entry.0 != peer);
        }
        self.unsent_acks.0.retain(|entry| entry.0 != peer);
        self.commands.drop_player_record(peer);
        self.action_queue.drop_player_record(peer);
        self.action_ledger.drop_player_record(peer);
        self.reliable_events.drop_backlog(peer);
        self.shot_samples.forget_client(peer);
    }
}

/// Drop every queue entry of a client that left the roster since the previous tick.
pub(super) fn discard_gone_peers(
    published: Res<HostStepNumber>,
    mut previous: ResMut<PreviousHostRoster>,
    mut queues: PeerQueues,
) {
    let Some(data) = &published.0 else { return };
    let present = data.roster();
    let before = std::mem::replace(&mut previous.0, present);
    for peer in before.into_iter().filter(|old| !previous.0.contains(old)) {
        queues.forget(peer);
        diag::info!(WireLink, "retire: peer {} removed from the roster", peer.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use frame::{GameSessionStamp, ShutdownCause, LevelGeneration};

    fn gate_says(role: ProcessDuty, extra: impl FnOnce(&mut World)) -> bool {
        let mut world = World::new();
        world.insert_resource(role);
        extra(&mut world);
        world.run_system_once(host_due_for_step).expect("run condition ran")
    }

    #[test]
    fn a_client_never_runs_the_authority() {
        assert!(!gate_says(ProcessDuty::Client, |_| {}));
    }

    #[test]
    fn a_server_without_a_level_does_not_tick() {
        assert!(!gate_says(ProcessDuty::Dedicated, |_| {}), "no world resource");
        let empty_level = |w: &mut World| w.insert_resource(HostSimulationWorld::default());
        assert!(!gate_says(ProcessDuty::Dedicated, empty_level), "world has no clip brushes yet");
    }

    #[test]
    fn pausing_stops_a_listen_server_only() {
        // Only combinations that need no loaded level are checked: with no clip brushes the gate
        // must stay shut for every role/pause/hold mix, and never panic.
        let everything_on = |w: &mut World| {
            w.insert_resource(HostSimulationWorld::default());
            w.insert_resource(GamePaused(true));
            w.insert_resource(HostLoadingPause(true));
        };
        assert!(!gate_says(ProcessDuty::Listen, everything_on));
        assert!(!gate_says(ProcessDuty::Dedicated, everything_on));
    }

    fn teardown_world() -> World {
        let mut world = World::new();
        world.init_resource::<Messages<GameSessionClosed>>();
        world.init_resource::<HostTimeKeeper>();
        world.init_resource::<PlayerMoveIntake>();
        world.init_resource::<PlayerActionIntake>();
        world.init_resource::<PlayerActionBook>();
        world.init_resource::<GuaranteedEventRouter>();
        world.init_resource::<ActionRequestCounter>();
        world.init_resource::<PreviousHostRoster>();
        world.init_resource::<PlayerShotOrigins>();
        world.init_resource::<HostInputValve>();
        world.init_resource::<StagedHostTickInput>();
        world.init_resource::<StagedReceipts>();
        world.init_resource::<StagedStepOutput>();
        world.init_resource::<HostStepNumber>();
        world
    }

    fn ack_from(world: &mut World, peer: u32) {
        world.resource_mut::<StagedReceipts>().0.push((ClientId(peer), Default::default()));
    }

    fn sample_from(world: &mut World, peer: u32) {
        world.resource_mut::<PlayerShotOrigins>().record_origin(ClientId(peer), 5, Default::default());
    }

    #[test]
    fn nothing_is_cleared_without_a_teardown_message() {
        let mut world = teardown_world();
        world.resource_mut::<HostTimeKeeper>().tick = 40;
        ack_from(&mut world, 1);
        world.run_system_once(reset_on_match_torn_down).unwrap();
        assert_eq!(world.resource::<HostTimeKeeper>().tick, 40);
        assert_eq!(world.resource::<StagedReceipts>().0.len(), 1);
    }

    #[test]
    fn a_teardown_resets_the_clock_and_the_tick_handoff() {
        let mut world = teardown_world();
        let mut clock = world.resource_mut::<HostTimeKeeper>();
        (clock.tick, clock.clock_stamp_ms) = (40, 2000);
        world.resource_mut::<HostInputValve>().local_cmds_enabled = true;
        ack_from(&mut world, 1);
        sample_from(&mut world, 1);
        world.resource_mut::<StagedHostTickInput>().0 = Some(sim::StepInputs::default());
        let ended = GameSessionClosed {
            reason: ShutdownCause::RoundOver,
            level_generation: LevelGeneration(None),
            game_session_key: GameSessionStamp::NONE,
            game_generation: 0,
        };
        world.resource_mut::<Messages<GameSessionClosed>>().write(ended);

        world.run_system_once(reset_on_match_torn_down).unwrap();

        let clock = world.resource::<HostTimeKeeper>();
        assert_eq!([clock.tick as i64, clock.clock_stamp_ms as i64], [0, 0]);
        assert_eq!(world.resource::<HostInputValve>().local_cmds_enabled, false);
        assert_eq!(world.resource::<StagedReceipts>().0.len(), 0);
        assert!(world.resource::<StagedHostTickInput>().0.is_none());
        assert_eq!(world.resource::<PlayerShotOrigins>().0.len(), 0);
    }

    #[test]
    fn a_roster_change_forgets_the_departed_peers_data() {
        let mut world = teardown_world();
        world.resource_mut::<PreviousHostRoster>().0 = vec![ClientId(1), ClientId(2)];
        let mut staged = sim::StepInputs::default();
        for peer in [1, 2] {
            ack_from(&mut world, peer);
            sample_from(&mut world, peer);
            staged.cmds.push((ClientId(peer), Default::default()));
        }
        world.resource_mut::<StagedHostTickInput>().0 = Some(staged);
        // A published tick whose snapshot has an empty roster: both peers have left.
        let published = super::super::loop_resources::HostStepOutput {
            input: sim::StepInputs::default(),
            snapshot: sim::Snapshot::unpublished(sim::Tick(3)),
            weapon_ids_by_script: Vec::<String>::new().into(),
            pending_last_kill: None,
            script_seats: Vec::new(),
            script_exit_level: false,
        };
        world.resource_mut::<HostStepNumber>().0 = Some(published);

        world.run_system_once(discard_gone_peers).unwrap();

        assert_eq!(world.resource::<StagedReceipts>().0.len(), 0);
        assert_eq!(world.resource::<PlayerShotOrigins>().0.len(), 0);
        let left = world.resource::<StagedHostTickInput>().0.as_ref().map(|i| i.cmds.len());
        assert_eq!(left, Some(0));
        assert_eq!(world.resource::<PreviousHostRoster>().0.len(), 0);
    }

    #[test]
    fn without_a_published_tick_the_roster_is_left_alone() {
        let mut world = teardown_world();
        world.resource_mut::<PreviousHostRoster>().0 = vec![ClientId(9)];
        world.run_system_once(discard_gone_peers).unwrap();
        assert_eq!(world.resource::<PreviousHostRoster>().0, [ClientId(9)]);
    }

    #[test]
    fn connection_faults_retire_the_client_from_the_world() {
        let mut world = World::new();
        world.insert_resource(HostSimulationWorld::default());
        let fault = (ClientId(4), String::from("ActionQueueOverflow: outcome unknown"));
        world.insert_resource(StagedLinkFailures(vec![fault]));
        world.run_system_once(inject_link_impairments).unwrap();
        assert_eq!(world.resource::<StagedLinkFailures>().0.len(), 0);
    }
}
