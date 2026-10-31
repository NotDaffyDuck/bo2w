//! The lobby service (room browser plus packet relay) and the small commands
//! that check on it or install it. `bo2mp-master` runs it on its own; a game
//! that hosts on a LAN runs it inside itself through [`lan`].

#![forbid(unsafe_code)]

pub mod args;
pub mod lan;
mod lobby;
mod pem;
pub mod probe;
pub mod service;
pub mod unit;
mod wire;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
