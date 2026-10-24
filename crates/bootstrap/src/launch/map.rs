//! A match on a zone: hosted from the window, joined through the master
//! server, served with no window, or played back from a recorded demo. The
//! window opens on the loading screen while the zone loads in the background.

use std::path::{Path, PathBuf};

use asset_transport::{
    GamesRoot, LoadProgress, ZoneFile, ZoneGame, find_runtime_common_mp, find_zone_file,
    map_load_title, split_zone_key, zone_game_for_path,
};
use assets::{LoadingPreviewSource, LoadingScreen, MatchLoadRequest, NamespaceTrees};
use bevy::{prelude::*, window::PresentMode, winit::WinitSettings};
use frame::LaunchReport;
use master_protocol::ContentFlags;
use net::MasterLaunchIntent;
use render::diag::acceptance::{
    ACCEPTANCE_HEIGHT, ACCEPTANCE_PRESENT_MODE, ACCEPTANCE_WIDTH, AcceptanceRun,
};
use replay::{Playback, ReplayPlayback};
use ui::{AppScreen, UiAssetRoot, UiDraw, UiLayer};

use super::{
    Boot, Role, content_flags, fatal, insert_screen, intent_or_off, queue_env_capture, window,
};
use crate::args::AcceptanceLaunch;
use crate::plugins::{add_runtime_plugins, add_runtime_plugins_with_role};

/// What the command line asked of a match.
pub(super) struct Request {
    zone_arg: String,
    role: Role,
    acceptance: Option<AcceptanceLaunch>,
    playback: Option<ReplayPlayback>,
}

impl Request {
    pub(super) fn listen(zone_arg: String, acceptance: Option<AcceptanceLaunch>) -> Self {
        Self {
            zone_arg,
            role: Role::Listen,
            acceptance,
            playback: None,
        }
    }

    pub(super) fn dedicated(zone_arg: String) -> Self {
        Self {
            role: Role::Dedicated,
            ..Self::listen(zone_arg, None)
        }
    }

    fn replay(zone_arg: String, playback: ReplayPlayback) -> Self {
        Self {
            role: Role::Replay,
            playback: Some(playback),
            ..Self::listen(zone_arg, None)
        }
    }
}

/// Plays back the demo `name` recorded under the artifacts folder, on the
/// zone it was recorded on unless `zone_override` names another.
pub(super) fn run_replay(boot: Boot, name: &str, zone_override: Option<String>) {
    let playback = Playback::open(&boot.artifacts, name)
        .unwrap_or_else(|error| fatal(&format!("play: {error}")));
    let recorded = playback.identity().zone_name().map(str::to_owned);
    let Some(zone) = replay_zone(zone_override, recorded.as_deref()) else {
        fatal(&format!(
            "play: recording {} has no zone — re-record on this build, or: make play {name} ZONE=<map>",
            playback.path().display()
        ));
    };
    diag::info!(Launch, "play: {} zone={zone}", playback.path().display());
    run(boot, Request::replay(zone, ReplayPlayback::new(playback)));
}

/// The zone a replay runs on: the one asked for, else the recorded one.
fn replay_zone(asked: Option<String>, recorded: Option<&str>) -> Option<String> {
    asked.or_else(|| recorded.map(str::to_owned))
}

/// Builds the app for a match on the requested zone and runs it.
pub(super) fn run(boot: Boot, request: Request) {
    let Request {
        zone_arg,
        role,
        acceptance,
        playback,
    } = request;
    let zone = Zone::find(&boot.games, &zone_arg);
    if let Err(message) = zone.require_bo2(&zone_arg) {
        fatal(&message);
    }
    let have = content_flags(&NamespaceTrees::discover(&boot.games));
    let requires = net::content_required_by_map(&zone.key)
        .unwrap_or_else(|error| fatal(&format!("master content: {error}")));
    let master = master_intent(role, &zone.name, have, requires);
    let role = role_after_master(role, master.is_join());
    let acceptance = acceptance_run(acceptance, &zone.name);
    let present = window::forced_present_mode(acceptance.is_some());

    let mut app = App::new();
    app.insert_resource(master);
    add_window_and_plugins(&mut app, role, &zone.name, present);
    add_loading(&mut app, &boot, role, &zone);
    insert_screen(
        &mut app,
        AppScreen::Loading,
        [UiLayer::Loading, UiLayer::Overlay],
    );
    if let Some(run) = acceptance {
        start_acceptance(&mut app, run);
    }
    add_runtime(&mut app, role);
    if let Some(playback) = playback {
        app.insert_resource(playback);
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
    game: Option<ZoneGame>,
    file: Result<PathBuf, String>,
    common_mp: Result<PathBuf, String>,
    alias: Option<String>,
}

impl Zone {
    fn find(games: &GamesRoot, typed: &str) -> Self {
        let key = typed.trim().to_ascii_lowercase();
        let found = find_zone_file(games, &key);
        Self::from_search(key, found, zone_game_for_path, |zone_ff| {
            find_runtime_common_mp(games, zone_ff).map(|common| common.path)
        })
    }

    /// The zone `key` names, given the search's answer. `game_of` tells which
    /// game a file belongs to when the key has no prefix, and `common_mp_for`
    /// finds the shared zone a map file loads with.
    fn from_search(
        key: String,
        found: Result<ZoneFile, String>,
        game_of: impl FnOnce(&Path) -> Option<ZoneGame>,
        common_mp_for: impl FnOnce(&Path) -> Result<PathBuf, String>,
    ) -> Self {
        let (prefix, stem) = split_zone_key(&key);
        let stem = stem.to_owned();
        let (file, name, alias) = match found {
            Ok(zone) => (Ok(zone.path), zone.zone_name, zone.alias_note),
            Err(error) => (Err(error), stem, None),
        };
        let game = prefix.or_else(|| file.as_deref().ok().and_then(game_of));
        let common_mp = match &file {
            Ok(path) => common_mp_for(path),
            Err(error) => Err(error.clone()),
        };
        Self {
            key,
            name,
            game,
            file,
            common_mp,
            alias,
        }
    }

    /// Only Black Ops II maps launch.
    fn require_bo2(&self, typed: &str) -> Result<(), String> {
        match (self.game, &self.file) {
            (Some(ZoneGame::T6), _) => Ok(()),
            (None, Err(error)) => Err(format!("BO2 maps only: no map `{typed}`: {error}")),
            (None, Ok(_)) => Err(format!("BO2 maps only: `{typed}` is not a BO2 map")),
        }
    }

    fn report(&self) -> LaunchReport {
        LaunchReport {
            zone: self.name.clone(),
            common_mp: self.common_mp.clone(),
            zone_ff: self.file.clone(),
            zone_alias: self.alias.clone(),
            sim_gap: "map loading has not reached clipmap yet",
            prediction_metrics: None,
            world_report: vec!["queued background zone load".into()],
        }
    }
}

/// Who the master server hears from for this match. A recording is watched
/// alone, and without Black Ops II files there is nothing to offer.
fn master_intent(
    role: Role,
    zone: &str,
    have: ContentFlags,
    requires: ContentFlags,
) -> MasterLaunchIntent {
    if role == Role::Replay {
        return MasterLaunchIntent::disabled();
    }
    if have.0 & net::CONTENT_T6 == 0 {
        diag::warn!(Net, "master launch is off: BO2 files are not installed");
        return MasterLaunchIntent::disabled();
    }
    intent_or_off(
        "master launch",
        MasterLaunchIntent::from_env_for_map(zone, have, requires),
    )
}

/// Joining another machine's match makes this process a client, whatever it
/// was started as.
fn role_after_master(started_as: Role, joining: bool) -> Role {
    if joining { Role::Client } else { started_as }
}

/// The render acceptance run this launch is: the flag's folder, else the
/// environment's.
fn acceptance_run(flagged: Option<AcceptanceLaunch>, zone: &str) -> Option<AcceptanceRun> {
    flagged
        .map(|launch| AcceptanceRun {
            artifact_dir: launch.dir,
            map: zone.to_owned(),
        })
        .or_else(|| AcceptanceRun::from_env(zone))
}

fn start_acceptance(app: &mut App, run: AcceptanceRun) {
    diag::info!(
        Launch,
        "render acceptance: dir={} map={} {}x{} {:?}",
        run.artifact_dir.display(),
        run.map,
        ACCEPTANCE_WIDTH,
        ACCEPTANCE_HEIGHT,
        ACCEPTANCE_PRESENT_MODE
    );
    app.insert_resource(UiDraw(false)).insert_resource(run);
}

/// The window (a dedicated server has none) and the plugins that need one.
fn add_window_and_plugins(
    app: &mut App,
    role: Role,
    zone: &str,
    present_mode: Option<PresentMode>,
) {
    if let Some(mode) = present_mode {
        app.insert_resource(ui::PresentModeOverride(mode));
    }
    let dedicated = role == Role::Dedicated;
    if dedicated {
        app.insert_resource(WinitSettings::continuous());
    }
    let asked_headless = frame::Headless::requested();
    if asked_headless {
        diag::info!(
            Launch,
            "headless: no window, no graphics device, no sound device (BO2MP_HEADLESS=1)"
        );
    }
    if dedicated || asked_headless {
        // Nothing waits for the world to be drawn or heard.
        app.insert_resource(frame::Headless);
    }
    let window = (!dedicated).then(|| window::match_window(role, zone, present_mode));
    app.add_plugins(crate::plugins::default_plugins_with_quiet_log(
        window::window_plugin(window),
    ));
}

/// What the loading screen and the background zone load start from.
fn add_loading(app: &mut App, boot: &Boot, role: Role, zone: &Zone) {
    if let Ok(path) = &zone.file {
        app.insert_resource(LoadingPreviewSource {
            path: path.clone(),
            map_name: zone.name.clone(),
            request_id: 0,
        });
    }
    let progress = LoadProgress::default();
    let title = map_load_title(&zone.key, zone.game);
    let ui_games_root = asset_game::ui_games_root(&boot.games)
        .ok()
        .map(|root| root.0);
    app.insert_resource(boot.identity(role, &zone.name))
        .insert_resource(zone.report())
        .insert_resource(MatchLoadRequest {
            request_id: 0,
            load_key: Default::default(),
            zone: zone.name.clone(),
            zone_ff: zone.file.clone(),
            common_mp: zone.common_mp.clone(),
            progress: progress.clone(),
        })
        .insert_resource(LoadingScreen::new(
            progress,
            title,
            sim::host_game_mode_kind().display_name(),
        ))
        .insert_resource(UiAssetRoot(ui_games_root));
}

fn add_runtime(app: &mut App, role: Role) {
    match role {
        Role::Replay => add_runtime_plugins_with_role(app, frame::RuntimeRole::Replay),
        Role::Client => add_runtime_plugins_with_role(app, frame::RuntimeRole::Client),
        Role::Listen | Role::Dedicated => add_runtime_plugins(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone_file(name: &str, alias: Option<&str>) -> ZoneFile {
        ZoneFile {
            path: PathBuf::from(format!("zone/all/{name}.ff")),
            zone_name: name.to_owned(),
            alias_note: alias.map(str::to_owned),
        }
    }

    fn never_peek(_: &Path) -> Option<ZoneGame> {
        panic!("the key already named the game");
    }

    fn never_common(_: &Path) -> Result<PathBuf, String> {
        panic!("there is no file to look beside");
    }

    fn common_beside(path: &Path) -> Result<PathBuf, String> {
        Ok(path.with_file_name("common_mp.ff"))
    }

    fn found(name: &str) -> Zone {
        Zone::from_search(
            name.to_owned(),
            Ok(zone_file(name, None)),
            |_| Some(ZoneGame::T6),
            common_beside,
        )
    }

    fn missing(key: &str) -> Zone {
        Zone::from_search(
            key.to_owned(),
            Err("no such file".into()),
            never_peek,
            never_common,
        )
    }

    #[test]
    fn a_found_zone_loads_under_the_files_name_with_the_shared_zone_beside_it() {
        let zone = Zone::from_search(
            "raid".into(),
            Ok(zone_file("mp_raid", Some("raid is mp_raid"))),
            |_| Some(ZoneGame::T6),
            common_beside,
        );
        assert_eq!(zone.key, "raid");
        assert_eq!(zone.name, "mp_raid");
        assert_eq!(zone.game, Some(ZoneGame::T6));
        assert_eq!(zone.file, Ok(PathBuf::from("zone/all/mp_raid.ff")));
        assert_eq!(zone.common_mp, Ok(PathBuf::from("zone/all/common_mp.ff")));
        assert_eq!(zone.alias.as_deref(), Some("raid is mp_raid"));
    }

    #[test]
    fn a_prefix_names_the_game_without_opening_the_file() {
        let zone = Zone::from_search(
            "t6:mp_raid".into(),
            Ok(zone_file("mp_raid", None)),
            never_peek,
            common_beside,
        );
        assert_eq!(zone.game, Some(ZoneGame::T6));
        assert_eq!(zone.key, "t6:mp_raid");
        assert_eq!(zone.name, "mp_raid");
    }

    #[test]
    fn a_zone_with_no_file_keeps_what_was_typed_and_the_search_error() {
        let zone = missing("t6:mp_nowhere");
        assert_eq!(zone.name, "mp_nowhere");
        assert_eq!(zone.game, Some(ZoneGame::T6));
        assert_eq!(zone.file, Err("no such file".to_owned()));
        assert_eq!(zone.common_mp, Err("no such file".to_owned()));
        assert_eq!(zone.alias, None);
        assert_eq!(missing("mp_nowhere").name, "mp_nowhere");
    }

    #[test]
    fn only_bo2_maps_launch() {
        assert_eq!(found("mp_raid").require_bo2("mp_raid"), Ok(()));
        assert_eq!(missing("t6:mp_x").require_bo2("t6:mp_x"), Ok(()));

        let not_bo2 = Zone::from_search(
            "old_map".into(),
            Ok(zone_file("old_map", None)),
            |_| None,
            common_beside,
        );
        let message = not_bo2.require_bo2("old_map").unwrap_err();
        assert_eq!(message, "BO2 maps only: `old_map` is not a BO2 map");

        let message = missing("mp_nowhere").require_bo2("MP_Nowhere").unwrap_err();
        assert_eq!(message, "BO2 maps only: no map `MP_Nowhere`: no such file");
    }

    #[test]
    fn the_report_starts_from_what_the_search_found() {
        let report = found("mp_raid").report();
        assert_eq!(report.zone, "mp_raid");
        assert_eq!(report.zone_ff, Ok(PathBuf::from("zone/all/mp_raid.ff")));
        assert_eq!(report.common_mp, Ok(PathBuf::from("zone/all/common_mp.ff")));
        assert_eq!(report.world_report, ["queued background zone load"]);
        assert_eq!(report.prediction_metrics, None);
    }

    #[test]
    fn joining_makes_a_client_whatever_the_launch_was() {
        for role in [Role::Listen, Role::Dedicated, Role::Client, Role::Replay] {
            assert_eq!(role_after_master(role, true), Role::Client);
            assert_eq!(role_after_master(role, false), role);
        }
    }

    #[test]
    fn a_replay_never_talks_to_the_master_server() {
        let all = ContentFlags(net::CONTENT_T6);
        assert!(!master_intent(Role::Replay, "mp_raid", all, all).enabled());
    }

    #[test]
    fn without_bo2_files_there_is_no_master_launch() {
        let none = ContentFlags(0);
        for role in [Role::Listen, Role::Dedicated, Role::Client] {
            assert!(!master_intent(role, "mp_raid", none, none).enabled());
        }
    }

    #[test]
    fn the_acceptance_flag_runs_on_the_zone_being_launched() {
        let flagged = Some(AcceptanceLaunch {
            dir: PathBuf::from("proof"),
        });
        let run = acceptance_run(flagged, "mp_raid").unwrap();
        assert_eq!(run.artifact_dir, PathBuf::from("proof"));
        assert_eq!(run.map, "mp_raid");
    }

    #[test]
    fn a_replay_runs_on_the_zone_asked_for_else_the_recorded_one() {
        let ask = |zone: &str| Some(zone.to_owned());
        assert_eq!(
            replay_zone(ask("mp_raid"), Some("mp_slums")),
            ask("mp_raid")
        );
        assert_eq!(replay_zone(None, Some("mp_slums")), ask("mp_slums"));
        assert_eq!(replay_zone(ask("mp_raid"), None), ask("mp_raid"));
        assert_eq!(replay_zone(None, None), None);
    }

    #[test]
    fn requests_say_which_role_starts_the_match() {
        let listen = Request::listen(
            "mp_raid".into(),
            Some(AcceptanceLaunch {
                dir: PathBuf::from("proof"),
            }),
        );
        assert_eq!(listen.role, Role::Listen);
        assert!(listen.acceptance.is_some());
        assert!(listen.playback.is_none());

        let dedicated = Request::dedicated("mp_raid".into());
        assert_eq!(dedicated.role, Role::Dedicated);
        assert_eq!(dedicated.zone_arg, "mp_raid");
        assert!(dedicated.acceptance.is_none());
        assert!(dedicated.playback.is_none());
    }

    #[test]
    fn loading_starts_from_the_zone_the_search_found() {
        let boot = Boot {
            games: GamesRoot(PathBuf::from("no-such-games-root")),
            artifacts: PathBuf::from("out"),
        };
        let mut app = App::new();
        add_loading(&mut app, &boot, Role::Listen, &found("mp_raid"));
        let world = app.world();
        let load = world.resource::<MatchLoadRequest>();
        assert_eq!(load.zone, "mp_raid");
        assert_eq!(load.zone_ff, Ok(PathBuf::from("zone/all/mp_raid.ff")));
        let preview = world.resource::<LoadingPreviewSource>();
        assert_eq!(preview.map_name, "mp_raid");
        assert_eq!(
            world.resource::<frame::LaunchIdentity>().role_label,
            "Listen"
        );
        assert!(world.resource::<UiAssetRoot>().0.is_none());
    }

    #[test]
    fn a_zone_that_was_not_found_has_no_preview_to_show() {
        let boot = Boot {
            games: GamesRoot(PathBuf::from("no-such-games-root")),
            artifacts: PathBuf::from("out"),
        };
        let mut app = App::new();
        add_loading(&mut app, &boot, Role::Client, &missing("mp_nowhere"));
        assert!(!app.world().contains_resource::<LoadingPreviewSource>());
        let load = app.world().resource::<MatchLoadRequest>();
        assert_eq!(load.zone, "mp_nowhere");
        assert!(load.zone_ff.is_err());
    }
}
