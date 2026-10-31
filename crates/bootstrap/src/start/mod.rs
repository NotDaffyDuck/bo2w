//! Starts the process the way the command line asked.
//!
//! Every mode builds its own Bevy app out of the same parts: a window (or
//! none), the screen it opens on, what the master server is told, and the
//! runtime plugins. [`menus`] is Black Ops II's menus with no map yet;
//! [`map`] is a match on a zone, whether hosted, joined, served without a
//! window or played back from a recorded demo.

mod menus;
mod map;
mod window;

use std::path::PathBuf;

use asset_core::AssetCategoryScope;
use asset_transport::GameInstallDir;
use assets::CategoryTreeSet;
use bevy::prelude::*;
use frame::StartupPersona;
use master_protocol::AssetCoverageBits;
use render::checks::capture::{ScreenshotBacklog, ScreenshotOrder};
use render_frontend::prepare::scene::world::LevelStage;
use session::BootConsoleScript;
use ui::{ShellScreen, OverlayTier, OverlayTierSet};

use crate::cli::{SignoffStartup, StartupFlavor};

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
    /// Every role, in the order the reports list them.
    #[cfg(test)]
    const ALL: [Self; 4] = [Self::Listen, Self::Dedicated, Self::Client, Self::Replay];

    /// The name the launch report and the state dump show.
    fn shown_as(self) -> &'static str {
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
    games: GameInstallDir,
    writes_to: PathBuf,
}

impl Boot {
    fn identity(&self, role: Role, on_zone: &str) -> StartupPersona {
        StartupPersona {
            duty_caption: role.shown_as().to_owned(),
            titles_install_dir: self.games.0.clone(),
            artifacts: self.writes_to.clone(),
            zone: on_zone.to_owned(),
        }
    }
}

/// Runs the process in `mode` until its window closes (or, for a served
/// match, until it is stopped).
pub fn run_mode(
    games: GameInstallDir,
    writes_to: PathBuf,
    mode: StartupFlavor,
    signoff: Option<SignoffStartup>,
) {
    diag::info!(Launch, "{}", asset_transport::install_folders_summary(&games));
    keep_frame_owner_fast();
    let boot = Boot { games, writes_to };
    match (mode, signoff) {
        (StartupFlavor::FrontendT6Mp, None) => menus::run(boot),
        (StartupFlavor::Frontend(zone), None) => {
            frame::set_bo2_frontend(&zone);
            map::run(boot, map::Request::listen(zone, None))
        }
        (StartupFlavor::Hosted(zone), signoff) => {
            map::run(boot, map::Request::listen(zone, signoff))
        }
        (StartupFlavor::Serve(zone), _) => map::run(boot, map::Request::dedicated(zone)),
        (
            StartupFlavor::Play {
                name,
                forced_pack,
            },
            None,
        ) => {
            map::run_replay(boot, &name, forced_pack);
        }
        (StartupFlavor::FrontendT6Mp, Some(_)) => fatal(&acceptance_needs_a_map("menu")),
        (StartupFlavor::Frontend(_), Some(_)) => fatal(&acceptance_needs_a_map("frontend")),
        (StartupFlavor::Play { .. }, Some(_)) => fatal(&acceptance_needs_a_map("play")),
    }
}

fn acceptance_needs_a_map(refused: &str) -> String {
    format!("render acceptance requires `bo2mp map <zone>` (not {refused})")
}

/// Narrows the process to the machine's fast cores, when it has some, and
/// tells the asset loaders which cores they may use alongside.
fn keep_frame_owner_fast() {
    let Some(layout) = crate::fast_cores::prefer_performance_cores() else {
        return;
    };
    diag::info!(
        Launch,
        "frame thread pinned to cpus {:?} (allowed {:?}, fast cores {:?})",
        layout.now_using,
        layout.could_use,
        layout.fast_cores
    );
    assets::announce_worker_cores(layout.could_use);
}

/// Ends the process with `message` in front of the player.
fn fatal(message: &str) -> ! {
    diag::abort_on_startup_fault(message)
}

/// The resources every first screen needs, with `layers` the only UI layers
/// showing on it.
fn insert_screen(app: &mut App, screen: ShellScreen, layers: impl IntoIterator<Item = OverlayTier>) {
    let mut shown = OverlayTierSet::default();
    shown.raise_exactly(layers);
    app.insert_resource(screen)
        .insert_resource(shown)
        .insert_resource(BootConsoleScript {
            commands: console::boot_orders(),
        })
        .insert_resource(LevelStage::default())
        .insert_resource(ClearColor(Color::linear_rgb(0.0, 0.0, 0.0)));
}

/// The game content this machine can offer to other players.
fn pack_presence_bits(trees: &CategoryTreeSet) -> AssetCoverageBits {
    content_flags_for(
        trees.get(AssetCategoryScope::T5).is_some(),
        trees.get(AssetCategoryScope::T6).is_some(),
    )
}

fn content_flags_for(has_t5: bool, has_t6: bool) -> AssetCoverageBits {
    content_catalog().held(|pack| match pack.bit {
        CONTENT_T5 => has_t5,
        CONTENT_T6 => has_t6,
        _ => false,
    })
}

/// Master-protocol content bits this game ships; bit 1 stays reserved.
const CONTENT_T5: u8 = 1 << 2;
const CONTENT_T6: u8 = 1 << 3;

/// The packs a lobby can need, installed for the net crate's master-link code.
fn content_catalog() -> net::ContentCatalog {
    net::ContentCatalog::new(
        vec![
            net::ContentPack { bit: CONTENT_T5, prefix: "t5", label: "t5" },
            net::ContentPack { bit: CONTENT_T6, prefix: "t6", label: "t6" },
        ],
        Some("t6"),
    )
}

/// Takes a master-server intent that could not be built as "no master
/// server", and says why.
fn intent_or_off<E: std::fmt::Display>(
    purpose: &str,
    built: Result<net::DirectoryLaunchRequest, E>,
) -> net::DirectoryLaunchRequest {
    built.unwrap_or_else(|error| {
        diag::warn!(WireLink, "{purpose} is off: {error}");
        net::DirectoryLaunchRequest::disabled()
    })
}

/// Queues the screenshot `BO2MP_SCREENSHOT` asks for, if it asks for one.
/// `then_exit` closes the app once the picture is on disk. The render plugin
/// makes the queue, so this goes after the plugins are added.
fn queue_env_capture(app: &mut App, then_exit: bool) {
    let Some(request) = ScreenshotOrder::from_env() else {
        return;
    };
    queue_capture(
        app,
        ScreenshotOrder {
            quit_when_shot_taken: then_exit || request.quit_when_shot_taken,
            ..request
        },
    );
}

fn queue_capture(app: &mut App, order: ScreenshotOrder) {
    app.world_mut()
        .get_resource_mut::<ScreenshotBacklog>()
        .expect("the render plugin makes the capture queue, so it goes in first")
        .push(order);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_keep_the_names_the_reports_show() {
        let labels: Vec<_> = Role::ALL
            .into_iter()
            .map(Role::shown_as)
            .collect();
        assert_eq!(labels, ["Listen", "Dedicated", "Client", "Replay"]);
    }

    #[test]
    fn the_identity_carries_where_the_run_reads_and_writes() {
        let boot = Boot {
            games: GameInstallDir(PathBuf::from("games")),
            writes_to: PathBuf::from("out"),
        };
        let identity = boot.identity(Role::Client, "mp_raid");
        assert_eq!(identity.duty_caption, "Client");
        assert_eq!(identity.titles_install_dir, PathBuf::from("games"));
        assert_eq!(identity.artifacts, PathBuf::from("out"));
        assert_eq!(identity.zone, "mp_raid");
        assert_eq!(boot.identity(Role::Listen, "").zone, "");
    }

    #[test]
    fn a_refusal_names_the_mode_that_cannot_take_acceptance() {
        let message = acceptance_needs_a_map("play");
        assert!(message.contains("bo2mp map <zone>"), "message was {message}");
        assert!(message.ends_with("(not play)"), "message was {message}");
    }

    #[test]
    fn content_is_offered_only_for_what_is_installed() {
        assert_eq!(content_flags_for(false, false), AssetCoverageBits(0));
        assert_eq!(
            content_flags_for(false, true),
            AssetCoverageBits(CONTENT_T6)
        );
        let both = content_flags_for(true, true);
        assert_eq!(both.0, CONTENT_T5 | CONTENT_T6);
        assert_eq!(content_flags_for(true, false).0, CONTENT_T5);
    }

    #[test]
    fn an_install_with_no_trees_offers_nothing() {
        assert_eq!(pack_presence_bits(&CategoryTreeSet::default()), AssetCoverageBits(0));
    }

    #[test]
    fn a_first_screen_shows_only_the_layers_it_names() {
        let mut app = App::new();
        insert_screen(
            &mut app,
            ShellScreen::Loading,
            [OverlayTier::Loading, OverlayTier::Overlay],
        );
        let layers = app.world().resource::<OverlayTierSet>();
        assert!(layers.is_raised(OverlayTier::Loading));
        assert!(layers.is_raised(OverlayTier::Overlay));
        assert!(!layers.is_raised(OverlayTier::Shell));
        assert!(app.world().contains_resource::<BootConsoleScript>());
        assert!(app.world().contains_resource::<LevelStage>());
        let clear = app.world().resource::<ClearColor>();
        assert_eq!(clear.0, Color::BLACK);
    }

    #[test]
    fn a_master_intent_that_failed_falls_back_to_none() {
        let off = intent_or_off::<String>("master test", Err("no address".into()));
        assert!(!off.enabled());
        let kept = intent_or_off::<String>("master test", Ok(net::DirectoryLaunchRequest::disabled()));
        assert!(!kept.enabled());
    }

    #[test]
    fn a_capture_goes_in_the_queue_the_render_plugin_made() {
        let mut app = App::new();
        app.init_resource::<ScreenshotBacklog>();
        queue_capture(
            &mut app,
            ScreenshotOrder {
                path: PathBuf::from("shot.png"),
                quit_when_shot_taken: false,
            },
        );
        assert_eq!(app.world().resource::<ScreenshotBacklog>().pending(), 1);
    }

    #[test]
    #[should_panic(expected = "render plugin")]
    fn a_capture_before_the_render_plugin_is_a_wiring_mistake() {
        let mut app = App::new();
        queue_capture(
            &mut app,
            ScreenshotOrder {
                path: PathBuf::from("shot.png"),
                quit_when_shot_taken: false,
            },
        );
    }
}
