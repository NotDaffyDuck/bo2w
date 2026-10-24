//! A DObj kept with the anim tree that plays on it, and the key that says when a cached
//! one can be reused.

use std::hash::{DefaultHasher, Hash, Hasher};

use dobj_t6::{Attach, DObj, DObjError, LocalPose, ModelPoseSrc, PartBits};
use glam::Mat4;

use crate::{
    CollisionBone, DObjPoseRequest, MaterializeError, RetainedModelCapability, XAnimTreeError,
    XAnimTreeRuntime, collision_dobj_with_controller,
};

/// What a cached DObj was built for: the entity type and its models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DObjReuseKey {
    pub e_type: i32,
    /// A [`model_token`] of the model names.
    pub model: i32,
}

/// A DObj cached for `cached` can stand in for one built for `current`.
pub fn reuse_matches(cached: DObjReuseKey, current: DObjReuseKey) -> bool {
    cached == current
}

/// A non-negative token for a list of model names; the same names in the same order give
/// the same token.
pub fn model_token(parts: &[&str]) -> i32 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    (hasher.finish() & 0x7fff_ffff) as i32
}

/// A DObj and its playing anim tree.
#[derive(Clone, Debug)]
pub struct DObjAnimRuntime {
    pub dobj: DObj,
    pub tree: XAnimTreeRuntime,
    pub reuse_key: Option<DObjReuseKey>,
}

impl DObjAnimRuntime {
    /// Builds the DObj from `models` to play `tree`.
    pub fn compose(
        models: &[(&ModelPoseSrc, Option<Attach>)],
        tree: XAnimTreeRuntime,
        reuse_key: Option<DObjReuseKey>,
    ) -> Result<Self, DObjError> {
        Ok(Self {
            dobj: DObj::build(models)?,
            tree,
            reuse_key,
        })
    }

    /// Runs the tree `dt` seconds on.
    pub fn update(&mut self, dt: f32) -> Result<(), XAnimTreeError> {
        self.tree.update(dt)
    }

    /// A request that poses the DObj with its tree.
    pub fn pose_request(&self) -> DObjPoseRequest {
        DObjPoseRequest {
            tree: Some(self.tree.clone()),
            ..DObjPoseRequest::bind_pose()
        }
    }

    /// The hit boxes of `models` (the ones the DObj was built from), posed by the tree.
    pub fn collision(
        &self,
        models: &[(&RetainedModelCapability, Option<Attach>)],
        world_from_model: Mat4,
        controller: impl FnOnce(&DObj, &PartBits, &mut LocalPose),
    ) -> Result<Vec<CollisionBone>, MaterializeError> {
        let request = self.pose_request();
        collision_dobj_with_controller(&self.dobj, models, &request, world_from_model, controller)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_needs_the_same_type_and_models() {
        let key = DObjReuseKey {
            e_type: 6,
            model: model_token(&["body", "head"]),
        };
        assert!(reuse_matches(key, key));
        assert!(!reuse_matches(key, DObjReuseKey { e_type: 7, ..key }));
        let swapped = DObjReuseKey {
            model: model_token(&["head", "body"]),
            ..key
        };
        assert!(!reuse_matches(key, swapped));
    }

    #[test]
    fn model_tokens_are_steady_and_never_negative() {
        let token = model_token(&["body", "head"]);
        assert_eq!(token, model_token(&["body", "head"]));
        assert!(token >= 0);
        assert_ne!(model_token(&["bodyhead"]), model_token(&["body", "head"]));
    }
}
