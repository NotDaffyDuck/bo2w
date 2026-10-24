//! Sound requests the menu UI makes.

use bevy::prelude::Message;

/// Play the sound alias once.
#[derive(Message, Clone, Debug)]
pub struct UiPlaySound {
    pub alias: String,
}

/// Start the music alias.
#[derive(Message, Clone, Debug)]
pub struct UiPlayMusic {
    pub alias: String,
}

/// Stop whatever music is playing.
#[derive(Message, Clone, Debug, Default)]
pub struct UiStopMusic;

#[cfg(test)]
mod tests {
    use bevy::prelude::*;

    use super::*;

    #[test]
    fn sound_requests_queue_in_order() {
        let mut app = App::new();
        app.add_message::<UiPlaySound>();
        for alias in ["click", "back"] {
            app.world_mut().write_message(UiPlaySound {
                alias: alias.into(),
            });
        }
        let queued = app.world().resource::<Messages<UiPlaySound>>();
        let aliases: Vec<&str> = queued
            .iter_current_update_messages()
            .map(|m| m.alias.as_str())
            .collect();
        assert_eq!(aliases, ["click", "back"]);
    }

    #[test]
    fn stopping_music_needs_no_argument() {
        let mut app = App::new();
        app.add_message::<UiStopMusic>();
        app.world_mut().write_message(UiStopMusic);
        assert_eq!(app.world().resource::<Messages<UiStopMusic>>().len(), 1);
    }
}
