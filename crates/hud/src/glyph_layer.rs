use bevy::prelude::*;
use bevy::ui::Display;
use bevy::ui::FocusPolicy;
use bevy::ui::PositionType;

use crate::draw_batch::ListLatch;

/// Adds a full-window, initially hidden node under `parent`. The tessellated
/// text of one HUD layer is drawn over it; `layer` says which layer.
pub(crate) fn spawn_layer_node(parent: &mut ChildSpawnerCommands, layer: impl Bundle) {
    let full_window = Node {
        position_type: PositionType::Absolute,
        display: Display::None,
        width: Val::Percent(100.0_f32),
        height: Val::Percent(100.0_f32),
        ..default()
    };
    parent.spawn((layer, ListLatch::default(), full_window, FocusPolicy::Pass));
}
