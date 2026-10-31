//! The fan-out phase: hand the finished tick to every player. Script output is folded into the
//! queues first, the local player's view is built (killcam seat included), guaranteed rows are
//! queued per peer, and the loopback and the UDP hub each get their frame.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::LevelQuitRequested;
use sim::{ClientId, Snapshot};

use super::census::LocalHostFanoutTally;
use super::dumps::{
    DumpConfigurationChangeLog, DeathLogDumper, GrantLogDumper, lifecycle_label, player_origin_flags,
};
use super::loop_resources::{
    HostStageTrail, HostSimulationWorld, LinkDiagnosticsReport, StagedReceipts, HostStepNumber, HostStepOutput,
};
use crate::host_tick::peer_mail::HostTimeKeeper;
use crate::client::run_ahead::InputCommandNumber;
use crate::client::presentation::shown_frame::HostViewClient;
use crate::viewer_policy::cam_view::LiveDeathReplays;
use crate::net_links::snapshot_history::SnapshotVault;
use crate::net_links::listen_link::LocalHostTickPipe;
use crate::net_links::relay::HostUdpSwitchboard;
use crate::{
    PlayerControlSnapshot, StagedAnnouncements, StagedCardUpdate, StagedScoreboardSend, StagedHostSounds,
    GuaranteedEventRouter, GuaranteedEnvelope, GuaranteedEntry, compose_standings_from_state,
};

/// Script-produced mail waiting for this tick's frames, plus the command receipts_batch staged for them.
#[derive(SystemParam)]
pub(super) struct ScriptMail<'w> {
    staged_acks: ResMut<'w, StagedReceipts>,
    sounds: ResMut<'w, StagedHostSounds>,
    cards: ResMut<'w, StagedCardUpdate>,
    notes: ResMut<'w, StagedAnnouncements>,
    board: ResMut<'w, StagedScoreboardSend>,
}

/// The host's own side: the sim, who plays locally, the debug census and the console dump logs.
#[derive(SystemParam)]
pub(super) struct HostSide<'w> {
    sim: ResMut<'w, HostSimulationWorld>,
    viewer: Option<Res<'w, HostViewClient>>,
    keys: Option<Res<'w, PlayerControlSnapshot>>,
    census: ResMut<'w, LocalHostFanoutTally>,
    death_log: ResMut<'w, DeathLogDumper>,
    give_log: ResMut<'w, GrantLogDumper>,
    change_log: ResMut<'w, DumpConfigurationChangeLog>,
    quit: MessageWriter<'w, LevelQuitRequested>,
}

impl HostSide<'_> {
    fn local_id(&self) -> Option<ClientId> {
        self.viewer.as_deref().map(|present| present.0)
    }

    fn scoreboard_held(&self) -> bool {
        self.keys.as_deref().map_or(false, |held| held.client.pressed_key_latch.scores.active)
    }
}

/// Where frames leave the host, and where a failed send is reported.
#[derive(SystemParam)]
pub(super) struct Outlets<'w> {
    loopback: Option<ResMut<'w, LocalHostTickPipe>>,
    relay: Option<ResMut<'w, HostUdpSwitchboard>>,
    diagnostics: ResMut<'w, LinkDiagnosticsReport>,
}

pub(super) fn fanout(
    published: Res<HostStepNumber>,
    mut guaranteed: ResMut<GuaranteedEventRouter>,
    mut mail: ScriptMail,
    mut host: HostSide,
    mut out: Outlets,
    frames: Res<SnapshotVault>,
    mut killcams: ResMut<LiveDeathReplays>,
    clock: Res<HostTimeKeeper>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Fanout");
    let Some(data) = &published.0 else { return };
    let now = clock.clock_stamp_ms;

    take_script_output(&mut mail, &mut host, data, &frames, &mut killcams, now);
    let (local_view, installed) = local_snapshot(&mut host, data, &frames, &killcams, now);
    fill_census(&mut host, data, &local_view, installed, now);

    let receipts_batch = std::mem::take(&mut mail.staged_acks.0);
    let local_board = mail
        .board
        .held_board_ready(host.scoreboard_held(), now)
        .then(|| compose_standings_from_state(&local_view));
    let board_for_everyone = mail.board.fanout_ready(now);
    queue_reliable_rows(&mut guaranteed, &mut mail, data, board_for_everyone);

    if let Some(link) = out.loopback.as_deref_mut() {
        let viewer = host.local_id();
        if let Err(why) = send_local_frame(link, viewer, &guaranteed, &mut mail, data, &local_view, &receipts_batch, local_board) {
            out.diagnostics.report(why);
        }
    }
    if let Some(relay) = out.relay.as_deref_mut() {
        let result = relay.broadcast_per_seat(
            &data.input,
            &data.snapshot,
            &receipts_batch,
            |peer, live| crate::viewer_policy::cam_view::state_for_viewer(&frames, &killcams, live, peer, now),
            Some(&mut mail.sounds),
            Some(&mut mail.cards),
            Some(&mut mail.notes),
            Some(&guaranteed),
            false,
        );
        if let Err(problem) = result {
            out.diagnostics.report(format!("udp fanout: {problem}"));
        }
    }
    log_stranded(&mut mail);
    mail.notes.reset_post_broadcast();
}

pub fn host_stage_upkeep(
    host_step_from_raw: Res<HostStepNumber>,
    mut frames: ResMut<SnapshotVault>,
    seats: Res<LiveDeathReplays>,
    trace: Option<ResMut<HostStageTrail>>,
) {
    HostStageTrail::mark(trace, "Upkeep");
    let Some(data) = &host_step_from_raw.0 else { return };
    let viewers = seats.watching();
    frames.append_snapshot(&data.snapshot, &viewers);
}

/// Take what the match scripts produced this tick: queued sounds, cards and notifications, killcam
/// seats, an exit-level request, and the events rows the console dumps show.
fn take_script_output(
    mail: &mut ScriptMail,
    host: &mut HostSide,
    data: &HostStepOutput,
    frames: &SnapshotVault,
    killcams: &mut LiveDeathReplays,
    now: i32,
) {
    let sim = &mut host.sim.0;
    mail.cards.absorb_sim_queue(sim);
    mail.notes.absorb_sim_queue(sim);
    mail.sounds.absorb_sim_queue(sim);
    let started = crate::viewer_policy::cam_requests::play_script_seats(killcams, frames, now, &data.script_seats);
    started.into_iter().for_each(|(viewer, ms)| sim.store_script_seat_archive(viewer, ms));
    if data.script_exit_level {
        host.quit.write(LevelQuitRequested);
        diag::info!(Sim, "match scripts asked for exitLevel");
    }
    let rows = &data.snapshot.meta.journal;
    host.death_log.fold_in(rows);
    host.give_log.fold_in(rows);
    host.change_log.fold_in(rows);
}

/// The snapshot the local player is sent, and 1 when a killcam seat reshaped it (otherwise 0).
/// The seat census fields are filled only while a seat applies.
fn local_snapshot(
    host: &mut HostSide,
    data: &HostStepOutput,
    frames: &SnapshotVault,
    killcams: &LiveDeathReplays,
    now: i32,
) -> (Snapshot, i32) {
    host.census.clear_seat();
    let Some(me) = host.local_id() else {
        return (data.snapshot.clone(), 0);
    };
    let (view, seat_sample) =
        crate::viewer_policy::cam_view::state_and_reading_for_viewer(frames, killcams, &data.snapshot, me, now);
    let Some(found) = seat_sample else {
        return (view, 0);
    };
    let seat = killcams.run_for(me);
    let watched = seat.map(|s| s.spectated_receiver);
    let watched_origin = watched.and_then(|who| host.sim.0.player(who)).map(|ps| ps.origin);
    let watched_stage = watched.and_then(|who| host.sim.0.client_meta(who)).map(|meta| lifecycle_label(meta.lifecycle));

    let census = &mut *host.census;
    census.seat_history_clock_millis = seat.map(|s| s.history_window_millis);
    census.seat_watched_receiver = watched.map(|who| who.0 as i32);
    census.seat_query_tick = found.lookup.found_tick.map(|t| t.0 as i32);
    census.seat_reached_millis = Some(found.lookup.reached_millis);
    census.seat_rebaseline_millis = Some(found.clock_shift_millis);
    census.seat_scene_in_history = Some(found.lookup.found_tick.is_some() as i32);
    census.seat_watched_live_position = watched_origin;
    census.seat_watched_life_stage = watched_stage;
    (view, 1)
}

/// Fill the debug census with the local player's row as the world, the snapshot and the sent
/// frame each see it, and feed the same numbers to the feel trace.
fn fill_census(host: &mut HostSide, data: &HostStepOutput, sent: &Snapshot, installed: i32, now: i32) {
    let me = host.local_id();
    let live = me.and_then(|id| host.sim.0.player(id)).map(|ps| (ps.origin, ps.secondary_state_bits as i32));
    let in_snapshot = me.and_then(|id| player_origin_flags(&data.snapshot.players, id));
    let in_frame = me.and_then(|id| player_origin_flags(&sent.players, id));
    let stage = me.and_then(|id| data.snapshot.meta.for_client(id)).map(|meta| lifecycle_label(meta.lifecycle));

    let census = &mut *host.census;
    census.fanned_tick = Some(data.snapshot.tick.0);
    census.viewer_seat_installed = Some(installed);
    census.scene_anchor = live.map(|(origin, _)| origin);
    census.world_misc_bits = live.map(|(_, other)| other);
    census.state_position = in_snapshot.map(|(origin, ..)| origin);
    census.transmitted_position = in_frame.map(|(origin, ..)| origin);
    census.transmitted_misc_flags = in_frame.map(|(_, _, other, _)| other);
    census.transmitted_step_gap = in_frame.map(|(.., delta)| delta);

    let (lookup, delta) = (census.seat_query_tick, census.transmitted_step_gap);
    perf::feel(now, stage, Some(installed), lookup, None, delta, None, None, None, None, None, None);
}

/// One peer's guaranteed rows for this tick, in send order: sounds, card slots, menus, splashes,
/// notifications and (when due for everyone) the scoreboard.
fn rows_for(peer: ClientId, mail: &mut ScriptMail, board: Option<&String>) -> Vec<GuaranteedEntry> {
    let sounds = mail.sounds.hand_to(peer);
    let (slots, menus, splashes) = mail.cards.hand_to(peer);
    let notes = mail.notes.hand_to(peer);
    sounds
        .into_iter()
        .map(GuaranteedEntry::Sound)
        .chain(slots.into_iter().map(GuaranteedEntry::Card))
        .chain(menus.into_iter().map(GuaranteedEntry::Menu))
        .chain(splashes.into_iter().map(GuaranteedEntry::Splash))
        .chain(notes.into_iter().map(GuaranteedEntry::Notify))
        .chain(board.cloned().map(GuaranteedEntry::Standings))
        .collect()
}

/// Queue this tick's script-driven guaranteed rows for every peer in the snapshot's roster.
fn queue_reliable_rows(guaranteed: &mut GuaranteedEventRouter, mail: &mut ScriptMail, data: &HostStepOutput, board_due: bool) {
    let board = board_due.then(|| compose_standings_from_state(&data.snapshot));
    for &(peer, _) in &data.snapshot.meta.clients {
        let rows = rows_for(peer, mail, board.as_ref());
        let queue = guaranteed.backlog_for_player(peer);
        for item in rows {
            queue.stash_row(item);
        }
    }
    mail.notes.reset_post_broadcast();
}

/// Send the local player's frame. The error is the diagnostics line to show.
fn send_local_frame(
    link: &mut LocalHostTickPipe,
    viewer: Option<ClientId>,
    guaranteed: &GuaranteedEventRouter,
    mail: &mut ScriptMail,
    data: &HostStepOutput,
    view: &Snapshot,
    receipts_batch: &[(ClientId, InputCommandNumber)],
    board: Option<String>,
) -> Result<(), String> {
    let (sounds, (slots, menus, splashes), notes, owed) = match viewer {
        Some(me) => (mail.sounds.hand_to(me), mail.cards.hand_to(me), mail.notes.hand_to(me), guaranteed.owed_envelope(me)),
        None => (Default::default(), Default::default(), Default::default(), GuaranteedEnvelope::has_no_debt()),
    };
    let sent = link.transmit_step(&data.input, view, receipts_batch.to_vec(), sounds, board, slots, menus, splashes, notes, owed);
    sent.map_err(|problem| format!("loopback: send_tick failed: {problem}"))
}

/// Commands nobody was addressed to this tick are dropped; say so in the log.
fn log_stranded(mail: &mut ScriptMail) {
    let lost_sounds = mail.sounds.flush_orphaned().len();
    if lost_sounds > 0 {
        let total = mail.sounds.unclaimed;
        diag::info!(WireLink, "svc p/h: {lost_sounds} command(s) had no recipient Frame this Fanout (stranded {total})");
    }
    let lost_cards = mail.cards.flush_orphaned();
    if lost_cards > 0 {
        let total = mail.cards.unclaimed;
        diag::info!(WireLink, "svc K/u: {lost_cards} command(s) had no recipient Frame this Fanout (stranded {total})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use sim::{HarmOrigin, EventRecipients, EventRecord, EventSequence, LifeSequence, MatchOccurrence, Tick};

    macro_rules! with_defaults {
        ($world:ident: $($res:ty),+ $(,)?) => {
            $( $world.init_resource::<$res>(); )+
        };
    }

    fn fanout_world() -> World {
        let mut world = World::new();
        with_defaults!(world:
            HostStepNumber, GuaranteedEventRouter, StagedReceipts, StagedHostSounds, StagedCardUpdate,
            StagedAnnouncements, StagedScoreboardSend, HostSimulationWorld, LocalHostFanoutTally, DeathLogDumper,
            GrantLogDumper, DumpConfigurationChangeLog, Messages<LevelQuitRequested>, LinkDiagnosticsReport,
            SnapshotVault, LiveDeathReplays, HostTimeKeeper, HostStageTrail,
        );
        world
    }

    fn published(number: u32, events: Vec<EventRecord>, exit_level: bool) -> HostStepOutput {
        let mut snapshot = Snapshot::unpublished(Tick(number));
        snapshot.meta.journal = events;
        HostStepOutput {
            input: sim::StepInputs::default(),
            snapshot,
            weapon_ids_by_script: Vec::<String>::new().into(),
            pending_last_kill: None,
            script_seats: Vec::new(),
            script_exit_level: exit_level,
        }
    }

    /// A world holding tick `number`, after one fan-out.
    fn fanned(number: u32, events: Vec<EventRecord>, exit_level: bool, prep: impl FnOnce(&mut World)) -> World {
        let mut world = fanout_world();
        prep(&mut world);
        world.resource_mut::<HostStepNumber>().0 = Some(published(number, events, exit_level));
        world.run_system_once(fanout).unwrap();
        world
    }

    fn melee_death() -> EventRecord {
        let died = MatchOccurrence::Died {
            victim: ClientId(1),
            life_sequence: LifeSequence(1),
            attacker: Some(ClientId(2)),
            attacker_life: Some(LifeSequence(1)),
            source: Some(HarmOrigin::Melee),
            gun: 3,
            cam_ent_since: 0,
        };
        EventRecord { sequence: EventSequence(0), tick: Tick(4), audience: EventRecipients::All, event: died }
    }

    #[test]
    fn nothing_is_published_without_a_server_tick() {
        let mut world = fanout_world();
        world.run_system_once(fanout).unwrap();
        assert!(world.resource::<LocalHostFanoutTally>().fanned_tick.is_none());
        assert_eq!(world.resource::<HostStageTrail>().0, ["Fanout"]);
    }

    #[test]
    fn a_tick_without_a_local_player_publishes_an_empty_seat_census() {
        let world = fanned(9, Vec::new(), false, |_| {});
        let census = world.resource::<LocalHostFanoutTally>();
        assert_eq!((census.fanned_tick, census.viewer_seat_installed), (Some(9), Some(0)));
        assert!(census.scene_anchor.is_none() && census.transmitted_position.is_none());
        assert!(census.seat_query_tick.is_none());
    }

    #[test]
    fn stale_seat_fields_are_cleared_on_every_tick() {
        let world = fanned(1, Vec::new(), false, |w| {
            w.resource_mut::<LocalHostFanoutTally>().seat_watched_receiver = Some(4);
        });
        assert!(world.resource::<LocalHostFanoutTally>().seat_watched_receiver.is_none());
    }

    #[test]
    fn the_journal_feeds_the_dump_logs() {
        let world = fanned(4, vec![melee_death()], false, |_| {});
        let sources: Vec<&str> = world.resource::<DeathLogDumper>().rows.iter().map(|r| r.source).collect();
        assert_eq!(sources, ["melee"]);
    }

    #[test]
    fn a_script_exit_request_becomes_a_message() {
        for (asked, expected) in [(true, 1), (false, 0)] {
            let world = fanned(2, Vec::new(), asked, |_| {});
            assert_eq!(world.resource::<Messages<LevelQuitRequested>>().len(), expected);
        }
    }

    #[test]
    fn staged_acks_are_consumed_by_the_fanout() {
        let world = fanned(3, Vec::new(), false, |w| {
            w.resource_mut::<StagedReceipts>().0.push((ClientId(1), InputCommandNumber::default()));
        });
        assert_eq!(world.resource::<StagedReceipts>().0.len(), 0);
    }

    fn bookkeeping_world(data: Option<HostStepOutput>) -> World {
        let mut world = World::new();
        with_defaults!(world: SnapshotVault, LiveDeathReplays, HostStageTrail);
        world.insert_resource(HostStepNumber(data));
        world.run_system_once(host_stage_upkeep).unwrap();
        world
    }

    #[test]
    fn bookkeeping_archives_the_published_tick() {
        let world = bookkeeping_world(Some(published(5, Vec::new(), false)));
        assert_eq!(world.resource::<HostStageTrail>().0, ["Upkeep"]);
    }

    #[test]
    fn bookkeeping_without_a_tick_archives_nothing() {
        bookkeeping_world(None);
    }
}
