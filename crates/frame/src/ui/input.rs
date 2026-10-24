//! Key binding requests from the menu UI.

use bevy::prelude::{Message, Resource};

/// The binding screen is waiting for a key for `command` (none: not waiting).
#[derive(Resource, Debug, Default)]
pub struct UiBindingCapture {
    pub command: Option<String>,
    /// The press that was captured must not also act as a game input.
    pub consumed_input: bool,
}

/// Start capturing a key for `command`.
#[derive(Message, Clone, Debug)]
pub struct UiBindRequest {
    pub command: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_captured_by_default() {
        let capture = UiBindingCapture::default();
        assert!(capture.command.is_none());
        assert!(!capture.consumed_input);
    }
}
