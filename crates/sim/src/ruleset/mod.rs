//! The free-for-all rules. Every number is read from the gametype's own constants in `gamemode_t6::dm`, so there is
//! one place to change them.
mod free_for_all;
pub use free_for_all::{DeathmatchRuleset, FFA};
