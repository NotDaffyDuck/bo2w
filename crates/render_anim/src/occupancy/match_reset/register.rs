//! Wiring for the match reset.

use bevy::prelude::*;
use frame::SessionSwapApplied;
use net::ClientSet;

use super::reset::reset_anim_for_match;
use super::teardown::reset_dyn_ent_phys_on_teardown;

/// Add both reset systems to the client load stage.
///
/// The physics reset waits until the session swap has been applied, so it
/// cannot run against the world that is being replaced.
pub(crate) fn register_match_reset_systems(app: &mut App) {
    app.add_systems(Update, reset_anim_for_match.in_set(ClientSet::Load));
    app.add_systems(
        Update,
        reset_dyn_ent_phys_on_teardown
            .after(SessionSwapApplied)
            .in_set(ClientSet::Load),
    );
}
