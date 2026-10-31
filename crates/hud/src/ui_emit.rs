use bevy::ui::Display;
use bevy::ui::Node;

/// Sets a node's display mode, touching it only when the mode really changes
/// so bevy's change detection stays quiet on idle frames.
pub(crate) fn take_over_monitor(node: &mut Node, wanted: Display) {
    if node.display == wanted {
        return;
    }
    node.display = wanted;
}
