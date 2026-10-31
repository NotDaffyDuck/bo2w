//! The first three authority phases: advance the clock, take in what the network delivered, and
//! assemble the input for the tick that is about to be simulated.

use bevy::prelude::*;
use sim::ClientId;

use super::loop_resources::{
    HostInputValve, HostStageTrail, PlayerShotOrigins, StagedReceipts,
    StagedHostTickInput, StagedLinkFailures, StagedStepOutput, HostStepNumber,
};
use crate::host_tick::peer_mail::{HostTimeKeeper, PlayerActionIntake, PlayerMoveIntake};
use crate::client::presentation::shown_frame::HostViewClient;
use crate::net_links::relay::HostUdpSwitchboard;
use crate::{
    ActionAcceptanceResult, ActionRulingNotice, PlayerActionBook, PlayerGuaranteedReceipt, GuaranteedEventRouter,
};

pub(super) fn advance_clock(
    mut clock: ResMut<HostTimeKeeper>,
    mut host_step_from_raw: ResMut<HostStepNumber>,
    mut staged_out: ResMut<StagedStepOutput>,
    mut trace: Option<ResMut<HostStageTrail>>,
) {
    if let Some(trace) = trace.as_deref_mut() {
        trace.0.clear();
    }
    HostStageTrail::mark(trace, "ClockStep");
    host_step_from_raw.0 = None;
    staged_out.0 = None;
    clock.step_forward();
}

pub(super) fn ingress(
    hub: Option<ResMut<HostUdpSwitchboard>>,
    mut move_queue: ResMut<PlayerMoveIntake>,
    mut action_queue: ResMut<PlayerActionIntake>,
    mut shot_log: ResMut<PlayerShotOrigins>,
    mut event_router: ResMut<GuaranteedEventRouter>,
    loopback_ack: Option<Res<PlayerGuaranteedReceipt>>,
    local: Option<Res<HostViewClient>>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Intake");

    if let (Some(owed_receipt), Some(me)) = (loopback_ack, local) {
        event_router.confirm_receipt(me.0, owed_receipt.0);
    }
    let Some(mut switchboard) = hub else {
        return;
    };
    let delivered = switchboard.take_datagram(
        &mut move_queue,
        &mut action_queue,
        Some(&mut shot_log),
        Some(&mut event_router),
    );
    if let Err(e) = delivered {
        diag::warn!(WireLink, "udp intake failed: {e}");
    }
}

/// Peers whose reliable-control or action queues overflowed; they are dropped at the end of the tick.
fn queue_overflow_faults(
    faults: &mut StagedLinkFailures,
    reliable: &GuaranteedEventRouter,
    actions: &mut PlayerActionIntake,
) {
    for peer in reliable.overflowed_clients() {
        faults
            .0
            .push((peer, "ReliableControlOverflow: outcome unknown".to_owned()));
    }
    for peer in actions.drain_flooded() {
        faults
            .0
            .push((peer, "ActionQueueOverflow: result not known".to_owned()));
    }
}

/// Run the queued actions past the ledger; only first-time requests reach the simulation.
fn admit_actions(
    actions: &mut PlayerActionIntake,
    ledger: &mut PlayerActionBook,
    reliable: &mut GuaranteedEventRouter,
) -> Vec<(ClientId, sim::PlayerIntent)> {
    let mut fresh = Vec::new();
    for (peer, action) in actions.collect_due() {
        let serial = sim::intent_serial(&action);
        let answer = match ledger.weigh(peer, &action) {
            ActionAcceptanceResult::Fresh => {
                fresh.push((peer, action));
                continue;
            }
            ActionAcceptanceResult::Pending => continue,
            ActionAcceptanceResult::Repeat(verdict) => verdict,
            ActionAcceptanceResult::DuplicateIdDifferentBody => {
                diag::warn!(
                    WireLink,
                    "action serial {serial} came again with other contents, from peer {}; refused",
                    peer.0
                );
                ActionRulingNotice::DuplicateIdDifferentBody
            }
            ActionAcceptanceResult::Expired => ActionRulingNotice::TimedOut,
        };
        reliable.enqueue_resolution(peer, serial, answer);
    }
    fresh
}

pub(super) fn gather_input(
    clock: Res<HostTimeKeeper>,
    mut inbox: ResMut<PlayerMoveIntake>,
    mut shot_log: ResMut<PlayerShotOrigins>,
    mut faults: ResMut<StagedLinkFailures>,
    mut action_queue: ResMut<PlayerActionIntake>,
    mut ledger: ResMut<PlayerActionBook>,
    mut reliable: ResMut<GuaranteedEventRouter>,
    gate: Res<HostInputValve>,
    local: Option<Res<HostViewClient>>,
    mut staged_in: ResMut<StagedHostTickInput>,
    mut staged_acks: ResMut<StagedReceipts>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Gather");
    queue_overflow_faults(&mut faults, &reliable, &mut action_queue);
    let mut actions = admit_actions(&mut action_queue, &mut ledger, &mut reliable);

    // The gate is the local player's arming switch, not a licence to hold every peer's input: a
    // queue the authority never drains ages until `backlog_fault` retires the peer for a stall the
    // authority itself caused. So drain on every live tick and drop only what the local player may
    // not contribute yet.
    let gathered = inbox.take_for_step(clock.clock_stamp_ms);

    shot_log.0.clear();
    for (peer, command_time, sample) in gathered.shot_claims {
        shot_log.record_origin(peer, command_time, sample);
    }
    for (peer, failure) in gathered.overflow_incidents {
        diag::warn!(WireLink, "link failure for peer {}: {failure}", peer.0);
        faults.0.push((peer, failure.to_string()));
    }

    let (mut sim_cmds, mut receipt_list) = (gathered.to_simulate, gathered.receipts_batch);
    if !gate.local_cmds_enabled
        && let Some(local) = local.as_ref()
    {
        sim_cmds.retain(|(who, _)| *who != local.0);
        receipt_list.retain(|(who, _)| *who != local.0);
        shot_log.forget_client(local.0);
    }
    for &(peer, _) in &faults.0 {
        sim_cmds.retain(|(who, _)| *who != peer);
        receipt_list.retain(|(who, _)| *who != peer);
        actions.retain(|(who, _)| *who != peer);
        shot_log.forget_client(peer);
        inbox.drop_player_record(peer);
        action_queue.drop_player_record(peer);
        ledger.drop_player_record(peer);
    }
    staged_acks.0 = receipt_list;
    staged_in.0 = Some(sim::StepInputs { cmds: sim_cmds, actions });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn gather_world() -> World {
        let mut world = World::new();
        world.init_resource::<HostTimeKeeper>();
        world.init_resource::<PlayerMoveIntake>();
        world.init_resource::<PlayerShotOrigins>();
        world.init_resource::<StagedLinkFailures>();
        world.init_resource::<PlayerActionIntake>();
        world.init_resource::<PlayerActionBook>();
        world.init_resource::<GuaranteedEventRouter>();
        world.init_resource::<HostInputValve>();
        world.init_resource::<StagedHostTickInput>();
        world.init_resource::<StagedReceipts>();
        world
    }

    #[test]
    fn the_clock_moves_one_tick_and_the_previous_result_is_dropped() {
        let mut world = World::new();
        world.init_resource::<HostTimeKeeper>();
        world.init_resource::<HostStepNumber>();
        world.init_resource::<StagedStepOutput>();
        world.insert_resource(HostStageTrail(vec!["Stale"]));
        world.run_system_once(advance_clock).unwrap();
        world.run_system_once(advance_clock).unwrap();
        assert_eq!(world.resource::<HostTimeKeeper>().tick, 2);
        assert_eq!(world.resource::<HostStageTrail>().0, vec!["ClockStep"]);
        assert!(world.resource::<HostStepNumber>().0.is_none());
    }

    #[test]
    fn gathering_always_stages_an_input_even_when_nobody_sent_anything() {
        let mut world = gather_world();
        world.run_system_once(gather_input).unwrap();
        let staged = world.resource::<StagedHostTickInput>().0.as_ref().unwrap();
        assert!(staged.cmds.is_empty() && staged.actions.is_empty());
        assert!(world.resource::<StagedReceipts>().0.is_empty());
        assert!(world.resource::<StagedLinkFailures>().0.is_empty());
    }

    #[test]
    fn a_fault_already_queued_removes_that_clients_staged_data() {
        let mut world = gather_world();
        world
            .resource_mut::<StagedLinkFailures>()
            .0
            .push((ClientId(3), "stalled".to_owned()));
        world.run_system_once(gather_input).unwrap();
        let staged = world.resource::<StagedHostTickInput>().0.as_ref().unwrap();
        assert!(staged.cmds.iter().all(|(who, _)| *who != ClientId(3)));
    }

    #[test]
    fn gather_trace_is_recorded_when_installed() {
        let mut world = gather_world();
        world.insert_resource(HostStageTrail::default());
        world.run_system_once(gather_input).unwrap();
        assert_eq!(world.resource::<HostStageTrail>().0, vec!["Gather"]);
    }

    #[test]
    fn ingress_without_a_hub_only_marks_the_phase() {
        let mut world = World::new();
        world.init_resource::<PlayerMoveIntake>();
        world.init_resource::<PlayerActionIntake>();
        world.init_resource::<PlayerShotOrigins>();
        world.init_resource::<GuaranteedEventRouter>();
        world.insert_resource(HostStageTrail::default());
        world.run_system_once(ingress).unwrap();
        assert_eq!(world.resource::<HostStageTrail>().0, vec!["Intake"]);
    }

    #[test]
    fn overflowed_action_queues_become_connection_faults() {
        let mut faults = StagedLinkFailures::default();
        let reliable = GuaranteedEventRouter::default();
        let mut actions = PlayerActionIntake::default();
        queue_overflow_faults(&mut faults, &reliable, &mut actions);
        assert!(faults.0.is_empty());
    }

    #[test]
    fn an_empty_action_queue_admits_nothing() {
        let mut actions = PlayerActionIntake::default();
        let mut ledger = PlayerActionBook::default();
        let mut reliable = GuaranteedEventRouter::default();
        assert!(admit_actions(&mut actions, &mut ledger, &mut reliable).is_empty());
    }
}
