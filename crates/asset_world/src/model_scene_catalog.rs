//! The models a map draws, by name: which zone each came from and which
//! material each of their surfaces ended up with.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use asset_core::ZoneOwner;
use asset_model::ModelSkel;
use bevy::prelude::Resource;

use crate::MaterialIndex;

/// A model's name, as the map refers to it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MapXModelAssetKey(pub String);

impl Borrow<str> for MapXModelAssetKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A model the map draws, or the reason it could not be had.
#[derive(Clone, Debug)]
pub enum MapXModelSceneAsset {
    T5(Arc<ModelSkel>),
    Unavailable { reason: &'static str },
}

impl MapXModelSceneAsset {
    fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

/// How the surfaces of every model were bound to materials.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapXModelMaterialCensus {
    pub models: usize,
    pub surfaces: usize,
    pub bound: usize,
    /// Surfaces whose material was not among the ones loaded.
    pub unbound: usize,
    /// Surfaces that name no material at all.
    pub absent: usize,
}

#[derive(Clone, Debug)]
struct Entry {
    key: MapXModelAssetKey,
    asset: MapXModelSceneAsset,
    zone: ZoneOwner,
    /// One material per surface, once `resolve_surface_materials` has run.
    materials: Vec<Option<MaterialIndex>>,
}

/// Models by name. Entries keep the order they were added in, which is what
/// `index_by_name` and `name_at` count by; `iter` walks them by name.
#[derive(Clone, Debug, Default, Resource)]
pub struct MapXModelSceneCatalog {
    entries: Vec<Entry>,
    by_key: BTreeMap<MapXModelAssetKey, usize>,
    capture_zone: ZoneOwner,
    resolved: bool,
}

impl MapXModelSceneCatalog {
    fn slot(&self, name: &str) -> Option<&Entry> {
        self.by_key.get(name).map(|&at| &self.entries[at])
    }

    fn push(&mut self, key: MapXModelAssetKey, asset: MapXModelSceneAsset, zone: ZoneOwner) {
        self.by_key.insert(key.clone(), self.entries.len());
        self.entries.push(Entry {
            key,
            asset,
            zone,
            materials: Vec::new(),
        });
    }

    pub fn get(&self, key: &MapXModelAssetKey) -> Option<&MapXModelSceneAsset> {
        self.get_name(&key.0)
    }

    pub fn get_name(&self, name: &str) -> Option<&MapXModelSceneAsset> {
        self.slot(name).map(|entry| &entry.asset)
    }

    /// Gives every surface of every model the material its walk-local id maps
    /// to in `walk_local_ids`, and says how that went.
    pub fn resolve_surface_materials(
        &mut self,
        walk_local_ids: &[Option<usize>],
    ) -> MapXModelMaterialCensus {
        let mut census = MapXModelMaterialCensus::default();
        for entry in &mut self.entries {
            entry.materials.clear();
            let MapXModelSceneAsset::T5(skel) = &entry.asset else {
                continue;
            };
            census.models += 1;
            for local in &skel.surface_materials {
                census.surfaces += 1;
                let bound = match local {
                    None => {
                        census.absent += 1;
                        None
                    }
                    Some(local) => {
                        let order = walk_local_ids.get(local.get()).copied().flatten();
                        match order {
                            Some(_) => census.bound += 1,
                            None => census.unbound += 1,
                        }
                        order.map(MaterialIndex::from_order)
                    }
                };
                entry.materials.push(bound);
            }
        }
        self.resolved = true;
        census
    }

    pub fn materials_resolved(&self) -> bool {
        self.resolved
    }

    /// The material of one surface of the model `key`.
    pub fn surface_material(
        &self,
        key: &MapXModelAssetKey,
        surface_index: usize,
    ) -> Option<MaterialIndex> {
        let entry = self.slot(&key.0)?;
        entry.materials.get(surface_index).copied().flatten()
    }

    /// Adds a model under the zone being captured. A name already present
    /// keeps what it has.
    pub fn insert(&mut self, key: MapXModelAssetKey, asset: MapXModelSceneAsset) {
        if !self.by_key.contains_key(&key) {
            let zone = self.capture_zone;
            self.push(key, asset, zone);
        }
    }

    /// Takes in the models of an earlier capture. Ones that were unavailable
    /// there are ignored; ones that are available fill a gap or replace an
    /// unavailable model of the same name.
    pub fn absorb_captured(&mut self, earlier: Self) {
        for old in earlier.entries {
            if old.asset.is_unavailable() {
                continue;
            }
            match self.by_key.get(&old.key).copied() {
                None => self.push(old.key, old.asset, old.zone),
                Some(at) if self.entries[at].asset.is_unavailable() => {
                    self.entries[at].asset = old.asset;
                    self.entries[at].zone = old.zone;
                }
                Some(_) => {}
            }
        }
    }

    /// Drops every model whose name is not in `names`.
    pub fn retain_names(&mut self, names: &BTreeSet<String>) {
        self.entries.retain(|entry| names.contains(&entry.key.0));
        self.by_key = self
            .entries
            .iter()
            .enumerate()
            .map(|(at, entry)| (entry.key.clone(), at))
            .collect();
    }

    /// The zone that models inserted from now on are credited to.
    pub fn set_capture_zone(&mut self, zone: ZoneOwner) {
        self.capture_zone = zone;
    }

    /// Makes `from` another name for the model `to`, zone and all. Fails when
    /// `to` has no available model.
    pub fn alias(&mut self, from: &str, to: &str) -> bool {
        let Some(target) = self.slot(to).filter(|entry| !entry.asset.is_unavailable()) else {
            return false;
        };
        let (asset, zone) = (target.asset.clone(), target.zone);
        match self.by_key.get(from).copied() {
            Some(at) => {
                self.entries[at].asset = asset;
                self.entries[at].zone = zone;
            }
            None => self.push(MapXModelAssetKey(from.to_owned()), asset, zone),
        }
        true
    }

    pub fn zone_of(&self, index: usize) -> ZoneOwner {
        self.entries.get(index).map_or_else(ZoneOwner::default, |entry| entry.zone)
    }

    pub fn index_by_name(&self, name: &str) -> Option<usize> {
        self.by_key.get(name).copied()
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|entry| entry.key.0.as_str())
    }

    /// Every model, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&MapXModelAssetKey, &MapXModelSceneAsset)> {
        self.by_key.iter().map(|(key, &at)| (key, &self.entries[at].asset))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing() -> MapXModelSceneAsset {
        MapXModelSceneAsset::Unavailable { reason: "test" }
    }

    fn key(name: &str) -> MapXModelAssetKey {
        MapXModelAssetKey(name.to_owned())
    }

    #[test]
    fn a_name_already_present_keeps_what_it_has() {
        let mut catalog = MapXModelSceneCatalog::default();
        catalog.set_capture_zone(ZoneOwner::TEST);
        catalog.insert(key("crate"), missing());
        catalog.set_capture_zone(ZoneOwner::UNKNOWN);
        catalog.insert(key("crate"), missing());
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.zone_of(0), ZoneOwner::TEST);
    }

    #[test]
    fn names_are_counted_in_the_order_they_came() {
        let mut catalog = MapXModelSceneCatalog::default();
        catalog.insert(key("b"), missing());
        catalog.insert(key("a"), missing());
        assert_eq!(catalog.index_by_name("b"), Some(0));
        assert_eq!(catalog.name_at(1), Some("a"));
        let by_name: Vec<_> = catalog.iter().map(|(key, _)| key.0.as_str()).collect();
        assert_eq!(by_name, ["a", "b"]);
    }

    #[test]
    fn retaining_names_closes_the_gaps() {
        let mut catalog = MapXModelSceneCatalog::default();
        for name in ["a", "b", "c"] {
            catalog.insert(key(name), missing());
        }
        catalog.retain_names(&BTreeSet::from(["a".to_owned(), "c".to_owned()]));
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.index_by_name("c"), Some(1));
        assert_eq!(catalog.name_at(1), Some("c"));
        assert!(catalog.get_name("b").is_none());
    }

    #[test]
    fn nothing_is_aliased_to_a_model_that_is_unavailable() {
        let mut catalog = MapXModelSceneCatalog::default();
        catalog.insert(key("crate"), missing());
        assert!(!catalog.alias("box", "crate"));
        assert!(!catalog.alias("box", "absent"));
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn an_empty_catalog_resolves_to_an_empty_census() {
        let mut catalog = MapXModelSceneCatalog::default();
        catalog.insert(key("crate"), missing());
        assert!(!catalog.materials_resolved());
        assert_eq!(catalog.resolve_surface_materials(&[]), MapXModelMaterialCensus::default());
        assert!(catalog.materials_resolved());
        assert_eq!(catalog.surface_material(&key("crate"), 0), None);
    }
}
