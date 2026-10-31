//! One weapon model as it is held for drawing other players' guns and guns
//! lying in the world: its skeleton and the materials its surfaces name.

use std::sync::Arc;

use crate::link::{bind_slot_keys, read_slot_keys};
use crate::material_slots::bound_material_name;
use crate::skel_model::MeshRig;
use asset_core::AssetCategoryScope;
use asset_core::MaterialKey;
use asset_material::MaterialDefinitions;
use asset_material::SurfaceShaderRoster;

#[derive(Clone, Debug)]
pub struct LevelWeaponRecord {
    pub namespace: AssetCategoryScope,
    pub bone_rig: Arc<MeshRig>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub surface_links: Vec<asset_core::AssetDependencyLink<asset_core::SurfaceShaderSpace>>,
}

impl LevelWeaponRecord {
    pub(crate) fn new(space: AssetCategoryScope, bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) -> Self {
        let defs = roster.map(|held| &**held);
        let (material_keys, surface_links) = read_slot_keys(&bone_rig.facet_materials, defs);
        let bone_rig = Arc::new(bone_rig);
        Self { namespace: space, bone_rig, material_keys, surface_links }
    }

    /// Binds the surfaces to the material rows that have since been loaded.
    pub(crate) fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        let pointers = &self.bone_rig.facet_materials;
        self.surface_links = bind_slot_keys(&self.material_keys, pointers, defs);
    }

    /// The material a surface drew from, when that material is loaded.
    pub fn material_name_of(&self, at: usize) -> Option<&str> {
        bound_material_name(&self.material_keys, &self.surface_links, at)
    }
}

