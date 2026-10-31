//! Sound requests the menu UI makes.

use bevy::ecs::message::Message;

/// Play the sound alias once.
#[derive(Message, Clone, Debug)]
pub struct MenuCueStart {
    pub alias: String,
}

/// Start the music alias.
#[derive(Message, Clone, Debug)]
pub struct MenuSongStart {
    pub alias: String,
}

/// Stop whatever music is playing.
#[derive(Message, Clone, Debug, Default)]
pub struct MenuSongHalt;

#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    use super::*;

    #[test]
    fn sound_requests_queue_in_order() {
        let mut app = App::new();
        app.add_message::<MenuCueStart>();
        for alias in ["click", "back"] {
            app.world_mut().write_message(MenuCueStart {
                alias: alias.into(),
            });
        }
        let queued = app.world().resource::<Messages<MenuCueStart>>();
        let aliases: Vec<&str> = queued
            .iter_current_update_messages()
            .map(|m| m.alias.as_str())
            .collect();
        assert_eq!(aliases, ["click", "back"]);
    }

    #[test]
    fn stopping_music_needs_no_argument() {
        let mut app = App::new();
        app.add_message::<MenuSongHalt>();
        app.world_mut().write_message(MenuSongHalt);
        assert_eq!(app.world().resource::<Messages<MenuSongHalt>>().len(), 1);
    }
}
