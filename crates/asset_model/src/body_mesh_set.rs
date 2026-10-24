//! The soldier models a session loaded (bodies, heads, first-person arms),
//! found by name, and the kit each team wears out of them.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::OnceLock;

use crate::material_slots::bound_material_name;
use crate::skel_model::ModelSkel;
use crate::soldier_kit::{SoldierKit, SoldierKits, body_has_tp_attach_bones, is_body_model, soldier_kits};
use crate::{AssetEdge, MaterialSpace};
use asset_core::{AssetNamespace, MaterialKey};
use asset_material::{MaterialCatalog, MaterialDefinitions};

use crate::asset_graph::{capture_xmodel_material_slots, stamp_xmodel_material_edges};

#[derive(Clone, Debug)]
pub struct BodyMeshEntry {
    pub skel: ModelSkel,
    pub namespace: AssetNamespace,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub material_edges: Vec<AssetEdge<MaterialSpace>>,
}

impl BodyMeshEntry {
    fn new(skel: ModelSkel, materials: Option<&MaterialCatalog>) -> Self {
        let (material_keys, material_edges) =
            capture_xmodel_material_slots(&skel.surface_materials, materials.map(|c| &**c));
        Self {
            skel,
            namespace: AssetNamespace::T6,
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
        bound_material_name(&self.material_keys, &self.material_edges, surface)
    }

    pub fn bone_count(&self) -> usize {
        self.skel.bones.len()
    }

    pub fn bone_names(&self) -> &[String] {
        &self.skel.bone_names
    }
}

/// The finished set. The kits are worked out the first time they are asked
/// for and kept.
#[derive(Clone, Debug, Default)]
pub struct BodyMeshCatalog {
    by_name: HashMap<String, BodyMeshEntry>,
    kits: OnceLock<SoldierKits>,
}

impl BodyMeshCatalog {
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&BodyMeshEntry> {
        self.by_name.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Each team's kit. A body model counts only if it has the bones to
    /// carry a head and a weapon.
    pub fn kits(&self) -> &SoldierKits {
        self.kits.get_or_init(|| {
            let usable: Vec<String> = self
                .by_name
                .iter()
                .filter(|(name, entry)| {
                    !is_body_model(name) || body_has_tp_attach_bones(entry.bone_names())
                })
                .map(|(name, _)| name.clone())
                .collect();
            soldier_kits(&usable)
        })
    }

    /// One line on the set and one per team on the kit it wears, for the load
    /// report.
    pub fn report_lines(&self) -> Vec<String> {
        let kits = self.kits();
        let mut lines = vec![format!("bodies: {} captured", self.by_name.len())];
        let teams = [("allies", &kits.allies), ("axis", &kits.axis)];
        for (team, kit) in teams {
            let Some(SoldierKit { body, head, arms }) = kit else {
                continue;
            };
            let shape = match self.by_name.get(body) {
                Some(entry) => format!(
                    "{} bones, {} vertices",
                    entry.bone_count(),
                    entry.skel.positions.len()
                ),
                None => "model missing".to_owned(),
            };
            lines.push(format!(
                "bodies [{team}]: {body} ({shape}); head {}, arms {}",
                head.as_deref().unwrap_or("none"),
                arms.as_deref().unwrap_or("none"),
            ));
        }
        if kits.allies.is_none() && kits.axis.is_none() && !self.by_name.is_empty() {
            lines.push("bodies: no kit could be put together".to_owned());
        }
        lines
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct BodyMeshBuild {
    catalog: BodyMeshCatalog,
}

impl Deref for BodyMeshBuild {
    type Target = BodyMeshCatalog;

    fn deref(&self) -> &BodyMeshCatalog {
        &self.catalog
    }
}

impl BodyMeshBuild {
    pub fn publish(self) -> BodyMeshCatalog {
        self.catalog
    }

    pub fn insert(&mut self, skel: ModelSkel) {
        self.insert_captured(skel, None);
    }

    /// Adds a model; one already held under the same name stays.
    pub fn insert_captured(&mut self, skel: ModelSkel, materials: Option<&MaterialCatalog>) {
        self.catalog.kits = OnceLock::new();
        self.catalog
            .by_name
            .entry(skel.name.clone())
            .or_insert_with(|| BodyMeshEntry::new(skel, materials));
    }

    pub fn resolve_materials(&mut self, materials: &MaterialDefinitions) {
        for entry in self.catalog.by_name.values_mut() {
            entry.resolve_materials(materials);
        }
    }
}
