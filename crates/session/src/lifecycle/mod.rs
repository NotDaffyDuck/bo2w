//! The life of a session: which match is on screen, how one is swapped for the
//! next, and how it is taken down.
//!
//! The swap is the one way a world appears or goes away. Everything that wants
//! one (the menu, the master lobby, the end of a match, a script's `map()`
//! call) asks `SessionSwapRequest`, and `run_session_swap` carries it out in
//! order: tear down what runs, load what is next, report how it went.

mod exit;
mod follow;
mod peer;
mod swap;
mod teardown;

use bevy::prelude::*;
use frame::{
    MapLoadApproved, MapLoadFailed, MatchInstalled, MatchTornDown, Retiring, ReturnedToMenu,
    WorldGeneration,
};
use net::ClientSet;

pub use swap::{SessionSwapCompletion, SessionSwapRequest, SessionSwapResult, SessionSwapTarget};
pub use teardown::{LiveWorldIdentity, TeardownGaps, TeardownRequest};

pub(crate) fn install(app: &mut App) {
    app.init_resource::<TeardownRequest>()
        .init_resource::<frame::UiMenuDvars>()
        .init_resource::<SessionSwapRequest>()
        .init_resource::<TeardownGaps>()
        .init_resource::<WorldGeneration>()
        .init_resource::<frame::WorldProducts>()
        .init_resource::<LiveWorldIdentity>()
        .init_resource::<Retiring>()
        .add_message::<MapLoadApproved>()
        .add_message::<MapLoadFailed>()
        .add_message::<MatchInstalled>()
        .add_message::<MatchTornDown>()
        .add_message::<ReturnedToMenu>()
        .configure_sets(Update, frame::SessionSwapApplied.in_set(ClientSet::Load))
        .add_systems(
            Update,
            (
                follow::follow_master_match,
                exit::run_menu_requests,
                exit::run_exit_level,
                peer::run_peer_lobby_return,
                peer::run_udp_peer_lobby_return,
                teardown::run_teardown,
                swap::run_session_swap,
            )
                .chain()
                .in_set(frame::SessionSwapApplied),
        )
        .add_systems(
            Update,
            teardown::stamp_loaded_zone
                .after(crate::apply_prepared_match)
                .in_set(ClientSet::Load),
        )
        .add_systems(Update, teardown::retire_handed_over.in_set(ClientSet::Diag));
}
