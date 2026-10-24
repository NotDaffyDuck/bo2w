//! Notifications the game scripts raise for the app layer.

use std::time::Instant;

use bevy::prelude::*;

/// The scripts asked to leave the level.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitLevelCalled;

/// Since when the end-of-match scoreboard has been on screen (written by the
/// UI). The match returns to the front end only after the board has been up
/// for the scripts' intermission hold.
#[derive(Resource, Default, Debug)]
pub struct EndBoardShown(pub Option<Instant>);

pub fn register_script_notify(app: &mut App) {
    app.add_message::<ExitLevelCalled>()
        .init_resource::<EndBoardShown>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_adds_the_message_and_an_unset_board_timer() {
        let mut app = App::new();
        register_script_notify(&mut app);
        assert!(app.world().resource::<EndBoardShown>().0.is_none());
        app.world_mut()
            .resource_mut::<Messages<ExitLevelCalled>>()
            .write(ExitLevelCalled);
        assert_eq!(app.world().resource::<Messages<ExitLevelCalled>>().len(), 1);
    }

    #[test]
    fn registering_twice_keeps_the_existing_timer() {
        let mut app = App::new();
        register_script_notify(&mut app);
        let shown = Instant::now();
        app.world_mut().resource_mut::<EndBoardShown>().0 = Some(shown);
        register_script_notify(&mut app);
        assert_eq!(app.world().resource::<EndBoardShown>().0, Some(shown));
    }
}
