//! Host-side bots for BO2 multiplayer. The bots themselves are BO2's: its bot script adds them as test clients and
//! its bot code steers them inside sim's t6 module. This crate carries the console's `bot` commands to the game,
//! seats and joins the clients the script adds, gives them gamertags and default classes, and keeps a command
//! queued for each one.
mod commands;
mod gamertags;
mod lineup;
mod wiring;
pub use commands::{BotAddQueue, BotHold, BotTpQueue, BotTpRequest, BotTpTarget, BotTpWhere};
pub use lineup::BotRoster;
pub use wiring::BotsPlugin;
