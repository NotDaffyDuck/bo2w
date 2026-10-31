//! What a model's pose is made of, by name: its models and how they attach, and its anim
//! tree with clips named rather than loaded. Turning it into a pose request looks the
//! clips up.

use std::sync::Arc;

use dobj_t6::{HiddenParts, PartSet};

use crate::{
    SkeletalClip, ModelPoseQuery, AnimGraphNodeSpec, GraphSlot, AnimGraphNodeType, NodeClock,
    AnimGraphBlueprint, AnimGraphFault, AnimGraphInstance,
};

/// One model of a DObj, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigMeshEntry {
    pub model: String,
    /// The model it hangs from, by index; `None` for the first.
    pub host_index: Option<u16>,
    /// The parent's bone it hangs from.
    pub mount_tag: Option<String>,
    pub no_hits: bool,
    /// bo2zm: where the model's root sits on `attach_tag`; none = on it.
    pub tag_offset: Option<TagOffset>,
}

/// bo2zm: a model attached at an offset from its tag, the way Black Ops II
/// hangs a gun's magazine, scope or rocket on the gun (`DObjModel_s` `shift_offset`
/// and `turn_matrix`, from `attachWorldModelOffsets` / `...Rotations`). Kept as bits
/// so a descriptor stays `Eq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagOffset([u32; 6]);

impl TagOffset {
    /// `rotation` is pitch, yaw, roll in degrees.
    pub fn new(offset: [f32; 3], rotation: [f32; 3]) -> Self {
        let mut bits = [0; 6];
        for i in 0..3 {
            bits[i] = offset[i].to_bits();
            bits[i + 3] = rotation[i].to_bits();
        }
        Self(bits)
    }

    pub fn offset(self) -> [f32; 3] {
        std::array::from_fn(|i| f32::from_bits(self.0[i]))
    }

    pub fn rotation(self) -> [f32; 3] {
        std::array::from_fn(|i| f32::from_bits(self.0[i + 3]))
    }

    /// The root bind `DObj::build_with_root_binds` takes: yaw about z, then
    /// pitch about y, then roll about x (as the view model's parts).
    pub fn root_bind(self) -> (glam::Quat, glam::Vec3) {
        let [pitch, yaw, roll] = self.rotation().map(f32::to_radians);
        let q = glam::Quat::from_rotation_z(yaw)
            * glam::Quat::from_rotation_y(pitch)
            * glam::Quat::from_rotation_x(roll);
        (q, glam::Vec3::from_array(self.offset()))
    }
}

/// A DObj's models. `revision` changes whenever they do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigLayout {
    pub revision: u32,
    pub models: Vec<RigMeshEntry>,
}

impl RigLayout {
    /// A DObj made of `mesh_name` alone: no parent, no attach bone, and it collides.
    pub fn single(epoch: u32, mesh_name: String) -> Self {
        let root = RigMeshEntry { model: mesh_name, host_index: None, mount_tag: None, no_hits: false, tag_offset: None };
        Self { revision: epoch, models: vec![root] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimGraphRole {
    Blend,
    Additive,
    Leaf,
}

/// A tree node with its clip named.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodeRecord {
    pub parent: Option<GraphSlot>,
    pub kind: AnimGraphRole,
    /// A leaf's clip.
    pub clip: Option<String>,
    /// The parts a leaf plays on; every part when `None`.
    pub parts: Option<PartSet>,
    pub state: NodeClock,
}

/// An anim tree by name, with each node's state. The revisions change when the tree's
/// shape or its states do.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphCapture {
    pub shape_epoch: u32,
    pub clock_epoch: u32,
    pub nodes: Vec<GraphNodeRecord>,
}

/// A limp body (`startragdoll`): each client drops the pose it had then under
/// gravity, thrown by `launch` (`launchragdoll`, counted by `launches`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DObjRagdoll {
    /// Server time it went limp (tells one fall from the next).
    pub began_at_ms: i32,
    pub launch: [f32; 3],
    pub launches: u16,
}

/// Everything that poses one DObj, by name.
#[derive(Clone, Debug, PartialEq)]
pub struct RigPoseRecord {
    pub composition: RigLayout,
    pub pose_epoch: u32,
    /// `None` holds the bind pose.
    pub tree: Option<GraphCapture>,
    /// The parts posed; every part when `None`.
    pub wanted_mask: Option<PartSet>,
    pub hidden_mask: HiddenParts,
    /// bo2mp: a vehicle's front wheels' steer (degrees, BO2's networked steer yaw), when it
    /// has wheels to turn.
    pub vehicle_steer: Option<f32>,
    /// bo2mp: a turret's gun (BO2's networked gun angles): its pitch (down as plus) and the
    /// facing it was set down with, which its legs keep while the turret turns to its gun's
    /// yaw.
    pub turret_aim: Option<[f32; 2]>,
    pub ragdoll: Option<DObjRagdoll>,
    /// bo2zm round 1: a zombie's eye brightness (`scriptVector2.y` from its
    /// client script: 2 alive, Origins 3, 0.25 once the eyes go out).
    pub eye_luminance: Option<f32>,
}

/// Why a snapshot could not be played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleLookupFault {
    /// Leaf `node` has no clip name.
    LeafWithoutClip { tree_node: usize },
    /// Leaf `node` names `clip`, and the lookup did not find it.
    ClipNotLoaded { node_ix: usize, cut: String },
    Tree(AnimGraphFault),
}

impl From<AnimGraphFault> for RoleLookupFault {
    fn from(error: AnimGraphFault) -> Self {
        Self::Tree(error)
    }
}

/// The first definition revision of a tree built in one piece.
const FIRST_DEFINITION: u32 = 1;

impl GraphNodeRecord {
    /// A root leaf playing `clip` in `state`, on every part.
    fn root_leaf(reel: String, state: NodeClock) -> Self {
        Self { parent: None, kind: AnimGraphRole::Leaf, clip: Some(reel), parts: None, state }
    }
}

impl RigPoseRecord {
    /// One model held in its bind pose.
    pub fn neutral_pose(body: String, composition_rev: u32, pose_rev: u32) -> Self {
        // No tree, nothing hidden, every part posed, and no wheels or gun to aim.
        Self {
            composition: RigLayout::single(composition_rev, body),
            pose_epoch: pose_rev,
            tree: None,
            wanted_mask: None,
            hidden_mask: HiddenParts::default(),
            vehicle_steer: None,
            turret_aim: None,
            ragdoll: None,
            eye_luminance: None,
        }
    }

    /// One model playing one clip at full weight, `time` (0..1) through it.
    pub fn single_clip(
        body: String,
        collision_label: String,
        composition_rev: u32,
        pose_rev: u32,
        at: f32,
    ) -> Self {
        let full = 1.0;
        let playing = NodeClock {
            target_w: full,
            weight: full,
            prev_t: at,
            time: at,
            ..Default::default()
        };
        let snapshot = GraphCapture {
            nodes: vec![GraphNodeRecord::root_leaf(collision_label, playing)],
            clock_epoch: pose_rev,
            shape_epoch: FIRST_DEFINITION,
        };
        Self {
            tree: Some(snapshot),
            ..Self::neutral_pose(body, composition_rev, pose_rev)
        }
    }

    /// The pose request, with each leaf's clip looked up by `clip`.
    pub fn look_up_pose_query(
        &self,
        lookup: impl FnMut(&str) -> Option<Arc<SkeletalClip>>,
    ) -> Result<ModelPoseQuery, RoleLookupFault> {
        let tree = match &self.tree {
            Some(snapshot) => Some(snapshot.settle(lookup)?),
            None => None,
        };
        Ok(ModelPoseQuery { tree, wanted_mask: self.wanted_mask, hidden_mask: self.hidden_mask })
    }
}

impl GraphCapture {
    /// The tree ready to play, with each leaf's clip looked up by `clip` and every node
    /// set to its state.
    pub fn settle(
        &self,
        mut find: impl FnMut(&str) -> Option<Arc<SkeletalClip>>,
    ) -> Result<AnimGraphInstance, RoleLookupFault> {
        let definitions = self
            .nodes
            .iter()
            .enumerate()
            .map(|(at, semantic)| {
                let kind = node_kind(semantic, at, &mut find)?;
                let parent = semantic.parent;
                Ok(AnimGraphNodeSpec { kind, owner: parent })
            })
            .collect::<Result<Vec<_>, RoleLookupFault>>()?;
        let shape = AnimGraphBlueprint::new(definitions)?;
        let mut live = AnimGraphInstance::new(Arc::new(shape));
        for (id, sem_node) in (0..=u16::MAX).map(GraphSlot).zip(&self.nodes) {
            live.set_state(id, sem_node.state)?;
        }
        Ok(live)
    }
}

/// The playable kind of `semantic`, the node at index `at`; a leaf's clip is looked up
/// with `find`.
fn node_kind(
    semantic: &GraphNodeRecord,
    at: usize,
    find: &mut impl FnMut(&str) -> Option<Arc<SkeletalClip>>,
) -> Result<AnimGraphNodeType, RoleLookupFault> {
    match semantic.kind {
        AnimGraphRole::Additive => Ok(AnimGraphNodeType::Additive),
        AnimGraphRole::Blend => Ok(AnimGraphNodeType::Blend),
        AnimGraphRole::Leaf => {
            let part_mask = semantic.parts;
            let name = semantic.clip.as_deref().ok_or(RoleLookupFault::LeafWithoutClip { tree_node: at })?;
            let trim = find(name).ok_or_else(|| RoleLookupFault::ClipNotLoaded {
                cut: String::from(name),
                node_ix: at,
            })?;
            Ok(AnimGraphNodeType::Leaf { clip: trim, parts: part_mask })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootShift;

    fn reel(name: &str) -> Option<Arc<SkeletalClip>> {
        (name == "walk").then(|| {
            Arc::new(SkeletalClip {
                name: name.into(),
                fps: 30.0,
                frame_total: 30,
                looping: true,
                tracks: Vec::new(),
                notifies: Vec::new(),
                root_motion_shift: RootShift::Default,
            })
        })
    }

    #[test]
    fn a_bind_pose_has_no_tree() {
        let state = RigPoseRecord::neutral_pose("tank".into(), 3, 4);
        assert_eq!(
            state.composition,
            RigLayout::single(3, "tank".into())
        );
        assert_eq!(state.pose_epoch, 4);
        assert_eq!(state.tree, None);
        let request = state.look_up_pose_query(reel).unwrap();
        assert!(request.tree.is_none());
        assert_eq!(request.wanted_mask, None);
    }

    #[test]
    fn one_leaf_plays_its_clip_at_full_weight() {
        let state = RigPoseRecord::single_clip("tank".into(), "walk".into(), 3, 4, 0.5);
        let tree = state.tree.as_ref().unwrap();
        assert_eq!((tree.shape_epoch, tree.clock_epoch), (1, 4));
        let leaf = &tree.nodes[0];
        assert_eq!((leaf.state.time, leaf.state.prev_t), (0.5, 0.5));
        assert_eq!(
            (leaf.state.weight, leaf.state.target_w, leaf.state.rate),
            (1.0, 1.0, 1.0)
        );

        let request = state.look_up_pose_query(reel).unwrap();
        let (clips_on, layers) = request.tree.as_ref().unwrap().active_pose();
        assert_eq!(clips_on.len(), 1);
        assert_eq!(
            (clips_on[0].reel.name.as_str(), clips_on[0].time),
            ("walk", 0.5)
        );
        assert!(layers.is_empty());
    }

    #[test]
    fn leaves_need_a_clip_that_can_be_found() {
        let mut state = RigPoseRecord::single_clip("tank".into(), "fly".into(), 0, 0, 0.0);
        assert_eq!(
            state.look_up_pose_query(reel).unwrap_err(),
            RoleLookupFault::ClipNotLoaded {
                node_ix: 0,
                cut: "fly".into()
            }
        );
        state.tree.as_mut().unwrap().nodes[0].clip = None;
        assert_eq!(
            state.look_up_pose_query(reel).unwrap_err(),
            RoleLookupFault::LeafWithoutClip { tree_node: 0 }
        );
    }

    #[test]
    fn tree_errors_come_through() {
        let mut state = RigPoseRecord::single_clip("tank".into(), "walk".into(), 0, 0, 0.0);
        let tree = state.tree.as_mut().unwrap();
        tree.nodes[0].state.rate = f32::INFINITY;
        assert_eq!(
            state.look_up_pose_query(reel).unwrap_err(),
            RoleLookupFault::Tree(AnimGraphFault::NanInNode { node: 0 })
        );
        state.tree.as_mut().unwrap().nodes.clear();
        assert_eq!(
            state.look_up_pose_query(reel).unwrap_err(),
            RoleLookupFault::Tree(AnimGraphFault::Empty)
        );
    }
}
