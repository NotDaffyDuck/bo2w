use bevy::prelude::*;
use bevy::ui::{Display, FocusPolicy};

use crate::gpu_list::GpuListLatch;

/// Adds a full-window, initially hidden node under `root`. The tessellated
/// text of one HUD layer is drawn over it; `marker` says which layer.
pub(crate) fn spawn_overlay(root: &mut ChildSpawnerCommands, marker: impl Bundle) {
    let full_window = Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    };
    root.spawn((marker, GpuListLatch::default(), full_window, FocusPolicy::Pass));
}
