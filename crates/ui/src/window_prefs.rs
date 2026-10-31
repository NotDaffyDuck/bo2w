//! Settings from the options screens that touch the OS window and the key bindings.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use crate::MainWindowMut;
use frame::{PlayerPrefsBundle, ScreenPixelSize};

/// Key bindings in the shape the options screen draws them.
#[derive(Resource, Clone, Debug, Default)]
pub struct KeymapSnapshot {
    /// Bound key text for each command id.
    pub key_combos: BTreeMap<u32, String>,
    /// A command that is waiting to be given a key, when the player is rebinding one.
    pub listening: Option<u32>,
    pub revision: u64,
}

impl KeymapSnapshot {
    pub fn key_combo(&self, cmd: u32) -> &str {
        match self.key_combos.get(&cmd) {
            Some(text) => text,
            None => "UNBOUND",
        }
    }
}

/// When present, this present mode wins over whatever the vsync option says.
#[derive(Resource)]
pub struct SwapChainPacingChoice(pub PresentMode);

/// What this system last pushed to the window: its size, borderless or not, and pacing.

/// True for a test run that keeps its window hidden. Such a run stays windowed so a
/// borderless full-screen window never ends up covering the player's own game.
fn hidden_window_run() -> bool {
    matches!(std::env::var("BO2MP_WINDOW_HIDDEN").as_deref(), Ok("1"))
}

/// Writes size, full screen and present mode to the primary window, skipping frames where
/// none of the three changed since the last write.
pub(crate) fn apply_window_prefs(
    prefs: Res<PlayerPrefsBundle>,
    pinned: Option<Res<SwapChainPacingChoice>>,
    mut primary: MainWindowMut,
    mut applied: Local<Option<(ScreenPixelSize, bool, PresentMode)>>,
) {
    let pacing = if let Some(pin) = pinned {
        pin.0
    } else if prefs.sync_frames {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    let target: (ScreenPixelSize, bool, PresentMode) = (prefs.resolution, prefs.full_screen, pacing);
    if applied.as_ref() == Some(&target) {
        return;
    }
    let Some(mut win) = primary.shown_mut() else {
        return;
    };
    win.resolution.set_physical_resolution(prefs.resolution.width, prefs.resolution.height);
    let borderless = prefs.full_screen && !hidden_window_run();
    win.mode = match borderless {
        true => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        false => WindowMode::Windowed,
    };
    win.present_mode = pacing;
    *applied = Some(target);
}
