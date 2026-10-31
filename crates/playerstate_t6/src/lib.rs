//! Black Ops II's player state (`playerState_s`) and user command
//! (`usercmd_s`), with BO2's own field order, numbers and flag bits.
//!
//! Pure data and BO2's small read-only tests on it (sprinting, sliding,
//! effective stance, perk bits, held-weapon slot). No ECS, bevy, or I/O.

pub mod consts;
pub mod flags;
#[path = "live/mod.rs"]
pub mod game;
pub mod hud;
pub mod perks;
pub mod playerstate;
pub mod usercmd;
pub mod weapon;

pub use hud::*;
pub use playerstate::*;
pub use usercmd::*;
pub use weapon::*;
