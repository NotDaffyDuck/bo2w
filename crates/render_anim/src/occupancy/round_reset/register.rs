//! Wiring for the match reset.

use bevy::prelude::*;
use frame::MatchSwapDone;
use net::ClientStep;

use super::reset::reset_anim_at_match_start;
use super::phys_clear::clear_dyn_ent_phys_at_teardown;

/// Add both reset systems to the client load stage.
///
/// The physics reset waits until the session swap has been applied, so it
/// cannot run against the world that is being replaced.
pub(crate) fn register_match_reset_steps(application: &mut App) {
    application.add_systems(Update, reset_anim_at_match_start.in_set(ClientStep::Load));
    application.add_systems(
        Update,
        clear_dyn_ent_phys_at_teardown
            .after(MatchSwapDone)
            .in_set(ClientStep::Load),
    );
}
