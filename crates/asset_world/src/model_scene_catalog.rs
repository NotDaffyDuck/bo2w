//! The models a map draws, by name: which zone each came from and which
//! material each of their surfaces ended up with.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use asset_core::ZoneCustodian;
use asset_model::MeshRig;
use bevy::prelude::Resource;

use crate::SurfaceMaterialId;

/// A model's name, as the map refers to it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LevelModelAssetId(pub String);

impl Borrow<str> for LevelModelAssetId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A model the map draws, or the reason it could not be had.
#[derive(Clone, Debug)]
pub enum LevelModelPlacementAsset {
    T5(Arc<MeshRig>),
    Unavailable { why: &'static str },
}

impl LevelModelPlacementAsset {
    fn is_usable(&self) -> bool {
        matches!(self, Self::T5(_))
    }
}

/// How the surfaces of every model were bound to materials.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneMaterialTally {
    pub model_total: usize,
    pub surface_total: usize,
    pub linked: usize,
    /// Surfaces whose material was not among the ones loaded.
    pub unlinked: usize,
    /// Surfaces that name no material at all.
    pub missing: usize,
}

#[derive(Clone, Debug)]
struct Entry {
    key: LevelModelAssetId,
    asset: LevelModelPlacementAsset,
    owner: ZoneCustodian,
    /// One material per surface, once `link_skin_surfaces` has run.
    bindings: Vec<Option<SurfaceMaterialId>>,
}

/// Models by name. Entries keep the order they were added in, which is what
/// `slot_for_label` and `name_at` count by; `iter` walks them by name.
#[derive(Clone, Debug, Default, Resource)]
pub struct LevelModelPlacementRoster {
    rows: Vec<Entry>,
    by_key: BTreeMap<LevelModelAssetId, usize>,
    capture_zone: ZoneCustodian,
    bound_once: bool,
}

impl LevelModelPlacementRoster {
    fn find_entry(&self, name: &str) -> Option<&Entry> {
        let &at = self.by_key.get(name)?;
        self.rows.get(at)
    }

    fn add_entry(&mut self, key: LevelModelAssetId, asset: LevelModelPlacementAsset, owner: ZoneCustodian) {
        let next = self.rows.len();
        self.by_key.insert(key.clone(), next);
        self.rows.push(Entry { bindings: Vec::new(), owner, asset, key });
    }

    pub fn get(&self, key: &LevelModelAssetId) -> Option<&LevelModelPlacementAsset> {
        self.look_up_label(&key.0)
    }

    pub fn look_up_label(&self, name: &str) -> Option<&LevelModelPlacementAsset> {
        self.find_entry(name).map(|entry| &entry.asset)
    }

    /// Gives every surface of every model the material its walk-local id maps
    /// to in `local_material_rows`, and says how that went.
    pub fn link_skin_surfaces(
        &mut self,
        local_material_rows: &[Option<usize>],
    ) -> SceneMaterialTally {
        let (mut models, mut surfaces, mut bound, mut unbound, mut absent) = (0, 0, 0, 0, 0);
        for row in self.rows.iter_mut() {
            row.bindings.clear();
            let LevelModelPlacementAsset::T5(bone_rig) = &row.asset else {
                continue;
            };
            models += 1;
            row.bindings = bone_rig
                .facet_materials
                .iter()
                .map(|wanted| {
                    surfaces += 1;
                    let Some(id) = wanted else {
                        absent += 1;
                        return None;
                    };
                    let picked = local_material_rows.get(id.get()).copied().flatten();
                    if picked.is_some() {
                        bound += 1;
                    } else {
                        unbound += 1;
                    }
                    picked.map(SurfaceMaterialId::from_sequence)
                })
                .collect();
        }
        self.bound_once = true;
        SceneMaterialTally { model_total: models, surface_total: surfaces, linked: bound, unlinked: unbound, missing: absent }
    }

    pub fn surfaces_linked(&self) -> bool {
        self.bound_once
    }

    /// The material of one surface of the model `key`.
    pub fn facet_material(
        &self,
        key: &LevelModelAssetId,
        surface_slot_number: usize,
    ) -> Option<SurfaceMaterialId> {
        self.find_entry(&key.0)?.bindings.get(surface_slot_number).copied().flatten()
    }

    /// Adds a model under the zone being captured. A name already present
    /// keeps what it has.
    pub fn file_asset(&mut self, key: LevelModelAssetId, asset: LevelModelPlacementAsset) {
        if self.by_key.contains_key(&key) {
            return;
        }
        self.add_entry(key, asset, self.capture_zone);
    }

    /// Takes in the models of an earlier capture. Ones that were unavailable
    /// there are ignored; ones that are available fill a gap or replace an
    /// unavailable model of the same name.
    pub fn take_in_captured(&mut self, earlier: Self) {
        let usable = earlier.rows.into_iter().filter(|row| row.asset.is_usable());
        for incoming in usable {
            let Some(at) = self.by_key.get(&incoming.key).copied() else {
                self.add_entry(incoming.key, incoming.asset, incoming.owner);
                continue;
            };
            let slot = &mut self.rows[at];
            if !slot.asset.is_usable() {
                slot.asset = incoming.asset;
                slot.owner = incoming.owner;
            }
        }
    }

    /// Drops every model whose name is not in `names`.
    pub fn retain_names(&mut self, names: &BTreeSet<String>) {
        self.rows.retain(|row| names.contains(&row.key.0));
        let mut rebuilt = BTreeMap::new();
        for (at, row) in self.rows.iter().enumerate() {
            rebuilt.insert(row.key.clone(), at);
        }
        self.by_key = rebuilt;
    }

    /// The zone that models inserted from now on are credited to.
    pub fn choose_capture_owner(&mut self, custodian: ZoneCustodian) {
        self.capture_zone = custodian;
    }

    /// Makes `from` another name for the model `to`, zone and all. Fails when
    /// `to` has no available model.
    pub fn point_to(&mut self, from: &str, to: &str) -> bool {
        let Some(target) = self.find_entry(to).filter(|t| t.asset.is_usable()) else {
            return false;
        };
        let (asset, keeper) = (target.asset.clone(), target.owner);
        if let Some(at) = self.by_key.get(from).copied() {
            let slot = &mut self.rows[at];
            slot.asset = asset;
            slot.owner = keeper;
        } else {
            self.add_entry(LevelModelAssetId(from.to_owned()), asset, keeper);
        }
        true
    }

    pub fn owner_at(&self, index: usize) -> ZoneCustodian {
        match self.rows.get(index) {
            Some(row) => row.owner,
            None => ZoneCustodian::default(),
        }
    }

    pub fn slot_for_label(&self, label: &str) -> Option<usize> {
        self.by_key.get(label).copied()
    }

    pub fn label_at(&self, nth: usize) -> Option<&str> {
        self.rows.get(nth).map(|row| row.key.0.as_str())
    }

    /// Every model, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&LevelModelAssetId, &LevelModelPlacementAsset)> {
        self.by_key.iter().map(|(key, &at)| (key, &self.rows[at].asset))
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing() -> LevelModelPlacementAsset {
        LevelModelPlacementAsset::Unavailable { why: "test" }
    }

    fn key(name: &str) -> LevelModelAssetId {
        LevelModelAssetId(name.to_owned())
    }

    #[test]
    fn a_name_already_present_keeps_what_it_has() {
        let mut catalog = LevelModelPlacementRoster::default();
        catalog.choose_capture_owner(ZoneCustodian::TEST);
        catalog.file_asset(key("crate"), missing());
        catalog.choose_capture_owner(ZoneCustodian::UNCLASSIFIED);
        catalog.file_asset(key("crate"), missing());
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.owner_at(0), ZoneCustodian::TEST);
    }

    #[test]
    fn names_are_counted_in_the_order_they_came() {
        let mut catalog = LevelModelPlacementRoster::default();
        catalog.file_asset(key("b"), missing());
        catalog.file_asset(key("a"), missing());
        assert_eq!(catalog.slot_for_label("b"), Some(0));
        assert_eq!(catalog.label_at(1), Some("a"));
        let by_name: Vec<_> = catalog.iter().map(|(key, _)| key.0.as_str()).collect();
        assert_eq!(by_name, ["a", "b"]);
    }

    #[test]
    fn retaining_names_closes_the_gaps() {
        let mut catalog = LevelModelPlacementRoster::default();
        for name in ["a", "b", "c"] {
            catalog.file_asset(key(name), missing());
        }
        catalog.retain_names(&BTreeSet::from(["a".to_owned(), "c".to_owned()]));
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.slot_for_label("c"), Some(1));
        assert_eq!(catalog.label_at(1), Some("c"));
        assert!(catalog.look_up_label("b").is_none());
    }

    #[test]
    fn nothing_is_aliased_to_a_model_that_is_unavailable() {
        let mut catalog = LevelModelPlacementRoster::default();
        catalog.file_asset(key("crate"), missing());
        assert!(!catalog.point_to("box", "crate"));
        assert!(!catalog.point_to("box", "absent"));
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn an_empty_catalog_resolves_to_an_empty_census() {
        let mut catalog = LevelModelPlacementRoster::default();
        catalog.file_asset(key("crate"), missing());
        assert!(!catalog.surfaces_linked());
        assert_eq!(catalog.link_skin_surfaces(&[]), SceneMaterialTally::default());
        assert!(catalog.surfaces_linked());
        assert_eq!(catalog.facet_material(&key("crate"), 0), None);
    }
}
