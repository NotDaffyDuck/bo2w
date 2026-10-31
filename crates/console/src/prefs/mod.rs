//! What the player has chosen: the options menu, the controls menu, and the
//! settings file that keeps both between runs.
//!
//! * `file` and `layout`: the text format, and moving older saved binds on.
//! * `persist`: where the file is, reading it at startup, saving changes.
//! * `menu`: `set ui_*` commands in, one dvar per option out.
//! * `capture` and `view`: choosing a button for a command, and the labels
//!   the controls menu shows.
//! * `sound` and `name`: the settings that reach the audio and the match.

mod capture;
mod file;
mod layout;
mod menu;
mod name;
mod persist;
mod sound;
mod view;

use bevy::{ecs::change_detection::DetectChanges, ecs::system::Local, ecs::system::Res, ecs::system::ResMut};
use frame::PlayerPrefsBundle;

pub(crate) use capture::{AwaitedMenuControl, take_menu_control};
pub(crate) use menu::native_menu_settings;
pub(crate) use name::rename_local_player;
pub(crate) use persist::{PrefsStorage, read_player_prefs, write_player_prefs};
pub(crate) use sound::set_overall_loudness;
pub(crate) use view::refresh_binding_rows;

use crate::BindingTable;

/// The preset pad layout to load into the binds, when the setting moved to one
/// from another. The first look only notes the setting: the file was loaded
/// with it, binds and all. Custom is whatever the binds already are.
fn layout_to_apply(previous: Option<u8>, now: u8) -> Option<usize> {
    let moved = previous.is_some_and(|before| before != now);
    (moved && now != PlayerPrefsBundle::PAD_LAYOUT_CUSTOM).then_some(usize::from(now))
}

pub(crate) fn apply_pad_layout_setting(
    settings: Res<PlayerPrefsBundle>,
    mut binds: ResMut<BindingTable>,
    mut seen: Local<Option<u8>>,
) {
    let now = settings.pad_layout;
    if let Some(layout) = layout_to_apply(seen.replace(now), now) {
        binds.apply_pad_layout(layout);
    }
}

pub(crate) fn push_omni_movement_setting(settings: Res<PlayerPrefsBundle>) {
    if settings.is_changed() {
        sim::set_omni_movement(settings.omni_movement);
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use crate::prefs::*;
    use bevy::app::Update;

    const CUSTOM: u8 = PlayerPrefsBundle::PAD_LAYOUT_CUSTOM;

    #[test]
    fn the_first_look_applies_nothing() {
        assert_eq!(layout_to_apply(None, 2), None);
        assert_eq!(layout_to_apply(None, CUSTOM), None);
    }

    #[test]
    fn a_move_to_a_preset_applies_it() {
        assert_eq!(layout_to_apply(Some(1), 2), Some(2));
        assert_eq!(layout_to_apply(Some(CUSTOM), 3), Some(3));
    }

    #[test]
    fn staying_put_or_going_custom_applies_nothing() {
        assert_eq!(layout_to_apply(Some(2), 2), None);
        assert_eq!(layout_to_apply(Some(2), CUSTOM), None);
        assert_eq!(layout_to_apply(Some(CUSTOM), CUSTOM), None);
    }

    fn pad_app() -> App {
        let mut app = App::default();
        app.init_resource::<PlayerPrefsBundle>()
            .init_resource::<BindingTable>()
            .add_systems(Update, apply_pad_layout_setting);
        app
    }

    fn lines(app: &App) -> Vec<String> {
        app.world().resource::<BindingTable>().control_listing()
    }

    fn lines_of_preset(layout: usize) -> Vec<String> {
        let mut binds = BindingTable::default();
        binds.apply_pad_layout(layout);
        binds.control_listing()
    }

    #[test]
    fn choosing_a_preset_in_the_menu_loads_its_pad_binds() {
        let mut app = pad_app();
        app.world_mut().resource_mut::<PlayerPrefsBundle>().pad_layout = 1;
        app.update();
        assert!(
            lines(&app).is_empty(),
            "the starting layout is left to the file"
        );
        app.world_mut().resource_mut::<PlayerPrefsBundle>().pad_layout = 2;
        app.update();
        assert_eq!(lines(&app), lines_of_preset(2));
    }

    #[test]
    fn a_hand_made_layout_keeps_its_binds() {
        let mut app = pad_app();
        app.world_mut().resource_mut::<PlayerPrefsBundle>().pad_layout = 1;
        app.update();
        app.world_mut().resource_mut::<PlayerPrefsBundle>().pad_layout = CUSTOM;
        app.update();
        assert!(lines(&app).is_empty());
    }
}
