//! Hooks recording, the clip ring and playback into the game's schedule.
//!
//! * A server (listen or dedicated) puts every tick it runs into the clip ring
//!   and, while a recording is open, into the file.
//! * A client puts every tick it receives into the clip ring.
//! * In theater mode ([`RuntimeRole::Replay`]) a recording takes the server's
//!   place: [`pump_playback`] hands the client its frames at the pace the
//!   server sent them.

use bevy::prelude::*;
use frame::{MatchTornDown, ReturnedToMenu, RuntimeRole, SessionSwapApplied};
use net::{
    AUTHORITY_MS, AuthorityClock, AuthorityLoadHold, AuthoritySet, ClientClock, ClientSet,
    ReceivedTick, ReceivedTicks, ServerTick, ServerTime, authority_should_tick,
};
use sim::TickInput;

use crate::clip::ClipRing;
use crate::session::{Playback, PlayedTick, Recording};

/// A recording waiting for theater mode to start; the console fills this in
/// before the match loads.
#[derive(Resource, Default)]
pub struct PendingReplayArm {
    pub playback: Option<Playback>,
    /// Whether the game closes when the recording ends.
    pub quit_on_end: bool,
}

/// The recording being written right now, if any.
#[derive(Resource, Default)]
pub struct ReplaySession(pub Option<Recording>);

/// The last thing recording or playback wants the player to be told.
#[derive(Resource, Default)]
pub struct ReplayDiagnostics {
    pub line: Option<String>,
}

/// A recording being played into the client.
#[derive(Resource)]
pub struct ReplayPlayback {
    playback: Playback,
    /// Wall time held back from the next tick, in ms. It starts with a whole
    /// tick, so the first frame shows at once.
    acc_ms: f32,
    pub ended: bool,
    pub quit_on_end: bool,
    /// Set by whoever acts on `quit_on_end`, so the game is asked to close
    /// only once.
    pub exit_queued: bool,
}

impl ReplayPlayback {
    pub fn new(playback: Playback) -> Self {
        Self {
            playback,
            acc_ms: AUTHORITY_MS as f32,
            ended: false,
            quit_on_end: true,
            exit_queued: false,
        }
    }

    pub fn with_quit_on_end(mut self, quit_on_end: bool) -> Self {
        self.quit_on_end = quit_on_end;
        self
    }

    pub fn ticks(&self) -> u64 {
        self.playback.ticks()
    }

    pub fn acc_ms(&self) -> f32 {
        self.acc_ms
    }

    pub fn path(&self) -> &std::path::Path {
        self.playback.path()
    }

    /// Lets `delta_ms` of wall time pass and returns the ticks it released,
    /// one per 50 ms. Reaching the end of the recording, or a frame that does
    /// not read, marks it `ended` and releases nothing more.
    fn pull(&mut self, delta_ms: f32) -> Vec<PlayedTick> {
        let slot = AUTHORITY_MS as f32;
        let mut released = Vec::new();
        self.acc_ms += delta_ms;
        while self.acc_ms >= slot {
            self.acc_ms -= slot;
            match self.playback.next_tick() {
                Ok(Some(tick)) => released.push(tick),
                Ok(None) => {
                    self.ended = true;
                    diag::info!(
                        Net,
                        "play: ended after {} ticks ({})",
                        self.playback.ticks(),
                        self.playback.path().display()
                    );
                    break;
                }
                Err(error) => {
                    self.ended = true;
                    diag::warn!(Net, "play: {error}");
                    break;
                }
            }
        }
        released
    }
}

pub struct ReplayPlugin;

impl Plugin for ReplayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplaySession>()
            .init_resource::<ReplayDiagnostics>()
            .init_resource::<PendingReplayArm>()
            .init_resource::<ClipRing>();

        let role = app.world().get_resource::<RuntimeRole>().copied();
        if role.is_some_and(|role| role.runs_authority() || role == RuntimeRole::Replay) {
            app.add_systems(
                FixedUpdate,
                record_server_tick
                    .in_set(AuthoritySet::Fanout)
                    .run_if(authority_should_tick),
            );
        }

        app.add_systems(
            Update,
            (
                record_received_ticks
                    .after(ClientSet::Receive)
                    .before(ClientSet::Reconcile),
                sync_theater_occupancy
                    .in_set(ClientSet::Load)
                    .after(SessionSwapApplied),
                pump_playback
                    .in_set(ClientSet::Load)
                    .run_if(resource_exists::<ReplayPlayback>)
                    .after(sync_theater_occupancy),
            ),
        );
    }
}

/// Starts a waiting recording when the role is theater mode, and drops
/// playback and the clip ring when the match ends.
fn sync_theater_occupancy(
    mut commands: Commands,
    mut torn_down: MessageReader<MatchTornDown>,
    mut returned: MessageReader<ReturnedToMenu>,
    mut pending: ResMut<PendingReplayArm>,
    mut ring: ResMut<ClipRing>,
    role: Res<RuntimeRole>,
) {
    // Both readers are drained every time, so neither replays old messages.
    let match_over = torn_down.read().count() + returned.read().count() > 0;
    if match_over {
        ring.clear();
        commands.remove_resource::<ReplayPlayback>();
        perf::theater(0, None, None);
    }

    if *role != RuntimeRole::Replay {
        return;
    }
    let Some(playback) = pending.playback.take() else {
        return;
    };
    let quit_on_end = pending.quit_on_end;
    commands.insert_resource(ReplayPlayback::new(playback).with_quit_on_end(quit_on_end));
    perf::theater(
        1,
        Some(i64::from(quit_on_end)),
        Some(i64::from(role.runs_authority())),
    );
}

/// Keeps what a client was sent. A server keeps its own ticks instead (see
/// [`record_server_tick`]).
fn record_received_ticks(
    received: Res<ReceivedTicks>,
    role: Res<RuntimeRole>,
    mut ring: ResMut<ClipRing>,
) {
    if role.runs_authority() {
        return;
    }
    for tick in &received.0 {
        let input = TickInput {
            cmds: tick.frame.cmds.clone(),
            actions: tick.frame.actions.clone(),
        };
        ring.push(input, tick.snapshot.clone());
    }
}

/// Keeps the tick the server just ran: in the clip ring, and in the open
/// recording. A recording that cannot be written is closed and the player is
/// told, instead of failing every tick after.
fn record_server_tick(
    server_tick: Res<ServerTick>,
    role: Res<RuntimeRole>,
    mut ring: ResMut<ClipRing>,
    mut session: ResMut<ReplaySession>,
    mut diagnostics: ResMut<ReplayDiagnostics>,
) {
    let Some(tick) = server_tick.0.as_ref() else {
        return;
    };
    if role.runs_authority() {
        ring.push(tick.input.clone(), tick.snapshot.clone());
    }
    let Some(recording) = session.0.as_mut() else {
        return;
    };
    if let Err(error) = recording.record(&tick.input, &tick.snapshot) {
        let line = format!("record: write failed, recording stopped: {error}");
        diag::info!(Net, "{line}");
        diagnostics.line = Some(line);
        session.0 = None;
    }
}

/// Feeds the client the recording's frames at 20 per second of wall time.
fn pump_playback(
    mut playback: ResMut<ReplayPlayback>,
    mut received: ResMut<ReceivedTicks>,
    mut authority_clock: Option<ResMut<AuthorityClock>>,
    mut client_clock: ResMut<ClientClock>,
    load_hold: Option<Res<AuthorityLoadHold>>,
    time: Res<Time>,
) {
    if playback.ended || load_hold.is_some_and(|hold| hold.0) {
        return;
    }
    let released = playback.pull(time.delta_secs() * 1000.0);

    if let Some(last) = released.last() {
        let tick = last.snapshot.tick;
        if let Some(clock) = authority_clock.as_mut() {
            clock.tick = tick.0;
            clock.time_ms = ServerTime::from_tick(tick).ms();
        }
        // Names the demo tick about to be shown: the same clip played at two
        // frame rates shows different frame numbers, but the same ticks.
        perf::frames::set_replay_tick(u64::from(tick.0));
    }
    for played in released {
        received.0.push_back(ReceivedTick {
            snapshot: played.snapshot,
            frame: played.frame,
        });
    }
    client_clock.accumulator_ms = playback.acc_ms;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::{frames, input, snapshot};
    use bevy::ecs::system::RunSystemOnce;
    use net::ServerTickData;
    use std::sync::Arc;
    use std::time::Duration;

    fn playback_of(dir: &TestDir, ticks: u32) -> Playback {
        let mut recording = Recording::start(dir.path(), "plugin").unwrap();
        for tick in 1..=ticks {
            recording.record(&input(tick), &snapshot(tick)).unwrap();
        }
        let (_, path) = recording.stop().unwrap();
        Playback::open_path(&path).unwrap()
    }

    fn world_with(role: RuntimeRole) -> World {
        let mut world = World::new();
        world.insert_resource(role);
        world.init_resource::<ClipRing>();
        world.init_resource::<PendingReplayArm>();
        world.init_resource::<Messages<MatchTornDown>>();
        world.init_resource::<Messages<ReturnedToMenu>>();
        world
    }

    fn playing_world(dir: &TestDir, ticks: u32) -> World {
        let mut world = World::new();
        world.insert_resource(ReplayPlayback::new(playback_of(dir, ticks)));
        world.init_resource::<ReceivedTicks>();
        world.init_resource::<ClientClock>();
        world.init_resource::<AuthorityClock>();
        world.init_resource::<Time>();
        world
    }

    fn received_ticks(world: &World) -> Vec<u32> {
        let received = world.resource::<ReceivedTicks>();
        received.0.iter().map(|tick| tick.snapshot.tick.0).collect()
    }

    fn server_tick(tick: u32) -> ServerTick {
        ServerTick(Some(ServerTickData {
            input: input(tick),
            snapshot: snapshot(tick),
            weapon_script_names: Arc::from(Vec::<String>::new()),
            pending_final_kill: None,
            script_seats: Vec::new(),
            script_exit_level: false,
        }))
    }

    fn server_world(role: RuntimeRole, dir: &TestDir, recording: bool) -> World {
        let mut world = world_with(role);
        world.insert_resource(server_tick(1));
        world.init_resource::<ReplayDiagnostics>();
        let open = recording.then(|| Recording::start(dir.path(), "server").unwrap());
        world.insert_resource(ReplaySession(open));
        world
    }

    fn ring_len(world: &World) -> usize {
        world.resource::<ClipRing>().len()
    }

    fn recorded_ticks(world: &World) -> Option<u64> {
        world
            .resource::<ReplaySession>()
            .0
            .as_ref()
            .map(Recording::ticks)
    }

    #[test]
    fn playback_starts_with_a_whole_tick_in_hand_and_quits_at_the_end() {
        let dir = TestDir::new("new");
        let playing = ReplayPlayback::new(playback_of(&dir, 1));
        assert_eq!(playing.acc_ms(), 50.0);
        assert!(playing.quit_on_end);
        assert!(!playing.ended);
        assert!(!playing.exit_queued);
        assert_eq!(playing.ticks(), 0);
        assert!(!playing.with_quit_on_end(false).quit_on_end);
    }

    #[test]
    fn the_first_tick_comes_out_at_once() {
        let dir = TestDir::new("first");
        let mut playing = ReplayPlayback::new(playback_of(&dir, 3));
        let released = playing.pull(0.0);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].snapshot.tick.0, 1);
        assert_eq!(playing.acc_ms(), 0.0);
    }

    #[test]
    fn a_tick_comes_out_every_50_ms() {
        let dir = TestDir::new("pace");
        let mut playing = ReplayPlayback::new(playback_of(&dir, 20));
        playing.pull(0.0);
        assert!(playing.pull(49.0).is_empty());
        assert_eq!(playing.pull(1.0).len(), 1);
        assert_eq!(playing.pull(120.0).len(), 2);
        assert_eq!(playing.acc_ms(), 20.0);
        assert_eq!(playing.ticks(), 4);
    }

    #[test]
    fn a_long_frame_releases_every_tick_it_covers_in_order() {
        let dir = TestDir::new("burst");
        let mut playing = ReplayPlayback::new(playback_of(&dir, 10));
        playing.pull(0.0);
        let ticks: Vec<u32> = playing
            .pull(200.0)
            .iter()
            .map(|tick| tick.snapshot.tick.0)
            .collect();
        assert_eq!(ticks, [2, 3, 4, 5]);
    }

    #[test]
    fn the_end_of_the_recording_ends_playback_and_it_stays_ended() {
        let dir = TestDir::new("end");
        let mut playing = ReplayPlayback::new(playback_of(&dir, 2));
        let released = playing.pull(500.0);
        assert_eq!(released.len(), 2);
        assert!(playing.ended);
        assert_eq!(playing.ticks(), 2);
    }

    #[test]
    fn a_frame_that_will_not_read_ends_playback_without_a_tick() {
        let dir = TestDir::new("bad");
        let mut recording = Recording::start(dir.path(), "bad").unwrap();
        recording.record(&input(1), &snapshot(1)).unwrap();
        let (_, path) = recording.stop().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0xFF; 4]);
        std::fs::write(&path, bytes).unwrap();

        let mut playing = ReplayPlayback::new(Playback::open_path(&path).unwrap());
        assert_eq!(playing.pull(500.0).len(), 1);
        assert!(playing.ended);
    }

    #[test]
    fn the_plugin_adds_what_recording_needs_to_any_role() {
        for role in [
            RuntimeRole::Listen,
            RuntimeRole::Client,
            RuntimeRole::Replay,
        ] {
            let mut app = App::new();
            app.insert_resource(role).add_plugins(ReplayPlugin);
            assert!(app.world().get_resource::<ReplaySession>().is_some());
            assert!(app.world().get_resource::<ReplayDiagnostics>().is_some());
            assert!(app.world().get_resource::<PendingReplayArm>().is_some());
            assert!(app.world().get_resource::<ClipRing>().is_some());
        }
    }

    #[test]
    fn the_plugin_builds_before_the_role_is_known() {
        let mut app = App::new();
        app.add_plugins(ReplayPlugin);
        assert!(app.world().get_resource::<ClipRing>().is_some());
    }

    #[test]
    fn theater_mode_takes_up_the_recording_waiting_for_it() {
        let dir = TestDir::new("arm");
        let mut world = world_with(RuntimeRole::Replay);
        world.insert_resource(PendingReplayArm {
            playback: Some(playback_of(&dir, 3)),
            quit_on_end: false,
        });

        world.run_system_once(sync_theater_occupancy).unwrap();

        let playing = world.resource::<ReplayPlayback>();
        assert!(!playing.quit_on_end);
        assert!(!playing.ended);
        assert!(world.resource::<PendingReplayArm>().playback.is_none());
    }

    #[test]
    fn other_roles_leave_a_waiting_recording_alone() {
        let dir = TestDir::new("keep");
        for role in [
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
            RuntimeRole::Client,
        ] {
            let mut world = world_with(role);
            world.insert_resource(PendingReplayArm {
                playback: Some(playback_of(&dir, 1)),
                quit_on_end: true,
            });
            world.run_system_once(sync_theater_occupancy).unwrap();
            assert!(world.get_resource::<ReplayPlayback>().is_none(), "{role:?}");
            assert!(
                world.resource::<PendingReplayArm>().playback.is_some(),
                "{role:?}"
            );
        }
    }

    #[test]
    fn with_nothing_waiting_theater_mode_starts_nothing() {
        let mut world = world_with(RuntimeRole::Replay);
        world.run_system_once(sync_theater_occupancy).unwrap();
        assert!(world.get_resource::<ReplayPlayback>().is_none());
    }

    #[test]
    fn going_back_to_the_menu_drops_playback_and_the_clip_ring() {
        let dir = TestDir::new("menu");
        let mut world = world_with(RuntimeRole::Replay);
        world.insert_resource(ReplayPlayback::new(playback_of(&dir, 2)));
        world.resource_mut::<ClipRing>().push(input(1), snapshot(1));
        world
            .resource_mut::<Messages<ReturnedToMenu>>()
            .write(ReturnedToMenu {
                swap_id: 1,
                had_world: true,
            });

        world.run_system_once(sync_theater_occupancy).unwrap();

        assert!(world.get_resource::<ReplayPlayback>().is_none());
        assert!(world.resource::<ClipRing>().is_empty());
    }

    #[test]
    fn without_a_match_ending_playback_and_the_ring_stay() {
        let dir = TestDir::new("stay");
        let mut world = world_with(RuntimeRole::Replay);
        world.insert_resource(ReplayPlayback::new(playback_of(&dir, 2)));
        world.resource_mut::<ClipRing>().push(input(1), snapshot(1));

        world.run_system_once(sync_theater_occupancy).unwrap();

        assert!(world.get_resource::<ReplayPlayback>().is_some());
        assert_eq!(ring_len(&world), 1);
    }

    #[test]
    fn a_client_keeps_the_ticks_it_receives_once_each() {
        let mut world = world_with(RuntimeRole::Client);
        let sent = frames(3);
        let mut received = ReceivedTicks::default();
        for (index, frame) in sent.into_iter().enumerate() {
            received.0.push_back(ReceivedTick {
                snapshot: snapshot(index as u32 + 1),
                frame,
            });
        }
        world.insert_resource(received);

        world.run_system_once(record_received_ticks).unwrap();
        world.run_system_once(record_received_ticks).unwrap();

        let ring = world.resource::<ClipRing>();
        let held: Vec<u32> = ring.iter().map(|(_, snapshot)| snapshot.tick.0).collect();
        assert_eq!(held, [1, 2, 3]);
        let (first_input, _) = ring.iter().next().unwrap();
        assert_eq!(first_input, &input(1));
    }

    #[test]
    fn a_server_does_not_keep_received_ticks_it_keeps_its_own() {
        for role in [RuntimeRole::Listen, RuntimeRole::Dedicated] {
            let mut world = world_with(role);
            let mut received = ReceivedTicks::default();
            received.0.push_back(ReceivedTick {
                snapshot: snapshot(1),
                frame: frames(1).remove(0),
            });
            world.insert_resource(received);
            world.run_system_once(record_received_ticks).unwrap();
            assert_eq!(ring_len(&world), 0, "{role:?}");
        }
    }

    #[test]
    fn a_server_keeps_the_tick_it_ran_in_the_ring_and_in_the_recording() {
        let dir = TestDir::new("server");
        let mut world = server_world(RuntimeRole::Listen, &dir, true);

        world.run_system_once(record_server_tick).unwrap();

        assert_eq!(ring_len(&world), 1);
        assert_eq!(recorded_ticks(&world), Some(1));
        assert!(world.resource::<ReplayDiagnostics>().line.is_none());
    }

    #[test]
    fn with_no_recording_open_the_server_still_fills_the_ring() {
        let dir = TestDir::new("noreel");
        let mut world = server_world(RuntimeRole::Dedicated, &dir, false);
        world.run_system_once(record_server_tick).unwrap();
        assert_eq!(ring_len(&world), 1);
        assert_eq!(recorded_ticks(&world), None);
    }

    #[test]
    fn theater_mode_records_but_does_not_clip_what_it_is_only_playing() {
        let dir = TestDir::new("theater");
        let mut world = server_world(RuntimeRole::Replay, &dir, true);
        world.run_system_once(record_server_tick).unwrap();
        assert_eq!(ring_len(&world), 0);
        assert_eq!(recorded_ticks(&world), Some(1));
    }

    #[test]
    fn a_server_that_has_not_run_a_tick_records_nothing() {
        let dir = TestDir::new("idle");
        let mut world = server_world(RuntimeRole::Listen, &dir, true);
        world.insert_resource(ServerTick(None));
        world.run_system_once(record_server_tick).unwrap();
        assert_eq!(ring_len(&world), 0);
        assert_eq!(recorded_ticks(&world), Some(0));
    }

    #[test]
    fn the_pump_hands_the_client_the_first_tick_and_sets_the_clocks() {
        let dir = TestDir::new("pump");
        let mut world = playing_world(&dir, 3);

        world.run_system_once(pump_playback).unwrap();

        assert_eq!(received_ticks(&world), [1]);
        let clock = world.resource::<AuthorityClock>();
        assert_eq!(clock.tick, 1);
        assert_eq!(clock.time_ms, 50);
        assert_eq!(world.resource::<ClientClock>().accumulator_ms, 0.0);
    }

    #[test]
    fn the_pump_follows_the_frame_time() {
        let dir = TestDir::new("time");
        let mut world = playing_world(&dir, 10);
        world.run_system_once(pump_playback).unwrap();

        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(120));
        world.run_system_once(pump_playback).unwrap();

        assert_eq!(received_ticks(&world), [1, 2, 3]);
        assert_eq!(world.resource::<AuthorityClock>().tick, 3);
        assert_eq!(world.resource::<AuthorityClock>().time_ms, 150);
        assert_eq!(world.resource::<ClientClock>().accumulator_ms, 20.0);
    }

    #[test]
    fn the_pump_works_with_no_authority_clock() {
        let dir = TestDir::new("noclock");
        let mut world = playing_world(&dir, 2);
        world.remove_resource::<AuthorityClock>();
        world.run_system_once(pump_playback).unwrap();
        assert_eq!(received_ticks(&world), [1]);
    }

    #[test]
    fn the_pump_waits_while_the_match_is_loading() {
        let dir = TestDir::new("hold");
        let mut world = playing_world(&dir, 3);
        world.insert_resource(AuthorityLoadHold(true));
        world.run_system_once(pump_playback).unwrap();
        assert!(received_ticks(&world).is_empty());
        assert_eq!(world.resource::<ReplayPlayback>().acc_ms(), 50.0);

        world.insert_resource(AuthorityLoadHold(false));
        world.run_system_once(pump_playback).unwrap();
        assert_eq!(received_ticks(&world), [1]);
    }

    #[test]
    fn the_pump_stops_for_good_when_the_recording_ends() {
        let dir = TestDir::new("pumpend");
        let mut world = playing_world(&dir, 2);
        world.run_system_once(pump_playback).unwrap();
        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(500));
        world.run_system_once(pump_playback).unwrap();

        assert_eq!(received_ticks(&world), [1, 2]);
        assert!(world.resource::<ReplayPlayback>().ended);

        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(500));
        world.run_system_once(pump_playback).unwrap();
        assert_eq!(received_ticks(&world), [1, 2]);
    }

    #[test]
    fn what_the_pump_hands_over_is_what_was_recorded() {
        let dir = TestDir::new("exact");
        let mut world = playing_world(&dir, 2);
        world.run_system_once(pump_playback).unwrap();
        let received = world.resource::<ReceivedTicks>();
        let tick = &received.0[0];
        assert_eq!(tick.frame.cmds, input(1).cmds);
        assert_eq!(tick.snapshot.players.len(), 1);
    }
}
