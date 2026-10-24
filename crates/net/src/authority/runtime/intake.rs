//! The first three authority phases: advance the clock, take in what the network delivered, and
//! assemble the input for the tick that is about to be simulated.

use bevy::prelude::*;
use sim::ClientId;

use super::state::{
    AuthorityInputGate, AuthorityPhaseTrace, ClientShotSamples, PendingAcks,
    PendingAuthorityInput, PendingConnectionFaults, PendingStepResult, ServerTick,
};
use crate::authority::inbox::{AuthorityClock, ClientActionInbox, ClientCommandInbox};
use crate::client::presentation::presented::LocalPresentClient;
use crate::transport::relay::UdpAuthorityHub;
use crate::{
    ActionAdmission, ActionVerdict, ClientActionLedger, ClientReliableAck, ReliableEventHub,
};

pub(super) fn advance_clock(
    mut clock: ResMut<AuthorityClock>,
    mut server_tick: ResMut<ServerTick>,
    mut pending_step: ResMut<PendingStepResult>,
    mut trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    if let Some(trace) = trace.as_deref_mut() {
        trace.0.clear();
    }
    AuthorityPhaseTrace::mark(trace, "Advance");
    server_tick.0 = None;
    pending_step.0 = None;
    clock.advance();
}

pub(super) fn ingress(
    hub: Option<ResMut<UdpAuthorityHub>>,
    mut cmd_inbox: ResMut<ClientCommandInbox>,
    mut action_inbox: ResMut<ClientActionInbox>,
    mut samples: ResMut<ClientShotSamples>,
    mut reliable: ResMut<ReliableEventHub>,
    loopback_ack: Option<Res<ClientReliableAck>>,
    local: Option<Res<LocalPresentClient>>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Ingress");

    if let (Some(ack), Some(local)) = (loopback_ack, local) {
        reliable.ack(local.0, ack.0);
    }
    let Some(mut hub) = hub else {
        return;
    };
    let delivered = hub.ingress(
        &mut cmd_inbox,
        &mut action_inbox,
        Some(&mut samples),
        Some(&mut reliable),
    );
    if let Err(e) = delivered {
        diag::warn!(Net, "udp ingress: {e}");
    }
}

/// Peers whose reliable-control or action queues overflowed; they are dropped at the end of the tick.
fn queue_overflow_faults(
    faults: &mut PendingConnectionFaults,
    reliable: &ReliableEventHub,
    actions: &mut ClientActionInbox,
) {
    for client in reliable.overflowed_clients() {
        faults
            .0
            .push((client, "ReliableControlOverflow: outcome unknown".to_owned()));
    }
    for client in actions.take_overflowed() {
        faults
            .0
            .push((client, "ActionQueueOverflow: outcome unknown".to_owned()));
    }
}

/// Run the queued actions past the ledger; only first-time requests reach the simulation.
fn admit_actions(
    actions: &mut ClientActionInbox,
    ledger: &mut ClientActionLedger,
    reliable: &mut ReliableEventHub,
) -> Vec<(ClientId, sim::ClientAction)> {
    let mut fresh = Vec::new();
    for (client, action) in actions.gather() {
        let request_id = sim::action_request_id(&action);
        let answer = match ledger.admit(client, &action) {
            ActionAdmission::Fresh => {
                fresh.push((client, action));
                continue;
            }
            ActionAdmission::Pending => continue,
            ActionAdmission::Repeat(verdict) => verdict,
            ActionAdmission::PayloadMismatch => {
                diag::warn!(
                    Net,
                    "action request_id {request_id} re-used with a different payload by client {} — refused",
                    client.0
                );
                ActionVerdict::PayloadMismatch
            }
            ActionAdmission::Expired => ActionVerdict::Expired,
        };
        reliable.push_outcome(client, request_id, answer);
    }
    fresh
}

pub(super) fn gather_input(
    clock: Res<AuthorityClock>,
    mut inbox: ResMut<ClientCommandInbox>,
    mut samples: ResMut<ClientShotSamples>,
    mut faults: ResMut<PendingConnectionFaults>,
    mut action_inbox: ResMut<ClientActionInbox>,
    mut ledger: ResMut<ClientActionLedger>,
    mut reliable: ResMut<ReliableEventHub>,
    gate: Res<AuthorityInputGate>,
    local: Option<Res<LocalPresentClient>>,
    mut pending: ResMut<PendingAuthorityInput>,
    mut pending_acks: ResMut<PendingAcks>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Gather");
    queue_overflow_faults(&mut faults, &reliable, &mut action_inbox);
    let mut actions = admit_actions(&mut action_inbox, &mut ledger, &mut reliable);

    // The gate is the local player's arming switch, not a licence to hold every peer's input: a
    // queue the authority never drains ages until `backlog_fault` retires the peer for a stall the
    // authority itself caused. So drain on every live tick and drop only what the local client may
    // not contribute yet.
    let gathered = inbox.take_for_tick(clock.time_ms);

    samples.0.clear();
    for (client, command_time, sample) in gathered.samples {
        samples.note(client, command_time, sample);
    }
    for (client, fault) in gathered.backlog_faults {
        diag::warn!(Net, "client {}: {fault}", client.0);
        faults.0.push((client, fault.to_string()));
    }

    let (mut cmds, mut acks) = (gathered.cmds, gathered.acks);
    if !gate.local_cmds_enabled
        && let Some(local) = local.as_ref()
    {
        cmds.retain(|(id, _)| *id != local.0);
        acks.retain(|(id, _)| *id != local.0);
        samples.forget_client(local.0);
    }
    for &(client, _) in &faults.0 {
        cmds.retain(|(id, _)| *id != client);
        acks.retain(|(id, _)| *id != client);
        actions.retain(|(id, _)| *id != client);
        samples.forget_client(client);
        inbox.retire_client(client);
        action_inbox.retire_client(client);
        ledger.retire_client(client);
    }
    pending_acks.0 = acks;
    pending.0 = Some(sim::TickInput { cmds, actions });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn gather_world() -> World {
        let mut world = World::new();
        world.init_resource::<AuthorityClock>();
        world.init_resource::<ClientCommandInbox>();
        world.init_resource::<ClientShotSamples>();
        world.init_resource::<PendingConnectionFaults>();
        world.init_resource::<ClientActionInbox>();
        world.init_resource::<ClientActionLedger>();
        world.init_resource::<ReliableEventHub>();
        world.init_resource::<AuthorityInputGate>();
        world.init_resource::<PendingAuthorityInput>();
        world.init_resource::<PendingAcks>();
        world
    }

    #[test]
    fn the_clock_moves_one_tick_and_the_previous_result_is_dropped() {
        let mut world = World::new();
        world.init_resource::<AuthorityClock>();
        world.init_resource::<ServerTick>();
        world.init_resource::<PendingStepResult>();
        world.insert_resource(AuthorityPhaseTrace(vec!["Stale"]));
        world.run_system_once(advance_clock).unwrap();
        world.run_system_once(advance_clock).unwrap();
        assert_eq!(world.resource::<AuthorityClock>().tick, 2);
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Advance"]);
        assert!(world.resource::<ServerTick>().0.is_none());
    }

    #[test]
    fn gathering_always_stages_an_input_even_when_nobody_sent_anything() {
        let mut world = gather_world();
        world.run_system_once(gather_input).unwrap();
        let staged = world.resource::<PendingAuthorityInput>().0.as_ref().unwrap();
        assert!(staged.cmds.is_empty() && staged.actions.is_empty());
        assert!(world.resource::<PendingAcks>().0.is_empty());
        assert!(world.resource::<PendingConnectionFaults>().0.is_empty());
    }

    #[test]
    fn a_fault_already_queued_removes_that_clients_staged_data() {
        let mut world = gather_world();
        world
            .resource_mut::<PendingConnectionFaults>()
            .0
            .push((ClientId(3), "stalled".to_owned()));
        world.run_system_once(gather_input).unwrap();
        let staged = world.resource::<PendingAuthorityInput>().0.as_ref().unwrap();
        assert!(staged.cmds.iter().all(|(id, _)| *id != ClientId(3)));
    }

    #[test]
    fn gather_trace_is_recorded_when_installed() {
        let mut world = gather_world();
        world.insert_resource(AuthorityPhaseTrace::default());
        world.run_system_once(gather_input).unwrap();
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Gather"]);
    }

    #[test]
    fn ingress_without_a_hub_only_marks_the_phase() {
        let mut world = World::new();
        world.init_resource::<ClientCommandInbox>();
        world.init_resource::<ClientActionInbox>();
        world.init_resource::<ClientShotSamples>();
        world.init_resource::<ReliableEventHub>();
        world.insert_resource(AuthorityPhaseTrace::default());
        world.run_system_once(ingress).unwrap();
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Ingress"]);
    }

    #[test]
    fn overflowed_action_queues_become_connection_faults() {
        let mut faults = PendingConnectionFaults::default();
        let reliable = ReliableEventHub::default();
        let mut actions = ClientActionInbox::default();
        queue_overflow_faults(&mut faults, &reliable, &mut actions);
        assert!(faults.0.is_empty());
    }

    #[test]
    fn an_empty_action_queue_admits_nothing() {
        let mut actions = ClientActionInbox::default();
        let mut ledger = ClientActionLedger::default();
        let mut reliable = ReliableEventHub::default();
        assert!(admit_actions(&mut actions, &mut ledger, &mut reliable).is_empty());
    }
}
