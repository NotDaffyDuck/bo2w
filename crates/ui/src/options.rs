//! Player-facing settings that reach into the window and the key table.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use frame::{DisplayResolution, GameSettings};

/// The key table as the options menu shows it.
#[derive(Resource, Clone, Debug, Default)]
pub struct BindingView {
    /// Command id to the text of the key bound to it.
    pub chords: BTreeMap<u32, String>,
    /// The command waiting for its next keypress, if any.
    pub listening: Option<u32>,
    pub revision: u64,
}

impl BindingView {
    pub fn chord(&self, command_id: u32) -> &str {
        self.chords.get(&command_id).map_or("UNBOUND", String::as_str)
    }
}

/// Forces a present mode regardless of the vsync setting.
#[derive(Resource)]
pub struct PresentModeOverride(pub PresentMode);

/// Pushes resolution, fullscreen and present mode onto the main window, but
/// only when one of them differs from what was last applied.
pub(crate) fn apply_window_settings(
    settings: Res<GameSettings>,
    forced: Option<Res<PresentModeOverride>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut last: Local<Option<(DisplayResolution, bool, PresentMode)>>,
) {
    let present_mode = match forced {
        Some(forced) => forced.0,
        None if settings.vsync => PresentMode::AutoVsync,
        None => PresentMode::AutoNoVsync,
    };
    let wanted = (settings.resolution, settings.fullscreen, present_mode);
    if *last == Some(wanted) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };

    let DisplayResolution { width, height } = settings.resolution;
    window.resolution.set_physical_resolution(width, height);
    window.mode = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    window.present_mode = present_mode;
    *last = Some(wanted);
}
