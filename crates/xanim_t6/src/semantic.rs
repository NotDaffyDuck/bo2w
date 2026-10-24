//! What a model's pose is made of, by name: its models and how they attach, and its anim
//! tree with clips named rather than loaded. Turning it into a pose request looks the
//! clips up.

use std::sync::Arc;

use dobj_t6::{HidePartBits, PartBits};

use crate::{
    AnimClip, DObjPoseRequest, XAnimNodeDefinition, XAnimNodeId, XAnimNodeKind, XAnimNodeState,
    XAnimTreeDefinition, XAnimTreeError, XAnimTreeRuntime,
};

/// One model of a DObj, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DObjModelDescriptor {
    pub model: String,
    /// The model it hangs from, by index; `None` for the first.
    pub parent_model: Option<u16>,
    /// The parent's bone it hangs from.
    pub attach_tag: Option<String>,
    pub ignore_collision: bool,
}

/// A DObj's models. `revision` changes whenever they do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DObjCompositionDescriptor {
    pub revision: u32,
    pub models: Vec<DObjModelDescriptor>,
}

impl DObjCompositionDescriptor {
    /// A DObj of one model.
    pub fn single(revision: u32, model: String) -> Self {
        Self {
            revision,
            models: vec![DObjModelDescriptor {
                model,
                parent_model: None,
                attach_tag: None,
                ignore_collision: false,
            }],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XAnimSemanticNodeKind {
    Blend,
    Additive,
    Leaf,
}

/// A tree node with its clip named.
#[derive(Clone, Debug, PartialEq)]
pub struct XAnimSemanticNode {
    pub parent: Option<XAnimNodeId>,
    pub kind: XAnimSemanticNodeKind,
    /// A leaf's clip.
    pub clip: Option<String>,
    /// The parts a leaf plays on; every part when `None`.
    pub parts: Option<PartBits>,
    pub state: XAnimNodeState,
}

/// An anim tree by name, with each node's state. The revisions change when the tree's
/// shape or its states do.
#[derive(Clone, Debug, PartialEq)]
pub struct XAnimTreeSnapshot {
    pub definition_revision: u32,
    pub state_revision: u32,
    pub nodes: Vec<XAnimSemanticNode>,
}

/// Everything that poses one DObj, by name.
#[derive(Clone, Debug, PartialEq)]
pub struct DObjSemanticState {
    pub composition: DObjCompositionDescriptor,
    pub pose_revision: u32,
    /// `None` holds the bind pose.
    pub tree: Option<XAnimTreeSnapshot>,
    /// The parts posed; every part when `None`.
    pub requested_parts: Option<PartBits>,
    pub hide_part_bits: HidePartBits,
    /// bo2mp: a vehicle's front wheels' steer (degrees, BO2's networked steer yaw), when it
    /// has wheels to turn.
    pub vehicle_steer: Option<f32>,
    /// bo2mp: a turret's gun (BO2's networked gun angles): its pitch (down as plus) and the
    /// facing it was set down with, which its legs keep while the turret turns to its gun's
    /// yaw.
    pub turret_aim: Option<[f32; 2]>,
}

/// Why a snapshot could not be played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticResolveError {
    /// A leaf names no clip.
    LeafClipMissing {
        node: usize,
    },
    /// The clip a leaf names could not be found.
    ClipUnavailable {
        node: usize,
        clip: String,
    },
    Tree(XAnimTreeError),
}

impl From<XAnimTreeError> for SemanticResolveError {
    fn from(error: XAnimTreeError) -> Self {
        Self::Tree(error)
    }
}

impl DObjSemanticState {
    /// One model held in its bind pose.
    pub fn bind_pose(model: String, composition_revision: u32, pose_revision: u32) -> Self {
        Self {
            composition: DObjCompositionDescriptor::single(composition_revision, model),
            pose_revision,
            tree: None,
            requested_parts: None,
            hide_part_bits: HidePartBits::default(),
            vehicle_steer: None,
            turret_aim: None,
        }
    }

    /// One model playing one clip at full weight, `time` (0..1) through it.
    pub fn one_leaf(
        model: String,
        clip: String,
        composition_revision: u32,
        pose_revision: u32,
        time: f32,
    ) -> Self {
        let leaf = XAnimSemanticNode {
            parent: None,
            kind: XAnimSemanticNodeKind::Leaf,
            clip: Some(clip),
            parts: None,
            state: XAnimNodeState {
                time,
                old_time: time,
                weight: 1.0,
                goal_weight: 1.0,
                rate: 1.0,
                ..Default::default()
            },
        };
        Self {
            tree: Some(XAnimTreeSnapshot {
                definition_revision: 1,
                state_revision: pose_revision,
                nodes: vec![leaf],
            }),
            ..Self::bind_pose(model, composition_revision, pose_revision)
        }
    }

    /// The pose request, with each leaf's clip looked up by `clip`.
    pub fn resolve_request(
        &self,
        clip: impl FnMut(&str) -> Option<Arc<AnimClip>>,
    ) -> Result<DObjPoseRequest, SemanticResolveError> {
        let tree = self
            .tree
            .as_ref()
            .map(|tree| tree.resolve(clip))
            .transpose()?;
        Ok(DObjPoseRequest {
            tree,
            requested_parts: self.requested_parts,
            hide_part_bits: self.hide_part_bits,
        })
    }
}

impl XAnimTreeSnapshot {
    /// The tree ready to play, with each leaf's clip looked up by `clip` and every node
    /// set to its state.
    pub fn resolve(
        &self,
        mut clip: impl FnMut(&str) -> Option<Arc<AnimClip>>,
    ) -> Result<XAnimTreeRuntime, SemanticResolveError> {
        let mut definitions = Vec::with_capacity(self.nodes.len());
        for (node, semantic) in self.nodes.iter().enumerate() {
            let kind = match semantic.kind {
                XAnimSemanticNodeKind::Blend => XAnimNodeKind::Blend,
                XAnimSemanticNodeKind::Additive => XAnimNodeKind::Additive,
                XAnimSemanticNodeKind::Leaf => {
                    let name = semantic
                        .clip
                        .as_deref()
                        .ok_or(SemanticResolveError::LeafClipMissing { node })?;
                    let found =
                        clip(name).ok_or_else(|| SemanticResolveError::ClipUnavailable {
                            node,
                            clip: name.to_owned(),
                        })?;
                    XAnimNodeKind::Leaf {
                        clip: found,
                        parts: semantic.parts,
                    }
                }
            };
            definitions.push(XAnimNodeDefinition {
                parent: semantic.parent,
                kind,
            });
        }
        let definition = XAnimTreeDefinition::new(definitions)?;
        let mut runtime = XAnimTreeRuntime::new(Arc::new(definition));
        for (node, semantic) in (0..=u16::MAX).zip(&self.nodes) {
            runtime.set_state(XAnimNodeId(node), semantic.state)?;
        }
        Ok(runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Translation;

    fn clip(name: &str) -> Option<Arc<AnimClip>> {
        (name == "walk").then(|| {
            Arc::new(AnimClip {
                name: name.into(),
                framerate: 30.0,
                numframes: 30,
                looping: true,
                tracks: Vec::new(),
                notifies: Vec::new(),
                delta_translation: Translation::Default,
            })
        })
    }

    #[test]
    fn a_bind_pose_has_no_tree() {
        let state = DObjSemanticState::bind_pose("tank".into(), 3, 4);
        assert_eq!(
            state.composition,
            DObjCompositionDescriptor::single(3, "tank".into())
        );
        assert_eq!(state.pose_revision, 4);
        assert_eq!(state.tree, None);
        let request = state.resolve_request(clip).unwrap();
        assert!(request.tree.is_none());
        assert_eq!(request.requested_parts, None);
    }

    #[test]
    fn one_leaf_plays_its_clip_at_full_weight() {
        let state = DObjSemanticState::one_leaf("tank".into(), "walk".into(), 3, 4, 0.5);
        let tree = state.tree.as_ref().unwrap();
        assert_eq!((tree.definition_revision, tree.state_revision), (1, 4));
        let leaf = &tree.nodes[0];
        assert_eq!((leaf.state.time, leaf.state.old_time), (0.5, 0.5));
        assert_eq!(
            (leaf.state.weight, leaf.state.goal_weight, leaf.state.rate),
            (1.0, 1.0, 1.0)
        );

        let request = state.resolve_request(clip).unwrap();
        let (leaves, layers) = request.tree.as_ref().unwrap().active_pose();
        assert_eq!(leaves.len(), 1);
        assert_eq!(
            (leaves[0].clip.name.as_str(), leaves[0].time),
            ("walk", 0.5)
        );
        assert!(layers.is_empty());
    }

    #[test]
    fn leaves_need_a_clip_that_can_be_found() {
        let mut state = DObjSemanticState::one_leaf("tank".into(), "fly".into(), 0, 0, 0.0);
        assert_eq!(
            state.resolve_request(clip).unwrap_err(),
            SemanticResolveError::ClipUnavailable {
                node: 0,
                clip: "fly".into()
            }
        );
        state.tree.as_mut().unwrap().nodes[0].clip = None;
        assert_eq!(
            state.resolve_request(clip).unwrap_err(),
            SemanticResolveError::LeafClipMissing { node: 0 }
        );
    }

    #[test]
    fn tree_errors_come_through() {
        let mut state = DObjSemanticState::one_leaf("tank".into(), "walk".into(), 0, 0, 0.0);
        let tree = state.tree.as_mut().unwrap();
        tree.nodes[0].state.rate = f32::INFINITY;
        assert_eq!(
            state.resolve_request(clip).unwrap_err(),
            SemanticResolveError::Tree(XAnimTreeError::NonFiniteState { node: 0 })
        );
        state.tree.as_mut().unwrap().nodes.clear();
        assert_eq!(
            state.resolve_request(clip).unwrap_err(),
            SemanticResolveError::Tree(XAnimTreeError::Empty)
        );
    }
}
