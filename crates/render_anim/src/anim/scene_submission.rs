use std::collections::HashMap;
use std::sync::Arc;

use asset_model::ModelSkel;
use bevy::prelude::*;

/// Ordering label for every system that writes an [`AnimDObjSceneSubmission`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimSceneSubmit;

/// One model of a submitted DObj: the skeleton, the LOD it is drawn at
/// (negative: none chosen) and its bone count, saturated to a byte.
#[derive(Clone, Debug)]
pub struct AnimDObjSceneModel {
    pub lod: i8,
    pub bone_count: u8,
    pub skel: Arc<ModelSkel>,
}

impl AnimDObjSceneModel {
    fn new(skel: Arc<ModelSkel>, lod: i8) -> Self {
        Self {
            lod,
            bone_count: u8::try_from(skel.bones.len()).unwrap_or(u8::MAX),
            skel,
        }
    }
}

/// A DObj the scene should hold this frame, as anim systems describe it.
#[derive(Message, Clone, Debug)]
pub struct AnimDObjSceneSubmission {
    pub render_fx_flags: u32,
    pub has_tree: bool,
    pub origin: [f32; 3],
    pub lighting_origin: [f32; 3],
    pub radius: Option<f32>,
    pub entnum: u32,
    pub quat: Option<[f32; 4]>,
    pub occupy_model_n: u8,
    pub models: Vec<AnimDObjSceneModel>,
    pub hide_part_bits: [u32; 5],
    pub store_skin: bool,
}

/// One `Arc` per skeleton name, so submissions share a skeleton rather than
/// each carrying a copy.
#[derive(Resource, Default)]
pub struct AnimDObjSceneSkels {
    by_name: HashMap<String, Arc<ModelSkel>>,
    seeded_for: Option<(usize, u64)>,
}

impl AnimDObjSceneSkels {
    /// Pre-registers the remote-player kit bodies and heads (both axes) and
    /// every world weapon, once per pair of catalogs.
    pub fn seed(&mut self, bodies: &assets::PreparedBodies, world: &assets::PreparedWorldWeapons) {
        let owner = (Arc::as_ptr(&bodies.0) as usize, world.0.identity());
        if self.seeded_for == Some(owner) {
            return;
        }
        let kits = bodies.0.kits();
        let wanted = [false, true]
            .into_iter()
            .filter_map(|axis| kits.kit(axis))
            .flat_map(|kit| std::iter::once(&kit.body).chain(kit.head.as_ref()));
        for entry in wanted.filter_map(|name| bodies.0.get(name)) {
            self.by_name
                .entry(entry.skel.name.clone())
                .or_insert_with(|| Arc::new(entry.skel.clone()));
        }
        for entry in (0..world.0.len()).filter_map(|index| world.0.get_at(index)) {
            self.by_name
                .entry(entry.skel.name.clone())
                .or_insert_with(|| Arc::clone(&entry.skel));
        }
        self.seeded_for = Some(owner);
    }

    /// The model for `name`, registering `skel` as its shared copy if none is.
    pub fn shared(&mut self, name: &str, skel: &Arc<ModelSkel>, lod: i8) -> AnimDObjSceneModel {
        let held = self
            .by_name
            .entry(name.to_owned())
            .or_insert_with(|| Arc::clone(skel));
        AnimDObjSceneModel::new(Arc::clone(held), lod)
    }

    /// As [`shared`](Self::shared), for a skeleton only held by reference: a
    /// clone is made the first time its name is seen.
    pub fn model(&mut self, name: &str, skel: &ModelSkel, lod: i8) -> AnimDObjSceneModel {
        let held = self
            .by_name
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(skel.clone()));
        AnimDObjSceneModel::new(Arc::clone(held), lod)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::anim::test_skel::skel_with_bones as skel;

    #[test]
    fn a_name_keeps_the_first_skeleton_it_was_given() {
        let mut skels = AnimDObjSceneSkels::default();
        let first = Arc::new(skel("body", 3));
        let a = skels.shared("body", &first, 0);
        let b = skels.shared("body", &Arc::new(skel("body", 9)), 1);
        assert!(Arc::ptr_eq(&a.skel, &b.skel));
        assert_eq!((a.bone_count, b.bone_count, b.lod), (3, 3, 1));
    }

    #[test]
    fn a_borrowed_skeleton_is_cloned_once() {
        let mut skels = AnimDObjSceneSkels::default();
        let source = skel("gun", 2);
        let a = skels.model("gun", &source, 0);
        let b = skels.model("gun", &source, 2);
        assert!(Arc::ptr_eq(&a.skel, &b.skel));
        assert_eq!(b.lod, 2);
    }

    #[test]
    fn the_bone_count_saturates_at_a_byte() {
        let mut skels = AnimDObjSceneSkels::default();
        let model = skels.shared("big", &Arc::new(skel("big", 300)), 0);
        assert_eq!(model.bone_count, u8::MAX);
    }
}
