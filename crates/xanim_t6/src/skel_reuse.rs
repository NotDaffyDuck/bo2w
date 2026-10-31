//! A DObj kept with the anim tree that plays on it, and the key that says when a cached
//! one can be reused.

use std::hash::{DefaultHasher, Hash, Hasher};

use dobj_t6::{MountedModel, PosedSkeleton, SkeletonBuildFault, LocalPose, PoseDonorModel, PartSet};
use glam::Mat4;

use crate::{
    HitShapeBone, ModelPoseQuery, RebuildFault, KeptModelAbility, AnimGraphFault,
    AnimGraphInstance, hit_skeleton_with_aim,
};

/// What a cached DObj was built for: the entity type and its models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkeletonCacheKey {
    pub entity_kind: i32,
    /// A [`model_token`] of the model names.
    pub model: i32,
}

/// A DObj cached for `cached` can stand in for one built for `current`.
pub fn reuse_matches(cached: SkeletonCacheKey, current: SkeletonCacheKey) -> bool {
    let same_type = cached.entity_kind == current.entity_kind;
    same_type && cached.model == current.model
}

/// A non-negative token for a list of model names; the same names in the same order give
/// the same token.
pub fn model_token(pieces: &[&str]) -> i32 {
    let mut state = DefaultHasher::new();
    pieces.hash(&mut state);
    let low_31_bits = state.finish() & 0x7fff_ffff;
    i32::try_from(low_31_bits).unwrap_or_default()
}

/// A DObj and its playing anim tree.
#[derive(Clone, Debug)]
pub struct SkeletonAnimState {
    pub rig: PosedSkeleton,
    pub tree: AnimGraphInstance,
    pub reuse_key: Option<SkeletonCacheKey>,
}

impl SkeletonAnimState {
    /// Builds the DObj from `models` to play `tree`.
    pub fn pose_all(
        models: &[(&PoseDonorModel, Option<MountedModel>)],
        tree: AnimGraphInstance,
        reuse_key: Option<SkeletonCacheKey>,
    ) -> Result<Self, SkeletonBuildFault> {
        let dobj = PosedSkeleton::build(models)?;
        Ok(Self {
            reuse_key,
            tree,
            rig: dobj,
        })
    }

    /// Runs the tree `step_s` seconds on.
    pub fn refresh(&mut self, step_s: f32) -> Result<(), AnimGraphFault> {
        self.tree.refresh(step_s)
    }

    /// A request that poses the DObj with its tree.
    pub fn pose_request(&self) -> ModelPoseQuery {
        let mut request = ModelPoseQuery::neutral_pose();
        request.tree = Some(self.tree.clone());
        request
    }

    /// The hit boxes of `models` (the ones the DObj was built from), posed by the tree.
    pub fn collision(
        &self,
        models: &[(&KeptModelAbility, Option<MountedModel>)],
        placement_matrix: Mat4,
        drive: impl FnOnce(&PosedSkeleton, &PartSet, &mut LocalPose),
    ) -> Result<Vec<HitShapeBone>, RebuildFault> {
        let pose = self.pose_request();
        let at = placement_matrix;
        hit_skeleton_with_aim(&self.rig, models, &pose, at, drive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_needs_the_same_type_and_models() {
        let model = model_token(&["body", "head"]);
        let key = SkeletonCacheKey { model, entity_kind: 6 };
        assert!(reuse_matches(key, key));
        let other_type = SkeletonCacheKey { entity_kind: 7, model };
        let other_order = SkeletonCacheKey {
            model: model_token(&["head", "body"]),
            entity_kind: 6,
        };
        assert!(!reuse_matches(key, other_type));
        assert!(!reuse_matches(key, other_order));
    }

    #[test]
    fn model_tokens_are_steady_and_never_negative() {
        let names = ["body", "head"];
        let token = model_token(&names);
        assert_eq!(model_token(&names), token);
        assert!(token.signum() >= 0);
        assert_ne!(model_token(&["bodyhead"]), token);
    }
}
