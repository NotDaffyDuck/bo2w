//! Keeping the first-person weapon in step with the gun in the player's hand.
//!
//! Each frame the local player's held weapon is compared with what the
//! first-person view is showing (or has queued to show). When they differ the
//! old view is thrown away and a new one is requested; when the player dies,
//! the match goes to intermission, or the hand is empty, the view is cleared.
//! The result is also written to a status value the launch report can print.
//!
//! The pieces:
//! - `slots`: the bundle of first-person state this system owns.
//! - `sync`: the per-frame decision.
//! - `probe`: copying the status into the launch report.
//! - `register`: wiring the systems into the app.

mod probe;
mod register;
mod slots;
mod sync;

mod tests;

pub use self::probe::mirror_fpv_status_probe;
pub(crate) use self::register::add_held_sync_systems;
pub use self::sync::match_fpv_to_held;
