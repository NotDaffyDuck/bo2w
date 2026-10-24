//! Every weapon world model a session loaded, found by namespace and name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::skel_model::ModelSkel;
use crate::weapon_world_entry::WorldWeaponEntry;
use crate::ZoneOwner;
use asset_core::AssetNamespace;
use asset_material::{MaterialCatalog, MaterialDefinitions};

/// A model's place in the set; names compare without case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Address {
    namespace: AssetNamespace,
    name: String,
}

impl Address {
    fn new(namespace: AssetNamespace, name: &str) -> Self {
        Self {
            namespace,
            name: name.to_ascii_lowercase(),
        }
    }
}

/// A model with the zone that supplied it.
#[derive(Clone, Debug)]
struct Row {
    address: Address,
    zone: ZoneOwner,
    entry: WorldWeaponEntry,
}

/// The finished set. Rows keep the order they were first added in, and a
/// row's position in that order is its index.
#[derive(Clone, Debug, Default)]
pub struct WorldWeaponCatalog {
    /// Zero until the set is sealed; any change to the set takes it back to zero.
    identity: u64,
    rows: Vec<Row>,
    by_address: HashMap<Address, usize>,
}

impl WorldWeaponCatalog {
    /// A number no other sealed set shares, for telling two apart.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn index_by_name(&self, ns: AssetNamespace, name: &str) -> Option<usize> {
        self.by_address.get(&Address::new(ns, name)).copied()
    }

    pub fn get(&self, ns: AssetNamespace, name: &str) -> Option<&WorldWeaponEntry> {
        self.get_at(self.index_by_name(ns, name)?)
    }

    pub fn get_at(&self, index: usize) -> Option<&WorldWeaponEntry> {
        self.rows.get(index).map(|row| &row.entry)
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.rows.get(index).map(|row| row.address.name.as_str())
    }

    pub fn zone_of(&self, index: usize) -> ZoneOwner {
        self.rows.get(index).map(|row| row.zone).unwrap_or_default()
    }

    pub fn contains(&self, ns: AssetNamespace, name: &str) -> bool {
        self.by_address.contains_key(&Address::new(ns, name))
    }

    /// The names held for a namespace, in the order they were added.
    pub fn names_in(&self, ns: AssetNamespace) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .filter(move |row| row.address.namespace == ns)
            .map(|row| row.address.name.as_str())
    }

    /// Every name, whatever its namespace, in the order they were added.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|row| row.address.name.as_str())
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct WorldWeaponBuild {
    catalog: WorldWeaponCatalog,
    capture_zone: ZoneOwner,
    capture_ns: AssetNamespace,
}

impl std::ops::Deref for WorldWeaponBuild {
    type Target = WorldWeaponCatalog;

    fn deref(&self) -> &WorldWeaponCatalog {
        &self.catalog
    }
}

impl WorldWeaponBuild {
    /// Gives the set its identity if it has none yet, and returns it.
    pub fn seal_identity(&mut self) -> u64 {
        static LAST_IDENTITY: AtomicU64 = AtomicU64::new(0);
        if self.catalog.identity == 0 {
            self.catalog.identity = LAST_IDENTITY.fetch_add(1, Ordering::Relaxed) + 1;
        }
        self.catalog.identity
    }

    pub fn publish(self) -> WorldWeaponCatalog {
        self.catalog
    }

    /// The zone the models added from now on are credited to.
    pub fn set_capture_zone(&mut self, zone: ZoneOwner) {
        self.capture_zone = zone;
    }

    /// The namespace `insert_captured` files models under.
    pub fn set_capture_ns(&mut self, ns: AssetNamespace) {
        self.capture_ns = ns;
    }

    pub fn insert_captured(&mut self, skel: ModelSkel, materials: Option<&MaterialCatalog>) {
        self.insert_in(self.capture_ns, skel, materials);
    }

    /// Adds a model, replacing the one with the same namespace and name.
    pub fn insert_in(
        &mut self,
        ns: AssetNamespace,
        skel: ModelSkel,
        materials: Option<&MaterialCatalog>,
    ) {
        let entry = WorldWeaponEntry::new(ns, skel, materials);
        let address = Address::new(ns, &entry.skel.name);
        self.put(Row {
            address,
            zone: self.capture_zone,
            entry,
        });
    }

    /// Whether the row was added, as opposed to replacing one.
    fn put(&mut self, row: Row) -> bool {
        let catalog = &mut self.catalog;
        catalog.identity = 0;
        if let Some(&at) = catalog.by_address.get(&row.address) {
            catalog.rows[at] = row;
            return false;
        }
        catalog.by_address.insert(row.address.clone(), catalog.rows.len());
        catalog.rows.push(row);
        true
    }

    /// Takes in another build's models, each keeping its own zone, and says
    /// how many names were new to this one.
    pub fn absorb(&mut self, other: Self) -> usize {
        other
            .catalog
            .rows
            .into_iter()
            .map(|row| self.put(row))
            .filter(|added| *added)
            .count()
    }

    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for row in &mut self.catalog.rows {
            row.entry.resolve_materials(materials);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skel(name: &str) -> ModelSkel {
        crate::fpv_mesh_catalog::test_support::skel(name, &[], false)
    }

    #[test]
    fn a_name_matches_without_regard_to_case_and_replaces_in_place() {
        let mut build = WorldWeaponBuild::default();
        build.insert_in(AssetNamespace::T6, skel("W_Rifle"), None);
        build.insert_in(AssetNamespace::T6, skel("w_rifle"), None);
        assert_eq!(build.len(), 1);
        assert_eq!(build.index_by_name(AssetNamespace::T6, "W_RIFLE"), Some(0));
        assert_eq!(build.name_at(0), Some("w_rifle"));
    }

    #[test]
    fn absorbing_counts_only_new_names_and_keeps_their_zones() {
        let mut ours = WorldWeaponBuild::default();
        ours.insert_in(AssetNamespace::T6, skel("w_rifle"), None);
        let mut theirs = WorldWeaponBuild::default();
        theirs.set_capture_zone(ZoneOwner::TEST);
        theirs.insert_in(AssetNamespace::T6, skel("w_rifle"), None);
        theirs.insert_in(AssetNamespace::T6, skel("w_pistol"), None);
        assert_eq!(ours.absorb(theirs), 1);
        assert_eq!(ours.len(), 2);
        assert_eq!(ours.zone_of(0), ZoneOwner::TEST);
        assert_eq!(ours.zone_of(1), ZoneOwner::TEST);
    }

    #[test]
    fn a_sealed_identity_lasts_until_the_set_changes() {
        let mut build = WorldWeaponBuild::default();
        let first = build.seal_identity();
        assert_ne!(first, 0);
        assert_eq!(build.seal_identity(), first);
        build.insert_in(AssetNamespace::T6, skel("w_rifle"), None);
        assert_eq!(build.identity(), 0);
        assert_ne!(build.seal_identity(), first);
    }
}
