//! Putting a loaded match into the running app.
//!
//! The loader hands over everything a match needs ([`assets::PreparedMatchReady`]).
//! This module checks it, builds the simulation and its content, and installs
//! the lot in one step:
//!
//! - [`plan`] checks the loaded match and chooses the game scripts, dvars and
//!   effect sets: everything that can be decided without touching the app.
//! - [`config`] reads the match settings out of the config file and the
//!   host's rules.
//! - [`t6_install`] turns the map's script set into what the simulation runs.
//! - [`content`] sets up weapons, bodies and combat facts, and the class
//!   offers.
//! - [`clip`] and [`script_models`] give the simulation its collision and the
//!   models scripts can move.
//! - [`bootstrap`] seats the first players and starts the match.
//! - [`refusal`] is how a failed step reads on the loading screen.
//! - [`apply`] is the system that drives all of it.

mod apply;
mod bootstrap;
mod clip;
mod config;
mod content;
mod plan;
mod refusal;
mod script_models;
mod t6_install;

use bevy::prelude::*;
use frame::HostClassLoadouts;

use crate::startup::{PendingConsoleLines, StartupCommands};

pub use apply::apply_prepared_match;
pub use script_models::install_script_model_id;

/// Install the match as soon as the loader hands one over.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<StartupCommands>()
        .init_resource::<PendingConsoleLines>()
        .init_resource::<HostClassLoadouts>()
        .add_systems(
            Update,
            apply_prepared_match
                .run_if(resource_exists::<assets::PreparedMatchReady>)
                .in_set(net::ClientSet::Load),
        );
}
