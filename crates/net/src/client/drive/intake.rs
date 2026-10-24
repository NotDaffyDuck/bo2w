//! The front half of the client frame: ticks come off the connection, and every tick newer
//! than what was last taken in is adopted - its effects queued, its acknowledgement applied,
//! its snapshot handed to the prediction and its small commands passed on.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::outbox::PendingClientSends;
use super::reliable::ReliableInbound;
use super::stores::{
    ClientPhaseTrace, ClientPredictionState, LastAdoptedSnapshot, PendingDebris, PendingPelletFx,
    PendingPresentedEntityEvents, ReceivedTicks, RemoteProxyState, note_phase,
};
use crate::client::presentation::presented::LocalPresentClient;
use crate::transport::loopback_live::{ListenLoopback, ReceivedTick};
use crate::transport::relay::UdpClientLink;

/// Remembers whether the "connected" line has been logged for the current link.
#[derive(Default)]
pub struct JoinLinkWatch {
    announced: bool,
}

impl JoinLinkWatch {
    /// `true` exactly once: the first time the link is seen with a live connection.
    fn first_connection(&mut self, connected: bool) -> bool {
        let first = connected && !self.announced;
        self.announced |= first;
        first
    }
}

/// The three queues of snapshot content the presentation spawns later.
#[derive(SystemParam)]
pub struct PendingFx<'w> {
    events: ResMut<'w, PendingPresentedEntityEvents>,
    pellets: ResMut<'w, PendingPelletFx>,
    debris: ResMut<'w, PendingDebris>,
}

impl PendingFx<'_> {
    /// Moves the entity events, pellet effects and debris out of a snapshot into the queues.
    /// A snapshot that is not newer than `newest_known` is refused and left untouched.
    fn take_from(
        &mut self,
        newest_known: Option<sim::Tick>,
        local: sim::ClientId,
        snapshot: &mut sim::Snapshot,
    ) -> bool {
        if !self.events.take_from(newest_known, local, snapshot) {
            return false;
        }
        self.pellets.0.append(&mut snapshot.meta.pellet_fx);
        self.debris.0.append(&mut snapshot.meta.debris);
        true
    }
}

fn drain_udp(link: &mut UdpClientLink, into: &mut VecDeque<ReceivedTick>) {
    match link.recv_ticks() {
        Ok(ticks) => into.extend(ticks),
        Err(e) if link.handshake_reject().is_some() => diag::error!(Net, "udp recv: {e}"),
        Err(e) => diag::warn!(Net, "udp recv: {e}"),
    }
}

fn drain_loopback(loopback: &mut ListenLoopback, into: &mut VecDeque<ReceivedTick>) {
    let mut arrived = Vec::new();
    let outcome = loopback.recv_all(&mut arrived);
    into.extend(arrived);
    if let Err(e) = outcome {
        diag::warn!(Net, "client_runtime: recv_all failed: {e}");
    }
}

/// When the host has given this client an id, the prediction and the presentation follow it.
fn follow_assigned_client(
    assigned: Option<sim::ClientId>,
    local: &mut LocalPresentClient,
    prediction: &mut ClientPredictionState,
) {
    if let Some(id) = assigned.filter(|id| *id != local.0) {
        local.0 = id;
        prediction.0.set_local(id);
    }
}

/// Reads whatever has arrived: from the network link when there is one, otherwise from the
/// in-process loopback a listen host uses to talk to itself.
pub fn receive_ticks(
    mut link: Option<ResMut<UdpClientLink>>,
    loopback: Option<ResMut<ListenLoopback>>,
    mut received: ResMut<ReceivedTicks>,
    mut local: ResMut<LocalPresentClient>,
    mut prediction: ResMut<ClientPredictionState>,
    trace: Option<ResMut<ClientPhaseTrace>>,
    mut watch: Local<JoinLinkWatch>,
    mut reliable: ReliableInbound,
) {
    note_phase(trace, "Receive");
    if let Some(link) = link.as_deref_mut() {
        drain_udp(link, &mut received.0);
        for payload in link.take_controls() {
            reliable.apply(link.assigned_client.unwrap_or(local.0), &payload);
        }
        if watch.first_connection(link.connection.is_some()) {
            diag::info!(
                Net,
                "udp client accepted as client {}",
                link.assigned_client.map_or(0, |id| id.0)
            );
        }
        follow_assigned_client(link.assigned_client, &mut local, &mut prediction);
    } else if let Some(mut loopback) = loopback {
        drain_loopback(&mut loopback, &mut received.0);
    }
}

/// Whether no tick still waiting in `queue` is newer than `tick`.
fn nothing_newer_waiting(queue: &VecDeque<ReceivedTick>, tick: sim::Tick) -> bool {
    !queue.iter().any(|waiting| waiting.snapshot.tick.0 > tick.0)
}

/// Takes in every waiting tick that is newer than the last one adopted, oldest first. Only the
/// newest of a batch is reconciled with the prediction; the older ones just retire the
/// commands they acknowledge.
pub fn reconcile_prediction(
    mut received: ResMut<ReceivedTicks>,
    mut prediction: ResMut<ClientPredictionState>,
    local: Res<LocalPresentClient>,
    mut reliable: ReliableInbound,
    mut adopted: ResMut<LastAdoptedSnapshot>,
    mut proxy: ResMut<RemoteProxyState>,
    mut sends: ResMut<PendingClientSends>,
    mut fx: PendingFx,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    note_phase(trace, "Reconcile");
    adopted.applied_this_frame = false;
    while let Some(mut tick) = received.0.pop_front() {
        let newest_known = adopted.next().map(|snap| snap.tick);
        if !fx.take_from(newest_known, local.0, &mut tick.snapshot) {
            continue;
        }
        reliable.apply(local.0, &tick.frame.reliable);

        let ack = tick.ack_for(local.0);
        if let Some(acked) = ack {
            sends.clear_acknowledged(acked);
        }

        adopted.applied_this_frame = true;
        let snapshot = Arc::new(tick.snapshot);
        proxy.0.push_arc(Arc::clone(&snapshot));
        adopted.take_in(snapshot);

        let latest = adopted.next().expect("a snapshot was just taken in");
        if nothing_newer_waiting(&received.0, latest.tick) {
            prediction.0.on_authority(latest, ack);
        } else {
            prediction.0.retire_acks(ack);
        }

        reliable.take_frame_commands(tick.frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::inbox::ClientActionInbox;
    use crate::client::predict::CmdSeq;
    use crate::transport::frame::frame_from_acked_tick;
    use crate::{
        ReliablePayload, Scoreboard, SignonState, SvcCardSlotCmd, SvcGameNotify, SvcHudSplash,
        SvcLocalSound, SvcOpenMenuCmd,
    };

    use super::super::stores::{ClientReliableAck, ReliableControlEvent};

    const ME: sim::ClientId = sim::ClientId(2);

    fn base_app() -> App {
        let mut app = App::new();
        app.init_resource::<ClientActionInbox>()
            .init_resource::<ClientReliableAck>()
            .init_resource::<Scoreboard>()
            .init_resource::<SignonState>()
            .init_resource::<ReceivedTicks>()
            .init_resource::<LastAdoptedSnapshot>()
            .init_resource::<RemoteProxyState>()
            .init_resource::<PendingClientSends>()
            .init_resource::<PendingPresentedEntityEvents>()
            .init_resource::<PendingPelletFx>()
            .init_resource::<PendingDebris>()
            .init_resource::<ClientPredictionState>()
            .insert_resource(LocalPresentClient(ME))
            .insert_resource(ClientPhaseTrace::default())
            .add_message::<ReliableControlEvent>()
            .add_message::<SvcLocalSound>()
            .add_message::<SvcCardSlotCmd>()
            .add_message::<SvcOpenMenuCmd>()
            .add_message::<SvcHudSplash>()
            .add_message::<SvcGameNotify>();
        app
    }

    fn snapshot_at(tick: u32) -> sim::Snapshot {
        let mut snapshot = sim::Snapshot::unpublished(sim::Tick(0));
        snapshot.tick = sim::Tick(tick);
        snapshot
    }

    fn tick_at(tick: u32, acks: Vec<(sim::ClientId, CmdSeq)>) -> ReceivedTick {
        let snapshot = snapshot_at(tick);
        let frame = frame_from_acked_tick(
            &mut crate::SnapshotEncoder::new(),
            &sim::TickInput::default(),
            &snapshot,
            acks,
        );
        ReceivedTick { snapshot, frame }
    }

    fn queue(app: &mut App, ticks: impl IntoIterator<Item = ReceivedTick>) {
        app.world_mut().resource_mut::<ReceivedTicks>().0.extend(ticks);
    }

    fn adopted_ticks(app: &App) -> (Option<u32>, Option<u32>) {
        let held = app.world().resource::<LastAdoptedSnapshot>();
        (
            held.snap.as_ref().map(|snap| snap.tick.0),
            held.next_snap.as_ref().map(|snap| snap.tick.0),
        )
    }

    fn loopback_with(ticks: &[u32]) -> ListenLoopback {
        let mut loopback = ListenLoopback::new();
        for tick in ticks {
            loopback
                .send_tick(
                    &sim::TickInput::default(),
                    &snapshot_at(*tick),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    ReliablePayload {
                        ack_through: 0,
                        rows: Vec::new(),
                        dropped_oldest: 0,
                    },
                )
                .expect("the loopback takes a tick");
        }
        loopback
    }

    #[test]
    fn the_connected_line_is_due_once_per_link() {
        let mut watch = JoinLinkWatch::default();
        assert!(!watch.first_connection(false));
        assert!(watch.first_connection(true));
        assert!(!watch.first_connection(true));
        assert!(!watch.first_connection(false));
    }

    #[test]
    fn the_prediction_and_presentation_follow_the_id_the_host_hands_out() {
        let mut local = LocalPresentClient(sim::ClientId(0));
        let mut prediction = ClientPredictionState::default();
        follow_assigned_client(None, &mut local, &mut prediction);
        assert_eq!(local.0, sim::ClientId(0));

        follow_assigned_client(Some(sim::ClientId(6)), &mut local, &mut prediction);
        assert_eq!(local.0, sim::ClientId(6));
        assert_eq!(prediction.0.local(), sim::ClientId(6));

        follow_assigned_client(Some(sim::ClientId(6)), &mut local, &mut prediction);
        assert_eq!(prediction.0.local(), sim::ClientId(6));
    }

    #[test]
    fn with_no_connection_at_all_receiving_only_notes_the_phase() {
        let mut app = base_app();
        app.add_systems(Update, receive_ticks);
        app.update();
        assert!(app.world().resource::<ReceivedTicks>().0.is_empty());
        assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Receive"]);
    }

    #[test]
    fn ticks_from_the_listen_loopback_are_queued_in_the_order_they_were_sent() {
        let mut app = base_app();
        app.insert_resource(loopback_with(&[7, 8, 9]));
        app.add_systems(Update, receive_ticks);
        app.update();
        let queued: Vec<u32> = app
            .world()
            .resource::<ReceivedTicks>()
            .0
            .iter()
            .map(|tick| tick.snapshot.tick.0)
            .collect();
        assert_eq!(queued, [7, 8, 9]);

        app.update();
        assert_eq!(app.world().resource::<ReceivedTicks>().0.len(), 3);
    }

    #[test]
    fn newer_waiting_ticks_are_recognised() {
        let queue: VecDeque<ReceivedTick> = [tick_at(4, Vec::new()), tick_at(6, Vec::new())].into();
        assert!(!nothing_newer_waiting(&queue, sim::Tick(5)));
        assert!(nothing_newer_waiting(&queue, sim::Tick(6)));
        assert!(nothing_newer_waiting(&queue, sim::Tick(9)));
        assert!(nothing_newer_waiting(&VecDeque::new(), sim::Tick(0)));
    }

    #[test]
    fn the_newest_tick_becomes_the_adopted_snapshot_and_feeds_the_proxy() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        queue(&mut app, [tick_at(3, Vec::new())]);
        app.update();

        assert_eq!(adopted_ticks(&app), (None, Some(3)));
        assert!(app.world().resource::<LastAdoptedSnapshot>().applied_this_frame);
        assert!(app.world().resource::<ReceivedTicks>().0.is_empty());
        let proxied = app.world().resource::<RemoteProxyState>().0.snapshot_at(i32::MAX);
        assert_eq!(proxied.map(|snap| snap.tick.0), Some(3));
        assert_eq!(app.world().resource::<ClientPhaseTrace>().0, ["Reconcile"]);
    }

    #[test]
    fn several_ticks_in_one_frame_leave_the_last_two_in_hand() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        queue(&mut app, [tick_at(3, Vec::new()), tick_at(4, Vec::new()), tick_at(5, Vec::new())]);
        app.update();
        assert_eq!(adopted_ticks(&app), (Some(4), Some(5)));
    }

    #[test]
    fn a_tick_that_is_not_newer_is_dropped_without_adopting_anything() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        queue(&mut app, [tick_at(5, Vec::new())]);
        app.update();

        queue(&mut app, [tick_at(5, Vec::new()), tick_at(4, Vec::new())]);
        app.update();
        assert_eq!(adopted_ticks(&app), (None, Some(5)));
        assert!(!app.world().resource::<LastAdoptedSnapshot>().applied_this_frame);
        assert!(app.world().resource::<ReceivedTicks>().0.is_empty());
    }

    #[test]
    fn a_tick_that_is_not_newer_keeps_its_commands_to_itself() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        queue(&mut app, [tick_at(5, Vec::new())]);
        app.update();

        let mut stale = tick_at(4, Vec::new());
        stale.frame.svc_scores = Some("late scores".to_owned());
        queue(&mut app, [stale]);
        app.update();
        assert_eq!(app.world().resource::<Scoreboard>().cmd, None);
    }

    #[test]
    fn an_acknowledgement_clears_the_commands_it_covers() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        {
            let mut sends = app.world_mut().resource_mut::<PendingClientSends>();
            for n in 1..=5 {
                sends.push(CmdSeq(n), Default::default(), Default::default());
            }
        }
        queue(&mut app, [tick_at(2, vec![(ME, CmdSeq(3))])]);
        app.update();
        let sends = app.world().resource::<PendingClientSends>();
        assert_eq!(sends.oldest_seq(), Some(CmdSeq(4)));
        assert_eq!(sends.len(), 2);
    }

    #[test]
    fn an_acknowledgement_meant_for_someone_else_changes_nothing() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        app.world_mut()
            .resource_mut::<PendingClientSends>()
            .push(CmdSeq(1), Default::default(), Default::default());
        queue(&mut app, [tick_at(2, vec![(sim::ClientId(9), CmdSeq(1))])]);
        app.update();
        assert_eq!(app.world().resource::<PendingClientSends>().len(), 1);
    }

    #[test]
    fn effects_leave_the_snapshot_for_their_queues() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        let mut tick = tick_at(2, Vec::new());
        tick.snapshot.meta.pellet_fx = vec![sim::PelletFxRecord { pellet: 3, ..Default::default() }];
        tick.snapshot.meta.debris = vec![sim::DebrisRecord {
            model: "chunk".to_owned(),
            origin: [1.0, 2.0, 3.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            mass: 1.0,
            bounce: 0.2,
            ..Default::default()
        }];
        queue(&mut app, [tick]);
        app.update();

        assert_eq!(app.world().resource::<PendingPelletFx>().0.len(), 1);
        assert_eq!(app.world().resource::<PendingDebris>().0.len(), 1);
        let held = app.world().resource::<LastAdoptedSnapshot>();
        let snap = held.next().expect("adopted");
        assert!(snap.meta.pellet_fx.is_empty() && snap.meta.debris.is_empty());
    }

    #[test]
    fn the_small_commands_of_an_adopted_tick_are_passed_on() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        let mut tick = tick_at(2, Vec::new());
        tick.frame.svc_scores = Some("scores".to_owned());
        tick.frame.svc_sounds = vec![crate::SvcSound { stop: false, index: 9 }];
        queue(&mut app, [tick]);
        app.update();

        assert_eq!(app.world().resource::<Scoreboard>().cmd.as_deref(), Some("scores"));
        let sounds: Vec<u8> = app
            .world_mut()
            .resource_mut::<Messages<SvcLocalSound>>()
            .drain()
            .map(|sound| sound.index)
            .collect();
        assert_eq!(sounds, [9]);
    }

    #[test]
    fn the_reliable_rows_of_an_adopted_tick_are_acted_on() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        let mut tick = tick_at(2, Vec::new());
        tick.frame.reliable = ReliablePayload {
            ack_through: 0,
            rows: vec![(1, crate::ReliableRow::Scores("from a row".to_owned()))],
            dropped_oldest: 0,
        };
        queue(&mut app, [tick]);
        app.update();
        assert_eq!(app.world().resource::<ClientReliableAck>().0, 1);
        assert_eq!(app.world().resource::<Scoreboard>().cmd.as_deref(), Some("from a row"));
    }

    #[test]
    fn entity_events_go_to_the_live_list_unless_the_player_is_in_a_replay() {
        let mut app = base_app();
        app.add_systems(Update, reconcile_prediction);
        let record = sim::EntityEventRecord {
            sequence: sim::EventSequence(1),
            tick: sim::Tick(2),
            audience: sim::EventAudience::All,
            event: entity_t6::EntityEventKind(4),
            payload: sim::EntityEventPayload::default(),
        };
        let mut live = tick_at(2, Vec::new());
        live.snapshot.meta.entity_events = vec![record.clone()];
        let mut replay = tick_at(3, Vec::new());
        replay.snapshot.meta.entity_events = vec![record];
        replay.snapshot.meta.clients.push((
            ME,
            sim::ClientSnapshotMeta {
                killcam_hud: Some(sim::KillcamHud { final_kill: true }),
                ..Default::default()
            },
        ));
        queue(&mut app, [live, replay]);
        app.update();

        let pending = app.world().resource::<PendingPresentedEntityEvents>();
        assert_eq!((pending.live.len(), pending.archived.len()), (1, 1));
        assert!(pending.in_killcam);
    }
}
