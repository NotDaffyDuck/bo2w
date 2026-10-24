//! Contracts between the menu UI and the rest of the app: the messages it
//! sends and the resources it shares.

use bevy::prelude::*;

pub mod audio;
pub mod input;
pub mod menu;

pub use audio::{UiPlayMusic, UiPlaySound, UiStopMusic};
pub use input::{UiBindRequest, UiBindingCapture};
pub use menu::{
    GamePaused, HostMatchRules, LuiMenus, MenuDims, UiExecCommand, UiMenuDvars, UiPartyState,
    WorldBlur, WorldDim,
};

/// Adds every UI resource and message type to the app.
pub fn register_ui_contracts(app: &mut App) {
    app.init_resource::<UiMenuDvars>()
        .init_resource::<LuiMenus>()
        .init_resource::<GamePaused>()
        .init_resource::<WorldBlur>()
        .init_resource::<WorldDim>()
        .init_resource::<MenuDims>()
        .init_resource::<UiBindingCapture>()
        .init_resource::<UiPartyState>()
        .add_message::<UiBindRequest>()
        .add_message::<UiPlaySound>()
        .add_message::<UiPlayMusic>()
        .add_message::<UiStopMusic>()
        .add_message::<UiExecCommand>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_adds_every_resource() {
        let mut app = App::new();
        register_ui_contracts(&mut app);
        let world = app.world();
        assert!(world.contains_resource::<UiMenuDvars>());
        assert!(world.contains_resource::<LuiMenus>());
        assert!(world.contains_resource::<GamePaused>());
        assert!(world.contains_resource::<WorldBlur>());
        assert!(world.contains_resource::<WorldDim>());
        assert!(world.contains_resource::<MenuDims>());
        assert!(world.contains_resource::<UiBindingCapture>());
        assert!(world.contains_resource::<UiPartyState>());
    }

    #[test]
    fn registration_adds_every_message_queue() {
        let mut app = App::new();
        register_ui_contracts(&mut app);
        let world = app.world();
        assert!(world.contains_resource::<Messages<UiBindRequest>>());
        assert!(world.contains_resource::<Messages<UiPlaySound>>());
        assert!(world.contains_resource::<Messages<UiPlayMusic>>());
        assert!(world.contains_resource::<Messages<UiStopMusic>>());
        assert!(world.contains_resource::<Messages<UiExecCommand>>());
    }

    #[test]
    fn registering_twice_keeps_set_values() {
        let mut app = App::new();
        register_ui_contracts(&mut app);
        app.world_mut().resource_mut::<MenuDims>().0 = 3;
        register_ui_contracts(&mut app);
        assert_eq!(app.world().resource::<MenuDims>().0, 3);
    }
}
