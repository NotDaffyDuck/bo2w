//! The window a launch opens, and the environment variables that steer it
//! during test runs.

use bevy::{
    math::IVec2,
    window::{ExitCondition, PresentMode, Window, WindowPlugin, WindowPosition},
};
use render::checks::acceptance::{TRIAL_RESOLUTION_ROWS, SMOKE_CHECK_SWAP_POLICY, TRIAL_RESOLUTION_COLUMNS};

use super::Role;

/// Names one of Bevy's present modes for a run that is not an acceptance run.
const PRESENT_MODE_ENV: &str = "BO2MP_PRESENT_MODE";
/// `x,y`: the screen position a test run's window opens at.
const POSITION_ENV: &str = "BO2MP_WINDOW_POS";
/// `1`: a test run whose window never shows.
const HIDDEN_ENV: &str = "BO2MP_WINDOW_HIDDEN";

/// The size every window opens at; the options screen resizes it afterwards.
const OPEN_SIZE: (u32, u32) = (TRIAL_RESOLUTION_COLUMNS, TRIAL_RESOLUTION_ROWS);

/// Where a test run's window goes and whether it shows. A run on another
/// monitor, or one with nothing to look at, must not take the keyboard from
/// whatever is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TestWindow {
    position: Option<IVec2>,
    unseen: bool,
}

impl TestWindow {
    pub(super) fn from_env() -> Self {
        Self::from_values(
            std::env::var(POSITION_ENV).ok().as_deref(),
            std::env::var(HIDDEN_ENV).ok().as_deref(),
        )
    }

    fn from_values(position: Option<&str>, unseen: Option<&str>) -> Self {
        Self {
            position: position.and_then(parse_position),
            unseen: unseen == Some("1"),
        }
    }

    fn placed(self) -> bool {
        self.position.is_some()
    }

    fn apply(self, window: &mut Window) {
        if let Some(at) = self.position {
            window.position = WindowPosition::At(at);
        }
        window.focused = self.position.is_none() && !self.unseen;
        window.visible = !self.unseen;
    }
}

/// Whether the environment asks for a window that never shows.
pub(crate) fn test_window_hidden() -> bool {
    TestWindow::from_env().unseen
}

/// Whether the environment places the window at a screen position.
pub(crate) fn test_window_placed() -> bool {
    TestWindow::from_env().placed()
}

fn parse_position(text: &str) -> Option<IVec2> {
    let (x, y) = text.split_once(',')?;
    Some(IVec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// The window of Black Ops II's menus.
pub(super) fn menu_window() -> Window {
    let mut window = Window {
        title: "bo2mp — Black Ops II".into(),
        resolution: OPEN_SIZE.into(),
        ..Window::default()
    };
    TestWindow::from_env().apply(&mut window);
    window
}

/// The window of a match on `zone`. `present_mode` is the one the launch
/// forces, if it forces one.
pub(super) fn match_window(role: Role, zone: &str, present_mode: Option<PresentMode>) -> Window {
    let mut window = Window {
        title: match_title(role, zone),
        resolution: OPEN_SIZE.into(),
        ..Window::default()
    };
    if let Some(mode) = present_mode {
        window.present_mode = mode;
    }
    TestWindow::from_env().apply(&mut window);
    window
}

fn match_title(role: Role, zone: &str) -> String {
    match role {
        Role::Replay => format!("bo2mp — play {zone}"),
        Role::Client => format!("bo2mp — join {zone}"),
        Role::Listen | Role::Dedicated => format!("bo2mp — {zone}"),
    }
}

/// The window plugin for a launch. With no window the process has nothing to
/// close, so it runs until it is stopped.
pub(super) fn window_plugin(primary_window: Option<Window>) -> WindowPlugin {
    WindowPlugin {
        exit_condition: if primary_window.is_some() {
            ExitCondition::OnAllClosed
        } else {
            ExitCondition::DontExit
        },
        primary_window,
        ..WindowPlugin::default()
    }
}

/// The present mode a launch insists on, or `None` to leave it to the options
/// screen. Acceptance runs measure uncapped; anything else may name a mode in
/// the environment.
pub(super) fn forced_present_mode(signoff_run: bool) -> Option<PresentMode> {
    choose_present_mode(signoff_run, std::env::var(PRESENT_MODE_ENV).ok().as_deref())
}

fn choose_present_mode(signoff_run: bool, requested: Option<&str>) -> Option<PresentMode> {
    if signoff_run {
        return Some(SMOKE_CHECK_SWAP_POLICY);
    }
    let name = requested?;
    Some(present_mode_named(name).unwrap_or_else(|| {
        diag::warn!(
            Launch,
            "{PRESENT_MODE_ENV}={name} names no present mode; using the default"
        );
        PresentMode::Fifo
    }))
}

fn present_mode_named(name: &str) -> Option<PresentMode> {
    Some(match name {
        "AutoNoVsync" => PresentMode::AutoNoVsync,
        "Immediate" => PresentMode::Immediate,
        "Mailbox" => PresentMode::Mailbox,
        "Fifo" => PresentMode::Fifo,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_is_two_whole_numbers() {
        assert_eq!(parse_position("1920,0"), Some(IVec2::new(1920, 0)));
        assert_eq!(parse_position(" -1280 , 40 "), Some(IVec2::new(-1280, 40)));
        for text in ["", "5", "5,", ",5", "a,b", "1.5,2", "1,2,3"] {
            assert_eq!(parse_position(text), None, "{text:?}");
        }
    }

    #[test]
    fn only_one_hides_the_window() {
        assert!(TestWindow::from_values(None, Some("1")).unseen);
        for text in [None, Some(""), Some("0"), Some("true"), Some("11")] {
            assert!(!TestWindow::from_values(None, text).unseen, "{text:?}");
        }
    }

    #[test]
    fn a_window_is_placed_only_by_a_position_that_parses() {
        assert!(TestWindow::from_values(Some("0,0"), None).placed());
        assert!(TestWindow::from_values(Some("2000,100"), Some("1")).placed());
        assert!(!TestWindow::from_values(Some("left"), None).placed());
        assert!(!TestWindow::from_values(None, Some("1")).placed());
        assert!(!TestWindow::default().placed());
    }

    #[test]
    fn a_plain_launch_leaves_the_window_to_the_system() {
        let mut window = Window::default();
        TestWindow::from_values(None, None).apply(&mut window);
        assert_eq!(window.position, WindowPosition::Automatic);
        assert!(window.focused && window.visible);
    }

    #[test]
    fn a_placed_window_opens_without_the_keyboard() {
        let mut window = Window::default();
        TestWindow::from_values(Some("2000,100"), None).apply(&mut window);
        assert_eq!(window.position, WindowPosition::At(IVec2::new(2000, 100)));
        assert!(!window.focused);
        assert!(window.visible);
    }

    #[test]
    fn a_hidden_window_is_neither_shown_nor_focused() {
        let mut window = Window::default();
        TestWindow::from_values(None, Some("1")).apply(&mut window);
        assert!(!window.visible);
        assert!(!window.focused);
    }

    #[test]
    fn a_position_that_does_not_parse_is_no_position() {
        let mut window = Window::default();
        TestWindow::from_values(Some("left"), None).apply(&mut window);
        assert_eq!(window.position, WindowPosition::Automatic);
        assert!(window.focused);
    }

    #[test]
    fn the_title_says_what_the_window_is_for() {
        assert_eq!(match_title(Role::Listen, "mp_raid"), "bo2mp — mp_raid");
        assert_eq!(match_title(Role::Dedicated, "mp_raid"), "bo2mp — mp_raid");
        assert_eq!(match_title(Role::Client, "mp_raid"), "bo2mp — join mp_raid");
        assert_eq!(match_title(Role::Replay, "mp_raid"), "bo2mp — play mp_raid");
    }

    #[test]
    fn windows_open_at_the_launch_size() {
        for window in [menu_window(), match_window(Role::Listen, "mp_raid", None)] {
            assert_eq!(window.resolution.width(), TRIAL_RESOLUTION_COLUMNS as f32);
            assert_eq!(window.resolution.height(), TRIAL_RESOLUTION_ROWS as f32);
        }
    }

    #[test]
    fn a_match_window_takes_the_forced_present_mode_only_when_given() {
        let forced = match_window(Role::Listen, "z", Some(PresentMode::Mailbox));
        assert_eq!(forced.present_mode, PresentMode::Mailbox);
        let left = match_window(Role::Listen, "z", None);
        assert_eq!(left.present_mode, Window::default().present_mode);
    }

    #[test]
    fn no_window_means_nothing_closing_ends_the_process() {
        let headless = window_plugin(None);
        assert!(headless.primary_window.is_none());
        assert!(matches!(headless.exit_condition, ExitCondition::DontExit));
        let shown = window_plugin(Some(Window::default()));
        assert!(shown.primary_window.is_some());
        assert!(matches!(shown.exit_condition, ExitCondition::OnAllClosed));
    }

    #[test]
    fn four_present_modes_have_names() {
        assert_eq!(present_mode_named("Fifo"), Some(PresentMode::Fifo));
        assert_eq!(present_mode_named("Mailbox"), Some(PresentMode::Mailbox));
        assert_eq!(
            present_mode_named("Immediate"),
            Some(PresentMode::Immediate)
        );
        assert_eq!(
            present_mode_named("AutoNoVsync"),
            Some(PresentMode::AutoNoVsync)
        );
        for name in ["", "fifo", "AutoVsync", "Vsync"] {
            assert_eq!(present_mode_named(name), None, "{name:?}");
        }
    }

    #[test]
    fn acceptance_runs_measure_uncapped_whatever_the_environment_says() {
        assert_eq!(
            choose_present_mode(true, Some("Fifo")),
            Some(SMOKE_CHECK_SWAP_POLICY)
        );
        assert_eq!(
            choose_present_mode(true, None),
            Some(SMOKE_CHECK_SWAP_POLICY)
        );
    }

    #[test]
    fn other_runs_force_a_present_mode_only_when_one_is_named() {
        assert_eq!(choose_present_mode(false, None), None);
        assert_eq!(
            choose_present_mode(false, Some("Immediate")),
            Some(PresentMode::Immediate)
        );
        assert_eq!(
            choose_present_mode(false, Some("bogus")),
            Some(PresentMode::default())
        );
    }
}
