//! Pending console commands and the pauses that pace them.
//!
//! One command runs per frame, oldest first. A few commands (`map`, `spawn`)
//! kick off slow work, and the following script line has to wait for it, so
//! the queue can be stalled three ways: by a [`Hold`] that ends once the game
//! reaches a given state, by a plain sleep, or by `command_running`, which the party
//! that began the work lifts through [`TypedOrderRouter::release`]. A hold
//! that stays unmet for [`HOLD_TIMEOUT_SECS`] aborts the script.

use std::{collections::HashSet, collections::VecDeque};

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use frame::LevelPresent;
use net::ShownWorldFrame;
use render_frontend::prepare::scene::world::LevelStage;

use super::state::PromptPanelOptions;
use super::state::PromptPanelStatus;
use crate::TypedOrder;
use crate::OrderInbox;
use crate::order_book::OrderCatalog;
use frame::ShellScreen;

/// Upper bound on one hold before the script is abandoned.
pub(super) const HOLD_TIMEOUT_SECS: f32 = 120.0;

/// Names of commands whose work the next script line must wait out.
const SYNC_COMMANDS: [&str; 6] = ["map", "map_restart", "disconnect", "demo", "play", "spawn"];

/// Names that still run when a script has been aborted.
const EXIT_COMMANDS: [&str; 3] = ["quit", "exit", "finish_run"];

#[derive(Default, Resource)]
pub struct TypedOrderBacklog(pub VecDeque<TypedOrder>);

impl TypedOrderBacklog {
    pub fn enqueue_script_text(&mut self, lines: &str) {
        let parsed = TypedOrder::parse_script(lines);
        self.0.extend(parsed);
    }
}

/// Target pose for `Hold::Move`, compared against the presented snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AwaitMotionPose {
    pub who: sim::ClientId,
    pub spot: [f32; 3],
    pub facing: [f32; 3],
}

/// The condition the queue is stalled on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hold {
    /// World geometry has been built.
    World,
    /// Class selection allows the spawn; only [`TypedOrderRouter::end_hold`]
    /// can end this variant.
    SpawnAdmit,
    /// The player has entered the match.
    Spawn,
    /// Ambient audio for the map is running.
    Ambient,
    /// Nothing of the world remains.
    Torn,
    /// The presented snapshot matches the pose.
    Move(AwaitMotionPose),
    /// The match has started.
    Playing,
    /// The client is alive in a life newer than the one supplied.
    Alive(sim::ClientId, sim::LifeSequence),
    /// The authority clock reaches the tick.
    Tick(u32),
}

impl Hold {
    /// Label this wait carries in log lines.
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

    /// Reason reported when the timeout hits.
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
        let outcome = if let Self::Tick(until) = self {
            format!("tick {until}")
        } else {
            let word = match self {
                Self::World => "spawned",
                Self::SpawnAdmit => "admitted",
                Self::Spawn => "InGame",
                Self::Ambient => "MapAmbientBooted",
                Self::Torn => "hold",
                Self::Move(_) => "presented pose",
                Self::Playing => "Playing",
                Self::Alive(..) => "Alive",
                Self::Tick(_) => unreachable!(),
            };
            word.to_owned()
        };
        format!("{}: {outcome} after {secs:.1}s", self.kind())
    }
}

/// Result of advancing the hold by one frame.
#[derive(Debug, PartialEq)]
enum Settled {
    /// No hold was active.
    Free,
    /// The condition came true after this many seconds.
    Met(Hold, f32),
    /// The deadline passed first.
    TimedOut(Hold),
    /// Neither yet.
    Waiting,
}

#[derive(Default, Resource)]
pub struct TypedOrderRouter {
    /// A started command has not finished yet. Nothing else leaves the
    /// queue until it calls [`TypedOrderRouter::unblock`].
    pub command_running: bool,
    /// Class named by `spawn`, remembered until the class menu admits it.
    pub queued_kit_choice: Option<String>,
    hold: Option<Hold>,
    held_for: f32,
    sleep: f32,
}

impl TypedOrderRouter {
    /// What a sync command leaves behind: one command still running.
    #[cfg(test)]
    pub(crate) fn in_flight() -> Self {
        let mut busy = Self::default();
        busy.command_running = true;
        busy
    }

    /// Stalls the queue on `hold` and zeroes its timer. Any earlier hold is
    /// overwritten.
    pub fn stall(&mut self, hold: Hold) {
        self.held_for = 0.0;
        self.hold = Some(hold);
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
        self.command_running || self.hold.is_some() || self.sleep > 0.0
    }

    /// Unblocks the queue whatever was blocking it. Only the hold timer
    /// survives; the next hold zeroes it anyway.
    pub fn unblock(&mut self) {
        let kept = self.held_for;
        *self = TypedOrderRouter::default();
        self.held_for = kept;
    }

    /// Advances the active hold by `dt` seconds, using `met` to ask whether
    /// the game now satisfies it.
    fn settle(&mut self, dt: f32, met: impl FnOnce(Hold) -> bool) -> Settled {
        let Some(active) = self.hold else {
            return Settled::Free;
        };
        if met(active) {
            self.hold = None;
            return Settled::Met(active, self.held_for);
        }
        self.held_for += dt;
        if self.held_for >= HOLD_TIMEOUT_SECS {
            self.hold = None;
            if matches!(active, Hold::SpawnAdmit) {
                self.queued_kit_choice = None;
            }
            return Settled::TimedOut(active);
        }
        Settled::Waiting
    }

    /// Burns `dt` seconds off the sleep; true if time is still left.
    fn sleeping(&mut self, dt: f32) -> bool {
        self.sleep = (self.sleep - dt).max(0.0);
        self.sleep > 0.0
    }
}

/// Read-only window onto the game for commands that wait on it.
#[derive(SystemParam)]
pub(super) struct GameView<'w> {
    level_stage: Option<Res<'w, LevelStage>>,
    screen: Option<Res<'w, ShellScreen>>,
    level_is_up: Option<Res<'w, LevelPresent>>,
    headless: Option<Res<'w, frame::Headless>>,
    ambient: Option<Res<'w, audio::AmbientBedStarted>>,
    shown_frame: Option<Res<'w, ShownWorldFrame>>,
    authority: Option<Res<'w, net::HostSimulationWorld>>,
    clock: Option<Res<'w, net::HostTimeKeeper>>,
    viewer: Option<Res<'w, net::HostViewClient>>,
}

impl GameView<'_> {
    fn scene_spawned(&self) -> bool {
        self.level_stage.as_ref().is_some_and(|scene| scene.spawned)
    }

    fn level_is_up(&self) -> bool {
        self.level_is_up.as_ref().is_some_and(|present| present.0)
    }

    /// A headless run has no scene to spawn; there the level flag is the
    /// only sign the world is up.
    fn world_up(&self) -> bool {
        if self.headless.is_some() {
            self.level_is_up()
        } else {
            self.scene_spawned()
        }
    }

    fn torn(&self) -> bool {
        !self.level_is_up() && !self.scene_spawned()
    }

    fn in_game(&self) -> bool {
        self.screen.as_deref() == Some(&ShellScreen::InGame)
    }

    fn tick(&self) -> Option<u32> {
        self.clock.as_ref().map(|keeper| keeper.tick)
    }

    /// True once a newer life than `previous` is alive for `client`.
    fn respawned(&self, player: sim::ClientId, previous: sim::LifeSequence) -> bool {
        let record = self.authority.as_ref().and_then(|world| world.0.client_meta(player));
        record.is_some_and(|record| {
            record.lifecycle == sim::PlayerSessionStage::Alive && record.life_sequence != previous
        })
    }

    fn met(&self, hold: Hold) -> bool {
        match hold {
            Hold::SpawnAdmit => false,
            Hold::World => self.world_up(),
            Hold::Spawn => self.in_game(),
            Hold::Torn => self.torn(),
            Hold::Ambient => self.ambient.as_ref().is_some_and(|booted| booted.0),
            Hold::Move(target) => self.shown_frame.as_ref().is_some_and(|frame| {
                crate::player_cmds::presented_matches_move(frame, target)
            }),
            Hold::Playing => self
                .authority
                .as_ref()
                .is_some_and(|world| world.0.phase() == sim::GameRoundStage::Playing),
            Hold::Alive(client, previous) => self.respawned(client, previous),
            Hold::Tick(until) => self.tick().is_some_and(|now| tick_reached(now, until)),
        }
    }

    /// Match numbers appended to a `mark` line (each preceded by a space);
    /// empty if no match exists.
    fn mark_facts(&self) -> String {
        let Some(authority) = &self.authority else {
            return String::from("");
        };
        let board = authority.0.clients_scoreboard();
        let alive = board
            .iter()
            .filter(|(_id, entry)| entry.lifecycle == sim::PlayerSessionStage::Alive)
            .count();
        let kills: i32 = board.iter().map(|(_, meta)| meta.frags).sum();
        let deaths: i32 = board.iter().map(|(_, meta)| meta.died).sum();
        let mut out = format!(
            " tick={} clients={} alive={alive} kills={kills} deaths={deaths}",
            self.tick().unwrap_or(0),
            board.len()
        );
        let Some(local) = &self.viewer else {
            return out;
        };
        match authority.0.client_meta(local.0) {
            None => out.push_str(" local=absent"),
            Some(mine) => out.push_str(&format!(
                " local={:?} local_life={} local_deaths={} local_cmds={} local_moving_cmds={} \
                 local_path={:.1}",
                mine.lifecycle,
                mine.life_sequence.0,
                mine.died,
                mine.input_receipt.applied_cmds,
                mine.input_receipt.moving_cmds,
                mine.input_receipt.path_units
            )),
        }
        let on_screen = self
            .shown_frame
            .as_ref()
            .and_then(|frame| frame.living_participant(local.0));
        if let Some(body) = on_screen {
            let [x, y, z] = body.origin;
            let yaw = body.look_angles[1];
            out += &format!(" origin={x:.1},{y:.1},{z:.1} yaw={yaw:.1}");
        }
        out
    }
}

/// Wrap-safe check that tick counter `now` has caught up with `until`.
fn tick_reached(now: u32, until: u32) -> bool {
    now.wrapping_sub(until) < u32::MAX / 2
}

/// Executes the next queued command, if nothing is holding the queue.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_console_queue(
    clock: Res<Time>,
    book: Res<OrderCatalog>,
    settings: Res<PromptPanelOptions>,
    view: GameView<'_>,
    mut router: ResMut<TypedOrderRouter>,
    mut queue: ResMut<TypedOrderBacklog>,
    mut panel: ResMut<PromptPanelStatus>,
    mut submitted: MessageWriter<TypedOrder>,
    mut marks: Local<u64>,
) {
    let backlog = settings.backlog_limit;
    let mut complain = |text: String| {
        diag::warn!(Console, "{}", text);
        panel.post_line(text, backlog);
    };

    if router.waiting() && let Some(jump) = hand_typed_jump(&mut queue) {
        match jump {
            QueueJump::Diagnostic(order) => {
                diag::info!(Console, "fifo: `{}` typed by hand runs now; the hold stays", order.name);
                submitted.write(order);
                return;
            }
            QueueJump::Leave(order) => {
                diag::info!(Console, "fifo: `{}` typed by hand jumps the queue and lifts the hold", order.name);
                queue.0.push_front(order);
                router.unblock();
            }
        }
    }
    if router.command_running {
        return;
    }

    let dt = clock.delta_secs();
    match router.settle(dt, |hold| view.met(hold)) {
        Settled::Free => {}
        Settled::Waiting => return,
        Settled::Met(hold, secs) => diag::info!(Console, "{}", hold.done_line(secs)),
        Settled::TimedOut(hold) => complain(abort_script(hold, &mut queue.0)),
    }
    if router.sleeping(dt) {
        return;
    }

    let Some(next) = queue.0.pop_front() else {
        return;
    };
    if book.look_up(&next.name).is_none() {
        complain(format!("unknown command `{}`", next.name));
        return;
    }
    if next.name == "mark" {
        run_mark(&next, &mut marks, &view, &mut panel, backlog);
    } else if next.name == "wait" {
        run_wait(&next, &view, &mut router, &mut panel, backlog);
    } else {
        begin_sync(&next, &mut router);
        submitted.write(next);
    }
}

/// A hand-typed command allowed to act while the queue is held.
enum QueueJump {
    /// Looks at the game without changing it, so it runs at once and the
    /// hold carries on.
    Diagnostic(TypedOrder),
    /// Leaves the game or the match; it goes to the front and the hold ends.
    Leave(TypedOrder),
}

fn hand_typed_jump(queue: &mut TypedOrderBacklog) -> Option<QueueJump> {
    if let Some(order) = take_interactive(queue, &["dump", "clip", "screenshot"]) {
        return Some(QueueJump::Diagnostic(order));
    }
    take_interactive(queue, &["quit", "exit", "disconnect"]).map(QueueJump::Leave)
}

/// Pulls out the oldest queued command that was typed by hand and whose
/// name is in `names`.
fn take_interactive(queue: &mut TypedOrderBacklog, names: &[&str]) -> Option<TypedOrder> {
    let idx = queue
        .0
        .iter()
        .position(|cmd| cmd.typed_by_hand && names.contains(&cmd.name.as_str()))?;
    queue.0.remove(idx)
}

/// Clears the queue except for exit commands; yields (dropped, kept).
fn drop_all_but_exits(queue: &mut VecDeque<TypedOrder>) -> (usize, usize) {
    let total = queue.len();
    queue.retain(|cmd| EXIT_COMMANDS.contains(&cmd.name.as_str()));
    let kept = queue.len();
    (total - kept, kept)
}

/// Abandons the script after `hold` timed out; returns the log message.
fn abort_script(hold: Hold, queue: &mut VecDeque<TypedOrder>) -> String {
    let (dropped, kept) = drop_all_but_exits(queue);
    format!(
        "{}: timed out after {HOLD_TIMEOUT_SECS:.0}s ({}) — script aborted, {dropped} queued \
         commands dropped, kept exit={kept}",
        hold.kind(),
        hold.missing()
    )
}

/// Stalls the queue for commands that start slow work, unless the command
/// was backgrounded with `&`.
fn begin_sync(command: &TypedOrder, router: &mut TypedOrderRouter) {
    let name = command.name.as_str();
    if !SYNC_COMMANDS.contains(&name) {
        return;
    }
    let backgrounded = command.detached;
    if backgrounded {
        diag::info!(Console, "{name} &: async — runs without blocking the FIFO");
        return;
    }
    // `spawn` sets up its own hold once the class menu has answered.
    let (hold, reached) = match name {
        "spawn" => return,
        "disconnect" => (Hold::Torn, "torn hold"),
        _ => (Hold::World, "scene.spawned"),
    };
    router.command_running = true;
    router.stall(hold);
    diag::info!(Console, "{name}: sync until {reached}");
}

/// Whether `label` is acceptable to `mark`: letters, digits, `_`, `.`, `-`.
fn valid_mark_label(label: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "_.-".contains(c);
    !label.is_empty() && label.chars().all(allowed)
}

/// ` name=<MiB>` when the size is known, otherwise an empty string.
fn mib_field(name: &str, bytes: Option<u64>) -> String {
    match bytes {
        Some(total) => format!(" {name}={}", total / (1 << 20)),
        None => String::from(""),
    }
}

const MARK_USAGE: &str = "usage: mark <label> (letters, digits, _, ., -)";

/// `mark <label>`: one line of process and match numbers that benchmark
/// tooling reads back out of stdout and the log. Its layout is fixed.
fn run_mark(
    command: &TypedOrder,
    marks: &mut u64,
    view: &GameView<'_>,
    panel: &mut PromptPanelStatus,
    room: usize,
) {
    let label = match command.words.as_slice() {
        [only] if valid_mark_label(only) => only,
        _ => return panel.post_line(MARK_USAGE, room),
    };
    *marks += 1;
    let seq = *marks;
    let pid = std::process::id();
    let ns = diag::process_elapsed_ns();
    let rss = mib_field("rss_mib", asset_transport::working_set_size());
    let heap = mib_field("heap_mib", diag::heap_bytes_in_use());
    let facts = view.mark_facts();
    let record = format!("benchmark-mark: pid={pid} seq={seq} ns={ns} label={label}{rss}{heap}{facts}");
    diag::info!(Console, "{record}");
    println!("{record}");
    panel.post_line(record, room);
}

/// Ceiling for `wait <seconds>`.
const MAX_WAIT_SECS: f32 = 120.0;

/// What a `wait` line asks the queue to do.
#[derive(Debug)]
enum Wait {
    /// Sleep this many seconds.
    Pause(f32),
    /// Hold for this many authority ticks.
    Ticks(u32),
    /// Hold until the game reaches a state.
    Until(Hold),
}

/// Sleep used when `wait` has no argument or one that isn't a number.
const DEFAULT_WAIT_SECS: f32 = 1.0;

/// Reads `wait world|spawn|torn|ambient`, `wait <n>t` and `wait <secs>[s]`,
/// ignoring case.
fn parse_wait(args: &[String]) -> Wait {
    let word = match args.first() {
        Some(first) => first.to_ascii_lowercase(),
        None => return Wait::Pause(DEFAULT_WAIT_SECS),
    };
    let state = [
        ("world", Hold::World),
        ("spawn", Hold::Spawn),
        ("torn", Hold::Torn),
        ("ambient", Hold::Ambient),
    ]
    .iter()
    .find_map(|&(name, hold)| (name == word).then_some(hold));
    if let Some(hold) = state {
        return Wait::Until(hold);
    }
    let tick_count = word.strip_suffix('t').map(str::parse::<u32>);
    if let Some(Ok(ticks)) = tick_count {
        return Wait::Ticks(ticks);
    }
    let secs = word
        .trim_end_matches('s')
        .parse()
        .ok()
        .filter(|secs: &f32| secs.is_finite())
        .unwrap_or(DEFAULT_WAIT_SECS);
    Wait::Pause(secs.clamp(0.0, MAX_WAIT_SECS))
}

/// Log wording for `wait <what>`: first when the game is already there,
/// second when a hold begins.
fn wait_text(hold: Hold) -> [&'static str; 2] {
    match hold {
        Hold::World => ["already spawned", "until scene.spawned"],
        Hold::Spawn => ["already InGame", "until AppScreen::InGame"],
        Hold::Torn => ["already hold", "until HasWorld=0 and scene.spawned=0"],
        Hold::Ambient => ["already MapAmbientBooted", "until MapAmbientBooted"],
        _ => ["already reached", "until reached"],
    }
}

fn run_wait(
    command: &TypedOrder,
    view: &GameView<'_>,
    router: &mut TypedOrderRouter,
    panel: &mut PromptPanelStatus,
    room: usize,
) {
    match parse_wait(&command.words) {
        Wait::Pause(secs) => {
            router.sleep_for(secs);
            diag::info!(Console, "wait: {:.1}s", secs);
        }
        Wait::Ticks(ticks) => match view.tick() {
            Some(now) => {
                router.stall(Hold::Tick(now.wrapping_add(ticks)));
                diag::info!(Console, "wait: {ticks} ticks from tick {now}");
            }
            None => {
                let refusal = format!("wait: {ticks}t needs an authority clock; not holding");
                diag::warn!(Console, "{}", refusal);
                panel.post_line(refusal, room);
            }
        },
        Wait::Until(hold) => {
            let [already, until] = wait_text(hold);
            let already_there = view.met(hold);
            if !already_there {
                router.stall(hold);
            }
            let note = if already_there { already } else { until };
            diag::info!(Console, "{}: {note}", hold.kind());
        }
    }
}

/// Transfers the commands the process launched with into the live queue.
pub(super) fn drain_startup_queue(
    mut startup: ResMut<OrderInbox>,
    mut queue: ResMut<TypedOrderBacklog>,
) {
    let launched = startup.drain();
    queue.0.extend(launched);
}

/// Commands issued by menus, kept apart from the main queue so they never
/// queue up behind a script.
#[derive(Default)]
pub(super) struct MenuScript {
    pending: VecDeque<TypedOrder>,
    delay: f32,
    /// Names of commands already warned about as unsupported.
    reported: HashSet<String>,
}

/// Pause length requested by a menu script's `wait`.
fn menu_delay(args: &[String]) -> f32 {
    let Some(first) = args.first() else {
        return 0.0;
    };
    first.trim_end_matches('s').parse().unwrap_or(0.0)
}

/// Runs what menus submit through `exec`. Each command is sent as a typed,
/// backgrounded one; commands the console lacks are skipped.
pub(super) fn run_menu_commands(
    clock: Res<Time>,
    book: Res<OrderCatalog>,
    mut incoming: MessageReader<frame::UiExecCommand>,
    mut script: Local<MenuScript>,
    mut submitted: MessageWriter<TypedOrder>,
) {
    for request in incoming.read() {
        let parsed = TypedOrder::parse_script(&request.text);
        script.pending.extend(parsed);
    }
    if script.delay > 0.0 {
        script.delay = (script.delay - clock.delta_secs()).max(0.0);
        return;
    }
    while let Some(mut cmd) = script.pending.pop_front() {
        if cmd.name.eq_ignore_ascii_case("wait") {
            script.delay = menu_delay(&cmd.words);
            break;
        }
        if book.look_up(&cmd.name).is_none() {
            if script.reported.insert(cmd.name.clone()) {
                diag::warn!(Ui, "menu: unsupported engine command `{}`", cmd.name);
            }
            continue;
        }
        cmd.typed_by_hand = true;
        cmd.detached = true;
        submitted.write(cmd);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::queue::*;

    fn command(line: &str, typed_by_hand: bool) -> TypedOrder {
        let mut command = TypedOrder::parse(line).expect("a command");
        command.typed_by_hand = typed_by_hand;
        command
    }

    fn queue_of(lines: &[&str]) -> VecDeque<TypedOrder> {
        lines.iter().map(|line| command(line, false)).collect()
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn wait_defaults_to_one_second() {
        assert!(matches!(parse_wait(&[]), Wait::Pause(s) if s == 1.0));
        assert!(matches!(parse_wait(&args(&["soon"])), Wait::Pause(s) if s == 1.0));
        assert!(matches!(parse_wait(&args(&["nan"])), Wait::Pause(s) if s == 1.0));
    }

    #[test]
    fn wait_reads_seconds_with_or_without_the_unit() {
        assert!(matches!(parse_wait(&args(&["2.5s"])), Wait::Pause(s) if s == 2.5));
        assert!(matches!(parse_wait(&args(&["3S"])), Wait::Pause(s) if s == 3.0));
        assert!(matches!(parse_wait(&args(&["500"])), Wait::Pause(s) if s == MAX_WAIT_SECS));
        assert!(matches!(parse_wait(&args(&["-4"])), Wait::Pause(s) if s == 0.0));
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
        let mut queue = TypedOrderBacklog::default();
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
        let expected = "wait playing: timed out after 120s (match phase still not Playing) — script \
         aborted, 1 queued commands dropped, kept exit=1";
        assert_eq!(message, expected);
    }

    #[test]
    fn a_background_map_does_not_block_the_queue() {
        let mut router = TypedOrderRouter::default();
        let mut map = command("map mp_nuketown_2020", false);
        map.detached = true;
        begin_sync(&map, &mut router);
        assert!(!router.waiting());
        begin_sync(&command("map mp_nuketown_2020", false), &mut router);
        assert!(router.command_running);
        assert!(matches!(router.holding(), Some(Hold::World)));
    }

    #[test]
    fn disconnect_waits_for_the_world_to_go_and_spawn_waits_for_nothing() {
        let mut router = TypedOrderRouter::default();
        begin_sync(&command("spawn", false), &mut router);
        assert!(!router.waiting());
        begin_sync(&command("disconnect", false), &mut router);
        assert!(router.command_running);
        assert!(matches!(router.holding(), Some(Hold::Torn)));
    }

    #[test]
    fn an_ordinary_command_does_not_hold_the_queue() {
        let mut router = TypedOrderRouter::default();
        begin_sync(&command("echo hi", false), &mut router);
        assert!(!router.waiting());
    }

    #[test]
    fn a_new_hold_replaces_the_old_one_and_restarts_its_clock() {
        let mut router = TypedOrderRouter::default();
        router.stall(Hold::Spawn);
        assert_eq!(router.settle(30.0, |_| false), Settled::Waiting);
        router.stall(Hold::Ambient);
        assert!(matches!(router.holding(), Some(Hold::Ambient)));
        assert!(matches!(
            router.settle(1.0, |hold| matches!(hold, Hold::Ambient)),
            Settled::Met(Hold::Ambient, secs) if secs == 0.0
        ));
        assert!(router.holding().is_none());
    }

    #[test]
    fn a_met_hold_reports_how_long_it_waited() {
        let mut router = TypedOrderRouter::default();
        assert_eq!(router.settle(1.0, |_| true), Settled::Free);
        router.stall(Hold::Spawn);
        assert_eq!(router.settle(0.5, |_| false), Settled::Waiting);
        assert_eq!(router.settle(0.5, |_| false), Settled::Waiting);
        assert!(matches!(
            router.settle(0.5, |_| true),
            Settled::Met(Hold::Spawn, secs) if secs == 1.0
        ));
    }

    #[test]
    fn a_hold_times_out_after_two_minutes() {
        let mut router = TypedOrderRouter::default();
        router.stall(Hold::Playing);
        assert_eq!(router.settle(119.0, |_| false), Settled::Waiting);
        assert!(matches!(
            router.settle(1.0, |_| false),
            Settled::TimedOut(Hold::Playing)
        ));
        assert!(router.holding().is_none());
    }

    #[test]
    fn a_spawn_that_never_gets_through_forgets_its_class() {
        let mut router = TypedOrderRouter {
            queued_kit_choice: Some("assault".to_owned()),
            ..TypedOrderRouter::default()
        };
        router.stall(Hold::SpawnAdmit);
        assert!(matches!(
            router.settle(HOLD_TIMEOUT_SECS, |_| false),
            Settled::TimedOut(Hold::SpawnAdmit)
        ));
        assert_eq!(router.queued_kit_choice, None);
    }

    #[test]
    fn release_lets_everything_go() {
        let mut router = TypedOrderRouter {
            command_running: true,
            queued_kit_choice: Some("smg".to_owned()),
            ..Default::default()
        };
        router.stall(Hold::World);
        router.sleep_for(5.0);
        assert!(router.waiting());
        router.unblock();
        assert!(!router.waiting());
        assert_eq!(router.queued_kit_choice, None);
    }

    #[test]
    fn a_sleep_ends_on_the_frame_it_runs_out() {
        let mut router = TypedOrderRouter::default();
        assert!(!router.sleeping(0.1));
        router.sleep_for(1.0);
        assert!(router.waiting());
        assert!(router.sleeping(0.5));
        assert!(!router.sleeping(0.5));
        assert!(!router.waiting());
    }

    #[test]
    fn a_finished_wait_reads_back_what_was_reached() {
        let expected = "wait ticks: tick 7 after 2.0s";
        assert_eq!(Hold::Tick(7).done_line(2.0), expected);
        let expected = "wait world: spawned after 0.0s";
        assert_eq!(Hold::World.done_line(0.04), expected);
        let expected = "force_spawn: Alive after 1.5s";
        assert_eq!(Hold::Alive(sim::ClientId(0), sim::LifeSequence(1)).done_line(1.5), expected);
    }

    #[test]
    fn a_script_pushed_to_the_queue_keeps_its_order() {
        let mut queue = TypedOrderBacklog::default();
        queue.enqueue_script_text("map a; wait 1; quit");
        let names: Vec<_> = queue.0.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["map", "wait", "quit"]);
    }
}
