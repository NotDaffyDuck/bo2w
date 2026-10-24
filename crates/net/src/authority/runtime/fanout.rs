//! The fan-out phase: hand the finished tick to every player. Script output is folded into the
//! queues first, the local player's view is built (killcam seat included), reliable rows are
//! queued per client, and the loopback and the UDP hub each get their frame.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::ExitLevelCalled;
use sim::Snapshot;

use super::census::ListenFanoutCensus;
use super::dumps::{
    DumpConfigurationChangeLog, DumpDeathLog, DumpGiveLog, lifecycle_label, player_origin_flags,
};
use super::state::{
    AuthorityPhaseTrace, AuthorityWorld, NetDiagnostics, PendingAcks, ServerTick, ServerTickData,
};
use crate::authority::inbox::AuthorityClock;
use crate::client::presentation::presented::LocalPresentClient;
use crate::policy::seat::ActiveKillcams;
use crate::transport::archive::FrameArchive;
use crate::transport::loopback_live::ListenLoopback;
use crate::transport::relay::UdpAuthorityHub;
use crate::{
    ClientActionInput, PendingGameNotify, PendingPlayerCard, PendingScoreboard, PendingSvcSounds,
    ReliableEventHub, ReliablePayload, ReliableRow, format_scoreboard_from_snapshot,
};

#[derive(SystemParam)]
pub(super) struct FanoutQueues<'w> {
    acks: ResMut<'w, PendingAcks>,
    svc: ResMut<'w, PendingSvcSounds>,
    playercard: ResMut<'w, PendingPlayerCard>,
    gamenotify: ResMut<'w, PendingGameNotify>,
    scores: ResMut<'w, PendingScoreboard>,
    world: ResMut<'w, AuthorityWorld>,
    local: Option<Res<'w, LocalPresentClient>>,
    keyboard: Option<Res<'w, ClientActionInput>>,
    census: ResMut<'w, ListenFanoutCensus>,
    deaths: ResMut<'w, DumpDeathLog>,
    gives: ResMut<'w, DumpGiveLog>,
    configuration_changes: ResMut<'w, DumpConfigurationChangeLog>,
    exit_level: MessageWriter<'w, ExitLevelCalled>,
}

pub(super) fn fanout(
    loopback: Option<ResMut<ListenLoopback>>,
    hub: Option<ResMut<UdpAuthorityHub>>,
    server_tick: Res<ServerTick>,
    mut reliable: ResMut<ReliableEventHub>,
    mut queues: FanoutQueues,
    mut net_diag: ResMut<NetDiagnostics>,
    archive: Res<FrameArchive>,
    mut seats: ResMut<ActiveKillcams>,
    clock: Res<AuthorityClock>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Fanout");
    let Some(tick) = server_tick.0.as_ref() else {
        return;
    };
    let now_ms = clock.time_ms;

    absorb_script_output(&mut queues, tick, &archive, &mut seats, now_ms);
    let (listen_snapshot, seat_applied) = listen_view(&mut queues, tick, &archive, &seats, now_ms);
    publish_census(&mut queues, tick, &listen_snapshot, seat_applied, now_ms);

    let acks = std::mem::take(&mut queues.acks.0);
    let scores_down = queues
        .keyboard
        .as_ref()
        .is_some_and(|input| input.client.kb.scores.active);
    let svc_scores = queues
        .scores
        .take_if_due(scores_down, now_ms)
        .then(|| format_scoreboard_from_snapshot(&listen_snapshot));
    let scores_for_all = queues.scores.broadcast_due(now_ms);
    queue_rows(&mut reliable, &mut queues, tick, scores_for_all);

    if let Some(mut loopback) = loopback
        && let Err(line) =
            send_loopback(&mut loopback, &reliable, &mut queues, tick, &listen_snapshot, &acks, svc_scores)
    {
        net_diag.report(line);
    }
    if let Some(mut hub) = hub {
        let sent = hub.fanout_with_seats(
            &tick.input,
            &tick.snapshot,
            &acks,
            |peer, live| {
                crate::policy::seat::snapshot_for_viewer(&archive, &seats, live, peer, now_ms)
            },
            Some(&mut queues.svc),
            Some(&mut queues.playercard),
            Some(&mut queues.gamenotify),
            Some(&reliable),
            false,
        );
        if let Err(e) = sent {
            net_diag.report(format!("udp fanout: {e}"));
        }
    }
    report_stranded(&mut queues);
    queues.gamenotify.clear_after_fanout();
}

pub fn authority_bookkeeping(
    server_tick: Res<ServerTick>,
    mut archive: ResMut<FrameArchive>,
    seats: Res<ActiveKillcams>,
    trace: Option<ResMut<AuthorityPhaseTrace>>,
) {
    AuthorityPhaseTrace::mark(trace, "Bookkeeping");
    if let Some(tick) = server_tick.0.as_ref() {
        archive.push_snapshot(&tick.snapshot, &seats.viewers());
    }
}

/// Take what the match scripts produced this tick: queued sounds, cards and notifications, killcam
/// seats, an exit-level request, and the journal rows the console dumps show.
fn absorb_script_output(
    queues: &mut FanoutQueues,
    tick: &ServerTickData,
    archive: &FrameArchive,
    seats: &mut ActiveKillcams,
    now_ms: i32,
) {
    queues.playercard.adopt_from_world(&mut queues.world.0);
    queues.gamenotify.adopt_from_world(&mut queues.world.0);
    queues.svc.adopt_from_world(&mut queues.world.0);
    for (viewer, archive_ms) in
        crate::policy::killcam::play_script_seats(seats, archive, now_ms, &tick.script_seats)
    {
        queues.world.0.store_script_seat_archive(viewer, archive_ms);
    }
    if tick.script_exit_level {
        queues.exit_level.write(ExitLevelCalled);
        diag::info!(Sim, "exitLevel: called by the match scripts");
    }
    let journal = &tick.snapshot.meta.journal;
    queues.deaths.push_journal(journal);
    queues.gives.push_journal(journal);
    queues.configuration_changes.push_journal(journal);
}

/// The snapshot the local player is sent, and 1 when a killcam seat reshaped it (otherwise 0).
/// The seat census fields are filled only while a seat applies.
fn listen_view(
    queues: &mut FanoutQueues,
    tick: &ServerTickData,
    archive: &FrameArchive,
    seats: &ActiveKillcams,
    now_ms: i32,
) -> (Snapshot, i32) {
    queues.census.clear_seat();
    let Some(local) = queues.local.as_ref().map(|local| local.0) else {
        return (tick.snapshot.clone(), 0);
    };
    let (view, sample) = crate::policy::seat::snapshot_and_sample_for_viewer(
        archive,
        seats,
        &tick.snapshot,
        local,
        now_ms,
    );
    let Some(sample) = sample else {
        return (view, 0);
    };
    let session = seats.get(local);
    let focus = session.map(|s| s.focus_client);
    let focus_origin = focus.and_then(|f| queues.world.0.player(f).map(|ps| ps.origin));
    let focus_lifecycle = focus.and_then(|f| {
        queues
            .world
            .0
            .client_meta(f)
            .map(|meta| lifecycle_label(meta.lifecycle))
    });
    let census = &mut queues.census;
    census.seat_archivetime_ms = session.map(|s| s.archivetime_ms);
    census.seat_focus_client = session.map(|s| s.focus_client.0 as i32);
    census.seat_lookup_tick = sample.lookup.tick.map(|t| t.0 as i32);
    census.seat_attained_ms = Some(sample.lookup.attained_ms);
    census.seat_rebase_ms = Some(sample.rebase_ms);
    census.seat_world_archived = Some(i32::from(sample.lookup.tick.is_some()));
    census.seat_focus_live_origin = focus_origin;
    census.seat_focus_lifecycle = focus_lifecycle;
    (view, 1)
}

/// Fill the debug census with the local player's row as the world, the snapshot and the sent
/// frame each see it, and feed the same numbers to the feel trace.
fn publish_census(
    queues: &mut FanoutQueues,
    tick: &ServerTickData,
    sent: &Snapshot,
    seat_applied: i32,
    now_ms: i32,
) {
    let local = queues.local.as_ref().map(|local| local.0);
    let world_row = local.and_then(|id| {
        queues
            .world
            .0
            .player(id)
            .map(|ps| (ps.origin, ps.e_flags as i32, ps.other_flags as i32))
    });
    let snapshot_row = local.and_then(|id| player_origin_flags(&tick.snapshot.players, id));
    let sent_row = local.and_then(|id| player_origin_flags(&sent.players, id));
    let census = &mut queues.census;
    census.tick = Some(tick.snapshot.tick.0);
    census.world_origin = world_row.map(|row| row.0);
    census.snapshot_origin = snapshot_row.map(|row| row.0);
    census.sent_origin = sent_row.map(|row| row.0);
    census.seat_applied = Some(seat_applied);
    census.world_other_flags = world_row.map(|row| row.2);
    census.sent_other_flags = sent_row.map(|row| row.2);
    census.sent_delta_time = sent_row.map(|row| row.3);

    let local_life = local.and_then(|id| {
        tick.snapshot
            .meta
            .for_client(id)
            .map(|meta| lifecycle_label(meta.lifecycle))
    });
    perf::feel(
        now_ms,
        local_life,
        Some(seat_applied),
        census.seat_lookup_tick,
        None,
        census.sent_delta_time,
        None,
        None,
        None,
        None,
        None,
        None,
    );
}

/// Queue this tick's script-driven reliable rows for every client in the snapshot's roster.
fn queue_rows(
    reliable: &mut ReliableEventHub,
    queues: &mut FanoutQueues,
    tick: &ServerTickData,
    scores_for_all: bool,
) {
    for (client, _) in &tick.snapshot.meta.clients {
        let queue = reliable.queue_mut(*client);
        for sound in queues.svc.take_for(*client) {
            queue.push(ReliableRow::Sound(sound));
        }
        let (cards, menus, splashes) = queues.playercard.take_for(*client);
        for card in cards {
            queue.push(ReliableRow::Card(card));
        }
        for menu in menus {
            queue.push(ReliableRow::Menu(menu));
        }
        for splash in splashes {
            queue.push(ReliableRow::Splash(splash));
        }
        for notify in queues.gamenotify.take_for(*client) {
            queue.push(ReliableRow::Notify(notify));
        }
        if scores_for_all {
            queue.push(ReliableRow::Scores(format_scoreboard_from_snapshot(
                &tick.snapshot,
            )));
        }
    }
    queues.gamenotify.clear_after_fanout();
}

/// Send the local player's frame. The error is the diagnostics line to show.
fn send_loopback(
    loopback: &mut ListenLoopback,
    reliable: &ReliableEventHub,
    queues: &mut FanoutQueues,
    tick: &ServerTickData,
    listen_snapshot: &Snapshot,
    acks: &[(sim::ClientId, crate::client::predict::CmdSeq)],
    svc_scores: Option<String>,
) -> Result<(), String> {
    let local = queues.local.as_ref().map(|local| local.0);
    let sounds = local.map(|id| queues.svc.take_for(id)).unwrap_or_default();
    let (cards, menus, splashes) = local
        .map(|id| queues.playercard.take_for(id))
        .unwrap_or_default();
    let notifies = local
        .map(|id| queues.gamenotify.take_for(id))
        .unwrap_or_default();
    let payload = local.map_or_else(ReliablePayload::owes_nothing, |id| reliable.payload(id));
    loopback
        .send_tick(
            &tick.input,
            listen_snapshot,
            acks.to_vec(),
            sounds,
            svc_scores,
            cards,
            menus,
            splashes,
            notifies,
            payload,
        )
        .map_err(|e| format!("loopback: send_tick failed: {e}"))
}

/// Commands nobody was addressed to this tick are dropped; say so in the log.
fn report_stranded(queues: &mut FanoutQueues) {
    let stranded = queues.svc.drain_stranded();
    if !stranded.is_empty() {
        diag::info!(
            Net,
            "svc p/h: {} command(s) had no recipient Frame this Fanout (stranded {})",
            stranded.len(),
            queues.svc.stranded
        );
    }
    let pc_stranded = queues.playercard.drain_stranded();
    if pc_stranded > 0 {
        diag::info!(
            Net,
            "svc K/u: {pc_stranded} command(s) had no recipient Frame this Fanout (stranded {})",
            queues.playercard.stranded
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use sim::{
        ClientId, DamageSource, EventAudience, EventRecord, EventSequence, LifeSequence, SimEvent,
        Tick,
    };

    fn fanout_world() -> World {
        let mut world = World::new();
        world.init_resource::<ServerTick>();
        world.init_resource::<ReliableEventHub>();
        world.init_resource::<PendingAcks>();
        world.init_resource::<PendingSvcSounds>();
        world.init_resource::<PendingPlayerCard>();
        world.init_resource::<PendingGameNotify>();
        world.init_resource::<PendingScoreboard>();
        world.init_resource::<AuthorityWorld>();
        world.init_resource::<ListenFanoutCensus>();
        world.init_resource::<DumpDeathLog>();
        world.init_resource::<DumpGiveLog>();
        world.init_resource::<DumpConfigurationChangeLog>();
        world.init_resource::<Messages<ExitLevelCalled>>();
        world.init_resource::<NetDiagnostics>();
        world.init_resource::<FrameArchive>();
        world.init_resource::<ActiveKillcams>();
        world.init_resource::<AuthorityClock>();
        world.init_resource::<AuthorityPhaseTrace>();
        world
    }

    fn tick(number: u32, journal: Vec<EventRecord>, exit_level: bool) -> ServerTickData {
        let mut snapshot = Snapshot::unpublished(Tick(number));
        snapshot.meta.journal = journal;
        ServerTickData {
            input: sim::TickInput::default(),
            snapshot,
            weapon_script_names: Vec::<String>::new().into(),
            pending_final_kill: None,
            script_seats: Vec::new(),
            script_exit_level: exit_level,
        }
    }

    fn death_record() -> EventRecord {
        EventRecord {
            sequence: EventSequence(0),
            tick: Tick(4),
            audience: EventAudience::All,
            event: SimEvent::Died {
                victim: ClientId(1),
                life_sequence: LifeSequence(1),
                attacker: Some(ClientId(2)),
                attacker_life: Some(LifeSequence(1)),
                source: Some(DamageSource::Melee),
                weapon: 3,
                killcam_entity_start_time: 0,
            },
        }
    }

    #[test]
    fn nothing_is_published_without_a_server_tick() {
        let mut world = fanout_world();
        world.run_system_once(fanout).unwrap();
        assert_eq!(world.resource::<ListenFanoutCensus>().tick, None);
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Fanout"]);
    }

    #[test]
    fn a_tick_without_a_local_player_publishes_an_empty_seat_census() {
        let mut world = fanout_world();
        world.resource_mut::<ServerTick>().0 = Some(tick(9, Vec::new(), false));
        world.run_system_once(fanout).unwrap();
        let census = world.resource::<ListenFanoutCensus>();
        assert_eq!(census.tick, Some(9));
        assert_eq!(census.seat_applied, Some(0));
        assert_eq!(census.world_origin, None);
        assert_eq!(census.sent_origin, None);
        assert_eq!(census.seat_lookup_tick, None);
    }

    #[test]
    fn stale_seat_fields_are_cleared_on_every_tick() {
        let mut world = fanout_world();
        world.resource_mut::<ListenFanoutCensus>().seat_focus_client = Some(4);
        world.resource_mut::<ServerTick>().0 = Some(tick(1, Vec::new(), false));
        world.run_system_once(fanout).unwrap();
        assert_eq!(world.resource::<ListenFanoutCensus>().seat_focus_client, None);
    }

    #[test]
    fn the_journal_feeds_the_dump_logs() {
        let mut world = fanout_world();
        world.resource_mut::<ServerTick>().0 = Some(tick(4, vec![death_record()], false));
        world.run_system_once(fanout).unwrap();
        let deaths = world.resource::<DumpDeathLog>();
        assert_eq!(deaths.rows.len(), 1);
        assert_eq!(deaths.rows[0].source, "melee");
    }

    #[test]
    fn a_script_exit_request_becomes_a_message() {
        let mut world = fanout_world();
        world.resource_mut::<ServerTick>().0 = Some(tick(2, Vec::new(), true));
        world.run_system_once(fanout).unwrap();
        assert_eq!(world.resource::<Messages<ExitLevelCalled>>().len(), 1);

        let mut quiet = fanout_world();
        quiet.resource_mut::<ServerTick>().0 = Some(tick(2, Vec::new(), false));
        quiet.run_system_once(fanout).unwrap();
        assert_eq!(quiet.resource::<Messages<ExitLevelCalled>>().len(), 0);
    }

    #[test]
    fn staged_acks_are_consumed_by_the_fanout() {
        let mut world = fanout_world();
        world.resource_mut::<PendingAcks>().0 =
            vec![(ClientId(1), crate::client::predict::CmdSeq::default())];
        world.resource_mut::<ServerTick>().0 = Some(tick(3, Vec::new(), false));
        world.run_system_once(fanout).unwrap();
        assert!(world.resource::<PendingAcks>().0.is_empty());
    }

    #[test]
    fn bookkeeping_archives_the_published_tick() {
        let mut world = World::new();
        world.init_resource::<FrameArchive>();
        world.init_resource::<ActiveKillcams>();
        world.insert_resource(ServerTick(Some(tick(5, Vec::new(), false))));
        world.insert_resource(AuthorityPhaseTrace::default());
        world.run_system_once(authority_bookkeeping).unwrap();
        assert_eq!(world.resource::<AuthorityPhaseTrace>().0, vec!["Bookkeeping"]);
    }

    #[test]
    fn bookkeeping_without_a_tick_archives_nothing() {
        let mut world = World::new();
        world.init_resource::<FrameArchive>();
        world.init_resource::<ActiveKillcams>();
        world.init_resource::<ServerTick>();
        world.run_system_once(authority_bookkeeping).unwrap();
    }
}
