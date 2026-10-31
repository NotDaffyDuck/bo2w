//! The first-person kick that a shot leaves behind, held as plain numbers.
//!
//! Two things move when a gun fires. The camera is pushed (the "view kick"),
//! and the gun model swings away from where it was pointed (the "gun kick").
//! This module keeps the state for both and steps it forward in time; it never
//! touches the ECS, so the caller decides when a shot happens and how long a
//! frame was.
//!
//! The pieces:
//! - `params`: the per-weapon numbers that shape a kick.
//! - `state`: the running values and the small random stream a shot draws from.
//! - `fire`: turning a shot into new velocities.
//! - `settle`: letting the kick relax over a frame.
//! - `angles`: applying the view kick to the camera angles.

mod angles;
mod fire;
mod params;
mod settle;
mod state;

mod tests;

pub use self::angles::kicked_view_angles;
pub use self::params::GunKickTuning;
pub use self::state::ViewRecoilState;
