use std::collections::HashMap;
use std::sync::Arc;

use asset_model::MeshRig;
use bevy::prelude::*;

/// Ordering label for every system that writes an [`SkeletonSceneHandoff`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkeletonSceneStage;

/// One model of a submitted DObj: the skeleton, the LOD it is drawn at
/// (negative: none chosen) and its bone count, saturated to a byte.
#[derive(Clone, Debug)]
pub struct SceneSkeletonModel {
    pub lod: i8,
    pub joint_total: u8,
    pub bone_rig: Arc<MeshRig>,
}

impl SceneSkeletonModel {
    fn new(bone_rig: Arc<MeshRig>, lod: i8) -> Self {
        let joint_total = bone_rig.bones.len().min(usize::from(u8::MAX)) as u8;
        Self { bone_rig, joint_total, lod }
    }
}

/// A DObj the scene should hold this frame, as anim systems describe it.
#[derive(Message)]
#[derive(Clone, Debug)]
pub struct SkeletonSceneHandoff {
    pub render_effect_flags: u32,
    pub carries_anim_graph: bool,
    pub origin: [f32; 3],
    pub illum_origin: [f32; 3],
    pub radius: Option<f32>,
    pub ent_ix: u32,
    pub rot_quat: Option<[f32; 4]>,
    pub occupy_model_tally: u8,
    pub models: Vec<SceneSkeletonModel>,
    pub hidden_mask: [u32; 5],
    pub keep_skinned_verts: bool,
}

/// One `Arc` per skeleton name, so submissions share a skeleton rather than
/// each carrying a copy.
#[derive(Resource, Default)]
pub struct SceneBonePoseSet {
    by_label: HashMap<String, Arc<MeshRig>>,
    seeded_for: Option<(usize, u64)>,
}

impl SceneBonePoseSet {
    /// Pre-registers the remote-player kit bodies and heads (both axes) and
    /// every world weapon, once per pair of catalogs.
    pub fn seed(&mut self, bodies: &assets::ReadiedCharacterBodies, world: &assets::ReadiedWorldWeapons) {
        let catalogs = (Arc::as_ptr(&bodies.0) as usize, world.0.identity());
        if self.seeded_for.is_some_and(|done| done == catalogs) {
            return;
        }
        let kit_set = bodies.0.kits();
        let parts = [false, true]
            .into_iter()
            .filter_map(|axis| kit_set.kit(axis))
            .flat_map(|kit| std::iter::once(&kit.torso).chain(kit.skull.as_ref()));
        for part in parts.filter_map(|name| bodies.0.by_label(name)) {
            let bone_rig = &part.bone_rig;
            self.share_rig(&bone_rig.name, || Arc::new(bone_rig.clone()));
        }
        for index in 0..world.0.len() {
            if let Some(weapon) = world.0.at_row(index) {
                self.share_rig(&weapon.bone_rig.name, || Arc::clone(&weapon.bone_rig));
            }
        }
        self.seeded_for = Some(catalogs);
    }

    /// The shared copy for `name`, made by `make` when nothing is held yet.
    fn share_rig(&mut self, name: &str, make: impl FnOnce() -> Arc<MeshRig>) -> Arc<MeshRig> {
        Arc::clone(self.by_label.entry(name.to_owned()).or_insert_with(make))
    }

    /// The model for `name`, registering `bone_rig` as its shared copy if none is.
    pub fn shared(&mut self, name: &str, bone_rig: &Arc<MeshRig>, lod: i8) -> SceneSkeletonModel {
        let held = self.share_rig(name, || Arc::clone(bone_rig));
        SceneSkeletonModel::new(held, lod)
    }

    /// As [`shared`](Self::shared), for a skeleton only held by reference: a
    /// clone is made the first time its name is seen.
    pub fn xmodel(&mut self, name: &str, bone_rig: &MeshRig, lod: i8) -> SceneSkeletonModel {
        let held = self.share_rig(name, || Arc::new(bone_rig.clone()));
        SceneSkeletonModel::new(held, lod)
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    use crate::motion::test_skel::skel_with_bones;

    #[test]
    fn a_name_keeps_the_first_skeleton_it_was_given() {
        let mut skels = SceneBonePoseSet::default();
        let first = Arc::new(skel_with_bones("body", 3));
        let a = skels.shared("body", &first, 0);
        let b = skels.shared("body", &Arc::new(skel_with_bones("body", 9)), 1);
        assert!(Arc::ptr_eq(&a.bone_rig, &b.bone_rig));
        assert_eq!((a.joint_total, b.joint_total, b.lod), (3, 3, 1));
    }

    #[test]
    fn a_borrowed_skeleton_is_cloned_once() {
        let mut skels = SceneBonePoseSet::default();
        let source = skel_with_bones("gun", 2);
        let a = skels.xmodel("gun", &source, 0);
        let b = skels.xmodel("gun", &source, 2);
        assert!(Arc::ptr_eq(&a.bone_rig, &b.bone_rig));
        assert_eq!(b.lod, 2);
    }

    #[test]
    fn the_bone_count_saturates_at_a_byte() {
        let mut skels = SceneBonePoseSet::default();
        let model = skels.shared("big", &Arc::new(skel_with_bones("big", 300)), 0);
        assert_eq!(model.joint_total, u8::MAX);
    }
}
