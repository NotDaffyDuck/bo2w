//! The first-person models a session loaded: gun, attachment and arms
//! skeletons, found by namespace and name.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::link::{bind_slot_keys, read_slot_keys};
use crate::material_slots::bound_material_name;
use crate::skel_model::MeshRig;
use asset_core::AssetCategoryScope;
use asset_core::MaterialKey;
use asset_core::ZoneCustodian;
use asset_material::MaterialDefinitions;
use asset_material::SurfaceShaderRoster;

/// Where a tag bone sits in its model's bind pose.
#[derive(Clone, Copy, Debug)]
pub struct AttachPointBinding {
    pub turn: [f32; 4],
    pub at: [f32; 3],
}

/// How many vertices of a pose were skinned to one bone and how many to
/// several, for the arms and the gun, and whether the idle clip was sampled.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkeletonTally {
    pub arms_stiff_flag: usize,
    pub arms_mix_weight: usize,
    pub weapon_stiff_flag: usize,
    pub weapon_mix_weight: usize,
    pub rest_pose_taken: bool,
}

/// Where a model is filed: its namespace and its lower-cased name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Shelf {
    space: AssetCategoryScope,
    folded: String,
}

impl Shelf {
    fn of(space: AssetCategoryScope, label: &str) -> Self {
        Shelf { folded: label.to_ascii_lowercase(), space }
    }
}

/// One model: its skeleton and mesh, and the materials its surfaces name.
#[derive(Clone, Debug)]
pub struct FirstPersonMeshRecord {
    pub namespace: AssetCategoryScope,
    pub bone_rig: Arc<MeshRig>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub surface_links: Vec<asset_core::AssetDependencyLink<asset_core::SurfaceShaderSpace>>,
}

impl FirstPersonMeshRecord {
    fn new(space: AssetCategoryScope, bone_rig: Arc<MeshRig>, roster: Option<&SurfaceShaderRoster>) -> Self {
        let defs = roster.map(|held| &**held);
        let (material_keys, surface_links) = read_slot_keys(&bone_rig.facet_materials, defs);
        FirstPersonMeshRecord { namespace: space, bone_rig, material_keys, surface_links }
    }

    fn shelf(&self) -> Shelf {
        Shelf::of(self.namespace, &self.bone_rig.name)
    }

    pub(crate) fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        let pointers = &self.bone_rig.facet_materials;
        self.surface_links = bind_slot_keys(&self.material_keys, pointers, defs);
    }

    /// The material a surface drew from, when that material is loaded.
    pub fn material_name_of(&self, at: usize) -> Option<&str> {
        bound_material_name(&self.material_keys, &self.surface_links, at)
    }

    fn rest_of(&self, joint: Option<usize>) -> Option<AttachPointBinding> {
        let posed = self.bone_rig.bones.get(joint?)?;
        Some(AttachPointBinding { at: posed.shift_offset, turn: posed.spin })
    }

    pub fn tag_view(&self) -> Option<AttachPointBinding> {
        self.rest_of(self.bone_rig.eye_joint)
    }

    pub fn tag_weapon(&self) -> Option<AttachPointBinding> {
        self.rest_of(self.bone_rig.gun_joint)
    }

    pub fn joint_total(&self) -> usize {
        self.bone_rig.bones.len()
    }
}

/// A model with the shelf it sits on and the zone that brought it.
#[derive(Clone, Debug)]
struct Held {
    shelf: Shelf,
    record: FirstPersonMeshRecord,
    brought_by: ZoneCustodian,
}

/// The finished set of first-person models. Models keep the order they first
/// came in, and that order gives each its row number.
#[derive(Clone, Debug, Default)]
pub struct FirstPersonMeshRoster {
    stamp: u64,
    held: Vec<Held>,
    rows: HashMap<Shelf, usize>,
    /// The namespace of the map the session loaded, once it is known.
    pub level_scope: Option<AssetCategoryScope>,
}

impl FirstPersonMeshRoster {
    /// A number no other published set shares, for telling two apart.
    pub fn identity(&self) -> u64 {
        let Self { stamp, .. } = self;
        *stamp
    }

    pub fn row_of(&self, space: AssetCategoryScope, label: &str) -> Option<usize> {
        self.rows.get(&Shelf::of(space, label)).copied()
    }

    pub fn by_label(&self, space: AssetCategoryScope, label: &str) -> Option<&FirstPersonMeshRecord> {
        self.row_of(space, label).and_then(|row| self.at_row(row))
    }

    pub fn at_row(&self, row: usize) -> Option<&FirstPersonMeshRecord> {
        self.held.get(row).map(|kept| &kept.record)
    }

    pub fn label_at(&self, row: usize) -> Option<&str> {
        self.held.get(row).map(|kept| kept.shelf.folded.as_str())
    }

    pub fn supplier_at(&self, row: usize) -> ZoneCustodian {
        self.held.get(row).map_or_else(ZoneCustodian::default, |kept| kept.brought_by)
    }

    /// The names on one namespace's shelves, in the order they came in.
    pub fn labels_in(&self, space: AssetCategoryScope) -> impl Iterator<Item = &str> {
        let here = self.held.iter().filter(move |kept| kept.shelf.space == space);
        here.map(|kept| kept.shelf.folded.as_str())
    }

    pub fn holds(&self, space: AssetCategoryScope, label: &str) -> bool {
        self.rows.contains_key(&Shelf::of(space, label))
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug)]
pub struct FpvMeshBuild {
    filling: FirstPersonMeshRoster,
    /// The namespace models added from now on are filed under.
    file_under: AssetCategoryScope,
    /// The zone models added from now on are credited to.
    credit_to: ZoneCustodian,
}

impl Default for FpvMeshBuild {
    fn default() -> Self {
        let file_under = AssetCategoryScope::T6;
        let credit_to = ZoneCustodian::default();
        FpvMeshBuild { filling: FirstPersonMeshRoster::default(), file_under, credit_to }
    }
}

impl std::ops::Deref for FpvMeshBuild {
    type Target = FirstPersonMeshRoster;

    fn deref(&self) -> &FirstPersonMeshRoster {
        &self.filling
    }
}

impl FpvMeshBuild {
    pub fn publish(self) -> FirstPersonMeshRoster {
        static STAMPS_GIVEN: AtomicU64 = AtomicU64::new(0);
        let mut done = self.filling;
        done.stamp = STAMPS_GIVEN.fetch_add(1, Ordering::Relaxed) + 1;
        done
    }

    /// The namespace `add_captured` files models under.
    pub fn file_new_under(&mut self, space: AssetCategoryScope) {
        self.file_under = space;
    }

    /// The zone the models added from now on are credited to.
    pub fn credit_new_to(&mut self, supplier: ZoneCustodian) {
        self.credit_to = supplier;
    }

    pub fn set_map_namespace(&mut self, space: Option<AssetCategoryScope>) {
        self.filling.level_scope = space;
    }

    pub fn add_captured(&mut self, model: MeshRig, roster: Option<&SurfaceShaderRoster>) {
        self.put_into_namespace(self.file_under, model, roster);
    }

    /// Adds a model, taking the place of one with the same namespace and name.
    pub fn put_into_namespace(
        &mut self,
        space: AssetCategoryScope,
        bone_rig: impl Into<Arc<MeshRig>>,
        roster: Option<&SurfaceShaderRoster>,
    ) {
        let record = FirstPersonMeshRecord::new(space, bone_rig.into(), roster);
        let shelf = record.shelf();
        self.shelve(Held { shelf, record, brought_by: self.credit_to });
    }

    fn shelve(&mut self, incoming: Held) {
        let set = &mut self.filling;
        match set.rows.get(&incoming.shelf).copied() {
            Some(row) => set.held[row] = incoming,
            None => {
                set.rows.insert(incoming.shelf.clone(), set.held.len());
                set.held.push(incoming);
            }
        }
    }

    /// Takes another build's models; each stays credited to its own zone, and
    /// a name already held is replaced.
    pub fn take_in(&mut self, incoming: Self) {
        incoming.filling.held.into_iter().for_each(|kept| self.shelve(kept));
    }

    pub fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        self.filling.held.iter_mut().for_each(|kept| kept.record.bind_materials(defs));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::MeshRig;
    use xmodel_runtime::PoseDonorModel;

    /// A model with these bones and nothing else; `posed` gives it a pose.
    pub(crate) fn bone_rig(label: &str, bones: &[&str], posed: bool) -> MeshRig {
        let joint_labels: Vec<String> = bones.iter().map(|bone| bone.to_string()).collect();
        let pose = match posed {
            false => None,
            true => Some(PoseDonorModel {
                name: label.to_owned(),
                skeleton_joint_total: joint_labels.len(),
                top_joint_total: 1,
                size_scale: 1.0,
                fixed_size_parts: Default::default(),
                joint_labels: joint_labels.clone(),
                ancestor_chain: vec![],
                rotation_quaternions: vec![],
                shift_offset: vec![],
                foundation_surface: vec![],
            }),
        };
        MeshRig { name: label.to_owned(), joint_labels, pose, ..MeshRig::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::bone_rig;
    use super::*;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    fn zone(serial: u32) -> ZoneCustodian {
        ZoneCustodian::intern(&format!("zone_{serial}"))
    }

    /// A build holding `models` (name, bone count), all credited to `credit`.
    fn build_in(credit: Option<u32>, models: &[(&str, usize)]) -> FpvMeshBuild {
        let mut set = FpvMeshBuild::default();
        if let Some(serial) = credit {
            set.credit_new_to(zone(serial));
        }
        for &(label, bones) in models {
            let joint_labels = ["a", "b", "c"];
            set.put_into_namespace(T6, bone_rig(label, &joint_labels[..bones], true), None);
        }
        set
    }

    #[test]
    fn names_match_without_case_and_keep_their_order() {
        let set = build_in(None, &[("Viewmodel_B", 1), ("viewmodel_a", 1)]);
        assert_eq!(set.row_of(T6, "VIEWMODEL_B"), Some(0));
        assert_eq!(set.label_at(1), Some("viewmodel_a"));
        assert_eq!(set.labels_in(T6).collect::<Vec<_>>(), ["viewmodel_b", "viewmodel_a"]);
        assert!(set.holds(T6, "Viewmodel_A") && set.len() == 2);
    }

    #[test]
    fn a_repeated_name_replaces_the_row_in_place() {
        let mut set = build_in(Some(1), &[("gun", 1), ("other", 1)]);
        set.credit_new_to(zone(2));
        set.put_into_namespace(T6, bone_rig("GUN", &["a", "b"], true), None);

        assert_eq!(set.len(), 2);
        assert_eq!(set.at_row(0).map(|row| row.bone_rig.joint_labels.len()), Some(2));
        assert_eq!([set.supplier_at(0), set.supplier_at(1)], [zone(2), zone(1)]);
    }

    #[test]
    fn taking_in_keeps_each_models_zone() {
        let mut base = build_in(Some(1), &[("gun", 1)]);
        base.take_in(build_in(Some(2), &[("arms", 1), ("gun", 2)]));

        assert_eq!(base.len(), 2);
        assert!((0..2).all(|row| base.supplier_at(row) == zone(2)));
    }

    #[test]
    fn published_sets_have_distinct_stamps() {
        let [left, right] = [(); 2].map(|()| FpvMeshBuild::default().publish());
        assert_ne!(left.identity(), right.identity());
        assert_ne!(left.identity(), 0);
    }

    #[test]
    fn the_filing_namespace_files_new_models() {
        let mut set = FpvMeshBuild::default();
        set.add_captured(bone_rig("gun", &["a"], true), None);
        assert_eq!(
            [AssetCategoryScope::T6, AssetCategoryScope::T5].map(|space| set.holds(space, "gun")),
            [true, false]
        );
    }
}
