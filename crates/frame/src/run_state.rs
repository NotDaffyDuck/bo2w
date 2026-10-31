//! What kind of process this is, which screen it is on, and the identity of
//! the match it is in. The match keys, lifecycle messages, class loadout
//! types and view state live in submodules and are re-exported from here.

use std::path::PathBuf;
use std::sync::OnceLock;

use bevy::prelude::*;

mod keys;
mod lifecycle;
mod loadout;
mod view;

pub use keys::{EntryPassToken, OwnLevelLoadTicket, GameSessionStamp, LevelGeneration, WorldProducts};
pub use lifecycle::DeathCircumstance;
pub use lifecycle::SpawnLifeOver;
pub use lifecycle::SpawnCause;
pub use lifecycle::SpawnLifeBegun;
pub use lifecycle::LevelOpenGranted;
pub use lifecycle::LevelOpenRejected;
pub use lifecycle::GameSessionArmed;
pub use lifecycle::GameSessionClosed;
pub use lifecycle::ReturnedToMenu;
pub use lifecycle::ShutdownCause;
pub use loadout::{CustomClassGunChoice, KitPickerPassover, ServerKitRoster, ServerKitSlot};
pub use view::BotNavigationReady;
pub use view::OverlayControlSnapshot;
pub use view::UiCamera;
pub use view::EyeSubject;

/// Which halves of the engine this process runs.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessDuty {
    /// A player hosting a match they also play in.
    Listen,
    /// A server with no local player.
    Dedicated,
    /// A player connected to someone else's match.
    Client,
    /// Playback of a recorded match.
    Replay,
}

impl ProcessDuty {
    pub const fn hosts_simulation(self) -> bool {
        let duty = self;
        matches!(duty, Self::Dedicated | Self::Listen)
    }

    pub const fn hosts_player_view(self) -> bool {
        let duty = self;
        matches!(duty, Self::Client | Self::Listen | Self::Replay)
    }
}

/// A run with no window, no graphics device and no sound device: the server,
/// its scripts, the bots and the client's game logic all run, nothing is drawn
/// or heard.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Headless;

impl Headless {
    const ENV: &'static str = "BO2MP_HEADLESS";

    /// Whether the environment asks for a headless run (`BO2MP_HEADLESS=1`).
    /// Read once; later changes to the environment are not seen.
    pub fn requested() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| Self::enabled_by(std::env::var(Self::ENV).ok().as_deref()))
    }

    fn enabled_by(value: Option<&str>) -> bool {
        value == Some("1")
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShellScreen {
    #[default]
    MainMenu,
    Loading,
    KitPicker,
    InGame,
}

/// Whether the UI is drawn this frame. On unless something turns it off.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuPaintPass(pub bool);

impl Default for MenuPaintPass {
    fn default() -> Self {
        Self(true)
    }
}

/// How this process was launched.
#[derive(Resource, Clone, Debug, Default)]
pub struct StartupPersona {
    pub duty_caption: String,
    pub titles_install_dir: PathBuf,
    pub artifacts: PathBuf,
    pub zone: String,
}

/// What the last map-load attempt found, for the diagnostics screen.
#[derive(Resource, Clone, Debug)]
pub struct StartupReport {
    pub zone: String,
    pub shared_zone: Result<PathBuf, String>,
    pub zone_archive_file: Result<PathBuf, String>,
    pub pack_nickname: Option<String>,
    pub stepping_lag: &'static str,
    pub forecast_figures: Option<String>,
    pub level_digest: Vec<String>,
}

impl Default for StartupReport {
    fn default() -> Self {
        let not_requested = || Err("not requested".to_owned());
        Self {
            zone: String::new(),
            shared_zone: not_requested(),
            zone_archive_file: not_requested(),
            pack_nickname: None,
            stepping_lag: "no map load yet",
            forecast_figures: None,
            level_digest: Vec::new(),
        }
    }
}

/// Whether a loaded world is installed.
#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct LevelPresent(pub bool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_runs_on_listen_and_dedicated_only() {
        let runs: Vec<bool> = [
            ProcessDuty::Listen,
            ProcessDuty::Dedicated,
            ProcessDuty::Client,
            ProcessDuty::Replay,
        ]
        .map(ProcessDuty::hosts_simulation)
        .into();
        assert_eq!(runs, [true, true, false, false]);
    }

    #[test]
    fn the_client_half_runs_everywhere_but_on_a_dedicated_server() {
        let runs: Vec<bool> = [
            ProcessDuty::Listen,
            ProcessDuty::Dedicated,
            ProcessDuty::Client,
            ProcessDuty::Replay,
        ]
        .map(ProcessDuty::hosts_player_view)
        .into();
        assert_eq!(runs, [true, false, true, true]);
    }

    #[test]
    fn every_role_runs_at_least_one_half() {
        for role in [
            ProcessDuty::Listen,
            ProcessDuty::Dedicated,
            ProcessDuty::Client,
            ProcessDuty::Replay,
        ] {
            assert!(role.hosts_simulation() || role.hosts_player_view(), "{role:?}");
        }
    }

    #[test]
    fn only_the_value_one_asks_for_a_headless_run() {
        assert!(Headless::enabled_by(Some("1")));
        for other in [None, Some(""), Some("0"), Some("true"), Some("11"), Some(" 1")] {
            assert!(!Headless::enabled_by(other), "{other:?}");
        }
    }

    #[test]
    fn the_app_starts_on_the_main_menu_with_the_ui_drawn() {
        assert_eq!(ShellScreen::default(), ShellScreen::MainMenu);
        assert_eq!(MenuPaintPass::default(), MenuPaintPass(true));
        assert_eq!(LevelPresent::default(), LevelPresent(false));
    }

    #[test]
    fn a_fresh_launch_report_says_nothing_was_requested() {
        let report = StartupReport::default();
        assert_eq!(report.shared_zone, Err("not requested".to_owned()));
        assert_eq!(report.zone_archive_file, Err("not requested".to_owned()));
        assert_eq!(report.stepping_lag, "no map load yet");
        assert!(report.zone.is_empty() && report.pack_nickname.is_none());
        assert!(report.forecast_figures.is_none() && report.level_digest.is_empty());
    }

    #[test]
    fn a_fresh_launch_identity_is_blank() {
        let identity = StartupPersona::default();
        assert!(identity.duty_caption.is_empty() && identity.zone.is_empty());
        assert!(identity.titles_install_dir.as_os_str().is_empty());
        assert!(identity.artifacts.as_os_str().is_empty());
    }
}
