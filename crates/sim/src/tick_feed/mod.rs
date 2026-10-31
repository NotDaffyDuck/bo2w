//! What the match is fed each tick: every client's movement command, and the one-off actions (joining, choosing a
//! class, spawning, menu answers) that clients and the host ask for.

mod action;
mod menu_field;
mod tick;

pub use action::{LoadoutClass, ClassId, PlayerIntent, SpawnPick, intent_serial};
pub use menu_field::{MENU_RESPONSE_BYTES, menu_response_field, menu_response_text};
pub use tick::StepInputs;
