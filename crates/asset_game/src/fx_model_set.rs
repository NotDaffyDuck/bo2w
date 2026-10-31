//! The models effects spawn (shell casings, debris), found by name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use asset_core::MaterialKey;
use asset_material::MaterialDefinitions;
use asset_model::link::{snapshot_mesh_surface_slots, write_mesh_surface_links};

use crate::{
    AssetDependencyLink, AssetCategoryScope, SurfaceShaderRoster, SurfaceMaterialId, SurfaceShaderSpace, MeshRig, ZoneCustodian,
};

/// A model an effect spawns, and the material each of its surfaces draws.
#[derive(Clone, Debug)]
pub struct EffectMeshRecord {
    pub bone_rig: Arc<MeshRig>,
    material_keys: Vec<Option<MaterialKey>>,
    surface_links: Vec<AssetDependencyLink<SurfaceShaderSpace>>,
}

impl EffectMeshRecord {
    fn new(bone_rig: MeshRig, mat_defs: &SurfaceShaderRoster) -> Self {
        let bone_rig = Arc::new(bone_rig);
        let slots = snapshot_mesh_surface_slots(&bone_rig.facet_materials, Some(mat_defs));
        let (material_keys, surface_links) = slots;
        Self { bone_rig, material_keys, surface_links }
    }

    fn bind_materials(&mut self, mat_list: &MaterialDefinitions) {
        self.surface_links = write_mesh_surface_links(
            &self.material_keys,
            &self.bone_rig.facet_materials,
            mat_list,
        );
    }

    /// The material row surface `surface` draws with. Nothing until the
    /// materials are loaded, or when the surface names none.
    pub fn surface_slot(&self, surf: usize) -> Option<SurfaceMaterialId> {
        let edge = self.surface_links.get(surf)?;
        edge.extent_position().map(SurfaceMaterialId::from_sequence)
    }
}

/// Every effect model, numbered in the order it first arrived. A model that
/// arrives again under the same name is ignored.
#[derive(Clone, Debug, Default)]
pub struct EffectMeshRoster {
    recs: Vec<((AssetCategoryScope, String), EffectMeshRecord)>,
    by_key: HashMap<(AssetCategoryScope, String), usize>,
}

impl EffectMeshRoster {
    fn put(&mut self, key: (AssetCategoryScope, String), entry: EffectMeshRecord) {
        if self.by_key.contains_key(&key) {
            return;
        }
        self.by_key.insert(key.clone(), self.recs.len());
        self.recs.push((key, entry));
    }

    /// Adds one Black Ops II model, its surfaces' materials being rows of
    /// `materials`.
    pub fn insert_skel(&mut self, bone_rig: MeshRig, mat_table: &SurfaceShaderRoster) {
        let key = (AssetCategoryScope::T6, bone_rig.name.clone());
        self.put(key, EffectMeshRecord::new(bone_rig, mat_table));
    }

    /// Takes in the models another catalog holds that this one lacks.
    pub fn merge_in(&mut self, rhs: Self) {
        for (key, entry) in rhs.recs {
            self.put(key, entry);
        }
    }

    /// Drops every model the effects do not name.
    pub fn retain_wanted(&mut self, wanted: &HashSet<(AssetCategoryScope, String)>) {
        self.recs.retain(|(key, _)| wanted.contains(key));
        self.by_key = self
            .recs
            .iter()
            .enumerate()
            .map(|(at, (key, _))| (key.clone(), at))
            .collect();
    }

    /// Binds every surface to the material row that has since been loaded.
    pub fn link_surfaces(&mut self, mat_list: &MaterialDefinitions) {
        for (_, entry) in &mut self.recs {
            entry.bind_materials(mat_list);
        }
    }

    pub fn slot_in(&self, scope: AssetCategoryScope, wanted: &str) -> Option<usize> {
        self.by_key.get(&(scope, wanted.to_owned())).copied()
    }

    pub fn record_at(&self, index: usize) -> Option<&EffectMeshRecord> {
        self.recs.get(index).map(|(_, entry)| entry)
    }

    pub fn label_at(&self, index: usize) -> Option<&str> {
        self.recs.get(index).map(|((_, name), _)| name.as_str())
    }

    /// Effect models are gathered with all their zones merged, so none of
    /// them has a zone of its own.
    pub fn source_zone(&self, _index: usize) -> ZoneCustodian {
        ZoneCustodian::default()
    }

    pub fn len(&self) -> usize {
        self.recs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.recs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_of(names: &[&str]) -> EffectMeshRoster {
        let mut catalog = EffectMeshRoster::default();
        let skels = names.iter().map(|name| MeshRig { name: name.to_string(), ..MeshRig::default() });
        skels.for_each(|bone_rig| catalog.insert_skel(bone_rig, &SurfaceShaderRoster::default()));
        catalog
    }

    #[test]
    fn models_are_numbered_in_arrival_order_and_the_first_one_stays() {
        let catalog = catalog_of(&["shell", "debris", "shell"]);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "shell"), Some(0));
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "debris"), Some(1));
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "gone"), None);
        assert_eq!(catalog.record_at(1).map(|e| e.bone_rig.name.as_str()), Some("debris"));
        assert_eq!(catalog.label_at(0), Some("shell"));
        assert_eq!(catalog.label_at(2), None);
    }

    #[test]
    fn absorbing_adds_only_the_models_not_yet_held() {
        let mut common = catalog_of(&["shell"]);
        common.merge_in(catalog_of(&["shell", "brass"]));
        assert_eq!(common.len(), 2);
        assert_eq!(common.slot_in(AssetCategoryScope::T6, "brass"), Some(1));
    }

    #[test]
    fn keeping_only_the_named_models_renumbers_the_rest() {
        let mut catalog = catalog_of(&["a", "b", "c"]);
        let wanted = HashSet::from([(AssetCategoryScope::T6, "c".to_owned()), (AssetCategoryScope::T6, "a".to_owned())]);
        catalog.retain_wanted(&wanted);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "a"), Some(0));
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "c"), Some(1));
        assert_eq!(catalog.slot_in(AssetCategoryScope::T6, "b"), None);
    }

    #[test]
    fn a_surface_with_no_bound_material_has_no_row() {
        let catalog = catalog_of(&["shell"]);
        let entry = catalog.record_at(0).expect("shell");
        assert_eq!(entry.surface_slot(0), None);
    }
}
