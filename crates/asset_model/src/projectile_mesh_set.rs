//! The projectile models (rockets, grenades, missiles) a session loaded, in
//! the order they arrived.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use crate::link::{bind_slot_keys, read_slot_keys};
use crate::material_slots::bound_material_name;
use crate::skel_model::MeshRig;
use asset_core::AssetCategoryScope;
use asset_core::MaterialKey;
use asset_material::MaterialDefinitions;
use asset_material::SurfaceShaderRoster;

/// A model's name in one namespace, lower-cased.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectileMeshKey {
    pub namespace: AssetCategoryScope,
    pub name: String,
}

impl ProjectileMeshKey {
    pub fn new(space: AssetCategoryScope, label: &str) -> Self {
        let name = label.to_ascii_lowercase();
        Self { name, namespace: space }
    }
}

#[derive(Clone, Debug)]
pub struct MissileMeshRecord {
    pub namespace: AssetCategoryScope,
    pub bone_rig: Arc<MeshRig>,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub surface_links: Vec<asset_core::AssetDependencyLink<asset_core::SurfaceShaderSpace>>,
}

impl MissileMeshRecord {
    fn new(bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) -> Self {
        let defs = roster.map(|held| &**held);
        let (material_keys, surface_links) = read_slot_keys(&bone_rig.facet_materials, defs);
        let bone_rig = Arc::new(bone_rig);
        Self { namespace: AssetCategoryScope::T6, bone_rig, material_keys, surface_links }
    }

    fn filed_as(&self) -> ProjectileMeshKey {
        let label = self.bone_rig.name.as_str();
        ProjectileMeshKey::new(self.namespace, label)
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

    /// The material row a surface is bound to.
    pub fn bound_material(&self, at: usize) -> Option<asset_core::SurfaceMaterialId> {
        self.surface_links.get(at).and_then(asset_core::AssetDependencyLink::bound)
    }
}

/// The finished set. Models are numbered in the order they came in.
#[derive(Clone, Debug, Default)]
pub struct MissileMeshRoster {
    held: Vec<(ProjectileMeshKey, MissileMeshRecord)>,
    rows: HashMap<ProjectileMeshKey, usize>,
}

impl MissileMeshRoster {
    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn by_label(&self, space: AssetCategoryScope, label: &str) -> Option<&MissileMeshRecord> {
        let row = self.rows.get(&ProjectileMeshKey::new(space, label)).copied()?;
        self.at_row(row)
    }

    pub fn at_row(&self, row: usize) -> Option<&MissileMeshRecord> {
        self.held.get(row).map(|(_, record)| record)
    }

    pub fn key_at(&self, row: usize) -> Option<&ProjectileMeshKey> {
        let (filed, _) = self.held.get(row)?;
        Some(filed)
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct ProjectileMeshBuild {
    filling: MissileMeshRoster,
}

impl Deref for ProjectileMeshBuild {
    type Target = MissileMeshRoster;

    fn deref(&self) -> &MissileMeshRoster {
        &self.filling
    }
}

impl ProjectileMeshBuild {
    pub fn publish(self) -> MissileMeshRoster {
        let Self { filling } = self;
        filling
    }

    /// Adds a model; one already held under the same name stays.
    pub fn add_captured(&mut self, bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) {
        let record = MissileMeshRecord::new(bone_rig, roster);
        let filed = record.filed_as();
        let set = &mut self.filling;
        if !set.rows.contains_key(&filed) {
            set.rows.insert(filed.clone(), set.held.len());
            set.held.push((filed, record));
        }
    }

    pub fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        self.filling.held.iter_mut().for_each(|(_, record)| record.bind_materials(defs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(label: &str, vertex_points: usize) -> MeshRig {
        MeshRig {
            name: label.to_owned(),
            positions: vec![[0.0; 3]; vertex_points],
            ..MeshRig::default()
        }
    }

    #[test]
    fn a_name_already_held_keeps_its_first_model_and_row() {
        let mut build = ProjectileMeshBuild::default();
        assert!(build.is_empty());
        build.add_captured(rig("Rocket", 1), None);
        build.add_captured(rig("grenade", 2), None);
        build.add_captured(rig("ROCKET", 3), None);
        let set = build.publish();
        assert_eq!(set.len(), 2);
        let rocket = set.by_label(AssetCategoryScope::T6, "rocket").unwrap();
        assert_eq!(rocket.bone_rig.positions.len(), 1);
        assert_eq!(set.at_row(1).map(|e| e.bone_rig.name.as_str()), Some("grenade"));
        assert_eq!(set.key_at(1).map(|k| k.name.as_str()), Some("grenade"));
        assert!(set.by_label(AssetCategoryScope::T6, "missing").is_none());
        assert!(set.at_row(2).is_none());
    }

    #[test]
    fn a_surface_with_no_material_has_no_row_or_name() {
        let mut build = ProjectileMeshBuild::default();
        build.add_captured(rig("rocket", 0), None);
        build.bind_materials(&MaterialDefinitions::default());
        let record = build.by_label(AssetCategoryScope::T6, "rocket").unwrap();
        assert_eq!(record.bound_material(0), None);
        assert_eq!(record.material_name_of(0), None);
    }
}
