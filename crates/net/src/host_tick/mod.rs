//! The host half: the fixed-rate tick, what clients send it, and what it remembers of them.

pub mod action_books; // request ids and the per-client action ledger
pub mod peer_mail; // command and action inboxes, the authority clock
pub mod tick_systems; // the tick systems and their census
