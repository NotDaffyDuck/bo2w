//! When the authority runs, what it clears when a match ends, and how it drops peers.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::{GamePaused, MatchTornDown, RuntimeRole};
use sim::ClientId;

use super::state::{
    AuthorityInputGate, AuthorityLoadHold, AuthorityWorld, ClientShotSamples,
    LastAuthorityRoster, PendingAcks, PendingAuthorityInput, PendingConnectionFaults,
    PendingStepResult, ServerTick,
};
use crate::authority::inbox::{AuthorityClock, ClientActionInbox, ClientCommandInbox};
use crate::transport::archive::FrameArchive;
use crate::transport::loopback_live::ListenLoopback;
use crate::transport::relay::UdpAuthorityHub;
use crate::{ActionRequestIds, ClientActionLedger, ReliableEventHub};

/// Run condition for every authority system.
pub fn authority_should_tick(
    role: Res<RuntimeRole>,
    world: Option<Res<AuthorityWorld>>,
    hold: Option<Res<AuthorityLoadHold>>,
    paused: Option<Res<GamePaused>>,
) -> bool {
    let has_level = world.is_some_and(|world| world.0.clip_brush_count() > 0);
    let loading = hold.is_some_and(|hold| hold.0);
    // A solo game's pause menu stops the server; a hosted game keeps running.
    let solo_paused = *role == RuntimeRole::Listen && paused.is_some_and(|p| p.0);
    role.runs_authority() && has_level && !loading && !solo_paused
}

/// The per-client bookkeeping that belongs to one match and starts over with the next.
#[derive(SystemParam)]
pub struct ActionTransactionScope<'w> {
    actions: ResMut<'w, ClientActionInbox>,
    ledger: ResMut<'w, ClientActionLedger>,
    reliable: ResMut<'w, ReliableEventHub>,
    request_ids: ResMut<'w, ActionRequestIds>,
    roster: ResMut<'w, LastAuthorityRoster>,
}

impl ActionTransactionScope<'_> {
    pub(super) fn open_new_scope(&mut self) {
        self.actions.clear();
        self.ledger.open_scope();
        self.reliable.clear();
        self.request_ids.reset();
        *self.roster = LastAuthorityRoster::default();
    }
}

/// Everything the tick pipeline carries between phases; emptied when a match is torn down.
#[derive(SystemParam)]
pub(super) struct TickHandoff<'w> {
    gate: ResMut<'w, AuthorityInputGate>,
    input: ResMut<'w, PendingAuthorityInput>,
    acks: ResMut<'w, PendingAcks>,
    stepped: ResMut<'w, PendingStepResult>,
    published: ResMut<'w, ServerTick>,
}

impl TickHandoff<'_> {
    fn discard_all(&mut self) {
        *self.gate = AuthorityInputGate::default();
        self.input.0 = None;
        self.acks.0.clear();
        self.stepped.0 = None;
        self.published.0 = None;
    }
}

pub(super) fn reset_on_match_torn_down(
    mut torn: MessageReader<MatchTornDown>,
    mut clock: ResMut<AuthorityClock>,
    mut world: Option<ResMut<AuthorityWorld>>,
    mut loopback: Option<ResMut<ListenLoopback>>,
    mut archive: Option<ResMut<FrameArchive>>,
    mut inbox: ResMut<ClientCommandInbox>,
    mut scope: ActionTransactionScope,
    mut samples: ResMut<ClientShotSamples>,
    mut handoff: TickHandoff,
) {
    if torn.read().count() == 0 {
        return;
    }
    *clock = AuthorityClock::default();
    inbox.clear();
    scope.open_new_scope();
    samples.0.clear();
    handoff.discard_all();

    if let Some(world) = world.as_mut() {
        world.0.shutdown_game();
    }
    if let Some(loopback) = loopback.as_mut() {
        loopback.reset();
    }
    if let Some(archive) = archive.as_mut() {
        archive.clear();
    }
    let (running, movers) = world.as_ref().map_or((false, 0), |world| {
        (world.0.is_running(), world.0.script_mover_count() as i64)
    });
    let queued = loopback.as_ref().map_or(0, |l| l.pending() as i64);
    perf::sim_hold(i64::from(running), movers, queued);
}

pub(super) fn apply_connection_faults(
    mut faults: ResMut<PendingConnectionFaults>,
    mut world: ResMut<AuthorityWorld>,
    mut hub: Option<ResMut<UdpAuthorityHub>>,
) {
    for (client, reason) in faults.0.drain(..) {
        if let Some(hub) = hub.as_mut() {
            hub.retire_client_with_reason(client, &reason);
        }
        world.0.retire_client(client);
    }
}

/// Drop every queue entry of a client that left the roster since the previous tick.
pub(super) fn retire_departed_peers(
    server_tick: Res<ServerTick>,
    mut last: ResMut<LastAuthorityRoster>,
    mut cmds: ResMut<ClientCommandInbox>,
    mut actions: ResMut<ClientActionInbox>,
    mut ledger: ResMut<ClientActionLedger>,
    mut reliable: ResMut<ReliableEventHub>,
    mut samples: ResMut<ClientShotSamples>,
    mut hub: Option<ResMut<UdpAuthorityHub>>,
    mut staged: ResMut<PendingAuthorityInput>,
    mut acks: ResMut<PendingAcks>,
) {
    let Some(tick) = server_tick.0.as_ref() else {
        return;
    };
    let roster = tick.roster();
    let departed: Vec<ClientId> = last
        .0
        .iter()
        .copied()
        .filter(|client| !roster.contains(client))
        .collect();
    for client in departed {
        if let Some(hub) = hub.as_mut() {
            hub.retire_client(client);
        }
        if let Some(input) = staged.0.as_mut() {
            input.cmds.retain(|(id, _)| *id != client);
            input.actions.retain(|(id, _)| *id != client);
        }
        acks.0.retain(|(id, _)| *id != client);
        cmds.retire_client(client);
        actions.retire_client(client);
        ledger.retire_client(client);
        reliable.retire(client);
        samples.forget_client(client);
        diag::info!(Net, "retire: client {} dropped from the roster", client.0);
    }
    last.0 = roster;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use frame::{MatchKey, TeardownReason, WorldGeneration};

    fn gate_world(role: RuntimeRole) -> World {
        let mut world = World::new();
        world.insert_resource(role);
        world
    }

    fn should_tick(world: &mut World) -> bool {
        world
            .run_system_once(authority_should_tick)
            .expect("run condition ran")
    }

    #[test]
    fn a_client_never_runs_the_authority() {
        let mut world = gate_world(RuntimeRole::Client);
        assert!(!should_tick(&mut world));
    }

    #[test]
    fn a_server_without_a_level_does_not_tick() {
        let mut world = gate_world(RuntimeRole::Dedicated);
        assert!(!should_tick(&mut world), "no world resource");
        world.insert_resource(AuthorityWorld::default());
        assert!(!should_tick(&mut world), "world has no clip brushes yet");
    }

    #[test]
    fn pausing_stops_a_listen_server_only() {
        // Only the role/pause/hold combinations that do not need a loaded level are checked here;
        // with no clip brushes every combination must stay false, never panic.
        for role in [RuntimeRole::Listen, RuntimeRole::Dedicated] {
            let mut world = gate_world(role);
            world.insert_resource(AuthorityWorld::default());
            world.insert_resource(GamePaused(true));
            world.insert_resource(AuthorityLoadHold(true));
            assert!(!should_tick(&mut world));
        }
    }

    fn teardown_world() -> World {
        let mut world = World::new();
        world.init_resource::<Messages<MatchTornDown>>();
        world.init_resource::<AuthorityClock>();
        world.init_resource::<ClientCommandInbox>();
        world.init_resource::<ClientActionInbox>();
        world.init_resource::<ClientActionLedger>();
        world.init_resource::<ReliableEventHub>();
        world.init_resource::<ActionRequestIds>();
        world.init_resource::<LastAuthorityRoster>();
        world.init_resource::<ClientShotSamples>();
        world.init_resource::<AuthorityInputGate>();
        world.init_resource::<PendingAuthorityInput>();
        world.init_resource::<PendingAcks>();
        world.init_resource::<PendingStepResult>();
        world.init_resource::<ServerTick>();
        world
    }

    fn tear_down_message() -> MatchTornDown {
        MatchTornDown {
            reason: TeardownReason::MatchEnded,
            world_generation: WorldGeneration(None),
            match_key: MatchKey::NONE,
            match_epoch: 0,
        }
    }

    #[test]
    fn nothing_is_cleared_without_a_teardown_message() {
        let mut world = teardown_world();
        world.resource_mut::<AuthorityClock>().tick = 40;
        world.resource_mut::<PendingAcks>().0.push((ClientId(1), Default::default()));
        world.run_system_once(reset_on_match_torn_down).unwrap();
        assert_eq!(world.resource::<AuthorityClock>().tick, 40);
        assert_eq!(world.resource::<PendingAcks>().0.len(), 1);
    }

    #[test]
    fn a_teardown_resets_the_clock_and_the_tick_handoff() {
        let mut world = teardown_world();
        {
            let mut clock = world.resource_mut::<AuthorityClock>();
            clock.tick = 40;
            clock.time_ms = 2000;
        }
        world.resource_mut::<AuthorityInputGate>().local_cmds_enabled = true;
        world.resource_mut::<PendingAcks>().0.push((ClientId(1), Default::default()));
        world.resource_mut::<PendingAuthorityInput>().0 = Some(sim::TickInput::default());
        world.resource_mut::<ClientShotSamples>().note(ClientId(1), 5, Default::default());
        world.resource_mut::<Messages<MatchTornDown>>().write(tear_down_message());

        world.run_system_once(reset_on_match_torn_down).unwrap();

        let clock = world.resource::<AuthorityClock>();
        assert_eq!((clock.tick, clock.time_ms), (0, 0));
        assert!(!world.resource::<AuthorityInputGate>().local_cmds_enabled);
        assert!(world.resource::<PendingAcks>().0.is_empty());
        assert!(world.resource::<PendingAuthorityInput>().0.is_none());
        assert!(world.resource::<ClientShotSamples>().0.is_empty());
    }

    #[test]
    fn a_roster_change_forgets_the_departed_peers_data() {
        let mut world = teardown_world();
        world.init_resource::<LastAuthorityRoster>();
        world.resource_mut::<LastAuthorityRoster>().0 = vec![ClientId(1), ClientId(2)];
        world.resource_mut::<PendingAcks>().0.push((ClientId(1), Default::default()));
        world.resource_mut::<PendingAcks>().0.push((ClientId(2), Default::default()));
        world.resource_mut::<ClientShotSamples>().note(ClientId(1), 5, Default::default());
        world.resource_mut::<ClientShotSamples>().note(ClientId(2), 5, Default::default());
        let mut input = sim::TickInput::default();
        input.cmds.push((ClientId(1), Default::default()));
        input.cmds.push((ClientId(2), Default::default()));
        world.resource_mut::<PendingAuthorityInput>().0 = Some(input);
        world.resource_mut::<ServerTick>().0 = Some(super::super::state::ServerTickData {
            input: sim::TickInput::default(),
            snapshot: sim::Snapshot::unpublished(sim::Tick(3)),
            weapon_script_names: Vec::<String>::new().into(),
            pending_final_kill: None,
            script_seats: Vec::new(),
            script_exit_level: false,
        });

        world.run_system_once(retire_departed_peers).unwrap();

        // The published snapshot has an empty roster, so both peers have left.
        assert!(world.resource::<PendingAcks>().0.is_empty());
        assert!(world.resource::<ClientShotSamples>().0.is_empty());
        let staged = world.resource::<PendingAuthorityInput>().0.as_ref().unwrap();
        assert!(staged.cmds.is_empty());
        assert!(world.resource::<LastAuthorityRoster>().0.is_empty());
    }

    #[test]
    fn without_a_published_tick_the_roster_is_left_alone() {
        let mut world = teardown_world();
        world.resource_mut::<LastAuthorityRoster>().0 = vec![ClientId(9)];
        world.run_system_once(retire_departed_peers).unwrap();
        assert_eq!(world.resource::<LastAuthorityRoster>().0, vec![ClientId(9)]);
    }

    #[test]
    fn connection_faults_retire_the_client_from_the_world() {
        let mut world = World::new();
        world.insert_resource(AuthorityWorld::default());
        world.insert_resource(PendingConnectionFaults(vec![(
            ClientId(4),
            "ActionQueueOverflow: outcome unknown".to_owned(),
        )]));
        world.run_system_once(apply_connection_faults).unwrap();
        assert!(world.resource::<PendingConnectionFaults>().0.is_empty());
    }
}
