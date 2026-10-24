use bevy::prelude::*;

/// Everything the session crate adds to the app.
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        crate::lifecycle::install(app);
        crate::match_apply::install(app);
        crate::admission::install(app);
        crate::local_arm::install(app);
        crate::life_front::install(app);
        crate::view_subject::install(app);
    }
}
