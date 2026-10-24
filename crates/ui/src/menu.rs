//! The map list the main menu offers, and the plugin that wires the menu's
//! resources and per-frame upkeep.

use asset_transport::MapPack;
use bevy::prelude::*;
use frame::ClientSet;

use crate::classes::store::{
    ClassStoreFile, load_class_store, save_class_store, sync_host_class_loadouts,
};

/// Installed map packs, in the order the menu lists them.
#[derive(Resource, Clone, Debug, Default)]
pub struct MenuMapList(pub Vec<MapPack>);

impl MenuMapList {
    /// Every map name, pack by pack.
    pub fn maps(&self) -> impl Iterator<Item = &String> {
        self.0.iter().flat_map(|pack| pack.maps.iter())
    }

    pub fn contains(&self, map: &str) -> bool {
        self.pack_of(map).is_some()
    }

    /// Index of the first pack holding `map`.
    pub fn pack_of(&self, map: &str) -> Option<usize> {
        self.0
            .iter()
            .position(|pack| pack.maps.iter().any(|name| name == map))
    }
}

pub(crate) struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuMapList>()
            .init_resource::<crate::ClassLoadoutCatalog>()
            .init_resource::<frame::GameSettings>()
            .init_resource::<crate::BindingView>()
            .init_resource::<crate::SessionClassStore>()
            .init_resource::<ClassStoreFile>()
            .init_resource::<frame::HostClassLoadouts>()
            .init_resource::<asset_game::LocalizeCatalog>()
            .add_systems(
                Update,
                (
                    crate::options::apply_window_settings,
                    load_class_store,
                    sync_host_class_loadouts,
                    save_class_store,
                )
                    .chain()
                    .in_set(ClientSet::Ui),
            );
    }
}
