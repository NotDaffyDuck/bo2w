//! Clearing animation state when a match starts or ends.
//!
//! Anything the animation side caches about one match (the first-person weapon
//! table, prepared model materials, scene skeletons, queued spawns) refers to
//! assets of that match. This module throws all of it away on both edges, so
//! the next match rebuilds from its own data.
//!
//! The pieces:
//! - `reset`: the system that clears the caches and the first-person state.
//! - `teardown`: the extra reset for loose-piece physics, on teardown only.
//! - `register`: wiring the systems into the app.

mod register;
mod reset;
mod phys_clear;

mod tests;

pub(crate) use self::register::register_match_reset_steps;
pub use self::reset::reset_anim_at_match_start;
