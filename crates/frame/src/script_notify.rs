//! Notifications the game scripts raise for the app layer.

use std::time::Instant;

use bevy::prelude::*;

/// The scripts asked to leave the level.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelQuitRequested;

/// Since when the end-of-match scoreboard has been on screen (written by the
/// UI). The match returns to the front end only after the board has been up
/// for the scripts' intermission hold.
#[derive(Resource, Default, Debug)]
pub struct EndBoardShown(pub Option<Instant>);

pub fn hook_script_signal(app: &mut App) {
    app.add_message::<LevelQuitRequested>()
        .init_resource::<EndBoardShown>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_adds_the_message_and_an_unset_board_timer() {
        let mut app = App::new();
        hook_script_signal(&mut app);
        assert!(app.world().resource::<EndBoardShown>().0.is_none());
        app.world_mut()
            .resource_mut::<Messages<LevelQuitRequested>>()
            .write(LevelQuitRequested);
        assert_eq!(app.world().resource::<Messages<LevelQuitRequested>>().len(), 1);
    }

    #[test]
    fn registering_twice_keeps_the_existing_timer() {
        let mut app = App::new();
        hook_script_signal(&mut app);
        let shown = Instant::now();
        app.world_mut().resource_mut::<EndBoardShown>().0 = Some(shown);
        hook_script_signal(&mut app);
        assert_eq!(app.world().resource::<EndBoardShown>().0, Some(shown));
    }
}
