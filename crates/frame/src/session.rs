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

pub use keys::{AdmissionKey, LocalLoadKey, MatchKey, WorldGeneration, WorldProducts};
pub use lifecycle::{
    LifeEndCause, LifeEnded, LifeStartReason, LifeStarted, MapLoadApproved, MapLoadFailed,
    MatchInstalled, MatchTornDown, ReturnedToMenu, TeardownReason,
};
pub use loadout::{CacWeaponOffer, ClassSelectHandoff, HostClassLoadouts, HostClassSlot};
pub use view::{BotNavigationReady, HudInputView, UiCamera, ViewSubject};

/// Which halves of the engine this process runs.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeRole {
    /// A player hosting a match they also play in.
    Listen,
    /// A server with no local player.
    Dedicated,
    /// A player connected to someone else's match.
    Client,
    /// Playback of a recorded match.
    Replay,
}

impl RuntimeRole {
    pub const fn runs_authority(self) -> bool {
        matches!(self, Self::Listen | Self::Dedicated)
    }

    pub const fn runs_client(self) -> bool {
        matches!(self, Self::Listen | Self::Client | Self::Replay)
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
pub enum AppScreen {
    #[default]
    MainMenu,
    Loading,
    ClassSelect,
    InGame,
}

/// Whether the UI is drawn this frame. On unless something turns it off.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiDraw(pub bool);

impl Default for UiDraw {
    fn default() -> Self {
        Self(true)
    }
}

/// How this process was launched.
#[derive(Resource, Clone, Debug, Default)]
pub struct LaunchIdentity {
    pub role_label: String,
    pub games_root: PathBuf,
    pub artifacts: PathBuf,
    pub zone: String,
}

/// What the last map-load attempt found, for the diagnostics screen.
#[derive(Resource, Clone, Debug)]
pub struct LaunchReport {
    pub zone: String,
    pub common_mp: Result<PathBuf, String>,
    pub zone_ff: Result<PathBuf, String>,
    pub zone_alias: Option<String>,
    pub sim_gap: &'static str,
    pub prediction_metrics: Option<String>,
    pub world_report: Vec<String>,
}

impl Default for LaunchReport {
    fn default() -> Self {
        let not_requested = || Err("not requested".to_owned());
        Self {
            zone: String::new(),
            common_mp: not_requested(),
            zone_ff: not_requested(),
            zone_alias: None,
            sim_gap: "no map load yet",
            prediction_metrics: None,
            world_report: Vec::new(),
        }
    }
}

/// Whether a loaded world is installed.
#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct HasWorld(pub bool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_runs_on_listen_and_dedicated_only() {
        let runs: Vec<bool> = [
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
            RuntimeRole::Client,
            RuntimeRole::Replay,
        ]
        .map(RuntimeRole::runs_authority)
        .into();
        assert_eq!(runs, [true, true, false, false]);
    }

    #[test]
    fn the_client_half_runs_everywhere_but_on_a_dedicated_server() {
        let runs: Vec<bool> = [
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
            RuntimeRole::Client,
            RuntimeRole::Replay,
        ]
        .map(RuntimeRole::runs_client)
        .into();
        assert_eq!(runs, [true, false, true, true]);
    }

    #[test]
    fn every_role_runs_at_least_one_half() {
        for role in [
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
            RuntimeRole::Client,
            RuntimeRole::Replay,
        ] {
            assert!(role.runs_authority() || role.runs_client(), "{role:?}");
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
        assert_eq!(AppScreen::default(), AppScreen::MainMenu);
        assert_eq!(UiDraw::default(), UiDraw(true));
        assert_eq!(HasWorld::default(), HasWorld(false));
    }

    #[test]
    fn a_fresh_launch_report_says_nothing_was_requested() {
        let report = LaunchReport::default();
        assert_eq!(report.common_mp, Err("not requested".to_owned()));
        assert_eq!(report.zone_ff, Err("not requested".to_owned()));
        assert_eq!(report.sim_gap, "no map load yet");
        assert!(report.zone.is_empty() && report.zone_alias.is_none());
        assert!(report.prediction_metrics.is_none() && report.world_report.is_empty());
    }

    #[test]
    fn a_fresh_launch_identity_is_blank() {
        let identity = LaunchIdentity::default();
        assert!(identity.role_label.is_empty() && identity.zone.is_empty());
        assert!(identity.games_root.as_os_str().is_empty());
        assert!(identity.artifacts.as_os_str().is_empty());
    }
}
