//! How a DObj plays animation: which of its parts each track of a clip moves,
//! and the blend of every playing anim into one local pose (BO2's
//! `DObjCalcAnim`).

use std::collections::HashMap;

use dobj_t6::{AnimMat, PosedSkeleton, IGNORED_PART, LocalPose, PartSet, PartPose};
use glam::Mat4;

use crate::SkeletalClip;

/// One anim that is currently playing on a DObj.
pub struct ClipPlayback<'a> {
    pub clip: &'a SkeletalClip,
    pub tracks: &'a [Option<usize>],
    pub time: f32,
    pub weight: f32,
    pub parts: Option<&'a PartSet>,
}

impl ClipPlayback<'_> {
    /// The part a track of this anim may drive, when the anim is allowed to
    /// touch it: not the ignored part, asked for by the caller, and inside this
    /// anim's own part filter if it has one.
    fn drivable_part(&self, lane: usize, wanted: &PartSet) -> Option<usize> {
        let part = (*self.tracks.get(lane)?)?;
        let allowed = part != IGNORED_PART
            && wanted.get(part)
            && self.parts.is_none_or(|filter| filter.get(part));
        allowed.then_some(part)
    }
}

/// The running sum of every playing anim, per part of a DObj.
struct Blend {
    sums: Vec<AnimMat>,
    driven: Vec<bool>,
}

impl Blend {
    fn new(part_count: usize) -> Self {
        Blend {
            sums: vec![AnimMat::ZERO; part_count],
            driven: vec![false; part_count],
        }
    }

    /// Adds every sample of one playing anim into the parts its tracks drive.
    fn accumulate(&mut self, play: &ClipPlayback, want: &PartSet) {
        let weight = play.weight;
        let drives = (0..play.tracks.len())
            .filter_map(|track| Some((track, play.drivable_part(track, want)?)));
        for (track, part) in drives {
            let Some(sample) = play.clip.lane_at(track, play.time) else {
                continue;
            };
            self.driven[part] = true;
            let sum = &mut self.sums[part];
            match sample.rotation {
                Some(rotation) => sum.blend_rotation(rotation, weight),
                None => sum.blend_identity(weight),
            }
            if let Some(translation) = sample.move_track {
                sum.blend_translation(translation, weight);
            }
        }
    }

    /// One pose per part: the blend where an anim drove it, else `rest_of`.
    fn finish(self, rest_of: impl Fn(usize) -> PartPose) -> (LocalPose, Vec<bool>) {
        let Blend { sums, driven } = self;
        let poses = sums
            .iter()
            .zip(&driven)
            .enumerate()
            .map(|(part, (sum, &was_driven))| {
                if !was_driven {
                    return rest_of(part);
                }
                let (rotation, shift) = sum.finish();
                PartPose {
                    rotation,
                    translation: shift,
                }
            })
            .collect();
        (LocalPose::new(poses), driven)
    }
}

pub trait DObjAnim {
    fn tracks_for(&self, clip: &SkeletalClip) -> Vec<Option<usize>>;

    /// Blends the playing anims the way BO2's `DObjCalcAnim` does
    /// (facts/xanim-dobj-t6.md 1 and 3). Every anim adds its samples into an
    /// [`dobj_t6::AnimMat`] per part: rotations are flipped toward the running
    /// sum and a translation is divided by its own weight total. Part 159 is
    /// never written by anim data. A part that no anim drove takes its base
    /// pose (identity for root parts) and gets no translation. The second
    /// result says which parts were driven.
    fn calc_anim_masked(&self, active: &[ClipPlayback], want_set: &PartSet) -> (LocalPose, Vec<bool>);

    fn calc_anim(&self, playing: &[ClipPlayback], wanted: &PartSet) -> LocalPose;

    fn pose_using_aim(
        &self,
        playing: &[ClipPlayback],
        wanted: &PartSet,
        world_xf: Mat4,
        steer: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
    ) -> Vec<Mat4>;

    fn pose(&self, playing: &[ClipPlayback], wanted: &PartSet, to_world: Mat4) -> Vec<Mat4>;
}

impl DObjAnim for PosedSkeleton {
    fn tracks_for(&self, clip: &SkeletalClip) -> Vec<Option<usize>> {
        let mut by_name: HashMap<&str, usize> = HashMap::with_capacity(self.bones.len());
        for (index, bone) in self.bones.iter().enumerate().rev() {
            by_name.insert(bone.name.as_str(), index);
        }
        let find = |name: &str| by_name.get(name).copied();
        clip.tracks
            .iter()
            .map(|track| find(track.name.as_str()))
            .collect()
    }

    fn calc_anim_masked(&self, active_set: &[ClipPlayback], wish_set: &PartSet) -> (LocalPose, Vec<bool>) {
        let mut blend = Blend::new(self.bones.len());
        for instance in active_set.iter().filter(|a| a.weight > 0.0) {
            blend.accumulate(instance, wish_set);
        }
        blend.finish(|part| PartPose::at_rest(self.bones[part].rest_orientation.to_array()))
    }

    fn calc_anim(&self, playing: &[ClipPlayback], wanted: &PartSet) -> LocalPose {
        self.calc_anim_masked(playing, wanted).0
    }

    fn pose_using_aim(
        &self,
        playing: &[ClipPlayback],
        wanted: &PartSet,
        root_xf: Mat4,
        pose_hook: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
    ) -> Vec<Mat4> {
        let mut local = self.calc_anim(playing, wanted);
        pose_hook(self, wanted, &mut local);
        self.copy_mirrored_parts(&mut local);
        self.world_pose(&local, root_xf)
    }

    fn pose(&self, playing: &[ClipPlayback], wanted: &PartSet, into_world: Mat4) -> Vec<Mat4> {
        self.pose_using_aim(playing, wanted, into_world, |_, _, _| {})
    }
}
