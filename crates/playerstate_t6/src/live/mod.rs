//! The state the running match keeps for each player between ticks, and the input a client sends each frame.
//!
//! This is the shape the simulation, the snapshot writer and the client view all share today. BO2's own
//! `playerState_s` lives at the crate root ([`crate::AvatarState`]); fields move over to it as the
//! remaining callers switch.
//!
//! - `record`: the per-player record, its blank value, and the entity numbers it refers to.
//! - `flag_bits`: the bit sets stored in that record, plus perk bits.
//! - `killcam`: replaying a victim's death through the killer's archived record.
//! - `chase_cam`: when the camera leaves the head, and where it goes.
//! - `command`: one frame of player input.

mod chase_cam;
mod command;
mod flag_bits;
mod kill_view;
mod record;

pub use chase_cam::*;
pub use command::*;
pub use flag_bits::*;
pub use kill_view::*;
pub use record::*;
