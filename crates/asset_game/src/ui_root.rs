//! Where the menus find their own assets.

use asset_transport::{GameInstallDir, locate_zone_file, install_root_of_zone};

/// The games root the menus read from: whichever one holds the multiplayer
/// common zone.
pub fn ui_games_root(games: &GameInstallDir) -> Result<GameInstallDir, String> {
    let common = locate_zone_file(games, "t6:common_mp")?;
    install_root_of_zone(&common.path).map(GameInstallDir)
}
