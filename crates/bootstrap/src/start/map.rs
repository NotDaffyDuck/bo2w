//! A match on a zone: hosted from the window, joined through the master
//! server, served with no window, or played back from a recorded demo. The
//! window opens on the loading screen while the zone loads in the background.

use std::path::{Path, PathBuf};

use assets::{LoadBackdropOrigin, LoadBackdrop, SessionLoadOrder, CategoryTreeSet};
use bevy::{prelude::*, window::PresentMode, winit::WinitSettings};
use frame::StartupReport;
use master_protocol::AssetCoverageBits;
use net::DirectoryLaunchRequest;
use render::checks::acceptance::{
    TRIAL_RESOLUTION_ROWS, SMOKE_CHECK_SWAP_POLICY, TRIAL_RESOLUTION_COLUMNS, AcceptanceTrial,
};
use replay::{DemoReplay, RecordingPlayer};
use ui::{ShellScreen, MenuMediaAnchor, MenuPaintPass, OverlayTier};

use super::{
    Boot, CONTENT_T6, Role, content_catalog, pack_presence_bits, fatal, insert_screen, intent_or_off,
    queue_env_capture, window,
};
use crate::cli::SignoffStartup;
use crate::app_stack::{install_gameplay_plugins, add_runtime_stack_as};
use asset_transport::{ZoneTitleKind, GameInstallDir, locate_zone_file, locate_shared_mp_zone, title_of_zone_path, ZoneArchive, zone_key_parts, LoadCompletion, loading_caption};

/// How the command line wants the match started, with what each way carries.
enum Start {
    /// Hosted from this window, maybe as a render acceptance run.
    Listen(Option<SignoffStartup>),
    /// Served with no window.
    Dedicated,
    /// A recorded demo played back.
    Replay(RecordingPlayer),
}

/// What the command line asked of a match: the zone as typed and how to start.
pub(super) struct Request {
    typed_zone: String,
    start: Start,
}

impl Request {
    pub(super) fn listen(zone_arg: String, acceptance: Option<SignoffStartup>) -> Self {
        Self { typed_zone: zone_arg, start: Start::Listen(acceptance) }
    }

    pub(super) fn dedicated(zone_arg: String) -> Self {
        Self { typed_zone: zone_arg, start: Start::Dedicated }
    }

    fn replay(zone_arg: String, playback: RecordingPlayer) -> Self {
        Self { typed_zone: zone_arg, start: Start::Replay(playback) }
    }

    /// The role the process starts as, before the master server has a say.
    fn role(&self) -> Role {
        match self.start {
            Start::Listen(_) => Role::Listen,
            Start::Dedicated => Role::Dedicated,
            Start::Replay(_) => Role::Replay,
        }
    }
}

/// Plays back the demo `name` recorded under the artifacts folder, on the
/// zone it was recorded on unless `forced_pack` names another.
pub(super) fn run_replay(boot: Boot, name: &str, forced_pack: Option<String>) {
    let playback = match DemoReplay::open(&boot.writes_to, name) {
        Ok(opened) => opened,
        Err(error) => fatal(&format!("play: {error}")),
    };
    let shown = playback.path().display().to_string();
    let recorded = playback.identity().map_name().map(str::to_owned);
    match replay_zone(forced_pack, recorded.as_deref()) {
        None => fatal(&format!(
            "play: recording {shown} has no zone — re-record on this build, or: make play {name} ZONE=<map>"
        )),
        Some(zone) => {
            diag::info!(Launch, "play: {shown} zone={zone}");
            run(boot, Request::replay(zone, RecordingPlayer::new(playback)));
        }
    }
}

/// The zone a replay runs on: the one asked for, else the recorded one.
fn replay_zone(asked: Option<String>, recorded: Option<&str>) -> Option<String> {
    match (asked, recorded) {
        (Some(zone), _) => Some(zone),
        (None, Some(zone)) => Some(zone.to_owned()),
        (None, None) => None,
    }
}

/// Builds the app for a match on the requested zone and runs it.
pub(super) fn run(boot: Boot, request: Request) {
    let started_as = request.role();
    let Request { typed_zone, start } = request;
    let (flagged, recording) = match start {
        Start::Listen(flag) => (flag, None),
        Start::Dedicated => (None, None),
        Start::Replay(demo) => (None, Some(demo)),
    };

    let zone = Zone::find(&boot.games, &typed_zone);
    zone.require_bo2(&typed_zone).unwrap_or_else(|message| fatal(&message));
    let have = pack_presence_bits(&CategoryTreeSet::discover(&boot.games));
    let catalog = content_catalog();
    let requires = match catalog.mask_for_map(&zone.key) {
        Ok(mask) => mask,
        Err(error) => fatal(&format!("master content: {error}")),
    };
    let master = master_intent(started_as, &zone.name, (have, requires));
    let role = role_after_master(started_as, master.denotes_arrival());
    let acceptance = acceptance_run(flagged, &zone.name);
    let present = window::forced_present_mode(acceptance.is_some());

    // Order matters here: plugins see the resources inserted before them.
    let mut app = App::new();
    app.insert_resource(master).insert_resource(catalog);
    add_window_and_plugins(&mut app, role, &zone.name, present);
    add_loading(&mut app, &boot, role, &zone);
    insert_screen(&mut app, ShellScreen::Loading, [OverlayTier::Loading, OverlayTier::Overlay]);
    if let Some(proof) = acceptance {
        start_acceptance(&mut app, proof);
    }
    add_runtime(&mut app, role);
    if let Some(demo) = recording {
        app.insert_resource(demo);
    }
    queue_env_capture(&mut app, false);
    app.run();
}

/// A zone the command line named, and what the install has for it.
struct Zone {
    /// What was typed, trimmed and lower-cased; a `game:` prefix stays.
    key: String,
    /// The name the match loads under: the file's, or the typed one without
    /// its prefix when no file was found.
    name: String,
    game: Option<ZoneTitleKind>,
    file: Result<PathBuf, String>,
    /// The shared zone loaded with the map, or why there is none.
    shared: Result<PathBuf, String>,
    nickname: Option<String>,
}

impl Zone {
    fn find(games: &GameInstallDir, typed: &str) -> Self {
        let key = typed.trim().to_ascii_lowercase();
        let search = locate_zone_file(games, &key);
        let shared_for = |map_ff: &Path| locate_shared_mp_zone(games, map_ff).map(|hit| hit.path);
        Self::from_search(key, search, title_of_zone_path, shared_for)
    }

    /// The zone `key` names, given the search's answer. `game_of` tells which
    /// game a file belongs to when the key has no prefix, and `common_mp_for`
    /// finds the shared zone a map file loads with.
    fn from_search(
        key: String,
        found: Result<ZoneArchive, String>,
        game_of: impl FnOnce(&Path) -> Option<ZoneTitleKind>,
        common_mp_for: impl FnOnce(&Path) -> Result<PathBuf, String>,
    ) -> Self {
        let (game_tag, typed_stem) = zone_key_parts(&key);
        let (name, nickname, file) = match found {
            Ok(hit) => (hit.zone_label, hit.nickname_remark, Ok(hit.path)),
            Err(why) => (typed_stem.to_owned(), None, Err(why)),
        };
        let game = match game_tag {
            Some(named) => Some(named),
            None => file.as_deref().ok().and_then(game_of),
        };
        let shared = file.as_deref().map_err(String::clone).and_then(common_mp_for);
        Self { key, name, game, file, shared, nickname }
    }

    /// Only Black Ops II maps launch.
    fn require_bo2(&self, typed: &str) -> Result<(), String> {
        if self.game == Some(ZoneTitleKind::T6) {
            return Ok(());
        }
        let why = match &self.file {
            Ok(_) => format!("`{typed}` is not a BO2 map"),
            Err(search) => format!("no map `{typed}`: {search}"),
        };
        Err(format!("BO2 maps only: {why}"))
    }

    fn report(&self) -> StartupReport {
        let mut report = StartupReport::default();
        report.zone.clone_from(&self.name);
        report.zone_archive_file.clone_from(&self.file);
        report.shared_zone.clone_from(&self.shared);
        report.pack_nickname.clone_from(&self.nickname);
        report.stepping_lag = "map loading has not reached clipmap yet";
        report.level_digest.push("queued background zone load".to_owned());
        report
    }
}

/// Who the master server hears from for this match. A recording is watched
/// alone, and without Black Ops II files there is nothing to offer.
/// `content` is what this install has, then what the map needs.
fn master_intent(
    role: Role,
    zone: &str,
    (have, requires): (AssetCoverageBits, AssetCoverageBits),
) -> DirectoryLaunchRequest {
    let watching_alone = role == Role::Replay;
    let bo2_installed = have.0 & CONTENT_T6 != 0;
    if !watching_alone && !bo2_installed {
        diag::warn!(WireLink, "master launch is off: BO2 files are not installed");
    }
    if watching_alone || !bo2_installed {
        DirectoryLaunchRequest::disabled()
    } else {
        let wanted = DirectoryLaunchRequest::from_env_for_map(zone, have, requires);
        intent_or_off("master launch", wanted)
    }
}

/// Joining another machine's match makes this process a client, whatever it
/// was started as.
fn role_after_master(started_as: Role, joining: bool) -> Role {
    [started_as, Role::Client][usize::from(joining)]
}

/// The render acceptance run this launch is: the flag's folder, else the
/// environment's.
fn acceptance_run(flagged: Option<SignoffStartup>, zone: &str) -> Option<AcceptanceTrial> {
    match flagged {
        Some(launch) => Some(AcceptanceTrial { output_folder: launch.dir, map: zone.to_owned() }),
        None => AcceptanceTrial::from_env(zone),
    }
}

fn start_acceptance(app: &mut App, proof: AcceptanceTrial) {
    let dir = proof.output_folder.display();
    diag::info!(
        Launch,
        "render acceptance: dir={dir} map={} {TRIAL_RESOLUTION_COLUMNS}x{TRIAL_RESOLUTION_ROWS} {SMOKE_CHECK_SWAP_POLICY:?}",
        proof.map
    );
    app.insert_resource(MenuPaintPass(false));
    app.insert_resource(proof);
}

/// The window (a dedicated server has none) and the plugins that need one.
fn add_window_and_plugins(
    app: &mut App,
    role: Role,
    zone: &str,
    present_mode: Option<PresentMode>,
) {
    if let Some(mode) = present_mode {
        app.insert_resource(ui::SwapChainPacingChoice(mode));
    }
    let serving = role == Role::Dedicated;
    let headless_env = frame::Headless::requested();
    if serving {
        app.insert_resource(WinitSettings::continuous());
    }
    if headless_env {
        diag::info!(
            Launch,
            "headless: no window, no graphics device, no sound device (BO2MP_HEADLESS=1)"
        );
    }
    // Nothing waits for the world to be drawn or heard.
    if serving || headless_env {
        app.insert_resource(frame::Headless);
    }
    let match_window = if serving { None } else { Some(window::match_window(role, zone, present_mode)) };
    let plugins = crate::app_stack::stock_plugins_hushed_logging(window::window_plugin(match_window));
    app.add_plugins(plugins);
}

/// The id a launch's first load goes out under: nothing has asked for it yet.
const NO_REQUEST: u64 = 0;

/// What the loading screen and the background zone load start from.
fn add_loading(app: &mut App, boot: &Boot, role: Role, zone: &Zone) {
    if let Ok(path) = &zone.file {
        let preview = LoadBackdropOrigin {
            path: path.clone(),
            level_name: zone.name.clone(),
            ticket: NO_REQUEST,
        };
        app.insert_resource(preview);
    }
    let shared_progress = LoadCompletion::default();
    let title = loading_caption(&zone.key, zone.game);
    let ui_root = asset_game::ui_games_root(&boot.games).ok().map(|found| found.0);
    let request = SessionLoadOrder {
        ticket: NO_REQUEST,
        decrypt_secret: Default::default(),
        zone: zone.name.clone(),
        zone_archive_file: zone.file.clone(),
        common_zone: zone.shared.clone(),
        progress: shared_progress.clone(),
    };
    let mode_name = sim::hosted_ruleset().shown_label();
    app.insert_resource(boot.identity(role, &zone.name));
    app.insert_resource(zone.report());
    app.insert_resource(request);
    app.insert_resource(LoadBackdrop::new(shared_progress, title, mode_name));
    app.insert_resource(MenuMediaAnchor(ui_root));
}

fn add_runtime(app: &mut App, role: Role) {
    let special = match role {
        Role::Replay => Some(frame::ProcessDuty::Replay),
        Role::Client => Some(frame::ProcessDuty::Client),
        Role::Listen | Role::Dedicated => None,
    };
    if let Some(runtime) = special {
        add_runtime_stack_as(app, runtime);
    } else {
        install_gameplay_plugins(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_transport::{ZoneArchive, ZoneTitleKind, GameInstallDir};

    fn zone_file(name: &str, nickname: Option<&str>) -> ZoneArchive {
        let path = PathBuf::from(format!("zone/all/{name}.ff"));
        ZoneArchive { path, zone_label: name.to_owned(), nickname_remark: nickname.map(str::to_owned) }
    }

    fn never_peek(_: &Path) -> Option<ZoneTitleKind> {
        panic!("the key already named the game");
    }

    fn never_common(_: &Path) -> Result<PathBuf, String> {
        panic!("there is no file to look beside");
    }

    fn common_beside(path: &Path) -> Result<PathBuf, String> {
        Ok(path.with_file_name("common_mp.ff"))
    }

    fn bo2(_: &Path) -> Option<ZoneTitleKind> {
        Some(ZoneTitleKind::T6)
    }

    fn found(name: &str) -> Zone {
        Zone::from_search(name.to_owned(), Ok(zone_file(name, None)), bo2, common_beside)
    }

    fn missing(key: &str) -> Zone {
        Zone::from_search(key.to_owned(), Err("no such file".into()), never_peek, never_common)
    }

    fn all_path(file: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(format!("zone/all/{file}")))
    }

    fn proof_flag() -> Option<SignoffStartup> {
        Some(SignoffStartup { dir: "proof".into() })
    }

    /// An app with only the loading resources for `zone`, launched as `role`.
    fn loading_app(role: Role, zone: &Zone) -> App {
        let boot = Boot {
            games: GameInstallDir("no-such-games-root".into()),
            writes_to: "out".into(),
        };
        let mut app = App::new();
        add_loading(&mut app, &boot, role, zone);
        app
    }

    #[test]
    fn a_found_zone_loads_under_the_files_name_with_the_shared_zone_beside_it() {
        let hit = zone_file("mp_raid", Some("raid is mp_raid"));
        let zone = Zone::from_search("raid".into(), Ok(hit), bo2, common_beside);
        assert_eq!((zone.key.as_str(), zone.name.as_str()), ("raid", "mp_raid"));
        assert_eq!(zone.game, Some(ZoneTitleKind::T6));
        assert_eq!(zone.file, all_path("mp_raid.ff"));
        assert_eq!(zone.shared, all_path("common_mp.ff"));
        assert_eq!(zone.nickname.as_deref(), Some("raid is mp_raid"));
    }

    #[test]
    fn a_prefix_names_the_game_without_opening_the_file() {
        let hit = zone_file("mp_raid", None);
        let zone = Zone::from_search("t6:mp_raid".into(), Ok(hit), never_peek, common_beside);
        assert_eq!(zone.game, Some(ZoneTitleKind::T6));
        assert_eq!((zone.key.as_str(), zone.name.as_str()), ("t6:mp_raid", "mp_raid"));
    }

    #[test]
    fn a_zone_with_no_file_keeps_what_was_typed_and_the_search_error() {
        let zone = missing("t6:mp_nowhere");
        let search_error: Result<PathBuf, String> = Err("no such file".into());
        assert_eq!(zone.name, "mp_nowhere");
        assert_eq!(zone.game, Some(ZoneTitleKind::T6));
        assert_eq!(zone.file, search_error);
        assert_eq!(zone.shared, search_error);
        assert!(zone.nickname.is_none());
        assert_eq!(missing("mp_nowhere").name, "mp_nowhere");
    }

    #[test]
    fn only_bo2_maps_launch() {
        assert_eq!(found("mp_raid").require_bo2("mp_raid"), Ok(()));
        assert_eq!(missing("t6:mp_x").require_bo2("t6:mp_x"), Ok(()));

        let other_game = Zone::from_search("old_map".into(), Ok(zone_file("old_map", None)), |_| None, common_beside);
        assert_eq!(
            other_game.require_bo2("old_map"),
            Err("BO2 maps only: `old_map` is not a BO2 map".to_owned())
        );
        assert_eq!(
            missing("mp_nowhere").require_bo2("MP_Nowhere"),
            Err("BO2 maps only: no map `MP_Nowhere`: no such file".to_owned())
        );
    }

    #[test]
    fn the_report_starts_from_what_the_search_found() {
        let report = found("mp_raid").report();
        assert_eq!(report.zone, "mp_raid");
        assert_eq!(report.zone_archive_file, all_path("mp_raid.ff"));
        assert_eq!(report.shared_zone, all_path("common_mp.ff"));
        assert_eq!(report.level_digest, ["queued background zone load"]);
        assert!(report.forecast_figures.is_none());
    }

    #[test]
    fn joining_makes_a_client_whatever_the_launch_was() {
        for role in Role::ALL {
            assert_eq!(role_after_master(role, true), Role::Client);
            assert_eq!(role_after_master(role, false), role);
        }
    }

    #[test]
    fn a_replay_never_talks_to_the_master_server() {
        let all = AssetCoverageBits(CONTENT_T6);
        assert!(!master_intent(Role::Replay, "mp_raid", (all, all)).enabled());
    }

    #[test]
    fn without_bo2_files_there_is_no_master_launch() {
        let none = AssetCoverageBits(0);
        for role in [Role::Listen, Role::Dedicated, Role::Client] {
            assert!(!master_intent(role, "mp_raid", (none, none)).enabled());
        }
    }

    #[test]
    fn the_acceptance_flag_runs_on_the_zone_being_launched() {
        let proof = acceptance_run(proof_flag(), "mp_raid").expect("the flag makes a run");
        assert_eq!(proof.output_folder, Path::new("proof"));
        assert_eq!(proof.map, "mp_raid");
    }

    #[test]
    fn a_replay_runs_on_the_zone_asked_for_else_the_recorded_one() {
        let cases = [
            (Some("mp_raid"), Some("mp_slums"), Some("mp_raid")),
            (None, Some("mp_slums"), Some("mp_slums")),
            (Some("mp_raid"), None, Some("mp_raid")),
            (None, None, None),
        ];
        for (asked, recorded, want) in cases {
            let got = replay_zone(asked.map(str::to_owned), recorded);
            assert_eq!(got.as_deref(), want);
        }
    }

    #[test]
    fn requests_say_which_role_starts_the_match() {
        let listen = Request::listen("mp_raid".into(), proof_flag());
        assert_eq!(listen.role(), Role::Listen);
        assert!(matches!(listen.start, Start::Listen(Some(_))));

        let dedicated = Request::dedicated("mp_raid".into());
        assert_eq!(dedicated.role(), Role::Dedicated);
        assert_eq!(dedicated.typed_zone, "mp_raid");
        assert!(matches!(dedicated.start, Start::Dedicated));
    }

    #[test]
    fn loading_starts_from_the_zone_the_search_found() {
        let app = loading_app(Role::Listen, &found("mp_raid"));
        let world = app.world();
        let load = world.resource::<SessionLoadOrder>();
        assert_eq!(load.zone, "mp_raid");
        assert_eq!(load.zone_archive_file, all_path("mp_raid.ff"));
        assert_eq!(world.resource::<LoadBackdropOrigin>().level_name, "mp_raid");
        assert_eq!(world.resource::<frame::StartupPersona>().duty_caption, "Listen");
        assert!(world.resource::<MenuMediaAnchor>().0.is_none());
    }

    #[test]
    fn a_zone_that_was_not_found_has_no_preview_to_show() {
        let app = loading_app(Role::Client, &missing("mp_nowhere"));
        let world = app.world();
        assert!(!world.contains_resource::<LoadBackdropOrigin>());
        let load = world.resource::<SessionLoadOrder>();
        assert_eq!(load.zone, "mp_nowhere");
        assert!(load.zone_archive_file.is_err());
    }
}
