//! bo2zm: what the probes share.

use std::path::PathBuf;

/// The Black Ops II folder the probes read: `BO2MP_GAMES` when it is set (as
/// the game reads it), else the working directory.
pub fn game_dir() -> PathBuf {
    std::env::var_os("BO2MP_GAMES").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// The game's zone files (`zone/all` under [`game_dir`]).
pub fn zones_dir() -> PathBuf {
    game_dir().join("zone").join("all")
}

/// One zone file in [`zones_dir`], e.g. `zm_nuked.ff`.
pub fn zone_file(name: &str) -> PathBuf {
    zones_dir().join(name)
}
