//! Where the menus find their own assets.

use asset_transport::{GamesRoot, find_zone_file, game_root_for_zone};

/// The games root the menus read from: whichever one holds the multiplayer
/// common zone.
pub fn ui_games_root(games: &GamesRoot) -> Result<GamesRoot, String> {
    let common = find_zone_file(games, "t6:common_mp")?;
    game_root_for_zone(&common.path).map(GamesRoot)
}
