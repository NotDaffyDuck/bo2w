use bevy::prelude::*;

/// Everything the session crate adds to the app.
pub struct LobbyLifecyclePlugin;

impl Plugin for LobbyLifecyclePlugin {
    fn build(&self, app: &mut App) {
        crate::match_cycle::install(app);
        crate::match_install::install(app);
        crate::entry_gate::install(app);
        crate::self_arm::install(app);
        crate::lives::install(app);
        crate::eye_target::install(app);
    }
}
