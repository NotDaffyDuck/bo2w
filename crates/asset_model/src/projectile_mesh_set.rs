//! The projectile models (rockets, grenades, missiles) a session loaded, in
//! the order they arrived.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use crate::asset_graph::{AssetEdge, capture_xmodel_material_slots, stamp_xmodel_material_edges};
use crate::material_slots::bound_material_name;
use crate::skel_model::ModelSkel;
use crate::{MaterialIndex, MaterialSpace};
use asset_core::{AssetNamespace, MaterialKey};
use asset_material::{MaterialCatalog, MaterialDefinitions};

/// A model's name in one namespace, lower-cased.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectileMeshKey {
    pub namespace: AssetNamespace,
    pub name: String,
}

impl ProjectileMeshKey {
    pub fn new(namespace: AssetNamespace, name: &str) -> Self {
        Self {
            namespace,
            name: name.to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProjectileMeshEntry {
    pub namespace: AssetNamespace,
    pub skel: Arc<ModelSkel>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub material_edges: Vec<AssetEdge<MaterialSpace>>,
}

impl ProjectileMeshEntry {
    fn new(skel: ModelSkel, materials: Option<&MaterialCatalog>) -> Self {
        let (material_keys, material_edges) =
            capture_xmodel_material_slots(&skel.surface_materials, materials.map(|c| &**c));
        Self {
            namespace: AssetNamespace::T6,
            skel: Arc::new(skel),
            material_keys,
            material_edges,
        }
    }

    fn key(&self) -> ProjectileMeshKey {
        ProjectileMeshKey::new(self.namespace, &self.skel.name)
    }

    /// Binds the surfaces to the material rows that have since been loaded.
    pub(crate) fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        self.material_edges = stamp_xmodel_material_edges(
            &self.material_keys,
            &self.skel.surface_materials,
            materials,
        );
    }

    /// The material a surface drew from, when that material is loaded.
    pub fn material_present_name(&self, surface: usize) -> Option<&str> {
        bound_material_name(&self.material_keys, &self.material_edges, surface)
    }

    /// The material row a surface is bound to.
    pub fn material_index(&self, surface: usize) -> Option<MaterialIndex> {
        self.material_edges.get(surface)?.bound()
    }
}

/// The finished set. Models are numbered in the order they were added.
#[derive(Clone, Debug, Default)]
pub struct ProjectileMeshCatalog {
    rows: Vec<(ProjectileMeshKey, ProjectileMeshEntry)>,
    by_key: HashMap<ProjectileMeshKey, usize>,
}

impl ProjectileMeshCatalog {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, namespace: AssetNamespace, name: &str) -> Option<&ProjectileMeshEntry> {
        let at = *self.by_key.get(&ProjectileMeshKey::new(namespace, name))?;
        self.get_at(at)
    }

    pub fn get_at(&self, index: usize) -> Option<&ProjectileMeshEntry> {
        self.rows.get(index).map(|(_, entry)| entry)
    }

    pub fn key_at(&self, index: usize) -> Option<&ProjectileMeshKey> {
        self.rows.get(index).map(|(key, _)| key)
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct ProjectileMeshBuild {
    catalog: ProjectileMeshCatalog,
}

impl Deref for ProjectileMeshBuild {
    type Target = ProjectileMeshCatalog;

    fn deref(&self) -> &ProjectileMeshCatalog {
        &self.catalog
    }
}

impl ProjectileMeshBuild {
    pub fn publish(self) -> ProjectileMeshCatalog {
        self.catalog
    }

    /// Adds a model; one already held under the same name stays.
    pub fn insert_captured(&mut self, skel: ModelSkel, materials: Option<&MaterialCatalog>) {
        let entry = ProjectileMeshEntry::new(skel, materials);
        let key = entry.key();
        if self.catalog.by_key.contains_key(&key) {
            return;
        }
        self.catalog.by_key.insert(key.clone(), self.catalog.rows.len());
        self.catalog.rows.push((key, entry));
    }

    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for (_, entry) in &mut self.catalog.rows {
            entry.resolve_materials(materials);
        }
    }
}
