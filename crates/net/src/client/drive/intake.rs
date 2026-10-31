//! The front half of the client frame: ticks come off the connection, and every tick newer
//! than what was last taken in is adopted - its effects queued, its acknowledgement applied,
//! its snapshot handed to the prediction and its small commands passed on.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::outbox::QueuedOutgoingPlayerMessages;
use super::reliable::GuaranteedIntakeParams;
use super::stores::{
    ReceiverStageLog, ClientGuessState, NewestTakenSnapshot, PendingDebris, QueuedPelletEffect,
    StagedShownEntityEvents, ArrivedStepQueue, OtherPlayerSmootherHolder, note_phase,
};
use crate::client::presentation::shown_frame::HostViewClient;
use crate::net_links::listen_link::{LocalHostTickPipe, ArrivedHostStep};
use crate::net_links::relay::PlayerUdpSession;

/// Flips `announced` and answers `true` the first time a live connection is seen on a link.
fn first_sight_of_connection(announced: &mut bool, connected: bool) -> bool {
    if *announced || !connected {
        return false;
    }
    *announced = true;
    true
}

/// Where ticks can arrive from: the network link, or the in-process loopback of a listen host.
#[derive(SystemParam)]
pub struct TickSources<'w> {
    udp: Option<ResMut<'w, PlayerUdpSession>>,
    loopback: Option<ResMut<'w, LocalHostTickPipe>>,
}

/// The queues of snapshot content the presentation spawns later.
#[derive(SystemParam)]
pub struct EffectQueues<'w> {
    entity_events: ResMut<'w, StagedShownEntityEvents>,
    pellet_fx: ResMut<'w, QueuedPelletEffect>,
    debris: ResMut<'w, PendingDebris>,
}

impl EffectQueues<'_> {
    /// Moves the entity events, pellet effects and debris out of a snapshot into the queues.
    /// A snapshot that is not newer than `newest_known` is refused (`false`) and left untouched.
    fn claim(&mut self, newest_known: Option<sim::Tick>, me: sim::ClientId, snapshot: &mut sim::Snapshot) -> bool {
        let accepted = self.entity_events.take_from(newest_known, me, snapshot);
        if accepted {
            self.pellet_fx.0.extend(std::mem::take(&mut snapshot.meta.pellet_fx));
            self.debris.0.extend(std::mem::take(&mut snapshot.meta.debris));
        }
        accepted
    }
}

/// What adopting a snapshot touches: the held pair, the remote-proxy history, the prediction and
/// the commands still waiting for an acknowledgement.
#[derive(SystemParam)]
pub struct Adoption<'w> {
    held: ResMut<'w, NewestTakenSnapshot>,
    proxy: ResMut<'w, OtherPlayerSmootherHolder>,
    forecast: ResMut<'w, ClientGuessState>,
    unacked: ResMut<'w, QueuedOutgoingPlayerMessages>,
}

fn pull_from_udp(link: &mut PlayerUdpSession, queue: &mut VecDeque<ArrivedHostStep>) {
    let failure = match link.arrived_steps() {
        Ok(arrived) => {
            queue.extend(arrived);
            return;
        }
        Err(failure) => failure,
    };
    // A refused handshake is an error worth shouting about; other read failures are transient.
    if link.greeting_refusal().is_none() {
        diag::warn!(WireLink, "udp recv: {failure}");
    } else {
        diag::error!(WireLink, "udp recv: {failure}");
    }
}

fn pull_from_loopback(link: &mut LocalHostTickPipe, queue: &mut VecDeque<ArrivedHostStep>) {
    let mut batch = Vec::new();
    let result = link.drain_into(&mut batch);
    queue.extend(batch);
    if let Err(failure) = result {
        diag::warn!(WireLink, "client_runtime: recv_all failed: {failure}");
    }
}

/// When the host has given this client an id, the prediction and the presentation follow it.
fn follow_assigned_client(
    assigned: Option<sim::ClientId>,
    local: &mut HostViewClient,
    forecast: &mut ClientGuessState,
) {
    let Some(handed_out) = assigned else { return };
    if handed_out != local.0 {
        forecast.0.assign_scoped(handed_out);
        local.0 = handed_out;
    }
}

/// Reads whatever has arrived: from the network link when there is one, otherwise from the
/// in-process loopback a listen host uses to talk to itself.
pub fn ingest_host_steps(
    mut sources: TickSources,
    mut inbox: ResMut<ArrivedStepQueue>,
    mut me: ResMut<HostViewClient>,
    mut forecast: ResMut<ClientGuessState>,
    trace: Option<ResMut<ReceiverStageLog>>,
    mut announced: Local<bool>,
    mut reliable: GuaranteedIntakeParams,
) {
    note_phase(trace, "Receive");
    let Some(udp) = sources.udp.as_deref_mut() else {
        if let Some(link) = sources.loopback.as_deref_mut() {
            pull_from_loopback(link, &mut inbox.0);
        }
        return;
    };
    pull_from_udp(udp, &mut inbox.0);
    let owner = udp.granted_player_slot.unwrap_or(me.0);
    udp.take_controls().iter().for_each(|control| reliable.apply(owner, control));
    if first_sight_of_connection(&mut announced, udp.connection.is_some()) {
        let id = udp.granted_player_slot.map(|c| c.0).unwrap_or(0);
        diag::info!(WireLink, "udp client accepted as client {id}");
    }
    follow_assigned_client(udp.granted_player_slot, &mut me, &mut forecast);
}

/// Whether no tick still waiting in `queue` is newer than `tick`.
fn nothing_newer_waiting(queue: &VecDeque<ArrivedHostStep>, tick: sim::Tick) -> bool {
    queue.iter().all(|waiting| waiting.snapshot.tick.0 <= tick.0)
}

/// Takes in every waiting tick that is newer than the last one adopted, oldest first. Only the
/// newest of a batch is reconciled with the prediction; the older ones just retire the
/// commands they acknowledge.
pub fn correct_forecast(
    mut inbox: ResMut<ArrivedStepQueue>,
    me: Res<HostViewClient>,
    mut reliable: GuaranteedIntakeParams,
    mut adoption: Adoption,
    mut effects: EffectQueues,
    trace: Option<ResMut<ReceiverStageLog>>,
) {
    note_phase(trace, "Reconcile");
    let me = me.0;
    adoption.held.taken_in_current_step = false;
    while let Some(arrival) = inbox.0.pop_front() {
        let ArrivedHostStep { mut snapshot, frame } = arrival;
        let newest_known = adoption.held.next().map(|held| held.tick);
        if !effects.claim(newest_known, me, &mut snapshot) {
            continue;
        }
        reliable.apply(me, &frame.reliable);
        let receipt_confirmed = frame.receipt_for(me);
        receipt_confirmed.into_iter().for_each(|seq| adoption.unacked.retire_through(seq));

        adoption.held.taken_in_current_step = true;
        let shared = Arc::new(snapshot);
        adoption.proxy.0.enqueue_shared_item(shared.clone());
        adoption.held.take_in(shared);

        let newest = adoption.held.next().expect("a snapshot was just taken in");
        // Older ticks of the batch only retire acknowledged commands; the newest is reconciled.
        if !nothing_newer_waiting(&inbox.0, newest.tick) {
            adoption.forecast.0.drop_confirmed(receipt_confirmed);
        } else {
            adoption.forecast.0.on_authority(newest, receipt_confirmed);
        }
        reliable.take_frame_commands(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_tick::peer_mail::PlayerActionIntake;
    use crate::client::run_ahead::InputCommandNumber;
    use crate::net_links::tick_frames::frame_for_confirmed_tick;
    use crate::{
        GuaranteedEnvelope, Scoreboard, JoinProgressRecord, CardSlotCommand, HostGameAnnouncement, HostScreenFlash,
        HostLocalSoundMsg, MenuOpenCommand,
    };

    use super::super::stores::{PlayerGuaranteedReceipt, GuaranteedSimEvent};

    const ME: sim::ClientId = sim::ClientId(2);

    macro_rules! defaults {
        ($app:ident: $($res:ty),+ $(,)?) => {
            $( $app.init_resource::<$res>(); )+
        };
    }

    macro_rules! messages {
        ($app:ident: $($msg:ty),+ $(,)?) => {
            $( $app.add_message::<$msg>(); )+
        };
    }

    /// An app with every store the two intake systems touch.
    fn stores_app() -> App {
        let mut app = App::new();
        defaults!(app:
            PlayerActionIntake, PlayerGuaranteedReceipt, Scoreboard, JoinProgressRecord, ArrivedStepQueue,
            NewestTakenSnapshot, OtherPlayerSmootherHolder, QueuedOutgoingPlayerMessages, StagedShownEntityEvents,
            QueuedPelletEffect, PendingDebris, ClientGuessState, ReceiverStageLog,
        );
        messages!(app:
            GuaranteedSimEvent, HostLocalSoundMsg, CardSlotCommand, MenuOpenCommand, HostScreenFlash,
            HostGameAnnouncement,
        );
        app.insert_resource(HostViewClient(ME));
        app
    }

    fn app_running(system: Which) -> App {
        let mut app = stores_app();
        match system {
            Which::Receive => app.add_systems(Update, ingest_host_steps),
            Which::Reconcile => app.add_systems(Update, correct_forecast),
        };
        app
    }

    enum Which {
        Receive,
        Reconcile,
    }

    fn snapshot_at(tick: u32) -> sim::Snapshot {
        let mut snapshot = sim::Snapshot::unpublished(sim::Tick(0));
        snapshot.tick = sim::Tick(tick);
        snapshot
    }

    fn arrival(tick: u32, receipts_batch: Vec<(sim::ClientId, InputCommandNumber)>) -> ArrivedHostStep {
        let snap = snapshot_at(tick);
        let mut diff_writer = crate::FrameDiffWriter::new();
        let frame = frame_for_confirmed_tick(&mut diff_writer, &sim::StepInputs::default(), &snap, receipts_batch);
        ArrivedHostStep { snapshot: snap, frame }
    }

    fn bare(tick: u32) -> ArrivedHostStep {
        arrival(tick, Vec::new())
    }

    fn deliver(app: &mut App, ticks: impl IntoIterator<Item = ArrivedHostStep>) {
        app.world_mut().resource_mut::<ArrivedStepQueue>().0.extend(ticks);
    }

    fn held_ids(app: &App) -> (Option<u32>, Option<u32>) {
        let held = app.world().resource::<NewestTakenSnapshot>();
        let tick_of = |slot: &Option<_>| slot.as_ref().map(|snap: &Arc<sim::Snapshot>| snap.tick.0);
        (tick_of(&held.frame_state), tick_of(&held.upcoming_world_state))
    }

    fn loopback_with(ticks: &[u32]) -> LocalHostTickPipe {
        let mut loopback = LocalHostTickPipe::new();
        let input = sim::StepInputs::default();
        for &tick in ticks {
            let sent = loopback.transmit_step(
                &input,
                &snapshot_at(tick),
                Vec::new(),
                Vec::new(),
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                GuaranteedEnvelope::has_no_debt(),
            );
            sent.expect("the loopback takes a tick");
        }
        loopback
    }

    #[test]
    fn the_connected_line_is_due_once_per_link() {
        let mut announced = false;
        let answers: Vec<bool> =
            [false, true, true, false].into_iter().map(|up| first_sight_of_connection(&mut announced, up)).collect();
        assert_eq!(answers, [false, true, false, false]);
    }

    #[test]
    fn the_prediction_and_presentation_follow_the_id_the_host_hands_out() {
        let mut local = HostViewClient(sim::ClientId(0));
        let mut forecast = ClientGuessState::default();
        follow_assigned_client(None, &mut local, &mut forecast);
        assert_eq!(local.0, sim::ClientId(0));
        for _ in 0..2 {
            follow_assigned_client(Some(sim::ClientId(6)), &mut local, &mut forecast);
            assert_eq!((local.0, forecast.0.local()), (sim::ClientId(6), sim::ClientId(6)));
        }
    }

    #[test]
    fn with_no_connection_at_all_receiving_only_notes_the_phase() {
        let mut app = app_running(Which::Receive);
        app.update();
        assert_eq!(app.world().resource::<ArrivedStepQueue>().0.len(), 0);
        assert_eq!(app.world().resource::<ReceiverStageLog>().0, ["Receive"]);
    }

    #[test]
    fn ticks_from_the_listen_loopback_are_queued_in_the_order_they_were_sent() {
        let mut app = app_running(Which::Receive);
        app.insert_resource(loopback_with(&[7, 8, 9]));
        let queued_ticks = |app: &App| -> Vec<u32> {
            app.world().resource::<ArrivedStepQueue>().0.iter().map(|t| t.snapshot.tick.0).collect()
        };
        app.update();
        assert_eq!(queued_ticks(&app), [7, 8, 9]);
        app.update();
        assert_eq!(queued_ticks(&app), [7, 8, 9], "nothing more arrives");
    }

    #[test]
    fn newer_waiting_ticks_are_recognised() {
        let queue = VecDeque::from([bare(4), bare(6)]);
        let answers: Vec<bool> = [5, 6, 9].into_iter().map(|t| nothing_newer_waiting(&queue, sim::Tick(t))).collect();
        assert_eq!(answers, [false, true, true]);
        assert!(nothing_newer_waiting(&VecDeque::new(), sim::Tick(0)));
    }

    #[test]
    fn the_newest_tick_becomes_the_adopted_snapshot_and_feeds_the_proxy() {
        let mut app = app_running(Which::Reconcile);
        deliver(&mut app, [bare(3)]);
        app.update();

        let world = app.world();
        assert_eq!(held_ids(&app), (None, Some(3)));
        assert!(world.resource::<NewestTakenSnapshot>().taken_in_current_step);
        assert_eq!(world.resource::<ArrivedStepQueue>().0.len(), 0);
        let proxied = world.resource::<OtherPlayerSmootherHolder>().0.blended_frame_at(i32::MAX).map(|s| s.tick.0);
        assert_eq!(proxied, Some(3));
        assert_eq!(world.resource::<ReceiverStageLog>().0, ["Reconcile"]);
    }

    #[test]
    fn several_ticks_in_one_frame_leave_the_last_two_in_hand() {
        let mut app = app_running(Which::Reconcile);
        deliver(&mut app, (3..=5).map(bare));
        app.update();
        assert_eq!(held_ids(&app), (Some(4), Some(5)));
    }

    #[test]
    fn a_tick_that_is_not_newer_is_dropped_without_adopting_anything() {
        let mut app = app_running(Which::Reconcile);
        deliver(&mut app, [bare(5)]);
        app.update();
        deliver(&mut app, [bare(5), bare(4)]);
        app.update();

        assert_eq!(held_ids(&app), (None, Some(5)));
        assert!(!app.world().resource::<NewestTakenSnapshot>().taken_in_current_step);
        assert_eq!(app.world().resource::<ArrivedStepQueue>().0.len(), 0);
    }

    #[test]
    fn a_tick_that_is_not_newer_keeps_its_commands_to_itself() {
        let mut app = app_running(Which::Reconcile);
        deliver(&mut app, [bare(5)]);
        app.update();
        let mut late = bare(4);
        late.frame.board_text = Some(String::from("late scores"));
        deliver(&mut app, [late]);
        app.update();
        assert!(app.world().resource::<Scoreboard>().command_text.is_none());
    }

    #[test]
    fn an_acknowledgement_clears_the_commands_it_covers() {
        let mut app = app_running(Which::Reconcile);
        let mut sends = app.world_mut().resource_mut::<QueuedOutgoingPlayerMessages>();
        (1..=5).for_each(|n| sends.enqueue(InputCommandNumber(n), Default::default(), Default::default()));
        deliver(&mut app, [arrival(2, vec![(ME, InputCommandNumber(3))])]);
        app.update();
        let sends = app.world().resource::<QueuedOutgoingPlayerMessages>();
        assert_eq!((sends.oldest_seq(), sends.len()), (Some(InputCommandNumber(4)), 2));
    }

    #[test]
    fn an_acknowledgement_meant_for_someone_else_changes_nothing() {
        let mut app = app_running(Which::Reconcile);
        let mut sends = app.world_mut().resource_mut::<QueuedOutgoingPlayerMessages>();
        sends.enqueue(InputCommandNumber(1), Default::default(), Default::default());
        deliver(&mut app, [arrival(2, vec![(sim::ClientId(9), InputCommandNumber(1))])]);
        app.update();
        assert_eq!(app.world().resource::<QueuedOutgoingPlayerMessages>().len(), 1);
    }

    #[test]
    fn effects_leave_the_snapshot_for_their_queues() {
        let mut app = app_running(Which::Reconcile);
        let mut carrying = bare(2);
        let meta = &mut carrying.snapshot.meta;
        meta.pellet_fx.push(sim::PelletFxRecord { pellet: 3, ..Default::default() });
        let mut chunk = sim::DebrisRecord { mesh: "chunk".to_owned(), ..Default::default() };
        (chunk.origin, chunk.rotation) = ([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0]);
        (chunk.mass, chunk.bounce) = (1.0, 0.2);
        meta.debris.push(chunk);
        deliver(&mut app, [carrying]);
        app.update();

        let world = app.world();
        assert_eq!(world.resource::<QueuedPelletEffect>().0.len(), 1);
        assert_eq!(world.resource::<PendingDebris>().0.len(), 1);
        let adopted = world.resource::<NewestTakenSnapshot>().next().expect("adopted");
        assert_eq!((adopted.meta.pellet_fx.len(), adopted.meta.debris.len()), (0, 0));
    }

    #[test]
    fn the_small_commands_of_an_adopted_tick_are_passed_on() {
        let mut app = app_running(Which::Reconcile);
        let mut carrying = bare(2);
        carrying.frame.board_text = Some(String::from("scores"));
        carrying.frame.host_sound_triggers.push(crate::HostSoundCue { halt: false, slot: 9 });
        deliver(&mut app, [carrying]);
        app.update();

        assert_eq!(app.world().resource::<Scoreboard>().command_text.as_deref(), Some("scores"));
        let mut played = app.world_mut().resource_mut::<Messages<HostLocalSoundMsg>>();
        let indices: Vec<u8> = played.drain().map(|sound| sound.index).collect();
        assert_eq!(indices, [9]);
    }

    #[test]
    fn the_reliable_rows_of_an_adopted_tick_are_acted_on() {
        let mut app = app_running(Which::Reconcile);
        let mut carrying = bare(2);
        carrying.frame.reliable.entries.push((1, crate::GuaranteedEntry::Standings(String::from("from a row"))));
        deliver(&mut app, [carrying]);
        app.update();
        assert_eq!(app.world().resource::<PlayerGuaranteedReceipt>().0, 1);
        assert_eq!(app.world().resource::<Scoreboard>().command_text.as_deref(), Some("from a row"));
    }

    #[test]
    fn entity_events_go_to_the_live_list_unless_the_player_is_in_a_replay() {
        let mut app = app_running(Which::Reconcile);
        let record = sim::EntityEventRecord {
            sequence: sim::EventSequence(1),
            tick: sim::Tick(2),
            audience: sim::EventRecipients::All,
            event: entity_t6::EventCode(4),
            payload: sim::EntityEventPayload::default(),
        };
        let mut live = bare(2);
        live.snapshot.meta.entity_events.push(record.clone());
        let mut replay = bare(3);
        replay.snapshot.meta.entity_events.push(record);
        let mut watching = sim::ClientSnapshotMeta::default();
        watching.killcam_hud = Some(sim::KillcamHud { final_kill: true });
        replay.snapshot.meta.clients.push((ME, watching));
        deliver(&mut app, [live, replay]);
        app.update();

        let pending = app.world().resource::<StagedShownEntityEvents>();
        assert_eq!((pending.live.len(), pending.archived.len(), pending.replaying_final_death), (1, 1, true));
    }
}
