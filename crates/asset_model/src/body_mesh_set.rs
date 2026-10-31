//! The soldier models a session loaded (bodies, heads, first-person arms),
//! found by name, and the kit each team wears out of them.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::OnceLock;

use crate::link::{bind_slot_keys, read_slot_keys};
use crate::material_slots::bound_material_name;
use crate::skel_model::MeshRig;
use crate::soldier_kit::SoldierKitTable;
use crate::soldier_kit::TrooperOutfit;
use crate::soldier_kit::bones_take_third_person_mounts;
use crate::soldier_kit::kit_table_from;
use crate::soldier_kit::names_a_body;
use asset_core::AssetCategoryScope;
use asset_core::MaterialKey;
use asset_material::MaterialDefinitions;
use asset_material::SurfaceShaderRoster;

#[derive(Clone, Debug)]
pub struct CharacterMeshRecord {
    pub bone_rig: MeshRig,
    pub namespace: AssetCategoryScope,
    /// What each surface's material is called, by surface.
    pub material_keys: Vec<Option<MaterialKey>>,
    /// Which material row each surface is bound to, by surface.
    pub surface_links: Vec<asset_core::AssetDependencyLink<asset_core::SurfaceShaderSpace>>,
}

impl CharacterMeshRecord {
    fn new(bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) -> Self {
        let defs = roster.map(|held| &**held);
        let (material_keys, surface_links) = read_slot_keys(&bone_rig.facet_materials, defs);
        Self { namespace: AssetCategoryScope::T6, material_keys, surface_links, bone_rig }
    }

    /// Binds the surfaces to the material rows that have since been loaded.
    fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        let pointers = &self.bone_rig.facet_materials;
        self.surface_links = bind_slot_keys(&self.material_keys, pointers, defs);
    }

    /// The material a surface drew from, when that material is loaded.
    pub fn material_name_of(&self, at: usize) -> Option<&str> {
        bound_material_name(&self.material_keys, &self.surface_links, at)
    }

    pub fn joint_total(&self) -> usize {
        let MeshRig { bones, .. } = &self.bone_rig;
        bones.iter().count()
    }

    pub fn joint_labels(&self) -> &[String] {
        self.bone_rig.joint_labels.as_slice()
    }
}

/// The finished set. The kits are worked out the first time they are asked
/// for and kept.
#[derive(Clone, Debug, Default)]
pub struct CharacterMeshRoster {
    models: HashMap<String, CharacterMeshRecord>,
    kits: OnceLock<SoldierKitTable>,
}

impl CharacterMeshRoster {
    pub fn len(&self) -> usize {
        HashMap::len(&self.models)
    }

    pub fn is_empty(&self) -> bool {
        HashMap::is_empty(&self.models)
    }

    pub fn by_label(&self, model_name: &str) -> Option<&CharacterMeshRecord> {
        HashMap::get(&self.models, model_name)
    }

    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.models.keys().map(String::as_str)
    }

    /// Each team's kit. A body model counts only if it has the bones to
    /// carry a head and a weapon.
    pub fn kits(&self) -> &SoldierKitTable {
        self.kits.get_or_init(|| {
            let mut usable = Vec::with_capacity(self.models.len());
            for (model_name, held) in &self.models {
                let carries = bones_take_third_person_mounts(held.joint_labels());
                if carries || !names_a_body(model_name) {
                    usable.push(model_name.clone());
                }
            }
            kit_table_from(&usable)
        })
    }

    /// One line on the set and one per team on the kit it wears, for the load
    /// report.
    pub fn summary_rows(&self) -> Vec<String> {
        let worn = self.kits();
        let summary = format!("bodies: {} captured", self.models.len());
        let per_team = [("allies", &worn.allies), ("axis", &worn.axis)]
            .into_iter()
            .filter_map(|(side, slot)| Some(self.kit_line(side, slot.as_ref()?)));
        let nobody = worn.allies.is_none() && worn.axis.is_none();
        let verdict = (nobody && !self.models.is_empty())
            .then(|| "bodies: no kit could be put together".to_owned());
        std::iter::once(summary).chain(per_team).chain(verdict).collect()
    }

    fn kit_line(&self, side: &str, kit: &TrooperOutfit) -> String {
        let TrooperOutfit { torso: body, skull: head, limbs: arms } = kit;
        let shape = self.models.get(body).map_or_else(
            || "model missing".to_owned(),
            |found| {
                let vertices = found.bone_rig.positions.len();
                format!("{} bones, {vertices} vertices", found.joint_total())
            },
        );
        let name_or_none = |part: &Option<String>| part.clone().unwrap_or_else(|| "none".into());
        format!(
            "bodies [{side}]: {body} ({shape}); head {}, arms {}",
            name_or_none(head),
            name_or_none(arms),
        )
    }
}

/// A set still being filled. It reads as the set it will become.
#[derive(Clone, Debug, Default)]
pub struct BodyMeshBuild {
    filling: CharacterMeshRoster,
}

impl Deref for BodyMeshBuild {
    type Target = CharacterMeshRoster;

    fn deref(&self) -> &CharacterMeshRoster {
        &self.filling
    }
}

impl BodyMeshBuild {
    pub fn publish(self) -> CharacterMeshRoster {
        let Self { filling } = self;
        filling
    }

    pub fn take(&mut self, bone_rig: MeshRig) {
        self.add_captured(bone_rig, None)
    }

    /// Adds a model; one already held under the same name stays.
    pub fn add_captured(&mut self, bone_rig: MeshRig, roster: Option<&SurfaceShaderRoster>) {
        self.filling.kits = OnceLock::new();
        if !self.filling.models.contains_key(&bone_rig.name) {
            let model_name = bone_rig.name.clone();
            self.filling.models.insert(model_name, CharacterMeshRecord::new(bone_rig, roster));
        }
    }

    pub fn bind_materials(&mut self, defs: &MaterialDefinitions) {
        self.filling.models.values_mut().for_each(|held| held.bind_materials(defs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skel_named(name: &str, bones: &[&str]) -> MeshRig {
        MeshRig {
            name: name.to_owned(),
            joint_labels: bones.iter().map(|b| b.to_string()).collect(),
            ..MeshRig::default()
        }
    }

    fn carrying_bones() -> [&'static str; 2] {
        [
            xmodel_runtime::THIRD_PERSON_HEAD_BONE,
            xmodel_runtime::THIRD_PERSON_GUN_BONES[0],
        ]
    }

    fn built(models: &[MeshRig]) -> CharacterMeshRoster {
        let mut build = BodyMeshBuild::default();
        models.iter().cloned().for_each(|bone_rig| build.take(bone_rig));
        build.publish()
    }

    #[test]
    fn an_empty_set_reports_one_line() {
        assert_eq!(built(&[]).summary_rows(), ["bodies: 0 captured"]);
    }

    #[test]
    fn a_body_without_attach_bones_makes_no_kit() {
        let set = built(&[skel_named("mp_body_nva_assault", &["j_spine"])]);
        assert_eq!(
            set.summary_rows(),
            [
                "bodies: 1 captured",
                "bodies: no kit could be put together"
            ]
        );
    }

    #[test]
    fn a_carrying_body_is_reported_with_its_team_and_shape() {
        let set = built(&[
            skel_named("mp_body_nva_assault", &carrying_bones()),
            skel_named("head_nva_a", &[]),
        ]);
        assert_eq!(
            set.summary_rows(),
            [
                "bodies: 2 captured",
                "bodies [axis]: mp_body_nva_assault (0 bones, 0 vertices); head head_nva_a, arms none"
            ]
        );
    }

    #[test]
    fn a_model_held_twice_keeps_the_first() {
        let mut build = BodyMeshBuild::default();
        build.take(skel_named("head_x", &["a"]));
        build.take(skel_named("head_x", &["b", "c"]));
        let set = build.publish();
        assert_eq!(set.len(), 1);
        assert_eq!(set.by_label("head_x").map(CharacterMeshRecord::joint_labels), Some(&["a".to_owned()][..]));
    }
}
