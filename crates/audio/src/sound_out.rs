use bevy::prelude::*;

use crate::play_core::PlayerNoisePlugin;

/// Installs every audio system; order matters only where noted by the
/// individual `wire_up` functions.
pub struct SoundOutPlugin;

impl bevy::app::Plugin for SoundOutPlugin {
    fn build(&self, app: &mut App) {
        let registrations: [fn(&mut App); 10] = [
            crate::backend::install,
            |app| {
                app.add_plugins(PlayerNoisePlugin);
            },
            crate::match_sounds::wire_up,
            crate::menu_track::install_menu_sound,
            crate::ent_sounds::register_entity_event_audio,
            crate::rumble::wire_up,
            crate::guest::wire_up,
            crate::menu_volume::wire_up,
            crate::duck::wire_up,
            crate::master::wire_up,
        ];
        registrations.into_iter().for_each(|register| register(app));
    }
}
