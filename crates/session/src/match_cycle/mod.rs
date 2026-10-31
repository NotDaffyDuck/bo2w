//! The life of a session: which match is on screen, how one is swapped for the
//! next, and how it is taken down.
//!
//! The swap is the one way a world appears or goes away. Everything that wants
//! one (the menu, the master lobby, the end of a match, a script's `map()`
//! call) asks `SessionHandoverOrder`, and `run_session_swap` carries it out in
//! order: tear down what runs, load what is next, report how it went.

mod exit;
mod follow;
mod peer;
mod swap;
mod takedown;

use bevy::prelude::*;
use frame::{
    LevelOpenGranted, LevelOpenRejected, GameSessionArmed, GameSessionClosed, Retiring, ReturnedToMenu,
    LevelGeneration,
};
use net::ClientStep;

pub use swap::{HandoverFinish, SessionHandoverOrder, HandoverVerdict, SessionReplacementGoal};
pub use takedown::{RunningWorldLabel, ShutdownLeftovers, ShutdownOrder};

pub(crate) fn install(host: &mut App) {
    const LOAD: ClientStep = ClientStep::Load;
    const DIAG: ClientStep = ClientStep::Diag;
    host.init_resource::<ShutdownOrder>()
        .init_resource::<frame::UiMenuDvars>()
        .init_resource::<SessionHandoverOrder>()
        .init_resource::<ShutdownLeftovers>()
        .init_resource::<LevelGeneration>()
        .init_resource::<frame::WorldProducts>()
        .init_resource::<RunningWorldLabel>()
        .init_resource::<Retiring>()
        .add_message::<LevelOpenGranted>()
        .add_message::<LevelOpenRejected>()
        .add_message::<GameSessionArmed>()
        .add_message::<GameSessionClosed>()
        .add_message::<ReturnedToMenu>()
        .configure_sets(bevy::app::Update, frame::MatchSwapDone.in_set(LOAD))
        .add_systems(
            Update,
            (
                follow::follow_master_match,
                exit::run_menu_requests,
                exit::run_exit_level,
                peer::run_peer_lobby_return,
                peer::run_udp_peer_lobby_return,
                takedown::run_teardown,
                swap::run_session_swap,
            )
                .chain()
                .in_set(frame::MatchSwapDone),
        )
        .add_systems(
            Update,
            takedown::stamp_loaded_zone
                .after(crate::install_staged_game)
                .in_set(LOAD),
        )
        .add_systems(bevy::app::Update, takedown::retire_handed_over.in_set(DIAG));
}
