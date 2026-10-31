//! Request ids for a client's actions, and the host's memory of which it has seen.
//!
//! A client numbers each action it asks for and may send the same one again until the host's
//! verdict reaches it. The ledger lets the host tell a resend from a new request, and refuse a
//! reused id that carries different contents.

mod ids;
mod ledger;

pub use ids::ActionRequestCounter;
pub use ledger::{ActionAcceptanceResult, PlayerActionBook, RECALLED_COMMANDS_PER_RECEIVER_CAP};
