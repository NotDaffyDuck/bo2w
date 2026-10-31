//! Glue between the front end and the crates that own animation and effects.

use bevy::prelude::*;

pub mod motion;
pub mod fx;
pub(crate) mod t6_exposure;
pub(crate) mod t6_flicker;
pub(crate) mod t6_fog;
pub(crate) mod t6_light_tweaks;
pub(crate) mod t6_lut;
pub(crate) mod t6_vision;

/// Registers every adapter's systems.
pub struct DrawAdaptersPlugin;

impl Plugin for DrawAdaptersPlugin {
    fn build(&self, app: &mut App) {
        fx::system::install_combat_fx(app);
        t6_fog::install(app);
        t6_vision::install(app);
        t6_exposure::install_into(app);
        t6_lut::install(app);
        t6_light_tweaks::install(app);
        t6_flicker::register(app);
        motion::debris_piece::register_debris_frontend(app);
        motion::free_body_brush::add_cell_filing_systems(app);
    }
}
