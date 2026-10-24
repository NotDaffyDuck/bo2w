//! The step and snapshot phases: simulate one authority frame, then publish its result and settle
//! the actions and reliable events it produced.

use bevy::prelude::*;
use frame::ExitLevelCalled;
use sim::{ActionRequestId, ClientId, SimEvent};

use super::state::{
    AuthorityPhaseTrace, AuthorityWorld, ClientShotSamples, PendingAuthorityInput,
    PendingStepResult, ServerTick, ServerTickData,
};
use crate::authority::inbox::{AuthorityClock, ClientActionInbox};
use crate::{
    AUTHORITY_MS, ActionVerdict, ClientActionLedger, PendingSvcSounds, ReliableEventHub,
};

pub(super) fn step_world(
    mut world: ResMut<AuthorityWorld>,
    clock: Res<AuthorityClock>,
    mut pending: ResMut<PendingAuthorityInput>,
    mut pending_step: ResMut<PendingStepResult>,
    mut pending_svc: ResMut<PendingSvcSounds>,
    samples: Res<ClientShotSamples>,
    mut exit_level: MessageWriter<ExitLevelCalled>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Step");
    pending_svc.occupy_cs(&mut world.0);
    let Some(input) = pending.0.take() else {
        return;
    };

    world
        .0
        .set_lagcomp_commands(samples.0.iter().map(|(key, sample)| (*key, *sample)));
    let outcome = sim::try_step(
        &mut world.0,
        sim::Tick(clock.tick),
        &input,
        AUTHORITY_MS,
        sim::StepReason::AuthorityFrame,
    );
    let snapshot = match outcome {
        Ok(snapshot) => snapshot,
        Err(_) => {
            // A terminal script error drops the match to the lobby; the world stays frozen until
            // the swap replaces it.
            if let Some(fault) = world.0.take_script_fault() {
                diag::error!(Sim, "GSC execution failed: {fault}");
                diag::script_boundary(
                    "fault",
                    &format!(" fault=\"{}\"", fault.message.replace('"', "'")),
                );
                exit_level.write(ExitLevelCalled);
            }
            return;
        }
    };
    pending_step.0 = Some(ServerTickData {
        input,
        snapshot,
        weapon_script_names: world.0.weapon_script_names(),
        pending_final_kill: world.0.take_pending_final_kill(),
        script_seats: world.0.script_seats(),
        script_exit_level: world.0.take_script_exit_level(),
    });
}

pub(super) fn publish_tick(
    mut pending_step: ResMut<PendingStepResult>,
    mut server_tick: ResMut<ServerTick>,
    mut ledger: ResMut<ClientActionLedger>,
    mut reliable: ResMut<ReliableEventHub>,
    mut actions: ResMut<ClientActionInbox>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Snapshot");
    if let Some(tick) = pending_step.0.as_ref() {
        settle_actions(&mut ledger, &mut reliable, &mut actions, tick);
        queue_reliable_events(&mut reliable, tick);
    }
    server_tick.0 = pending_step.0.take();
}

/// Does `event` refuse the action with this request id?
fn refuses(event: &SimEvent, request_id: ActionRequestId) -> bool {
    match event {
        SimEvent::ClassRejected { request_id: rid, .. }
        | SimEvent::GiveRejected { request_id: rid, .. }
        | SimEvent::ConfigurationChangeRejected { request_id: rid, .. } => *rid == request_id,
        _ => false,
    }
}

/// Record the verdict of every action that went into this tick and retire it from the inbox.
fn settle_actions(
    ledger: &mut ClientActionLedger,
    reliable: &mut ReliableEventHub,
    inbox: &mut ClientActionInbox,
    tick: &ServerTickData,
) {
    let journal = &tick.snapshot.meta.journal;
    for (client, action) in &tick.input.actions {
        let request_id = sim::action_request_id(action);
        let verdict = if journal.iter().any(|record| refuses(&record.event, request_id)) {
            ActionVerdict::Refused
        } else {
            ActionVerdict::Applied
        };
        ledger.record(*client, *action, verdict);
        reliable.push_outcome(*client, request_id, verdict);
        inbox.retire(*client, request_id);
    }
}

/// Give every client the reliable events of this tick that are addressed to it.
fn queue_reliable_events(reliable: &mut ReliableEventHub, tick: &ServerTickData) {
    let recipients: Vec<ClientId> = tick.roster();
    let reliable_records = tick
        .snapshot
        .meta
        .journal
        .iter()
        .filter(|record| sim::sim_event_is_reliable(&record.event));
    for record in reliable_records {
        for &client in recipients.iter().filter(|&&c| record.audience.projects_to(c)) {
            reliable.push_event(client, record.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{
        ClassId, ClientAction, ConfigurationChangeRejectReason, EventAudience, EventRecord,
        EventSequence, GiveRejectReason, Tick,
    };

    fn tick_with(clients: &[u32], journal: Vec<EventRecord>, actions: Vec<(ClientId, ClientAction)>) -> ServerTickData {
        let mut snapshot = sim::Snapshot::unpublished(Tick(1));
        snapshot.meta.clients = clients
            .iter()
            .map(|&id| (ClientId(id), Default::default()))
            .collect();
        snapshot.meta.journal = journal;
        ServerTickData {
            input: sim::TickInput {
                cmds: Vec::new(),
                actions,
            },
            snapshot,
            weapon_script_names: Vec::<String>::new().into(),
            pending_final_kill: None,
            script_seats: Vec::new(),
            script_exit_level: false,
        }
    }

    fn record(audience: EventAudience, event: SimEvent) -> EventRecord {
        EventRecord {
            sequence: EventSequence(0),
            tick: Tick(1),
            audience,
            event,
        }
    }

    fn give(request_id: u32) -> ClientAction {
        ClientAction::GiveWeapon {
            request_id,
            weapon: 2,
        }
    }

    #[test]
    fn rejections_refuse_only_their_own_request() {
        let give_no = SimEvent::GiveRejected {
            request_id: 5,
            weapon: 1,
            reason: GiveRejectReason::NotAlive,
        };
        let config_no = SimEvent::ConfigurationChangeRejected {
            request_id: 6,
            from: 1,
            to: 2,
            reason: ConfigurationChangeRejectReason::Busy,
        };
        let class_no = SimEvent::ClassRejected {
            request_id: 7,
            class_id: ClassId(0),
            revision: 0,
            reason: sim::ClassRejectReason::LockedContent,
        };
        assert!(refuses(&give_no, 5) && !refuses(&give_no, 6));
        assert!(refuses(&config_no, 6) && !refuses(&config_no, 5));
        assert!(refuses(&class_no, 7) && !refuses(&class_no, 8));
        assert!(!refuses(&SimEvent::AttackReleased, 5));
        let accepted = SimEvent::GiveAccepted {
            request_id: 5,
            weapon: 1,
        };
        assert!(!refuses(&accepted, 5));
    }

    #[test]
    fn a_refused_action_is_remembered_as_refused() {
        let mut ledger = ClientActionLedger::default();
        let mut reliable = ReliableEventHub::default();
        let mut inbox = ClientActionInbox::default();
        let rejection = record(
            EventAudience::Client(ClientId(1)),
            SimEvent::GiveRejected {
                request_id: 5,
                weapon: 2,
                reason: GiveRejectReason::NotAlive,
            },
        );
        let tick = tick_with(&[1], vec![rejection], vec![(ClientId(1), give(5))]);
        settle_actions(&mut ledger, &mut reliable, &mut inbox, &tick);
        assert!(matches!(
            ledger.admit(ClientId(1), &give(5)),
            crate::ActionAdmission::Repeat(ActionVerdict::Refused)
        ));
        assert_eq!(reliable.queue(ClientId(1)).map(|q| q.pending_len()), Some(1));
    }

    #[test]
    fn an_action_without_a_rejection_is_applied() {
        let mut ledger = ClientActionLedger::default();
        let mut reliable = ReliableEventHub::default();
        let mut inbox = ClientActionInbox::default();
        let tick = tick_with(&[1, 2], Vec::new(), vec![(ClientId(2), give(9))]);
        settle_actions(&mut ledger, &mut reliable, &mut inbox, &tick);
        assert!(matches!(
            ledger.admit(ClientId(2), &give(9)),
            crate::ActionAdmission::Repeat(ActionVerdict::Applied)
        ));
        assert!(reliable.queue(ClientId(1)).is_none());
    }

    #[test]
    fn reliable_events_reach_only_their_audience() {
        let mut reliable = ReliableEventHub::default();
        let accepted = SimEvent::GiveAccepted {
            request_id: 1,
            weapon: 3,
        };
        let tick = tick_with(
            &[1, 2, 3],
            vec![
                record(EventAudience::Client(ClientId(2)), accepted),
                record(EventAudience::AllExcept(ClientId(3)), SimEvent::AttackReleased),
            ],
            Vec::new(),
        );
        queue_reliable_events(&mut reliable, &tick);
        let pending = |id: u32| reliable.queue(ClientId(id)).map_or(0, |q| q.pending_len());
        let attack_reliable = sim::sim_event_is_reliable(&SimEvent::AttackReleased);
        assert_eq!(pending(1), usize::from(attack_reliable));
        assert_eq!(pending(2), 1 + usize::from(attack_reliable));
        assert_eq!(pending(3), 0);
    }

    #[test]
    fn events_for_clients_outside_the_roster_are_dropped() {
        let mut reliable = ReliableEventHub::default();
        let accepted = SimEvent::GiveAccepted {
            request_id: 1,
            weapon: 3,
        };
        let tick = tick_with(&[1], vec![record(EventAudience::Client(ClientId(8)), accepted)], Vec::new());
        queue_reliable_events(&mut reliable, &tick);
        assert!(reliable.queue(ClientId(8)).is_none());
        assert_eq!(reliable.queue(ClientId(1)).map_or(0, |q| q.pending_len()), 0);
    }

    #[test]
    fn publishing_moves_the_pending_result_into_the_server_tick() {
        let mut world = World::new();
        world.init_resource::<ServerTick>();
        world.init_resource::<ClientActionLedger>();
        world.init_resource::<ReliableEventHub>();
        world.init_resource::<ClientActionInbox>();
        world.insert_resource(PendingStepResult(Some(tick_with(&[1], Vec::new(), Vec::new()))));
        world.insert_resource(AuthorityPhaseTrace::default());
        bevy::ecs::system::RunSystemOnce::run_system_once(&mut world, publish_tick).unwrap();
        assert!(world.resource::<PendingStepResult>().0.is_none());
        assert!(world.resource::<ServerTick>().0.is_some());
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Snapshot"]);
    }

    #[test]
    fn publishing_with_nothing_pending_clears_the_server_tick() {
        let mut world = World::new();
        world.insert_resource(ServerTick(Some(tick_with(&[], Vec::new(), Vec::new()))));
        world.init_resource::<PendingStepResult>();
        world.init_resource::<ClientActionLedger>();
        world.init_resource::<ReliableEventHub>();
        world.init_resource::<ClientActionInbox>();
        bevy::ecs::system::RunSystemOnce::run_system_once(&mut world, publish_tick).unwrap();
        assert!(world.resource::<ServerTick>().0.is_none());
    }
}
