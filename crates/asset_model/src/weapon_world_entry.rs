//! One weapon model as it is held for drawing other players' guns and guns
//! lying in the world: its skeleton and the materials its surfaces name.

use std::sync::Arc;

use crate::asset_graph::{AssetEdge, capture_xmodel_material_slots, stamp_xmodel_material_edges};
use crate::skel_model::ModelSkel;
use crate::MaterialSpace;
use asset_core::{AssetNamespace, MaterialKey};
use asset_material::{MaterialCatalog, MaterialDefinitions};

#[derive(Clone, Debug)]
pub struct WorldWeaponEntry {
    pub namespace: AssetNamespace,
    pub skel: Arc<ModelSkel>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub material_edges: Vec<AssetEdge<MaterialSpace>>,
}

impl WorldWeaponEntry {
    pub(crate) fn new(
        namespace: AssetNamespace,
        skel: ModelSkel,
        materials: Option<&MaterialCatalog>,
    ) -> Self {
        let (material_keys, material_edges) =
            capture_xmodel_material_slots(&skel.surface_materials, materials.map(|c| &**c));
        Self {
            namespace,
            skel: Arc::new(skel),
            material_keys,
            material_edges,
        }
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
        if !self.material_edges.get(surface)?.is_bound() {
            return None;
        }
        Some(self.material_keys.get(surface)?.as_ref()?.name.as_str())
    }
}
