use bevy::prelude::Node;
use bevy::ui::Display;

/// Sets a node's display mode, touching it only when the mode really changes
/// so bevy's change detection stays quiet on idle frames.
pub(crate) fn adopt_display(node: &mut Node, wanted: Display) {
    if node.display == wanted {
        return;
    }
    node.display = wanted;
}
