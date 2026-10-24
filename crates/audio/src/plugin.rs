use bevy::prelude::*;

use crate::playback::PlayerSoundPlugin;

/// Installs every audio system; order matters only where noted by the
/// individual `register` functions.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        crate::backend::register(app);
        app.add_plugins(PlayerSoundPlugin);

        let registrations: [fn(&mut App); 8] = [
            crate::match_set::register,
            crate::frontend::register_frontend_audio,
            crate::entity_events::register_entity_event_audio,
            crate::rumble::register,
            crate::guest::register,
            crate::menu_volume::register,
            crate::duck::register,
            crate::master::register,
        ];
        for register in registrations {
            register(app);
        }
    }
}
