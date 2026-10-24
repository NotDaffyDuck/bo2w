//! Starts the process the way the command line asked.
//!
//! Every mode builds its own Bevy app out of the same parts: a window (or
//! none), the screen it opens on, what the master server is told, and the
//! runtime plugins. [`frontend`] is Black Ops II's menus with no map yet;
//! [`map`] is a match on a zone, whether hosted, joined, served without a
//! window or played back from a recorded demo.

mod frontend;
mod map;
mod window;

use std::path::PathBuf;

use asset_core::AssetNamespace;
use asset_transport::GamesRoot;
use assets::NamespaceTrees;
use bevy::prelude::*;
use frame::LaunchIdentity;
use master_protocol::ContentFlags;
use render::diag::capture::{CaptureQueue, CaptureRequest};
use render_frontend::prepare::scene::world::WorldScene;
use session::StartupCommands;
use ui::{AppScreen, UiLayer, UiLayers};

use crate::args::{AcceptanceLaunch, LaunchMode};

pub(crate) use window::{test_window_hidden, test_window_placed};

/// What this process does in the match it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Hosts the match and plays in it.
    Listen,
    /// Hosts the match with no window.
    Dedicated,
    /// Plays in a match another machine hosts.
    Client,
    /// Watches a recorded match.
    Replay,
}

impl Role {
    /// The name the launch report and the state dump show.
    fn label(self) -> &'static str {
        match self {
            Self::Listen => "Listen",
            Self::Dedicated => "Dedicated",
            Self::Client => "Client",
            Self::Replay => "Replay",
        }
    }
}

/// Where the game files are and where this run writes.
struct Boot {
    games: GamesRoot,
    artifacts: PathBuf,
}

impl Boot {
    fn identity(&self, role: Role, zone: &str) -> LaunchIdentity {
        LaunchIdentity {
            role_label: role.label().to_owned(),
            games_root: self.games.0.clone(),
            artifacts: self.artifacts.clone(),
            zone: zone.to_owned(),
        }
    }
}

/// Runs the process in `mode` until its window closes (or, for a served
/// match, until it is stopped).
pub fn launch(
    games: GamesRoot,
    artifacts: PathBuf,
    mode: LaunchMode,
    acceptance: Option<AcceptanceLaunch>,
) {
    diag::info!(Launch, "{}", asset_transport::games_root_report(&games));
    keep_frame_owner_fast();
    let boot = Boot { games, artifacts };
    match (mode, acceptance) {
        (LaunchMode::FrontendT6Mp, None) => frontend::run(boot),
        (LaunchMode::Map(zone), acceptance) => {
            map::run(boot, map::Request::listen(zone, acceptance))
        }
        (LaunchMode::Serve(zone), _) => map::run(boot, map::Request::dedicated(zone)),
        (
            LaunchMode::Play {
                name,
                zone_override,
            },
            None,
        ) => {
            map::run_replay(boot, &name, zone_override);
        }
        (LaunchMode::FrontendT6Mp, Some(_)) => fatal(&acceptance_needs_a_map("menu")),
        (LaunchMode::Play { .. }, Some(_)) => fatal(&acceptance_needs_a_map("play")),
    }
}

fn acceptance_needs_a_map(refused: &str) -> String {
    format!("render acceptance requires `bo2mp map <zone>` (not {refused})")
}

/// Narrows the process to the machine's fast cores, when it has some, and
/// tells the asset loaders which cores they may use alongside.
fn keep_frame_owner_fast() {
    let Some(plan) = crate::frame_owner::prefer_performance_cores() else {
        return;
    };
    diag::info!(
        Launch,
        "frame owner: cpus {:?} of {:?} allowed (performance cores {:?})",
        plan.chosen,
        plan.allowed,
        plan.performance
    );
    assets::publish_process_cpus(plan.allowed);
}

/// Ends the process with `message` in front of the player.
fn fatal(message: &str) -> ! {
    diag::exit_launch_error(message)
}

/// The resources every first screen needs, with `layers` the only UI layers
/// showing on it.
fn insert_screen(app: &mut App, screen: AppScreen, layers: impl IntoIterator<Item = UiLayer>) {
    let mut shown = UiLayers::default();
    shown.show_only(layers);
    app.insert_resource(screen)
        .insert_resource(shown)
        .insert_resource(StartupCommands {
            lines: console::startup_commands(),
        })
        .insert_resource(WorldScene::default())
        .insert_resource(ClearColor(Color::BLACK));
}

/// The game content this machine can offer to other players.
fn content_flags(trees: &NamespaceTrees) -> ContentFlags {
    content_flags_for(
        trees.get(AssetNamespace::T5).is_some(),
        trees.get(AssetNamespace::T6).is_some(),
    )
}

fn content_flags_for(has_t5: bool, has_t6: bool) -> ContentFlags {
    let mut flags = net::content_inventory(has_t5);
    if has_t6 {
        flags.0 |= net::CONTENT_T6;
    }
    flags
}

/// Takes a master-server intent that could not be built as "no master
/// server", and says why.
fn intent_or_off<E: std::fmt::Display>(
    purpose: &str,
    built: Result<net::MasterLaunchIntent, E>,
) -> net::MasterLaunchIntent {
    built.unwrap_or_else(|error| {
        diag::warn!(Net, "{purpose} is off: {error}");
        net::MasterLaunchIntent::disabled()
    })
}

/// Queues the screenshot `BO2MP_SCREENSHOT` asks for, if it asks for one.
/// `then_exit` closes the app once the picture is on disk. The render plugin
/// makes the queue, so this goes after the plugins are added.
fn queue_env_capture(app: &mut App, then_exit: bool) {
    let Some(request) = CaptureRequest::from_env() else {
        return;
    };
    queue_capture(
        app,
        CaptureRequest {
            exit_after_capture: then_exit || request.exit_after_capture,
            ..request
        },
    );
}

fn queue_capture(app: &mut App, request: CaptureRequest) {
    app.world_mut()
        .get_resource_mut::<CaptureQueue>()
        .expect("the render plugin makes the capture queue, so it goes in first")
        .push(request);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_keep_the_names_the_reports_show() {
        let labels: Vec<_> = [Role::Listen, Role::Dedicated, Role::Client, Role::Replay]
            .into_iter()
            .map(Role::label)
            .collect();
        assert_eq!(labels, ["Listen", "Dedicated", "Client", "Replay"]);
    }

    #[test]
    fn the_identity_carries_where_the_run_reads_and_writes() {
        let boot = Boot {
            games: GamesRoot(PathBuf::from("games")),
            artifacts: PathBuf::from("out"),
        };
        let identity = boot.identity(Role::Client, "mp_raid");
        assert_eq!(identity.role_label, "Client");
        assert_eq!(identity.games_root, PathBuf::from("games"));
        assert_eq!(identity.artifacts, PathBuf::from("out"));
        assert_eq!(identity.zone, "mp_raid");
        assert_eq!(boot.identity(Role::Listen, "").zone, "");
    }

    #[test]
    fn a_refusal_names_the_mode_that_cannot_take_acceptance() {
        let message = acceptance_needs_a_map("play");
        assert!(message.contains("bo2mp map <zone>"), "{message}");
        assert!(message.ends_with("(not play)"), "{message}");
    }

    #[test]
    fn content_is_offered_only_for_what_is_installed() {
        assert_eq!(content_flags_for(false, false), ContentFlags(0));
        assert_eq!(
            content_flags_for(false, true),
            ContentFlags(net::CONTENT_T6)
        );
        let both = content_flags_for(true, true);
        assert_eq!(both.0, net::CONTENT_T5 | net::CONTENT_T6);
        assert_eq!(content_flags_for(true, false).0, net::CONTENT_T5);
    }

    #[test]
    fn an_install_with_no_trees_offers_nothing() {
        assert_eq!(content_flags(&NamespaceTrees::default()), ContentFlags(0));
    }

    #[test]
    fn a_first_screen_shows_only_the_layers_it_names() {
        let mut app = App::new();
        insert_screen(
            &mut app,
            AppScreen::Loading,
            [UiLayer::Loading, UiLayer::Overlay],
        );
        let layers = app.world().resource::<UiLayers>();
        assert!(layers.is_visible(UiLayer::Loading));
        assert!(layers.is_visible(UiLayer::Overlay));
        assert!(!layers.is_visible(UiLayer::Shell));
        assert!(app.world().contains_resource::<StartupCommands>());
        assert!(app.world().contains_resource::<WorldScene>());
        let clear = app.world().resource::<ClearColor>();
        assert_eq!(clear.0, Color::BLACK);
    }

    #[test]
    fn a_master_intent_that_failed_falls_back_to_none() {
        let off = intent_or_off::<String>("master test", Err("no address".into()));
        assert!(!off.enabled());
        let kept = intent_or_off::<String>("master test", Ok(net::MasterLaunchIntent::disabled()));
        assert!(!kept.enabled());
    }

    #[test]
    fn a_capture_goes_in_the_queue_the_render_plugin_made() {
        let mut app = App::new();
        app.init_resource::<CaptureQueue>();
        queue_capture(
            &mut app,
            CaptureRequest {
                path: PathBuf::from("shot.png"),
                exit_after_capture: false,
            },
        );
        assert_eq!(app.world().resource::<CaptureQueue>().pending(), 1);
    }

    #[test]
    #[should_panic(expected = "render plugin")]
    fn a_capture_before_the_render_plugin_is_a_wiring_mistake() {
        let mut app = App::new();
        queue_capture(
            &mut app,
            CaptureRequest {
                path: PathBuf::from("shot.png"),
                exit_after_capture: false,
            },
        );
    }
}
