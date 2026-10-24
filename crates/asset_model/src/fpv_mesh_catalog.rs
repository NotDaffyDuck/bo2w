//! The first-person models a session loaded: gun, attachment and arms
//! skeletons, found by namespace and name.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::asset_graph::{AssetEdge, capture_xmodel_material_slots, stamp_xmodel_material_edges};
use crate::skel_parts::FpvSkel;
use crate::{MaterialSpace, ZoneOwner};
use asset_core::{AssetNamespace, MaterialKey};
use asset_material::{MaterialCatalog, MaterialDefinitions};

/// Where a tag bone sits in its model's bind pose.
#[derive(Clone, Copy, Debug)]
pub struct TagViewBind {
    pub quat: [f32; 4],
    pub trans: [f32; 3],
}

/// How many vertices of a pose were skinned to one bone and how many to
/// several, for the arms and the gun, and whether the idle clip was sampled.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoseStats {
    pub hands_rigid: usize,
    pub hands_blend: usize,
    pub gun_rigid: usize,
    pub gun_blend: usize,
    pub idle_sampled: bool,
}

/// A model's identity in the catalog; names compare without case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FpvMeshKey {
    namespace: AssetNamespace,
    name: String,
}

impl FpvMeshKey {
    fn new(namespace: AssetNamespace, name: &str) -> Self {
        Self {
            namespace,
            name: name.to_ascii_lowercase(),
        }
    }
}

/// One model: its skeleton and mesh, and the materials its surfaces name.
#[derive(Clone, Debug)]
pub struct FpvMeshEntry {
    pub namespace: AssetNamespace,
    pub skel: Arc<FpvSkel>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub material_edges: Vec<AssetEdge<MaterialSpace>>,
}

impl FpvMeshEntry {
    fn new(
        namespace: AssetNamespace,
        skel: Arc<FpvSkel>,
        materials: Option<&MaterialCatalog>,
    ) -> Self {
        let (material_keys, material_edges) =
            capture_xmodel_material_slots(&skel.surface_materials, materials.map(|c| &**c));
        Self {
            namespace,
            skel,
            material_keys,
            material_edges,
        }
    }

    fn key(&self) -> FpvMeshKey {
        FpvMeshKey::new(self.namespace, &self.skel.name)
    }

    pub(crate) fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        self.material_edges = stamp_xmodel_material_edges(
            &self.material_keys,
            &self.skel.surface_materials,
            materials,
        );
    }

    /// The material a surface drew from, when that material is loaded.
    pub fn material_present_name(&self, surface: usize) -> Option<&str> {
        if !self.material_edges.get(surface)?.is_bound() {
            return None;
        }
        Some(self.material_keys.get(surface)?.as_ref()?.name.as_str())
    }

    fn bind_of(&self, bone: Option<usize>) -> Option<TagViewBind> {
        let bone = self.skel.bones.get(bone?)?;
        Some(TagViewBind {
            quat: bone.quat,
            trans: bone.trans,
        })
    }

    pub fn tag_view(&self) -> Option<TagViewBind> {
        self.bind_of(self.skel.tag_view)
    }

    pub fn tag_weapon(&self) -> Option<TagViewBind> {
        self.bind_of(self.skel.tag_weapon)
    }

    pub fn bone_count(&self) -> usize {
        self.skel.bones.len()
    }
}

/// A model with the zone that supplied it.
#[derive(Clone, Debug)]
struct Slot {
    key: FpvMeshKey,
    zone: ZoneOwner,
    entry: FpvMeshEntry,
}

/// The finished set of first-person models. Rows keep the order they were
/// first added in, and the position in that order is the row's index.
#[derive(Clone, Debug, Default)]
pub struct FpvMeshCatalog {
    identity: u64,
    slots: Vec<Slot>,
    by_key: HashMap<FpvMeshKey, usize>,
    /// The namespace of the map the session loaded, once it is known.
    pub map_namespace: Option<AssetNamespace>,
}

impl FpvMeshCatalog {
    /// A number no other published catalog shares, for telling two apart.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn index_by_name(&self, ns: AssetNamespace, name: &str) -> Option<usize> {
        self.by_key.get(&FpvMeshKey::new(ns, name)).copied()
    }

    pub fn get(&self, ns: AssetNamespace, name: &str) -> Option<&FpvMeshEntry> {
        self.get_at(self.index_by_name(ns, name)?)
    }

    pub fn get_at(&self, index: usize) -> Option<&FpvMeshEntry> {
        self.slots.get(index).map(|slot| &slot.entry)
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.slots.get(index).map(|slot| slot.key.name.as_str())
    }

    pub fn zone_of(&self, index: usize) -> ZoneOwner {
        self.slots.get(index).map(|slot| slot.zone).unwrap_or_default()
    }

    /// The names held for a namespace, in the order they were added.
    pub fn names_in(&self, ns: AssetNamespace) -> impl Iterator<Item = &str> {
        self.slots
            .iter()
            .filter(move |slot| slot.key.namespace == ns)
            .map(|slot| slot.key.name.as_str())
    }

    pub fn contains(&self, ns: AssetNamespace, name: &str) -> bool {
        self.by_key.contains_key(&FpvMeshKey::new(ns, name))
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// A catalog still being filled. It reads as the catalog it will become.
#[derive(Clone, Debug)]
pub struct FpvMeshBuild {
    catalog: FpvMeshCatalog,
    capture_zone: ZoneOwner,
    capture_ns: AssetNamespace,
}

impl Default for FpvMeshBuild {
    fn default() -> Self {
        Self {
            catalog: FpvMeshCatalog::default(),
            capture_zone: ZoneOwner::default(),
            capture_ns: AssetNamespace::T6,
        }
    }
}

impl std::ops::Deref for FpvMeshBuild {
    type Target = FpvMeshCatalog;

    fn deref(&self) -> &FpvMeshCatalog {
        &self.catalog
    }
}

impl FpvMeshBuild {
    pub fn publish(self) -> FpvMeshCatalog {
        static LAST_IDENTITY: AtomicU64 = AtomicU64::new(0);
        let mut catalog = self.catalog;
        catalog.identity = LAST_IDENTITY.fetch_add(1, Ordering::Relaxed) + 1;
        catalog
    }

    /// The namespace `insert_captured` files models under.
    pub fn set_capture_ns(&mut self, ns: AssetNamespace) {
        self.capture_ns = ns;
    }

    /// The zone the models added from now on are credited to.
    pub fn set_capture_zone(&mut self, zone: ZoneOwner) {
        self.capture_zone = zone;
    }

    pub fn set_map_namespace(&mut self, ns: Option<AssetNamespace>) {
        self.catalog.map_namespace = ns;
    }

    pub fn insert_captured(&mut self, skel: FpvSkel, materials: Option<&MaterialCatalog>) {
        self.insert_in(self.capture_ns, skel, materials);
    }

    /// Adds a model, replacing the one with the same namespace and name.
    pub fn insert_in(
        &mut self,
        ns: AssetNamespace,
        skel: impl Into<Arc<FpvSkel>>,
        materials: Option<&MaterialCatalog>,
    ) {
        let entry = FpvMeshEntry::new(ns, skel.into(), materials);
        self.put(Slot {
            key: entry.key(),
            zone: self.capture_zone,
            entry,
        });
    }

    /// Whether the slot added a row, as opposed to replacing one.
    fn put(&mut self, slot: Slot) -> bool {
        let catalog = &mut self.catalog;
        if let Some(&at) = catalog.by_key.get(&slot.key) {
            catalog.slots[at] = slot;
            return false;
        }
        catalog.by_key.insert(slot.key.clone(), catalog.slots.len());
        catalog.slots.push(slot);
        true
    }

    /// Takes in another build's models; each keeps its own zone, and a name
    /// already held is replaced.
    pub fn absorb(&mut self, other: Self) {
        for slot in other.catalog.slots {
            self.put(slot);
        }
    }

    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for slot in &mut self.catalog.slots {
            slot.entry.resolve_materials(materials);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::FpvSkel;
    use crate::dobj::ModelPoseSrc;

    /// A model with these bones and nothing else; `posed` gives it a pose.
    pub(crate) fn skel(name: &str, bones: &[&str], posed: bool) -> FpvSkel {
        let bone_names: Vec<String> = bones.iter().map(|b| (*b).to_owned()).collect();
        FpvSkel {
            name: name.to_owned(),
            bones: Vec::new(),
            bone_collision: Vec::new(),
            bone_names: bone_names.clone(),
            tag_view: None,
            tag_weapon: None,
            pose: posed.then(|| ModelPoseSrc {
                name: name.to_owned(),
                num_bones: bone_names.len(),
                num_root_bones: 1,
                scale: 1.0,
                no_scale_part_bits: [0; 5],
                bone_names,
                parent_list: Vec::new(),
                quats: Vec::new(),
                trans: Vec::new(),
                base_mat: Vec::new(),
            }),
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            surface_materials: Vec::new(),
            surface_vertex_ranges: Vec::new(),
            surface_index_ranges: Vec::new(),
            surface_part_bits: Vec::new(),
            surface_deformed: Vec::new(),
            surface_vert_list_count: Vec::new(),
            vert_skin: Vec::new(),
            packed_vertices: Vec::new(),
            radius: None,
            bounds: None,
            contents: None,
            coll_lod: 0,
            coll_surfs: Vec::new(),
            collmap: None,
            lod: None,
            lod_smc: None,
            lod_surf_span: [(0, 0); 4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::skel;
    use super::*;

    const T6: AssetNamespace = AssetNamespace::T6;

    fn zone(n: u32) -> ZoneOwner {
        ZoneOwner::intern(&format!("zone_{n}"))
    }

    #[test]
    fn names_match_without_case_and_keep_their_order() {
        let mut build = FpvMeshBuild::default();
        build.insert_in(T6, skel("Viewmodel_B", &["tag"], true), None);
        build.insert_in(T6, skel("viewmodel_a", &["tag"], true), None);
        assert_eq!(build.index_by_name(T6, "VIEWMODEL_B"), Some(0));
        assert_eq!(build.name_at(1), Some("viewmodel_a"));
        assert_eq!(build.names_in(T6).collect::<Vec<_>>(), ["viewmodel_b", "viewmodel_a"]);
        assert!(build.contains(T6, "Viewmodel_A"));
        assert_eq!(build.len(), 2);
    }

    #[test]
    fn a_repeated_name_replaces_the_row_in_place() {
        let mut build = FpvMeshBuild::default();
        build.set_capture_zone(zone(1));
        build.insert_in(T6, skel("gun", &["a"], true), None);
        build.insert_in(T6, skel("other", &["a"], true), None);
        build.set_capture_zone(zone(2));
        build.insert_in(T6, skel("GUN", &["a", "b"], true), None);
        assert_eq!(build.len(), 2);
        assert_eq!(build.get_at(0).unwrap().skel.bone_names.len(), 2);
        assert_eq!(build.zone_of(0), zone(2));
        assert_eq!(build.zone_of(1), zone(1));
    }

    #[test]
    fn absorbing_keeps_each_models_zone() {
        let mut first = FpvMeshBuild::default();
        first.set_capture_zone(zone(1));
        first.insert_in(T6, skel("gun", &["a"], true), None);
        let mut second = FpvMeshBuild::default();
        second.set_capture_zone(zone(2));
        second.insert_in(T6, skel("arms", &["a"], true), None);
        second.insert_in(T6, skel("gun", &["a", "b"], true), None);
        first.absorb(second);
        assert_eq!(first.len(), 2);
        assert_eq!(first.zone_of(0), zone(2));
        assert_eq!(first.zone_of(1), zone(2));
    }

    #[test]
    fn published_catalogs_have_distinct_identities() {
        let one = FpvMeshBuild::default().publish();
        let two = FpvMeshBuild::default().publish();
        assert_ne!(one.identity(), two.identity());
        assert_ne!(one.identity(), 0);
    }

    #[test]
    fn the_capture_namespace_files_new_models() {
        let mut build = FpvMeshBuild::default();
        build.insert_captured(skel("gun", &["a"], true), None);
        assert!(build.contains(T6, "gun"));
        assert!(!build.contains(AssetNamespace::T5, "gun"));
    }
}
