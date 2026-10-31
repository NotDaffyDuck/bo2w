//! Every weapon world model a session loaded, found by namespace and name.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::skel_model::MeshRig;
use crate::weapon_world_entry::LevelWeaponRecord;
use asset_core::AssetCategoryScope;
use asset_core::ZoneCustodian;
use asset_material::MaterialDefinitions;
use asset_material::SurfaceShaderRoster;

fn shelf_for(space: AssetCategoryScope, label: &str) -> (AssetCategoryScope, String) {
    let folded = label.to_ascii_lowercase();
    (space, folded)
}

/// A model with the shelf it sits on and the zone that brought it.
#[derive(Clone, Debug)]
struct Held {
    shelf: (AssetCategoryScope, String),
    brought_by: ZoneCustodian,
    record: LevelWeaponRecord,
}

/// The finished set. Models keep the order they first came in, and that
/// order gives each its row number.
#[derive(Clone, Debug, Default)]
pub struct LevelWeaponRoster {
    /// Zero until sealed; any change to the set clears it again.
    stamp: u64,
    held: Vec<Held>,
    rows: HashMap<(AssetCategoryScope, String), usize>,
}

impl LevelWeaponRoster {
    /// A number no other sealed set shares, for telling two apart.
    pub fn identity(&self) -> u64 {
        let Self { stamp, .. } = self;
        *stamp
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn row_of(&self, space: AssetCategoryScope, label: &str) -> Option<usize> {
        self.rows.get(&shelf_for(space, label)).copied()
    }

    pub fn by_label(&self, space: AssetCategoryScope, label: &str) -> Option<&LevelWeaponRecord> {
        self.row_of(space, label).and_then(|row| self.at_row(row))
    }

    pub fn at_row(&self, row: usize) -> Option<&LevelWeaponRecord> {
        self.held.get(row).map(|kept| &kept.record)
    }

    pub fn label_at(&self, row: usize) -> Option<&str> {
        self.held.get(row).map(|kept| kept.shelf.1.as_str())
    }

    pub fn supplier_at(&self, row: usize) -> ZoneCustodian {
        match self.held.get(row) {
            Some(kept) => kept.brought_by,
            None => ZoneCustodian::default(),
        }
    }

    pub fn holds(&self, space: AssetCategoryScope, label: &str) -> bool {
        self.rows.contains_key(&shelf_for(space, label))
    }

    /// The names on one namespace's shelves, in the order they came in.
    pub fn labels_in(&self, space: AssetCategoryScope) -> impl Iterator<Item = &str> {
        self.held
            .iter()
            .filter_map(move |kept| (kept.shelf.0 == space).then_some(kept.shelf.1.as_str()))
    }

    /// Every name in every namespace, in the order they came in.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.held.iter().map(|kept| kept.shelf.1.as_str())
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct WorldWeaponBuild {
    filling: LevelWeaponRoster,
    credit_to: ZoneCustodian,
    file_under: AssetCategoryScope,
}

impl std::ops::Deref for WorldWeaponBuild {
    type Target = LevelWeaponRoster;

    fn deref(&self) -> &LevelWeaponRoster {
        &self.filling
    }
}

impl WorldWeaponBuild {
    /// Stamps the set if it has no stamp yet, and returns the stamp.
    pub fn seal_identity(&mut self) -> u64 {
        static STAMPS_GIVEN: AtomicU64 = AtomicU64::new(0);
        if self.filling.stamp == 0 {
            self.filling.stamp = STAMPS_GIVEN.fetch_add(1, Ordering::Relaxed) + 1;
        }
        self.filling.stamp
    }

    /// The finished set; what models are filed and credited under goes with
    /// the build.
    pub fn publish(self) -> LevelWeaponRoster {
        let Self { filling, .. } = self;
        filling
    }

    /// The zone the models added from now on are credited to.
    pub fn credit_new_to(&mut self, supplier: ZoneCustodian) {
        self.credit_to = supplier;
    }

    /// The namespace `add_captured` files models under.
    pub fn file_new_under(&mut self, space: AssetCategoryScope) {
        self.file_under = space;
    }

    pub fn add_captured(&mut self, bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) {
        self.put_into_namespace(self.file_under, bone_rig, roster);
    }

    /// Adds a model, taking the place of one with the same namespace and name.
    pub fn put_into_namespace(
        &mut self,
        space: AssetCategoryScope,
        bone_rig: MeshRig,
        roster: Option<&SurfaceShaderRoster>,
    ) {
        let record = LevelWeaponRecord::new(space, bone_rig, roster);
        let shelf = shelf_for(space, &record.bone_rig.name);
        let brought_by = self.credit_to;
        self.shelve(Held { shelf, brought_by, record });
    }

    /// True when the model went on a new shelf rather than replacing one.
    fn shelve(&mut self, incoming: Held) -> bool {
        let set = &mut self.filling;
        set.stamp = 0;
        match set.rows.get(&incoming.shelf).copied() {
            Some(row) => {
                set.held[row] = incoming;
                false
            }
            None => {
                set.rows.insert(incoming.shelf.clone(), set.held.len());
                set.held.push(incoming);
                true
            }
        }
    }

    /// Takes another build's models, each still credited to its own zone, and
    /// counts the names this build did not have yet.
    pub fn take_in(&mut self, incoming: Self) -> usize {
        let held = incoming.filling.held.into_iter();
        held.map(|kept| self.shelve(kept)).filter(|&fresh| fresh).count()
    }

    pub fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        self.filling.held.iter_mut().for_each(|kept| kept.record.bind_materials(defs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(label: &str) -> MeshRig {
        crate::fpv_mesh_catalog::test_support::bone_rig(label, &[], false)
    }

    #[test]
    fn a_name_matches_without_regard_to_case_and_replaces_in_place() {
        let mut build = WorldWeaponBuild::default();
        build.put_into_namespace(AssetCategoryScope::T6, rig("W_Rifle"), None);
        build.put_into_namespace(AssetCategoryScope::T6, rig("w_rifle"), None);
        assert_eq!(build.len(), 1);
        assert_eq!(build.row_of(AssetCategoryScope::T6, "W_RIFLE"), Some(0));
        assert_eq!(build.label_at(0), Some("w_rifle"));
    }

    #[test]
    fn taking_in_counts_only_new_names_and_keeps_their_zones() {
        let mut ours = WorldWeaponBuild::default();
        ours.put_into_namespace(AssetCategoryScope::T6, rig("w_rifle"), None);
        let mut theirs = WorldWeaponBuild::default();
        theirs.credit_new_to(ZoneCustodian::TEST);
        theirs.put_into_namespace(AssetCategoryScope::T6, rig("w_rifle"), None);
        theirs.put_into_namespace(AssetCategoryScope::T6, rig("w_pistol"), None);
        assert_eq!(ours.take_in(theirs), 1);
        assert_eq!(ours.len(), 2);
        assert_eq!(ours.supplier_at(0), ZoneCustodian::TEST);
        assert_eq!(ours.supplier_at(1), ZoneCustodian::TEST);
    }

    #[test]
    fn a_stamp_lasts_until_the_set_changes() {
        let mut build = WorldWeaponBuild::default();
        let first = build.seal_identity();
        assert_ne!(first, 0);
        assert_eq!(build.seal_identity(), first);
        build.put_into_namespace(AssetCategoryScope::T6, rig("w_rifle"), None);
        assert_eq!(build.identity(), 0);
        assert_ne!(build.seal_identity(), first);
    }
}
