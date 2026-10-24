//! Which controller is active and how the UI should label its buttons.

use bevy::prelude::{Entity, Message, Resource};

/// The gamepad entity input currently comes from, if any.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActivePad(pub Option<Entity>);

/// Which glyph set button prompts use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptStyle {
    Xbox,
    PlayStation,
    #[default]
    Generic,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDevices {
    /// Prompts show controller glyphs instead of key names.
    pub pad_prompts: bool,
    pub style: PromptStyle,
    /// The window has input focus.
    pub focused: bool,
    pub aiming_with_pad: bool,
}

/// Asks the input layer to buzz the active pad once, for the options screen.
#[derive(Message)]
pub struct TestControllerRumble;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_active_or_focused_by_default() {
        assert_eq!(ActivePad::default(), ActivePad(None));
        let devices = InputDevices::default();
        assert!(!devices.pad_prompts && !devices.focused && !devices.aiming_with_pad);
    }

    #[test]
    fn prompts_default_to_the_generic_glyph_set() {
        assert_eq!(PromptStyle::default(), PromptStyle::Generic);
        assert_eq!(InputDevices::default().style, PromptStyle::Generic);
    }

    #[test]
    fn the_rumble_request_is_a_message() {
        let mut world = bevy::prelude::World::new();
        world.init_resource::<bevy::prelude::Messages<TestControllerRumble>>();
        world
            .resource_mut::<bevy::prelude::Messages<TestControllerRumble>>()
            .write(TestControllerRumble);
        assert_eq!(
            world
                .resource::<bevy::prelude::Messages<TestControllerRumble>>()
                .len(),
            1
        );
    }
}
