//! The models effects spawn (shell casings, debris), found by name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use asset_core::MaterialKey;
use asset_material::MaterialDefinitions;
use asset_model::link::{capture_xmodel_material_slots, stamp_xmodel_material_edges};

use crate::{
    AssetEdge, AssetNamespace, MaterialCatalog, MaterialIndex, MaterialSpace, ModelSkel, ZoneOwner,
};

/// A model an effect spawns, and the material each of its surfaces draws.
#[derive(Clone, Debug)]
pub struct FxModelEntry {
    pub skel: Arc<ModelSkel>,
    material_keys: Vec<Option<MaterialKey>>,
    material_edges: Vec<AssetEdge<MaterialSpace>>,
}

impl FxModelEntry {
    fn new(skel: ModelSkel, materials: &MaterialCatalog) -> Self {
        let (material_keys, material_edges) =
            capture_xmodel_material_slots(&skel.surface_materials, Some(materials));
        Self {
            skel: Arc::new(skel),
            material_keys,
            material_edges,
        }
    }

    fn bind_materials(&mut self, materials: &MaterialDefinitions) {
        self.material_edges = stamp_xmodel_material_edges(
            &self.material_keys,
            &self.skel.surface_materials,
            materials,
        );
    }

    /// The material row surface `surface` draws with. Nothing until the
    /// materials are loaded, or when the surface names none.
    pub fn material_index(&self, surface: usize) -> Option<MaterialIndex> {
        let edge = self.material_edges.get(surface)?;
        edge.bound_index().map(MaterialIndex::from_order)
    }
}

type ModelKey = (AssetNamespace, String);

/// Every effect model, numbered in the order it first arrived. A model that
/// arrives again under the same name is ignored.
#[derive(Clone, Debug, Default)]
pub struct FxModelCatalog {
    rows: Vec<(ModelKey, FxModelEntry)>,
    by_key: HashMap<ModelKey, usize>,
}

impl FxModelCatalog {
    fn push(&mut self, key: ModelKey, entry: FxModelEntry) {
        if self.by_key.contains_key(&key) {
            return;
        }
        self.by_key.insert(key.clone(), self.rows.len());
        self.rows.push((key, entry));
    }

    /// Adds one Black Ops II model, its surfaces' materials being rows of
    /// `materials`.
    pub fn insert_skel(&mut self, skel: ModelSkel, materials: &MaterialCatalog) {
        let key = (AssetNamespace::T6, skel.name.clone());
        self.push(key, FxModelEntry::new(skel, materials));
    }

    /// Takes in the models another catalog holds that this one lacks.
    pub fn absorb(&mut self, other: Self) {
        for (key, entry) in other.rows {
            self.push(key, entry);
        }
    }

    /// Drops every model the effects do not name.
    pub fn keep_referenced(&mut self, wanted: &HashSet<(AssetNamespace, String)>) {
        self.rows.retain(|(key, _)| wanted.contains(key));
        self.by_key = self
            .rows
            .iter()
            .enumerate()
            .map(|(at, (key, _))| (key.clone(), at))
            .collect();
    }

    /// Binds every surface to the material row that has since been loaded.
    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for (_, entry) in &mut self.rows {
            entry.bind_materials(materials);
        }
    }

    pub fn index_in(&self, namespace: AssetNamespace, name: &str) -> Option<usize> {
        self.by_key.get(&(namespace, name.to_owned())).copied()
    }

    pub fn get_at(&self, index: usize) -> Option<&FxModelEntry> {
        self.rows.get(index).map(|(_, entry)| entry)
    }

    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.rows.get(index).map(|((_, name), _)| name.as_str())
    }

    /// Effect models are gathered with all their zones merged, so none of
    /// them has a zone of its own.
    pub fn zone_of(&self, _index: usize) -> ZoneOwner {
        ZoneOwner::default()
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

    fn catalog_of(names: &[&str]) -> FxModelCatalog {
        let mut catalog = FxModelCatalog::default();
        for name in names {
            let skel = ModelSkel {
                name: (*name).to_owned(),
                ..ModelSkel::default()
            };
            catalog.insert_skel(skel, &MaterialCatalog::default());
        }
        catalog
    }

    #[test]
    fn models_are_numbered_in_arrival_order_and_the_first_one_stays() {
        let catalog = catalog_of(&["shell", "debris", "shell"]);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.index_in(AssetNamespace::T6, "shell"), Some(0));
        assert_eq!(catalog.index_in(AssetNamespace::T6, "debris"), Some(1));
        assert_eq!(catalog.index_in(AssetNamespace::T6, "gone"), None);
        assert_eq!(catalog.get_at(1).map(|e| e.skel.name.as_str()), Some("debris"));
        assert_eq!(catalog.name_at(0), Some("shell"));
        assert_eq!(catalog.name_at(2), None);
    }

    #[test]
    fn absorbing_adds_only_the_models_not_yet_held() {
        let mut common = catalog_of(&["shell"]);
        common.absorb(catalog_of(&["shell", "brass"]));
        assert_eq!(common.len(), 2);
        assert_eq!(common.index_in(AssetNamespace::T6, "brass"), Some(1));
    }

    #[test]
    fn keeping_only_the_named_models_renumbers_the_rest() {
        let mut catalog = catalog_of(&["a", "b", "c"]);
        let wanted = HashSet::from([(AssetNamespace::T6, "c".to_owned()), (AssetNamespace::T6, "a".to_owned())]);
        catalog.keep_referenced(&wanted);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.index_in(AssetNamespace::T6, "a"), Some(0));
        assert_eq!(catalog.index_in(AssetNamespace::T6, "c"), Some(1));
        assert_eq!(catalog.index_in(AssetNamespace::T6, "b"), None);
    }

    #[test]
    fn a_surface_with_no_bound_material_has_no_row() {
        let catalog = catalog_of(&["shell"]);
        let entry = catalog.get_at(0).expect("shell");
        assert_eq!(entry.material_index(0), None);
    }
}
