//! The installed map packs the front end can pick from, plus the module that sets up the
//! menu-side resources and the class store's load/save upkeep.

use asset_transport::MapPack;
use bevy::prelude::*;

use crate::classes::store::{
    ClassStoreFile, load_class_store, save_class_store, refresh_hosted_loadout_roster,
};

/// The map packs on disk, kept in the order the menu lists them.
#[derive(Resource, Clone, Debug, Default)]
pub struct ScreenArenaRoster(pub Vec<MapPack>);

impl ScreenArenaRoster {
    /// All map names, walking the packs in order.
    pub fn maps(&self) -> impl Iterator<Item = &String> {
        self.0.iter().flat_map(|pack| &pack.maps)
    }

    pub fn contains(&self, map: &str) -> bool {
        self.pack_of(map) != None
    }

    /// Where the first pack that lists `map` sits in the roster.
    pub fn pack_of(&self, map: &str) -> Option<usize> {
        let lists_it = |pack: &MapPack| pack.maps.iter().any(|entry| entry == map);
        self.0.iter().position(lists_it)
    }
}

pub(crate) struct MenuStateModule;

impl Plugin for MenuStateModule {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenArenaRoster>();
        app.init_resource::<crate::LoadoutRoster>();
        app.init_resource::<frame::PlayerPrefsBundle>();
        app.init_resource::<crate::KeymapSnapshot>();
        app.init_resource::<crate::MatchLoadoutVault>();
        app.init_resource::<ClassStoreFile>();
        app.init_resource::<frame::ServerKitRoster>();
        app.init_resource::<asset_game::TranslationStringIndex>();
        let upkeep = (
            crate::window_prefs::apply_window_prefs,
            load_class_store,
            refresh_hosted_loadout_roster,
            save_class_store,
        );
        crate::every_ui_frame(app, upkeep.chain());
    }
}
