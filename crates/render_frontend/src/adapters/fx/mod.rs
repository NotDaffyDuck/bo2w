//! Effect adapters: `render_fx`'s entry points under the names the front end uses, and the
//! per-effect modules around the combat effects system.

pub use render_fx::shot_fx;
pub use render_fx::effect_play;

pub mod system;
pub mod tracer;

pub(crate) mod bolt_impact;
pub(crate) mod entity_decal;
pub(crate) mod gun_loops;
pub(crate) mod melee_glass;
pub(crate) mod sensor_laser;
pub(crate) mod tag_laser;
pub(crate) mod world_decal;
