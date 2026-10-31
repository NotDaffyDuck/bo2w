//! Hooks recording, the clip ring and playback into the game's schedule.
//!
//! * A server (listen or dedicated) puts every tick it runs into the clip ring
//!   and, while a recording is open, into the file.
//! * A client puts every tick it receives into the clip ring.
//! * In theater mode ([`ProcessDuty::Replay`]) a recording takes the server's
//!   place: [`advance_playback`] hands the client its frames at the pace the
//!   server sent them.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::GameSessionClosed;
use frame::ProcessDuty;
use frame::ReturnedToMenu;
use frame::MatchSwapDone;
use net::{
    HOST_STEP_MILLIS, HostTimeKeeper, HostLoadingPause, PlayerTimeStepper, ClientStep,
    ArrivedHostStep, ArrivedStepQueue, HostStepNumber, HostInstant, host_due_for_step,
};
use sim::StepInputs;

use crate::clip::ClipLoopBuffer;
use crate::demo::{DecodedTick, DemoRecorder, DemoReplay};

/// A recording waiting for theater mode to start; the console fills this in
/// before the match loads.
#[derive(Resource, Default)]
pub struct StagedRecordingStart {
    pub playback: Option<DemoReplay>,
    /// Whether the game closes when the recording ends.
    pub exit_when_finished: bool,
}

/// The recording being written right now, if any.
#[derive(Resource, Default)]
pub struct PlaybackSession(pub Option<DemoRecorder>);

/// The last thing recording or playback wants the player to be told.
#[derive(Resource, Default)]
pub struct RecordingStatusInfo {
    pub line: Option<String>,
}

/// One server tick of wall time, in ms.
const TICK_MS: f32 = HOST_STEP_MILLIS as f32;

/// A recording being played into the client.
#[derive(Resource)]
pub struct RecordingPlayer {
    source: DemoReplay,
    /// Wall time held back from the next tick, in ms. It starts with a whole
    /// tick, so the first frame shows at once.
    carry_ms: f32,
    pub over: bool,
    pub exit_when_finished: bool,
    /// Set by whoever acts on `exit_when_finished`, so the game is asked to close
    /// only once.
    pub shutdown_requested: bool,
}

impl RecordingPlayer {
    /// Plays `replay` and, by default, closes the game at its end.
    pub fn new(replay: DemoReplay) -> Self {
        Self::paced(replay, true)
    }

    fn paced(source: DemoReplay, exit_when_finished: bool) -> Self {
        Self {
            carry_ms: TICK_MS,
            source,
            exit_when_finished,
            shutdown_requested: false,
            over: false,
        }
    }

    pub fn with_quit_on_end(self, exit_when_finished: bool) -> Self {
        Self { exit_when_finished, ..self }
    }

    /// How many ticks have been handed to the client so far.
    pub fn ticks_played(&self) -> u64 {
        self.source.ticks_played()
    }

    pub fn accumulated_millis(&self) -> f32 {
        self.carry_ms
    }

    pub fn path(&self) -> &std::path::Path {
        self.source.path()
    }

    /// Lets `delta_ms` of wall time pass and returns the ticks it released,
    /// one per 50 ms. Reaching the end of the recording, or a frame that does
    /// not read, marks it `ended` and releases nothing more.
    fn pull(&mut self, delta_ms: f32) -> Vec<DecodedTick> {
        self.carry_ms += delta_ms;
        std::iter::from_fn(|| self.release_one()).collect()
    }

    /// Spends one tick of carried time on the next frame. `None` when there is
    /// not a whole tick carried, or the recording just ran out.
    fn release_one(&mut self) -> Option<DecodedTick> {
        if self.carry_ms < TICK_MS {
            return None;
        }
        self.carry_ms -= TICK_MS;
        let Some(read) = self.source.next_tick().transpose() else {
            self.over = true;
            let (total, from) = (self.source.ticks_played(), self.source.path().display());
            diag::info!(WireLink, "play: ended after {total} ticks ({from})");
            return None;
        };
        read.inspect_err(|unreadable| diag::warn!(WireLink, "play: {unreadable}"))
            .map_err(|_| self.over = true)
            .ok()
    }
}

pub struct RecordingPlugin;

impl Plugin for RecordingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClipLoopBuffer>();
        app.init_resource::<StagedRecordingStart>();
        app.init_resource::<RecordingStatusInfo>();
        app.init_resource::<PlaybackSession>();

        // A server, or theater mode standing in for one, records the ticks it runs.
        let role = app.world().get_resource::<ProcessDuty>();
        let serves = role.is_some_and(|role| *role == ProcessDuty::Replay || role.hosts_simulation());
        if serves {
            app.add_systems(
                FixedUpdate,
                log_host_step.run_if(host_due_for_step).in_set(net::ServerStage::Fanout),
            );
        }

        // Theater first (it may start playback), then the pump, after any session swap.
        let theater_then_pump = (
            start_or_drop_theater,
            advance_playback.run_if(resource_exists::<RecordingPlayer>),
        )
            .chain()
            .in_set(net::ClientStep::Load)
            .after(MatchSwapDone);
        let keep_received = log_arrived_steps.before(ClientStep::Reconcile).after(ClientStep::Receive);
        app.add_systems(bevy::app::Update, (keep_received, theater_then_pump));
    }
}

/// Tells perf whether theater mode is playing: `Some((quits_at_end, hosts_simulation))`
/// while it is, `None` once it stopped.
fn report_theater(playing: Option<(bool, bool)>) {
    match playing {
        None => perf::theater(0, None, None),
        Some((quits, authority)) => {
            perf::theater(1, Some(i64::from(quits)), Some(i64::from(authority)))
        }
    }
}

/// Starts a waiting recording when the role is theater mode, and drops
/// playback and the clip ring when the match ends.
fn start_or_drop_theater(
    mut commands: Commands,
    mut torn_down: MessageReader<GameSessionClosed>,
    mut returned: MessageReader<ReturnedToMenu>,
    mut waiting: ResMut<StagedRecordingStart>,
    mut clips: ResMut<ClipLoopBuffer>,
    role: Res<ProcessDuty>,
) {
    // Both readers are drained every time, so neither replays old messages.
    let match_over = [torn_down.read().count(), returned.read().count()];
    if match_over.iter().any(|&seen| seen != 0) {
        clips.clear();
        commands.remove_resource::<RecordingPlayer>();
        report_theater(None);
    }

    let theater = *role == ProcessDuty::Replay;
    let Some(source) = theater.then(|| waiting.playback.take()).flatten() else {
        return;
    };
    let quits = waiting.exit_when_finished;
    commands.insert_resource(RecordingPlayer::paced(source, quits));
    report_theater(Some((quits, role.hosts_simulation())));
}

/// Keeps what a client was sent. A server keeps its own ticks instead (see
/// [`log_host_step`]).
fn log_arrived_steps(
    inbox: Res<ArrivedStepQueue>,
    role: Res<ProcessDuty>,
    mut clips: ResMut<ClipLoopBuffer>,
) {
    let sent = inbox.0.iter().map(|arrived| {
        let got = &arrived.frame;
        let input = StepInputs { cmds: got.inputs.clone(), actions: got.actions.clone() };
        (input, arrived.snapshot.clone())
    });
    if !role.hosts_simulation() {
        sent.for_each(|(input, snap)| clips.record_tick(input, snap));
    }
}

/// Keeps the tick the server just ran: in the clip ring, and in the open
/// recording. A recording that cannot be written is closed and the player is
/// told, instead of failing every tick after.
fn log_host_step(
    host_step_from_raw: Res<HostStepNumber>,
    role: Res<ProcessDuty>,
    mut clips: ResMut<ClipLoopBuffer>,
    mut open: ResMut<PlaybackSession>,
    mut diagnostics: ResMut<RecordingStatusInfo>,
) {
    let Some(ran) = &host_step_from_raw.0 else {
        return;
    };
    // Theater mode records what it plays but does not clip it.
    let clipped = role.hosts_simulation().then(|| (ran.input.clone(), ran.snapshot.clone()));
    clipped.into_iter().for_each(|(input, snap)| clips.record_tick(input, snap));
    let failure = match open.0.as_mut().map(|rec| rec.append_tick(&ran.input, &ran.snapshot)) {
        Some(Err(failure)) => failure,
        Some(Ok(_)) | None => return,
    };
    let told = format!("record: write failed, recording stopped: {failure}");
    diag::info!(WireLink, "{told}");
    open.0 = None;
    diagnostics.line = Some(told);
}

/// The clocks playback drives, and what holds it back.
#[derive(SystemParam)]
struct PlaybackClocks<'w> {
    wall: Res<'w, Time>,
    hold: Option<Res<'w, HostLoadingPause>>,
    server: Option<ResMut<'w, HostTimeKeeper>>,
    client: ResMut<'w, PlayerTimeStepper>,
}

impl PlaybackClocks<'_> {
    fn loading(&self) -> bool {
        self.hold.as_ref().is_some_and(|hold| hold.0)
    }

    /// Puts the server clock (when there is one) on the tick about to be shown.
    fn show(&mut self, tick: sim::Tick) {
        if let Some(server) = self.server.as_deref_mut() {
            server.tick = tick.0;
            server.clock_stamp_ms = HostInstant::from_step_number(tick).as_millis();
        }
        // Names the demo tick about to be shown: the same clip played at two
        // frame rates shows different frame numbers, but the same ticks.
        perf::frames::set_replay_tick(u64::from(tick.0));
    }
}

/// Feeds the client the recording's frames at 20 per second of wall time.
fn advance_playback(
    mut demo: ResMut<RecordingPlayer>,
    mut inbox: ResMut<ArrivedStepQueue>,
    mut clocks: PlaybackClocks,
) {
    if demo.over || clocks.loading() {
        return;
    }
    let released = demo.pull(clocks.wall.delta_secs() * 1000.0);
    if let Some(last) = released.last() {
        clocks.show(last.state.tick);
    }
    let handed = released
        .into_iter()
        .map(|DecodedTick { state, source_frame, .. }| ArrivedHostStep { snapshot: state, frame: source_frame });
    inbox.0.extend(handed);
    clocks.client.carryover_millis = demo.carry_ms;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use crate::testing::{frames, input, snapshot};
    use bevy::ecs::system::RunSystemOnce;
    use net::HostStepOutput;
    use std::sync::Arc;
    use std::time::Duration;

    fn playback_of(dir: &TestDir, ticks: u32) -> DemoReplay {
        let mut demo = DemoRecorder::start(dir.path(), "plugin").unwrap();
        for tick in 1..=ticks {
            demo.append_tick(&input(tick), &snapshot(tick)).unwrap();
        }
        let (_, path) = demo.stop().unwrap();
        DemoReplay::open_file(&path).unwrap()
    }

    fn world_with(role: ProcessDuty) -> World {
        let mut world = World::new();
        world.insert_resource(role);
        world.init_resource::<ClipLoopBuffer>();
        world.init_resource::<StagedRecordingStart>();
        world.init_resource::<Messages<GameSessionClosed>>();
        world.init_resource::<Messages<ReturnedToMenu>>();
        world
    }

    fn playing_world(dir: &TestDir, ticks: u32) -> World {
        let mut world = World::new();
        world.insert_resource(RecordingPlayer::new(playback_of(dir, ticks)));
        world.init_resource::<ArrivedStepQueue>();
        world.init_resource::<PlayerTimeStepper>();
        world.init_resource::<HostTimeKeeper>();
        world.init_resource::<Time>();
        world
    }

    fn received_ticks(world: &World) -> Vec<u32> {
        let received = world.resource::<ArrivedStepQueue>();
        received.0.iter().map(|tick| tick.snapshot.tick.0).collect()
    }

    fn host_step_from_raw(tick: u32) -> HostStepNumber {
        HostStepNumber(Some(HostStepOutput {
            input: input(tick),
            snapshot: snapshot(tick),
            weapon_ids_by_script: Arc::from(Vec::<String>::new()),
            pending_last_kill: Option::None,
            script_seats: Vec::new(),
            script_exit_level: false,
        }))
    }

    fn server_world(role: ProcessDuty, dir: &TestDir, recording: bool) -> World {
        let mut world = world_with(role);
        world.insert_resource(host_step_from_raw(1));
        world.init_resource::<RecordingStatusInfo>();
        let open = recording.then(|| DemoRecorder::start(dir.path(), "server").unwrap());
        world.insert_resource(PlaybackSession(open));
        world
    }

    fn ring_len(world: &World) -> usize {
        world.resource::<ClipLoopBuffer>().len()
    }

    fn recorded_ticks(world: &World) -> Option<u64> {
        world
            .resource::<PlaybackSession>()
            .0
            .as_ref()
            .map(DemoRecorder::ticks_written)
    }

    #[test]
    fn playback_starts_with_a_whole_tick_in_hand_and_quits_at_the_end() {
        let dir = TestDir::new("new");
        let playing = RecordingPlayer::new(playback_of(&dir, 1));
        assert_eq!(playing.accumulated_millis(), 50.0);
        assert!(playing.exit_when_finished);
        assert!(!playing.over);
        assert!(!playing.shutdown_requested);
        assert_eq!(playing.ticks_played(), 0);
        assert!(!playing.with_quit_on_end(false).exit_when_finished);
    }

    #[test]
    fn the_first_tick_comes_out_at_once() {
        let dir = TestDir::new("first");
        let mut playing = RecordingPlayer::new(playback_of(&dir, 3));
        let released = playing.pull(0.0);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].state.tick.0, 1);
        assert_eq!(playing.accumulated_millis(), 0.0);
    }

    #[test]
    fn a_tick_comes_out_every_50_ms() {
        let dir = TestDir::new("pace");
        let mut playing = RecordingPlayer::new(playback_of(&dir, 20));
        playing.pull(0.0);
        assert!(playing.pull(49.0).is_empty());
        assert_eq!(playing.pull(1.0).len(), 1);
        assert_eq!(playing.pull(120.0).len(), 2);
        assert_eq!(playing.accumulated_millis(), 20.0);
        assert_eq!(playing.ticks_played(), 4);
    }

    #[test]
    fn a_long_frame_releases_every_tick_it_covers_in_order() {
        let dir = TestDir::new("burst");
        let mut playing = RecordingPlayer::new(playback_of(&dir, 10));
        playing.pull(0.0);
        let ticks: Vec<u32> = playing
            .pull(200.0)
            .iter()
            .map(|tick| tick.state.tick.0)
            .collect();
        assert_eq!(ticks, [2, 3, 4, 5]);
    }

    #[test]
    fn the_end_of_the_recording_ends_playback_and_it_stays_ended() {
        let dir = TestDir::new("end");
        let mut playing = RecordingPlayer::new(playback_of(&dir, 2));
        let released = playing.pull(500.0);
        assert_eq!(released.len(), 2);
        assert!(playing.over);
        assert_eq!(playing.ticks_played(), 2);
    }

    #[test]
    fn a_frame_that_will_not_read_ends_playback_without_a_tick() {
        let dir = TestDir::new("bad");
        let mut demo = DemoRecorder::start(dir.path(), "bad").unwrap();
        demo.append_tick(&input(1), &snapshot(1)).unwrap();
        let (_, path) = demo.stop().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0xFF; 4]);
        std::fs::write(&path, bytes).unwrap();

        let mut playing = RecordingPlayer::new(DemoReplay::open_file(&path).unwrap());
        assert_eq!(playing.pull(500.0).len(), 1);
        assert!(playing.over);
    }

    /// An app with the plugin added, for `role` or before any role is known.
    fn plugin_world(role: Option<ProcessDuty>) -> World {
        let mut app = App::default();
        role.map(|known| app.insert_resource(known));
        app.add_plugins(RecordingPlugin);
        std::mem::take(app.world_mut())
    }

    #[test]
    fn the_plugin_adds_what_recording_needs_to_any_role() {
        let roles = [ProcessDuty::Listen, ProcessDuty::Client, ProcessDuty::Replay];
        for world in roles.map(|role| plugin_world(Some(role))) {
            assert!(world.contains_resource::<PlaybackSession>());
            assert!(world.contains_resource::<RecordingStatusInfo>());
            assert!(world.contains_resource::<StagedRecordingStart>());
            assert!(world.contains_resource::<ClipLoopBuffer>());
        }
    }

    #[test]
    fn the_plugin_builds_before_the_role_is_known() {
        assert!(plugin_world(None).contains_resource::<ClipLoopBuffer>());
    }

    #[test]
    fn theater_mode_takes_up_the_recording_waiting_for_it() {
        let dir = TestDir::new("arm");
        let mut world = world_with(ProcessDuty::Replay);
        world.insert_resource(StagedRecordingStart {
            playback: Some(playback_of(&dir, 3)),
            exit_when_finished: false,
        });

        world.run_system_once(start_or_drop_theater).unwrap();

        let playing = world.resource::<RecordingPlayer>();
        assert!(!playing.exit_when_finished);
        assert!(!playing.over);
        assert!(world.resource::<StagedRecordingStart>().playback.is_none());
    }

    #[test]
    fn other_roles_leave_a_waiting_recording_alone() {
        let dir = TestDir::new("keep");
        for role in [
            ProcessDuty::Listen,
            ProcessDuty::Dedicated,
            ProcessDuty::Client,
        ] {
            let mut world = world_with(role);
            let mut waiting = StagedRecordingStart::default();
            waiting.playback = Some(playback_of(&dir, 1));
            world.insert_resource(waiting);
            world.run_system_once(start_or_drop_theater).unwrap();
            assert!(world.get_resource::<RecordingPlayer>().is_none(), "{role:?}");
            assert!(
                world.resource::<StagedRecordingStart>().playback.is_some(),
                "{role:?}"
            );
        }
    }

    #[test]
    fn with_nothing_waiting_theater_mode_starts_nothing() {
        let mut world = world_with(ProcessDuty::Replay);
        world.run_system_once(start_or_drop_theater).unwrap();
        assert!(world.get_resource::<RecordingPlayer>().is_none());
    }

    #[test]
    fn going_back_to_the_menu_drops_playback_and_the_clip_ring() {
        let dir = TestDir::new("menu");
        let mut world = world_with(ProcessDuty::Replay);
        world.insert_resource(RecordingPlayer::new(playback_of(&dir, 2)));
        world.resource_mut::<ClipLoopBuffer>().record_tick(input(1), snapshot(1));
        world
            .resource_mut::<Messages<ReturnedToMenu>>()
            .write(ReturnedToMenu {
                swap_id: 1,
                had_world: true,
            });

        world.run_system_once(start_or_drop_theater).unwrap();

        assert!(world.get_resource::<RecordingPlayer>().is_none());
        assert!(world.resource::<ClipLoopBuffer>().is_empty());
    }

    #[test]
    fn without_a_match_ending_playback_and_the_ring_stay() {
        let dir = TestDir::new("stay");
        let mut world = world_with(ProcessDuty::Replay);
        world.insert_resource(RecordingPlayer::new(playback_of(&dir, 2)));
        world.resource_mut::<ClipLoopBuffer>().record_tick(input(1), snapshot(1));

        world.run_system_once(start_or_drop_theater).unwrap();

        assert!(world.get_resource::<RecordingPlayer>().is_some());
        assert_eq!(ring_len(&world), 1);
    }

    #[test]
    fn a_client_keeps_the_ticks_it_receives_once_each() {
        let mut world = world_with(ProcessDuty::Client);
        let mut inbox = ArrivedStepQueue::default();
        for (index, frame) in frames(3).into_iter().enumerate() {
            let snap = snapshot(index as u32 + 1);
            inbox.0.push_back(ArrivedHostStep { snapshot: snap, frame });
        }
        world.insert_resource(inbox);

        world.run_system_once(log_arrived_steps).unwrap();
        world.run_system_once(log_arrived_steps).unwrap();

        let ring = world.resource::<ClipLoopBuffer>();
        let held: Vec<u32> = ring.oldest_first().map(|(_, snapshot)| snapshot.tick.0).collect();
        assert_eq!(held, [1, 2, 3]);
        let (first_input, _) = ring.oldest_first().next().unwrap();
        assert_eq!(first_input, &input(1));
    }

    #[test]
    fn a_server_does_not_keep_received_ticks_it_keeps_its_own() {
        for role in [ProcessDuty::Listen, ProcessDuty::Dedicated] {
            let mut world = world_with(role);
            let mut inbox = ArrivedStepQueue::default();
            let only = ArrivedHostStep {
                snapshot: snapshot(1),
                frame: frames(1).remove(0),
            };
            inbox.0.push_back(only);
            world.insert_resource(inbox);
            world.run_system_once(log_arrived_steps).unwrap();
            assert_eq!(ring_len(&world), 0, "{role:?}");
        }
    }

    #[test]
    fn a_server_keeps_the_tick_it_ran_in_the_ring_and_in_the_recording() {
        let dir = TestDir::new("server");
        let mut world = server_world(ProcessDuty::Listen, &dir, true);

        world.run_system_once(log_host_step).unwrap();

        assert_eq!(ring_len(&world), 1);
        assert_eq!(recorded_ticks(&world), Some(1));
        assert!(world.resource::<RecordingStatusInfo>().line.is_none());
    }

    #[test]
    fn with_no_recording_open_the_server_still_fills_the_ring() {
        let dir = TestDir::new("noreel");
        let mut world = server_world(ProcessDuty::Dedicated, &dir, false);
        world.run_system_once(log_host_step).unwrap();
        assert_eq!(ring_len(&world), 1);
        assert_eq!(recorded_ticks(&world), None);
    }

    #[test]
    fn theater_mode_records_but_does_not_clip_what_it_is_only_playing() {
        let dir = TestDir::new("theater");
        let mut world = server_world(ProcessDuty::Replay, &dir, true);
        world.run_system_once(log_host_step).unwrap();
        assert_eq!(ring_len(&world), 0);
        assert_eq!(recorded_ticks(&world), Some(1));
    }

    #[test]
    fn a_server_that_has_not_run_a_tick_records_nothing() {
        let dir = TestDir::new("idle");
        let mut world = server_world(ProcessDuty::Listen, &dir, true);
        world.insert_resource(HostStepNumber(None));
        world.run_system_once(log_host_step).unwrap();
        assert_eq!(ring_len(&world), 0);
        assert_eq!(recorded_ticks(&world), Some(0));
    }

    #[test]
    fn the_pump_hands_the_client_the_first_tick_and_sets_the_clocks() {
        let dir = TestDir::new("pump");
        let mut world = playing_world(&dir, 3);

        world.run_system_once(advance_playback).unwrap();

        assert_eq!(received_ticks(&world), [1]);
        let clock = world.resource::<HostTimeKeeper>();
        assert_eq!(clock.tick, 1);
        assert_eq!(clock.clock_stamp_ms, 50);
        assert_eq!(world.resource::<PlayerTimeStepper>().carryover_millis, 0.0);
    }

    #[test]
    fn the_pump_follows_the_frame_time() {
        let dir = TestDir::new("time");
        let mut world = playing_world(&dir, 10);
        world.run_system_once(advance_playback).unwrap();

        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(120));
        world.run_system_once(advance_playback).unwrap();

        assert_eq!(received_ticks(&world), [1, 2, 3]);
        assert_eq!(world.resource::<HostTimeKeeper>().tick, 3);
        assert_eq!(world.resource::<HostTimeKeeper>().clock_stamp_ms, 150);
        assert_eq!(world.resource::<PlayerTimeStepper>().carryover_millis, 20.0);
    }

    #[test]
    fn the_pump_works_with_no_authority_clock() {
        let dir = TestDir::new("noclock");
        let mut world = playing_world(&dir, 2);
        world.remove_resource::<HostTimeKeeper>();
        world.run_system_once(advance_playback).unwrap();
        assert_eq!(received_ticks(&world), [1]);
    }

    #[test]
    fn the_pump_waits_while_the_match_is_loading() {
        let dir = TestDir::new("hold");
        let mut world = playing_world(&dir, 3);
        world.insert_resource(HostLoadingPause(true));
        world.run_system_once(advance_playback).unwrap();
        assert!(received_ticks(&world).is_empty());
        assert_eq!(world.resource::<RecordingPlayer>().accumulated_millis(), 50.0);

        world.insert_resource(HostLoadingPause(false));
        world.run_system_once(advance_playback).unwrap();
        assert_eq!(received_ticks(&world), [1]);
    }

    #[test]
    fn the_pump_stops_for_good_when_the_recording_ends() {
        let dir = TestDir::new("pumpend");
        let mut world = playing_world(&dir, 2);
        world.run_system_once(advance_playback).unwrap();
        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(500));
        world.run_system_once(advance_playback).unwrap();

        assert_eq!(received_ticks(&world), [1, 2]);
        assert!(world.resource::<RecordingPlayer>().over);

        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(500));
        world.run_system_once(advance_playback).unwrap();
        assert_eq!(received_ticks(&world), [1, 2]);
    }

    #[test]
    fn what_the_pump_hands_over_is_what_was_recorded() {
        let dir = TestDir::new("exact");
        let mut world = playing_world(&dir, 2);
        world.run_system_once(advance_playback).unwrap();
        let received = world.resource::<ArrivedStepQueue>();
        let tick = &received.0[0];
        assert_eq!(tick.frame.inputs, input(1).cmds);
        assert_eq!(tick.snapshot.players.len(), 1);
    }
}
