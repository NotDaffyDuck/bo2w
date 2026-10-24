//! The line of commands waiting to run, and the holds that pace it.
//!
//! Commands run one per frame, oldest first. Some commands start work that
//! takes a while (`map`, `spawn`), and a script must not run its next line
//! until that work is done, so the queue can be held: on a [`Hold`] that the
//! game ends by reaching some state, on a plain sleep, or on `paused`, which
//! whoever started the work clears with [`ConsoleDispatch::release`]. A hold
//! that is not met in [`HOLD_TIMEOUT_SECS`] aborts the script.

use std::collections::{HashSet, VecDeque};

use bevy::{ecs::system::SystemParam, prelude::*};
use frame::{AppScreen, HasWorld};
use net::PresentedSnapshot;
use render_frontend::prepare::scene::world::WorldScene;

use super::state::{ConsoleSettings, ConsoleState};
use crate::{ConsoleCommand, ConsoleQueue, registry::ConsoleRegistry};

/// How long a hold may wait for the game before the script is given up on.
pub(super) const HOLD_TIMEOUT_SECS: f32 = 120.0;

/// Commands that start work the next script line has to wait for.
const SYNC_COMMANDS: [&str; 6] = ["map", "map_restart", "disconnect", "demo", "play", "spawn"];

/// Commands a script may still run after it has been aborted.
const EXIT_COMMANDS: [&str; 3] = ["quit", "exit", "finish_run"];

#[derive(Resource, Default)]
pub struct ConsoleCommandQueue(pub VecDeque<ConsoleCommand>);

impl ConsoleCommandQueue {
    pub fn push_script(&mut self, script: &str) {
        self.0.extend(ConsoleCommand::parse_script(script));
    }
}

/// The pose a `Hold::Move` waits for the presented snapshot to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaitMovePose {
    pub client: sim::ClientId,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// What the queue is waiting for the game to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hold {
    /// The world is built.
    World,
    /// The class menu lets the player spawn. Only [`ConsoleDispatch::end_hold`]
    /// ends this one.
    SpawnAdmit,
    /// The player is in the match.
    Spawn,
    /// The map's ambient sound is playing.
    Ambient,
    /// No world is left.
    Torn,
    /// The presented snapshot shows the pose.
    Move(WaitMovePose),
    /// The match has begun.
    Playing,
    /// The client is alive in a life newer than the one given.
    Alive(sim::ClientId, sim::LifeSequence),
    /// The authority clock reaches the tick.
    Tick(u32),
}

impl Hold {
    /// What the log calls the wait.
    fn kind(self) -> &'static str {
        match self {
            Self::World => "wait world",
            Self::SpawnAdmit => "spawn",
            Self::Spawn => "wait spawn",
            Self::Ambient => "wait ambient",
            Self::Torn => "wait torn",
            Self::Move(_) => "wait move",
            Self::Playing => "wait playing",
            Self::Alive(..) => "force_spawn",
            Self::Tick(_) => "wait ticks",
        }
    }

    /// What was still missing when the wait ran out.
    fn missing(self) -> &'static str {
        match self {
            Self::World => "spawned=false",
            Self::SpawnAdmit => "class select never allowed",
            Self::Spawn => "screen not InGame",
            Self::Ambient => "MapAmbientBooted=0",
            Self::Torn => "HasWorld or scene.spawned still set",
            Self::Move(_) => "presented pose still not the move target",
            Self::Playing => "match phase still not Playing",
            Self::Alive(..) => "no newer life became Alive",
            Self::Tick(_) => "authority clock did not reach the tick",
        }
    }

    fn done_line(self, secs: f32) -> String {
        let reached = match self {
            Self::World => "spawned".to_owned(),
            Self::SpawnAdmit => "admitted".to_owned(),
            Self::Spawn => "InGame".to_owned(),
            Self::Ambient => "MapAmbientBooted".to_owned(),
            Self::Torn => "hold".to_owned(),
            Self::Move(_) => "presented pose".to_owned(),
            Self::Playing => "Playing".to_owned(),
            Self::Alive(..) => "Alive".to_owned(),
            Self::Tick(until) => format!("tick {until}"),
        };
        format!("{}: {reached} after {secs:.1}s", self.kind())
    }
}

/// How a hold stands after a frame has passed.
#[derive(Debug, PartialEq)]
enum Settled {
    /// There was no hold.
    Free,
    /// The game reached the state, this many seconds in.
    Met(Hold, f32),
    /// The game did not reach it in time.
    TimedOut(Hold),
    /// Still waiting.
    Waiting,
}

#[derive(Resource, Default)]
pub struct ConsoleDispatch {
    /// A command is in flight, and the queue stays put until it calls
    /// [`ConsoleDispatch::release`].
    pub paused: bool,
    /// The class `spawn` named, kept for when the class menu lets it through.
    pub pending_spawn_class: Option<String>,
    hold: Option<Hold>,
    held_for: f32,
    sleep: f32,
}

impl ConsoleDispatch {
    /// A queue with a command in flight, as a sync command leaves it.
    #[cfg(test)]
    pub(crate) fn in_flight() -> Self {
        Self {
            paused: true,
            ..Self::default()
        }
    }

    /// Holds the queue for `hold`, starting its clock now. A hold already
    /// there is replaced.
    pub fn hold(&mut self, hold: Hold) {
        self.hold = Some(hold);
        self.held_for = 0.0;
    }

    pub fn holding(&self) -> Option<Hold> {
        self.hold
    }

    pub fn end_hold(&mut self) {
        self.hold = None;
    }

    fn sleep_for(&mut self, secs: f32) {
        self.sleep = secs;
    }

    fn waiting(&self) -> bool {
        self.paused || self.hold.is_some() || self.sleep > 0.0
    }

    /// Lets the queue run again, whatever it was waiting for.
    pub fn release(&mut self) {
        self.paused = false;
        self.hold = None;
        self.sleep = 0.0;
        self.pending_spawn_class = None;
    }

    /// Lets `dt` seconds pass for the current hold, asking `met` whether the
    /// game has reached what it waits for.
    fn settle(&mut self, dt: f32, met: impl FnOnce(Hold) -> bool) -> Settled {
        let Some(hold) = self.hold else {
            return Settled::Free;
        };
        if met(hold) {
            self.hold = None;
            return Settled::Met(hold, self.held_for);
        }
        self.held_for += dt;
        if self.held_for < HOLD_TIMEOUT_SECS {
            return Settled::Waiting;
        }
        self.hold = None;
        if matches!(hold, Hold::SpawnAdmit) {
            self.pending_spawn_class = None;
        }
        Settled::TimedOut(hold)
    }

    /// Lets `dt` seconds of the sleep pass. True while it still runs.
    fn sleeping(&mut self, dt: f32) -> bool {
        self.sleep = (self.sleep - dt).max(0.0);
        self.sleep > 0.0
    }
}

/// What the game looks like to a command that waits on it.
#[derive(SystemParam)]
pub(super) struct GameView<'w> {
    scene: Option<Res<'w, WorldScene>>,
    screen: Option<Res<'w, AppScreen>>,
    has_world: Option<Res<'w, HasWorld>>,
    headless: Option<Res<'w, frame::Headless>>,
    ambient: Option<Res<'w, audio::MapAmbientBooted>>,
    presented: Option<Res<'w, PresentedSnapshot>>,
    authority: Option<Res<'w, net::AuthorityWorld>>,
    clock: Option<Res<'w, net::AuthorityClock>>,
    local: Option<Res<'w, net::LocalPresentClient>>,
}

impl GameView<'_> {
    fn scene_spawned(&self) -> bool {
        self.scene.as_ref().is_some_and(|scene| scene.spawned)
    }

    fn has_world(&self) -> bool {
        self.has_world.as_ref().is_some_and(|has| has.0)
    }

    /// A headless run has no scene, so it goes by `HasWorld`.
    fn world_up(&self) -> bool {
        if self.headless.is_some() {
            self.has_world()
        } else {
            self.scene_spawned()
        }
    }

    fn torn(&self) -> bool {
        !self.has_world() && !self.scene_spawned()
    }

    fn in_game(&self) -> bool {
        self.screen
            .as_ref()
            .is_some_and(|screen| matches!(**screen, AppScreen::InGame))
    }

    fn tick(&self) -> Option<u32> {
        self.clock.as_ref().map(|clock| clock.tick)
    }

    fn met(&self, hold: Hold) -> bool {
        match hold {
            Hold::World => self.world_up(),
            Hold::SpawnAdmit => false,
            Hold::Spawn => self.in_game(),
            Hold::Ambient => self.ambient.as_ref().is_some_and(|booted| booted.0),
            Hold::Torn => self.torn(),
            Hold::Move(pose) => self.presented.as_ref().is_some_and(|presented| {
                crate::player_cmds::presented_matches_move(presented, pose)
            }),
            Hold::Playing => self
                .authority
                .as_ref()
                .is_some_and(|world| world.0.phase() == sim::MatchPhase::Playing),
            Hold::Alive(client, life) => self
                .authority
                .as_ref()
                .and_then(|world| world.0.client_meta(client))
                .is_some_and(|meta| {
                    meta.lifecycle == sim::ClientLifecycle::Alive && meta.life_sequence != life
                }),
            Hold::Tick(until) => self.tick().is_some_and(|now| tick_reached(now, until)),
        }
    }

    /// The match figures a `mark` line carries, each led by a space; nothing
    /// when there is no match.
    fn mark_facts(&self) -> String {
        let Some(authority) = self.authority.as_ref() else {
            return String::new();
        };
        let board = authority.0.clients_scoreboard();
        let alive = board
            .iter()
            .filter(|(_, meta)| meta.lifecycle == sim::ClientLifecycle::Alive)
            .count();
        let kills: i32 = board.iter().map(|(_, meta)| meta.kills).sum();
        let deaths: i32 = board.iter().map(|(_, meta)| meta.deaths).sum();
        let mut facts = format!(
            " tick={} clients={} alive={alive} kills={kills} deaths={deaths}",
            self.tick().unwrap_or(0),
            board.len()
        );
        let Some(local) = self.local.as_ref() else {
            return facts;
        };
        let meta = authority.0.client_meta(local.0);
        match meta {
            Some(meta) => facts.push_str(&format!(
                " local={:?} local_life={} local_deaths={} local_cmds={} local_moving_cmds={} \
                 local_path={:.1}",
                meta.lifecycle,
                meta.life_sequence.0,
                meta.deaths,
                meta.input_receipt.applied_cmds,
                meta.input_receipt.moving_cmds,
                meta.input_receipt.path_units
            )),
            None => facts.push_str(" local=absent"),
        }
        let shown = self
            .presented
            .as_ref()
            .and_then(|presented| presented.alive_player(local.0));
        if let Some(player) = shown {
            facts.push_str(&format!(
                " origin={:.1},{:.1},{:.1} yaw={:.1}",
                player.origin[0], player.origin[1], player.origin[2], player.viewangles[1]
            ));
        }
        facts
    }
}

/// Whether the wrapping tick counter `now` has got to `until`.
fn tick_reached(now: u32, until: u32) -> bool {
    now.wrapping_sub(until) < u32::MAX / 2
}

/// Runs the oldest queued command, unless something holds the queue.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_console_queue(
    time: Res<Time>,
    registry: Res<ConsoleRegistry>,
    settings: Res<ConsoleSettings>,
    view: GameView<'_>,
    mut dispatch: ResMut<ConsoleDispatch>,
    mut queue: ResMut<ConsoleCommandQueue>,
    mut console: ResMut<ConsoleState>,
    mut submitted: MessageWriter<ConsoleCommand>,
    mut marks: Local<u64>,
) {
    let capacity = settings.log_capacity;
    if dispatch.waiting() {
        if let Some(command) = take_interactive(&mut queue, &["dump", "clip", "screenshot"]) {
            diag::info!(
                Console,
                "fifo: interactive diagnostic `{}` — hold preserved",
                command.name
            );
            submitted.write(command);
            return;
        }
        if let Some(command) = take_interactive(&mut queue, &["quit", "exit", "disconnect"]) {
            diag::info!(
                Console,
                "fifo: interactive jump `{}` — released FIFO hold",
                command.name
            );
            queue.0.push_front(command);
            dispatch.release();
        }
    }
    if dispatch.paused {
        return;
    }
    let dt = time.delta_secs();
    match dispatch.settle(dt, |hold| view.met(hold)) {
        Settled::Waiting => return,
        Settled::Met(hold, secs) => diag::info!(Console, "{}", hold.done_line(secs)),
        Settled::TimedOut(hold) => {
            let message = abort_script(hold, &mut queue.0);
            diag::warn!(Console, "{message}");
            console.echo(message, capacity);
        }
        Settled::Free => {}
    }
    if dispatch.sleeping(dt) {
        return;
    }
    let Some(command) = queue.0.pop_front() else {
        return;
    };
    if registry.resolve(&command.name).is_none() {
        let message = format!("unknown command `{}`", command.name);
        diag::warn!(Console, "{message}");
        console.echo(message, capacity);
        return;
    }
    match command.name.as_str() {
        "mark" => run_mark(&command, &mut marks, &view, &mut console, capacity),
        "wait" => run_wait(&command, &view, &mut dispatch, &mut console, capacity),
        _ => {
            begin_sync(&command, &mut dispatch);
            submitted.write(command);
        }
    }
}

/// Removes and returns the oldest queued command that was typed at the
/// console and is one of `names`.
fn take_interactive(queue: &mut ConsoleCommandQueue, names: &[&str]) -> Option<ConsoleCommand> {
    let at = queue
        .0
        .iter()
        .position(|command| command.interactive && names.contains(&command.name.as_str()))?;
    queue.0.remove(at)
}

/// Drops every queued command but the ones that end the run. Returns how many
/// went and how many stayed.
fn drop_all_but_exits(queue: &mut VecDeque<ConsoleCommand>) -> (usize, usize) {
    let before = queue.len();
    queue.retain(|command| EXIT_COMMANDS.contains(&command.name.as_str()));
    (before - queue.len(), queue.len())
}

/// Gives up on the script because `hold` never ended. Returns what to say.
fn abort_script(hold: Hold, queue: &mut VecDeque<ConsoleCommand>) -> String {
    let (dropped, kept) = drop_all_but_exits(queue);
    format!(
        "{}: timed out after {HOLD_TIMEOUT_SECS:.0}s ({}) — script aborted, {dropped} queued \
         commands dropped, kept exit={kept}",
        hold.kind(),
        hold.missing()
    )
}

/// Pauses the queue for a command that starts work the script has to wait
/// for, unless it was sent to the background with `&`.
fn begin_sync(command: &ConsoleCommand, dispatch: &mut ConsoleDispatch) {
    let name = command.name.as_str();
    if !SYNC_COMMANDS.contains(&name) {
        return;
    }
    if command.background {
        diag::info!(Console, "{name} &: async — FIFO not blocked");
        return;
    }
    match name {
        "disconnect" => {
            dispatch.paused = true;
            dispatch.hold(Hold::Torn);
            diag::info!(Console, "disconnect: sync until torn hold");
        }
        "spawn" => {}
        _ => {
            dispatch.paused = true;
            dispatch.hold(Hold::World);
            diag::info!(Console, "{name}: sync until scene.spawned");
        }
    }
}

/// A label `mark` accepts: letters, digits, `_`, `.` and `-`.
fn valid_mark_label(label: &str) -> bool {
    !label.is_empty()
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

/// ` name=<mebibytes>`, or nothing when the figure is not known.
fn mib_field(name: &str, bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => format!(" {name}={}", bytes >> 20),
        None => String::new(),
    }
}

/// `mark <label>`: writes one line of process and match figures, for a
/// benchmark to find in the log.
fn run_mark(
    command: &ConsoleCommand,
    marks: &mut u64,
    view: &GameView<'_>,
    console: &mut ConsoleState,
    capacity: usize,
) {
    let label = match command.args.as_slice() {
        [label] if valid_mark_label(label) => label,
        _ => {
            console.echo("usage: mark <label> (letters, digits, _, ., -)", capacity);
            return;
        }
    };
    *marks += 1;
    let line = format!(
        "benchmark-mark: pid={} seq={} ns={} label={label}{}{}{}",
        std::process::id(),
        *marks,
        diag::process_elapsed_ns(),
        mib_field("rss_mib", asset_transport::process_resident_bytes()),
        mib_field("heap_mib", diag::process_live_heap_bytes()),
        view.mark_facts()
    );
    diag::info!(Console, "{line}");
    println!("{line}");
    console.echo(line, capacity);
}

/// The longest `wait <seconds>` runs.
const MAX_WAIT_SECS: f32 = 120.0;

/// What a `wait` command asks for.
#[derive(Debug)]
enum Wait {
    Seconds(f32),
    Ticks(u32),
    Until(Hold),
}

/// `wait world|spawn|torn|ambient`, `wait <n>t` or `wait <seconds>[s]`.
fn parse_wait(args: &[String]) -> Wait {
    let Some(first) = args.first().map(|arg| arg.to_ascii_lowercase()) else {
        return Wait::Seconds(1.0);
    };
    match first.as_str() {
        "world" => return Wait::Until(Hold::World),
        "spawn" => return Wait::Until(Hold::Spawn),
        "torn" => return Wait::Until(Hold::Torn),
        "ambient" => return Wait::Until(Hold::Ambient),
        _ => {}
    }
    if let Some(ticks) = first
        .strip_suffix('t')
        .and_then(|digits| digits.parse::<u32>().ok())
    {
        return Wait::Ticks(ticks);
    }
    let secs = first
        .trim_end_matches('s')
        .parse::<f32>()
        .ok()
        .filter(|secs| secs.is_finite())
        .unwrap_or(1.0);
    Wait::Seconds(secs.clamp(0.0, MAX_WAIT_SECS))
}

/// What the log says when `wait <what>` finds the game already there, and
/// when it has to hold.
fn wait_text(hold: Hold) -> (&'static str, &'static str) {
    match hold {
        Hold::World => ("already spawned", "until scene.spawned"),
        Hold::Spawn => ("already InGame", "until AppScreen::InGame"),
        Hold::Torn => ("already hold", "until HasWorld=0 and scene.spawned=0"),
        Hold::Ambient => ("already MapAmbientBooted", "until MapAmbientBooted"),
        _ => ("already reached", "until reached"),
    }
}

fn run_wait(
    command: &ConsoleCommand,
    view: &GameView<'_>,
    dispatch: &mut ConsoleDispatch,
    console: &mut ConsoleState,
    capacity: usize,
) {
    match parse_wait(&command.args) {
        Wait::Seconds(secs) => {
            dispatch.sleep_for(secs);
            diag::info!(Console, "wait: {secs:.1}s");
        }
        Wait::Ticks(ticks) => match view.tick() {
            Some(now) => {
                dispatch.hold(Hold::Tick(now.wrapping_add(ticks)));
                diag::info!(Console, "wait: {ticks} ticks from tick {now}");
            }
            None => {
                let message = format!("wait: {ticks}t needs an authority clock; not holding");
                diag::warn!(Console, "{message}");
                console.echo(message, capacity);
            }
        },
        Wait::Until(hold) => {
            let (already, until) = wait_text(hold);
            if view.met(hold) {
                diag::info!(Console, "{}: {already}", hold.kind());
            } else {
                dispatch.hold(hold);
                diag::info!(Console, "{}: {until}", hold.kind());
            }
        }
    }
}

/// Moves the commands the process was started with into the queue.
pub(super) fn drain_startup_queue(
    mut startup: ResMut<ConsoleQueue>,
    mut queue: ResMut<ConsoleCommandQueue>,
) {
    queue.0.extend(startup.drain());
}

/// Commands the menus run: kept apart from the queue so that they never wait
/// behind a script.
#[derive(Default)]
pub(super) struct MenuScript {
    pending: VecDeque<ConsoleCommand>,
    delay: f32,
    /// Command names already reported as unsupported.
    reported: HashSet<String>,
}

/// How long a menu script's `wait` pauses it.
fn menu_delay(args: &[String]) -> f32 {
    args.first()
        .and_then(|secs| secs.trim_end_matches('s').parse().ok())
        .unwrap_or(0.0)
}

/// Runs what menus ask for with `exec`. Each command goes out as if typed in
/// the background, and the ones the console does not know are skipped.
pub(super) fn run_menu_commands(
    time: Res<Time>,
    registry: Res<ConsoleRegistry>,
    mut incoming: MessageReader<frame::UiExecCommand>,
    mut script: Local<MenuScript>,
    mut submitted: MessageWriter<ConsoleCommand>,
) {
    for exec in incoming.read() {
        script
            .pending
            .extend(ConsoleCommand::parse_script(&exec.text));
    }
    if script.delay > 0.0 {
        script.delay = (script.delay - time.delta_secs()).max(0.0);
        return;
    }
    while let Some(mut command) = script.pending.pop_front() {
        if command.name.eq_ignore_ascii_case("wait") {
            script.delay = menu_delay(&command.args);
            break;
        }
        if registry.resolve(&command.name).is_none() {
            if script.reported.insert(command.name.clone()) {
                diag::warn!(Ui, "menu: unsupported engine command `{}`", command.name);
            }
            continue;
        }
        command.interactive = true;
        command.background = true;
        submitted.write(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(line: &str, interactive: bool) -> ConsoleCommand {
        let mut command = ConsoleCommand::parse(line).expect("a command");
        command.interactive = interactive;
        command
    }

    fn queue_of(lines: &[&str]) -> VecDeque<ConsoleCommand> {
        lines.iter().map(|line| command(line, false)).collect()
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn wait_defaults_to_one_second() {
        assert!(matches!(parse_wait(&[]), Wait::Seconds(s) if s == 1.0));
        assert!(matches!(parse_wait(&args(&["soon"])), Wait::Seconds(s) if s == 1.0));
        assert!(matches!(parse_wait(&args(&["nan"])), Wait::Seconds(s) if s == 1.0));
    }

    #[test]
    fn wait_reads_seconds_with_or_without_the_unit() {
        assert!(matches!(parse_wait(&args(&["2.5s"])), Wait::Seconds(s) if s == 2.5));
        assert!(matches!(parse_wait(&args(&["3S"])), Wait::Seconds(s) if s == 3.0));
        assert!(matches!(parse_wait(&args(&["500"])), Wait::Seconds(s) if s == MAX_WAIT_SECS));
        assert!(matches!(parse_wait(&args(&["-4"])), Wait::Seconds(s) if s == 0.0));
    }

    #[test]
    fn wait_reads_ticks_and_events() {
        assert!(matches!(parse_wait(&args(&["10t"])), Wait::Ticks(10)));
        assert!(matches!(parse_wait(&args(&["5T"])), Wait::Ticks(5)));
        assert!(matches!(
            parse_wait(&args(&["WORLD"])),
            Wait::Until(Hold::World)
        ));
        assert!(matches!(
            parse_wait(&args(&["spawn"])),
            Wait::Until(Hold::Spawn)
        ));
        assert!(matches!(
            parse_wait(&args(&["torn"])),
            Wait::Until(Hold::Torn)
        ));
        assert!(matches!(
            parse_wait(&args(&["ambient"])),
            Wait::Until(Hold::Ambient)
        ));
    }

    #[test]
    fn menu_waits_default_to_no_pause() {
        assert_eq!(menu_delay(&[]), 0.0);
        assert_eq!(menu_delay(&args(&["0.5s"])), 0.5);
        assert_eq!(menu_delay(&args(&["later"])), 0.0);
    }

    #[test]
    fn mark_labels_are_plain_words() {
        assert!(valid_mark_label("round-2_start.v1"));
        assert!(!valid_mark_label(""));
        assert!(!valid_mark_label("two words"));
        assert!(!valid_mark_label("café"));
        assert!(!valid_mark_label("a/b"));
    }

    #[test]
    fn mark_sizes_are_in_mebibytes() {
        assert_eq!(mib_field("rss_mib", Some(5 << 20)), " rss_mib=5");
        assert_eq!(mib_field("heap_mib", None), "");
    }

    #[test]
    fn ticks_are_compared_around_the_wrap() {
        assert!(tick_reached(10, 10));
        assert!(tick_reached(11, 10));
        assert!(!tick_reached(9, 10));
        assert!(tick_reached(3, u32::MAX - 2));
        assert!(!tick_reached(u32::MAX - 5, 3));
    }

    #[test]
    fn only_typed_commands_of_the_asked_names_jump_the_queue() {
        let mut queue = ConsoleCommandQueue::default();
        queue.0.push_back(command("quit", false));
        queue.0.push_back(command("map mp_nuketown_2020", true));
        queue.0.push_back(command("dump", true));
        queue.0.push_back(command("exit", true));
        assert!(take_interactive(&mut queue, &["quit"]).is_none());
        let dump = take_interactive(&mut queue, &["dump", "clip"]).expect("dump");
        assert_eq!(dump.name, "dump");
        let exit = take_interactive(&mut queue, &["quit", "exit", "disconnect"]).expect("exit");
        assert_eq!(exit.name, "exit");
        assert_eq!(queue.0.len(), 2);
    }

    #[test]
    fn an_aborted_script_keeps_only_its_exits() {
        let mut queue = queue_of(&["map a", "quit", "wait 2", "finish_run", "echo hi"]);
        assert_eq!(drop_all_but_exits(&mut queue), (3, 2));
        let names: Vec<_> = queue.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["quit", "finish_run"]);
    }

    #[test]
    fn the_abort_message_names_the_wait_and_what_was_missing() {
        let mut queue = queue_of(&["wait 1", "quit"]);
        let message = abort_script(Hold::Playing, &mut queue);
        assert_eq!(
            message,
            "wait playing: timed out after 120s (match phase still not Playing) — script \
             aborted, 1 queued commands dropped, kept exit=1"
        );
    }

    #[test]
    fn a_background_map_does_not_block_the_queue() {
        let mut dispatch = ConsoleDispatch::default();
        let mut map = command("map mp_nuketown_2020", false);
        map.background = true;
        begin_sync(&map, &mut dispatch);
        assert!(!dispatch.waiting());
        begin_sync(&command("map mp_nuketown_2020", false), &mut dispatch);
        assert!(dispatch.paused);
        assert!(matches!(dispatch.holding(), Some(Hold::World)));
    }

    #[test]
    fn disconnect_waits_for_the_world_to_go_and_spawn_waits_for_nothing() {
        let mut dispatch = ConsoleDispatch::default();
        begin_sync(&command("spawn", false), &mut dispatch);
        assert!(!dispatch.waiting());
        begin_sync(&command("disconnect", false), &mut dispatch);
        assert!(dispatch.paused);
        assert!(matches!(dispatch.holding(), Some(Hold::Torn)));
    }

    #[test]
    fn an_ordinary_command_does_not_hold_the_queue() {
        let mut dispatch = ConsoleDispatch::default();
        begin_sync(&command("echo hi", false), &mut dispatch);
        assert!(!dispatch.waiting());
    }

    #[test]
    fn a_new_hold_replaces_the_old_one_and_restarts_its_clock() {
        let mut dispatch = ConsoleDispatch::default();
        dispatch.hold(Hold::Spawn);
        assert_eq!(dispatch.settle(30.0, |_| false), Settled::Waiting);
        dispatch.hold(Hold::Ambient);
        assert!(matches!(dispatch.holding(), Some(Hold::Ambient)));
        assert!(matches!(
            dispatch.settle(1.0, |hold| matches!(hold, Hold::Ambient)),
            Settled::Met(Hold::Ambient, secs) if secs == 0.0
        ));
        assert!(dispatch.holding().is_none());
    }

    #[test]
    fn a_met_hold_reports_how_long_it_waited() {
        let mut dispatch = ConsoleDispatch::default();
        assert_eq!(dispatch.settle(1.0, |_| true), Settled::Free);
        dispatch.hold(Hold::Spawn);
        assert_eq!(dispatch.settle(0.5, |_| false), Settled::Waiting);
        assert_eq!(dispatch.settle(0.5, |_| false), Settled::Waiting);
        assert!(matches!(
            dispatch.settle(0.5, |_| true),
            Settled::Met(Hold::Spawn, secs) if secs == 1.0
        ));
    }

    #[test]
    fn a_hold_times_out_after_two_minutes() {
        let mut dispatch = ConsoleDispatch::default();
        dispatch.hold(Hold::Playing);
        assert_eq!(dispatch.settle(119.0, |_| false), Settled::Waiting);
        assert!(matches!(
            dispatch.settle(1.0, |_| false),
            Settled::TimedOut(Hold::Playing)
        ));
        assert!(dispatch.holding().is_none());
    }

    #[test]
    fn a_spawn_that_never_gets_through_forgets_its_class() {
        let mut dispatch = ConsoleDispatch {
            pending_spawn_class: Some("assault".to_owned()),
            ..ConsoleDispatch::default()
        };
        dispatch.hold(Hold::SpawnAdmit);
        assert!(matches!(
            dispatch.settle(HOLD_TIMEOUT_SECS, |_| false),
            Settled::TimedOut(Hold::SpawnAdmit)
        ));
        assert_eq!(dispatch.pending_spawn_class, None);
    }

    #[test]
    fn release_lets_everything_go() {
        let mut dispatch = ConsoleDispatch {
            paused: true,
            pending_spawn_class: Some("smg".to_owned()),
            ..Default::default()
        };
        dispatch.hold(Hold::World);
        dispatch.sleep_for(5.0);
        assert!(dispatch.waiting());
        dispatch.release();
        assert!(!dispatch.waiting());
        assert_eq!(dispatch.pending_spawn_class, None);
    }

    #[test]
    fn a_sleep_ends_on_the_frame_it_runs_out() {
        let mut dispatch = ConsoleDispatch::default();
        assert!(!dispatch.sleeping(0.1));
        dispatch.sleep_for(1.0);
        assert!(dispatch.waiting());
        assert!(dispatch.sleeping(0.5));
        assert!(!dispatch.sleeping(0.5));
        assert!(!dispatch.waiting());
    }

    #[test]
    fn a_finished_wait_reads_back_what_was_reached() {
        assert_eq!(
            Hold::Tick(7).done_line(2.0),
            "wait ticks: tick 7 after 2.0s"
        );
        assert_eq!(
            Hold::World.done_line(0.04),
            "wait world: spawned after 0.0s"
        );
        assert_eq!(
            Hold::Alive(sim::ClientId(0), sim::LifeSequence(1)).done_line(1.5),
            "force_spawn: Alive after 1.5s"
        );
    }

    #[test]
    fn a_script_pushed_to_the_queue_keeps_its_order() {
        let mut queue = ConsoleCommandQueue::default();
        queue.push_script("map a; wait 1; quit");
        let names: Vec<_> = queue.0.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["map", "wait", "quit"]);
    }
}
