use bevy::prelude::*;

use crate::match_load_walk::add_match_load_systems;
use crate::session_teardown::hook_round_cleanup;

/// Brings in the systems that load a match's assets and put them away again.
pub struct ZoneAssetPlugin;

impl Plugin for ZoneAssetPlugin {
    fn build(&self, app: &mut App) {
        add_match_load_systems(app);
        hook_round_cleanup(app);
    }
}
