//! The anim side of a DObj: which part each of a clip's tracks drives, and
//! `DObjCalcAnim`'s blend of the playing anims into a local pose.

use dobj_t6::{DObj, LocalPose, PartBits, PartPose};
use glam::Mat4;

use crate::AnimClip;

pub struct AnimInstance<'a> {
    pub clip: &'a AnimClip,
    pub tracks: &'a [Option<usize>],
    pub time: f32,
    pub weight: f32,
    pub parts: Option<&'a PartBits>,
}

pub trait DObjAnim {
    fn tracks_for(&self, clip: &AnimClip) -> Vec<Option<usize>>;

    fn calc_anim(&self, anims: &[AnimInstance<'_>], requested: &PartBits) -> LocalPose;

    /// BO2's `DObjCalcAnim` blend (facts/xanim-dobj-t6.md 1 and 3): each
    /// anim adds its samples into a [`dobj_t6::AnimMat`] per bone (rotations
    /// flipped toward the sum, translation over its own weight total); part
    /// 159 is never written by anim data; a bone no anim drove gets its
    /// base pose (identity for root bones) and no translation.
    fn calc_anim_masked(
        &self,
        anims: &[AnimInstance<'_>],
        requested: &PartBits,
    ) -> (LocalPose, Vec<bool>);

    fn pose(&self, anims: &[AnimInstance<'_>], requested: &PartBits, placement: Mat4)
    -> Vec<Mat4>;

    fn pose_with_controller(
        &self,
        anims: &[AnimInstance<'_>],
        requested: &PartBits,
        placement: Mat4,
        controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
    ) -> Vec<Mat4>;
}

impl DObjAnim for DObj {
    fn tracks_for(&self, clip: &AnimClip) -> Vec<Option<usize>> {
        let mut first_by_name = std::collections::HashMap::with_capacity(self.bones.len());
        for (index, bone) in self.bones.iter().enumerate() {
            first_by_name.entry(bone.name.as_str()).or_insert(index);
        }
        clip.tracks
            .iter()
            .map(|track| first_by_name.get(track.name.as_str()).copied())
            .collect()
    }

    fn calc_anim(&self, anims: &[AnimInstance<'_>], requested: &PartBits) -> LocalPose {
        self.calc_anim_masked(anims, requested).0
    }

    fn calc_anim_masked(
        &self,
        anims: &[AnimInstance<'_>],
        requested: &PartBits,
    ) -> (LocalPose, Vec<bool>) {
        let mut mats = vec![dobj_t6::AnimMat::ZERO; self.bones.len()];
        let mut controlled = vec![false; self.bones.len()];

        for anim in anims {
            if anim.weight <= 0.0 {
                continue;
            }
            for (track, bone) in anim.tracks.iter().enumerate() {
                let Some(bone) = *bone else { continue };
                if bone == dobj_t6::IGNORED_PART || !requested.get(bone) {
                    continue;
                }
                if let Some(parts) = anim.parts
                    && !parts.get(bone)
                {
                    continue;
                }
                let Some(sample) = anim.clip.sample_track(track, anim.time) else {
                    continue;
                };
                controlled[bone] = true;
                let mat = &mut mats[bone];
                match sample.rotation {
                    Some(q) => mat.add_quat(q, anim.weight),
                    None => mat.add_identity(anim.weight),
                }
                if let Some(t) = sample.translation {
                    mat.add_trans(t, anim.weight);
                }
            }
        }

        let locals = (0..self.bones.len())
            .map(|bone| {
                if controlled[bone] {
                    let (rotation, translation) = mats[bone].finish();
                    PartPose {
                        rotation,
                        translation,
                    }
                } else {
                    PartPose::at_rest(self.bones[bone].bind_rotation.to_array())
                }
            })
            .collect();
        (LocalPose::new(locals), controlled)
    }

    fn pose(
        &self,
        anims: &[AnimInstance<'_>],
        requested: &PartBits,
        placement: Mat4,
    ) -> Vec<Mat4> {
        self.pose_with_controller(anims, requested, placement, |_, _, _| {})
    }

    fn pose_with_controller(
        &self,
        anims: &[AnimInstance<'_>],
        requested: &PartBits,
        placement: Mat4,
        controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
    ) -> Vec<Mat4> {
        let mut locals = self.calc_anim(anims, requested);
        controller(self, requested, &mut locals);
        self.apply_duplicates(&mut locals);
        self.compose(&locals, placement)
    }
}
