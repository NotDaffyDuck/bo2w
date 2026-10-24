//! Glue between the front end and the crates that own animation and effects.

use bevy::prelude::*;

pub mod anim;
pub mod fx;
pub(crate) mod t6_fog;

/// Registers every adapter's systems.
pub struct RenderAdaptersPlugin;

impl Plugin for RenderAdaptersPlugin {
    fn build(&self, app: &mut App) {
        fx::system::register_combat_fx_systems(app);
        t6_fog::register(app);
        anim::dyn_ent::register_dyn_ent_frontend(app);
        anim::dyn_ent_brush::register_dyn_ent_brush_systems(app);
    }
}
