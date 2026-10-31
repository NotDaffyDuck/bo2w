//! The step and snapshot phases: simulate one authority frame, then publish its result and settle
//! the actions and guaranteed events it produced.

use bevy::prelude::*;
use frame::LevelQuitRequested;
use sim::{IntentSerial, ClientId, MatchOccurrence};

use super::loop_resources::{
    HostStageTrail, HostSimulationWorld, PlayerShotOrigins, StagedHostTickInput,
    StagedStepOutput, HostStepNumber, HostStepOutput,
};
use crate::host_tick::peer_mail::{HostTimeKeeper, PlayerActionIntake};
use crate::{
    HOST_STEP_MILLIS, ActionRulingNotice, PlayerActionBook, StagedHostSounds, GuaranteedEventRouter,
};

pub(super) fn step_world(
    mut world: ResMut<HostSimulationWorld>,
    timer: Res<HostTimeKeeper>,
    mut staged_in: ResMut<StagedHostTickInput>,
    mut staged_out: ResMut<StagedStepOutput>,
    mut pending_svc: ResMut<StagedHostSounds>,
    shot_log: Res<PlayerShotOrigins>,
    mut exit_level: MessageWriter<LevelQuitRequested>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Step");
    pending_svc.claim_critical_section(&mut world.0);
    let Some(input) = staged_in.0.take() else {
        return;
    };

    world
        .0
        .store_rewind_inputs(shot_log.0.iter().map(|(slot, rec)| (*slot, *rec)));
    let outcome = sim::try_step(
        &mut world.0,
        sim::Tick(timer.tick),
        &input,
        HOST_STEP_MILLIS,
        sim::AdvanceCause::ServerStepFrame,
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
                exit_level.write(LevelQuitRequested);
            }
            return;
        }
    };
    staged_out.0 = Some(HostStepOutput {
        input,
        snapshot,
        weapon_ids_by_script: world.0.weapon_ids_by_script(),
        pending_last_kill: world.0.take_pending_final_kill(),
        script_seats: world.0.script_seats(),
        script_exit_level: world.0.take_script_exit_level(),
    });
}

pub(super) fn publish_tick(
    mut staged_out: ResMut<StagedStepOutput>,
    mut host_step_from_raw: ResMut<HostStepNumber>,
    mut ledger: ResMut<PlayerActionBook>,
    mut guaranteed: ResMut<GuaranteedEventRouter>,
    mut intake_queue: ResMut<PlayerActionIntake>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Publish");
    if let Some(done) = staged_out.0.as_ref() {
        settle_actions(&mut ledger, &mut guaranteed, &mut intake_queue, done);
        stage_guaranteed_occurrences(&mut guaranteed, done);
    }
    host_step_from_raw.0 = staged_out.0.take();
}

/// Does `event` refuse the action with this request id?
fn refuses(event: &MatchOccurrence, serial: IntentSerial) -> bool {
    match event {
        MatchOccurrence::LoadoutRefused { request_id: got, .. }
        | MatchOccurrence::HandoutRefused { request_id: got, .. }
        | MatchOccurrence::ConfigurationChangeRejected { request_id: got, .. } => *got == serial,
        _ => false,
    }
}

/// Record the verdict of every action that went into this tick and retire it from the inbox.
fn settle_actions(
    ledger: &mut PlayerActionBook,
    guaranteed: &mut GuaranteedEventRouter,
    inbox: &mut PlayerActionIntake,
    tick: &HostStepOutput,
) {
    let events = &tick.snapshot.meta.journal;
    for (peer, peer_action) in &tick.input.actions {
        let serial = sim::intent_serial(peer_action);
        let verdict = if events.iter().any(|record| refuses(&record.event, serial)) {
            ActionRulingNotice::Declined
        } else {
            ActionRulingNotice::TakenIntoEffect
        };
        ledger.log_ruling(*peer, *peer_action, verdict);
        guaranteed.enqueue_resolution(*peer, serial, verdict);
        inbox.resolve(*peer, serial);
    }
}

/// Give every client the guaranteed events of this tick that are addressed to it.
fn stage_guaranteed_occurrences(guaranteed: &mut GuaranteedEventRouter, tick: &HostStepOutput) {
    let recipients: Vec<ClientId> = tick.roster();
    let reliable_records = tick
        .snapshot
        .meta
        .journal
        .iter()
        .filter(|record| sim::occurrence_needs_delivery(&record.event));
    for ev_row in reliable_records {
        for &peer in recipients.iter().filter(|&&c| ev_row.audience.is_visible_to(c)) {
            guaranteed.post_event(peer, ev_row.event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{
        ClassId, PlayerIntent, ConfigurationChangeRejectReason, EventRecipients, EventRecord,
        EventSequence, GiveRejectReason, Tick,
    };

    fn tick_with(clients: &[u32], events: Vec<EventRecord>, actions: Vec<(ClientId, PlayerIntent)>) -> HostStepOutput {
        let mut snapshot = sim::Snapshot::unpublished(Tick(1));
        snapshot.meta.clients = clients
            .iter()
            .map(|&id| (ClientId(id), Default::default()))
            .collect();
        snapshot.meta.journal = events;
        HostStepOutput {
            input: sim::StepInputs {
                cmds: Vec::new(),
                actions,
            },
            snapshot,
            weapon_ids_by_script: Vec::<String>::new().into(),
            pending_last_kill: None,
            script_seats: Vec::new(),
            script_exit_level: false,
        }
    }

    fn record(audience: EventRecipients, event: MatchOccurrence) -> EventRecord {
        EventRecord {
            sequence: EventSequence(0),
            tick: Tick(1),
            audience,
            event,
        }
    }

    fn give(serial: u32) -> PlayerIntent {
        PlayerIntent::HandOutGun {
            request_id: serial,
            gun: 2,
        }
    }

    #[test]
    fn rejections_refuse_only_their_own_request() {
        let give_no = MatchOccurrence::HandoutRefused {
            request_id: 5,
            gun: 1,
            reason: GiveRejectReason::NotAlive,
        };
        let config_no = MatchOccurrence::ConfigurationChangeRejected {
            request_id: 6,
            from: 1,
            to: 2,
            reason: ConfigurationChangeRejectReason::Busy,
        };
        let class_no = MatchOccurrence::LoadoutRefused {
            request_id: 7,
            class_id: ClassId(0),
            revision: 0,
            reason: sim::ClassRejectReason::LockedContent,
        };
        assert!(refuses(&give_no, 5) && !refuses(&give_no, 6));
        assert!(refuses(&config_no, 6) && !refuses(&config_no, 5));
        assert!(refuses(&class_no, 7) && !refuses(&class_no, 8));
        assert!(!refuses(&MatchOccurrence::TriggerLet, 5));
        let accepted = MatchOccurrence::HandoutTaken {
            request_id: 5,
            gun: 1,
        };
        assert!(!refuses(&accepted, 5));
    }

    #[test]
    fn a_refused_action_is_remembered_as_refused() {
        let mut ledger = PlayerActionBook::default();
        let mut guaranteed = GuaranteedEventRouter::default();
        let mut inbox = PlayerActionIntake::default();
        let rejection = record(
            EventRecipients::Client(ClientId(1)),
            MatchOccurrence::HandoutRefused {
                request_id: 5,
                gun: 2,
                reason: GiveRejectReason::NotAlive,
            },
        );
        let tick = tick_with(&[1], vec![rejection], vec![(ClientId(1), give(5))]);
        settle_actions(&mut ledger, &mut guaranteed, &mut inbox, &tick);
        assert!(matches!(
            ledger.weigh(ClientId(1), &give(5)),
            crate::ActionAcceptanceResult::Repeat(ActionRulingNotice::Declined)
        ));
        assert_eq!(guaranteed.backlog_of(ClientId(1)).map(|q| q.waiting_count()), Some(1));
    }

    #[test]
    fn an_action_without_a_rejection_is_applied() {
        let mut ledger = PlayerActionBook::default();
        let mut guaranteed = GuaranteedEventRouter::default();
        let mut inbox = PlayerActionIntake::default();
        let tick = tick_with(&[1, 2], Vec::new(), vec![(ClientId(2), give(9))]);
        settle_actions(&mut ledger, &mut guaranteed, &mut inbox, &tick);
        assert!(matches!(
            ledger.weigh(ClientId(2), &give(9)),
            crate::ActionAcceptanceResult::Repeat(ActionRulingNotice::TakenIntoEffect)
        ));
        assert!(guaranteed.backlog_of(ClientId(1)).is_none());
    }

    #[test]
    fn reliable_events_reach_only_their_audience() {
        let mut guaranteed = GuaranteedEventRouter::default();
        let accepted = MatchOccurrence::HandoutTaken {
            request_id: 1,
            gun: 3,
        };
        let tick = tick_with(
            &[1, 2, 3],
            vec![
                record(EventRecipients::Client(ClientId(2)), accepted),
                record(EventRecipients::AllExcept(ClientId(3)), MatchOccurrence::TriggerLet),
            ],
            Vec::new(),
        );
        stage_guaranteed_occurrences(&mut guaranteed, &tick);
        let pending = |id: u32| guaranteed.backlog_of(ClientId(id)).map_or(0, |q| q.waiting_count());
        let attack_reliable = sim::occurrence_needs_delivery(&MatchOccurrence::TriggerLet);
        assert_eq!(pending(1), usize::from(attack_reliable));
        assert_eq!(pending(2), 1 + usize::from(attack_reliable));
        assert_eq!(pending(3), 0);
    }

    #[test]
    fn events_for_clients_outside_the_roster_are_dropped() {
        let mut guaranteed = GuaranteedEventRouter::default();
        let accepted = MatchOccurrence::HandoutTaken {
            request_id: 1,
            gun: 3,
        };
        let tick = tick_with(&[1], vec![record(EventRecipients::Client(ClientId(8)), accepted)], Vec::new());
        stage_guaranteed_occurrences(&mut guaranteed, &tick);
        assert!(guaranteed.backlog_of(ClientId(8)).is_none());
        assert_eq!(guaranteed.backlog_of(ClientId(1)).map_or(0, |q| q.waiting_count()), 0);
    }

    #[test]
    fn publishing_moves_the_pending_result_into_the_server_tick() {
        let mut world = World::new();
        world.init_resource::<HostStepNumber>();
        world.init_resource::<PlayerActionBook>();
        world.init_resource::<GuaranteedEventRouter>();
        world.init_resource::<PlayerActionIntake>();
        world.insert_resource(StagedStepOutput(Some(tick_with(&[1], Vec::new(), Vec::new()))));
        world.insert_resource(HostStageTrail::default());
        bevy::ecs::system::RunSystemOnce::run_system_once(&mut world, publish_tick).unwrap();
        assert!(world.resource::<StagedStepOutput>().0.is_none());
        assert!(world.resource::<HostStepNumber>().0.is_some());
        assert_eq!(world.resource::<HostStageTrail>().0, vec!["Publish"]);
    }

    #[test]
    fn publishing_with_nothing_pending_clears_the_server_tick() {
        let mut world = World::new();
        world.insert_resource(HostStepNumber(Some(tick_with(&[], Vec::new(), Vec::new()))));
        world.init_resource::<StagedStepOutput>();
        world.init_resource::<PlayerActionBook>();
        world.init_resource::<GuaranteedEventRouter>();
        world.init_resource::<PlayerActionIntake>();
        bevy::ecs::system::RunSystemOnce::run_system_once(&mut world, publish_tick).unwrap();
        assert!(world.resource::<HostStepNumber>().0.is_none());
    }
}
