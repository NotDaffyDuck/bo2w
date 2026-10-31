//! An XAnim tree: blend, additive and leaf nodes. Each node eases its weight toward a goal,
//! and each leaf with weight moves along its clip, once per frame.

use std::sync::Arc;

use dobj_t6::PartSet;

use crate::{SkeletalClip, advance_leaf_time};

/// A node's place in its tree's node list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphSlot(pub u16);

#[derive(Clone, Debug)]
pub enum AnimGraphNodeType {
    /// Its children's poses, mixed by weight.
    Blend,
    /// Its children's poses laid on top of the pose under it.
    Additive,
    /// A clip, on the given parts (every part when `None`).
    Leaf {
        clip: Arc<SkeletalClip>,
        parts: Option<PartSet>,
    },
}

#[derive(Clone, Debug)]
pub struct AnimGraphNodeSpec {
    /// `None` for a root.
    pub owner: Option<GraphSlot>,
    pub kind: AnimGraphNodeType,
}

/// A tree's shape: its nodes, each listed after its parent.
#[derive(Clone, Debug)]
pub struct AnimGraphBlueprint {
    nodes: Vec<AnimGraphNodeSpec>,
}

/// Why a tree or a state was refused. Nodes are given by index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimGraphFault {
    Empty,
    NodeCountOver { count: usize },
    ParentMissing { node_ix: usize, parent_ix: usize },
    ParentAfterChild { node_ix: usize, parent_ix: usize },
    LeafWithChildren { node: usize },
    NanInNode { node: usize },
}

impl core::fmt::Display for AnimGraphFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&format!("{self:?}"))
    }
}

impl std::error::Error for AnimGraphFault {}

impl AnimGraphBlueprint {
    pub fn new(nodes: Vec<AnimGraphNodeSpec>) -> Result<Self, AnimGraphFault> {
        let count = nodes.len();
        match count {
            0 => return Err(AnimGraphFault::Empty),
            c if c > usize::from(u16::MAX) => return Err(AnimGraphFault::NodeCountOver { count }),
            _ => {}
        }
        match (0..count).find_map(|at| parent_flaw(&nodes, at)) {
            Some(flaw) => Err(flaw),
            None => Ok(Self { nodes }),
        }
    }

    pub fn nodes(&self) -> &[AnimGraphNodeSpec] {
        &self.nodes
    }
}

/// What is wrong with how `nodes[at]` is hung under its parent, if anything.
fn parent_flaw(nodes: &[AnimGraphNodeSpec], at: usize) -> Option<AnimGraphFault> {
    let parent = usize::from(nodes[at].owner?.0);
    if nodes.len() <= parent {
        Some(AnimGraphFault::ParentMissing { node_ix: at, parent_ix: parent })
    } else if parent >= at {
        Some(AnimGraphFault::ParentAfterChild { node_ix: at, parent_ix: parent })
    } else if matches!(nodes[parent].kind, AnimGraphNodeType::Leaf { .. }) {
        Some(AnimGraphFault::LeafWithChildren { node: parent })
    } else {
        None
    }
}

/// One node's playback state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeClock {
    /// How far a leaf is through its clip, 0..1.
    pub time: f32,
    /// `time` at the start of the last update.
    pub prev_t: f32,
    /// How many times a looping leaf has wrapped.
    pub laps: i16,
    pub prev_laps: i16,
    /// Seconds left to reach `goal_weight`.
    pub blend_left: f32,
    pub target_w: f32,
    pub weight: f32,
    /// Playback speed; 1 is the clip's own.
    pub rate: f32,
}

impl Default for NodeClock {
    /// At the start, weightless with no weight to ease toward, at the clip's own speed.
    fn default() -> Self {
        let (time, t_prev, laps, laps_prev) = (0.0, 0.0, 0, 0);
        let (w0, target_w, goal_t) = (0.0, 0.0, 0.0);
        Self { time, prev_t: t_prev, laps, prev_laps: laps_prev, blend_left: goal_t, target_w, weight: w0, rate: 1.0 }
    }
}

impl NodeClock {
    /// Standing still at `weight`, with nothing left to ease.
    fn resting_at(weight: f32) -> Self {
        Self {
            target_w: weight,
            weight,
            ..Self::default()
        }
    }

    fn is_finite(&self) -> bool {
        let fields = [
            self.rate,
            self.weight,
            self.target_w,
            self.blend_left,
            self.time,
            self.prev_t,
        ];
        fields.into_iter().all(f32::is_finite)
    }
}

/// A tree playing: its shape and a state per node.
#[derive(Clone, Debug)]
pub struct AnimGraphInstance {
    blueprint: Arc<AnimGraphBlueprint>,
    node_states: Vec<NodeClock>,
}

/// A leaf that poses the model this frame.
pub(crate) struct LiveClipLeaf<'a> {
    pub reel: &'a SkeletalClip,
    /// Seconds into the clip.
    pub time: f32,
    /// Its weight times every weight above it.
    pub weight: f32,
    pub pieces: Option<&'a PartSet>,
}

/// An additive node with weight: its own leaves and the additive nodes under it, laid on
/// the pose below.
pub(crate) struct LiveOverlayLayer<'a> {
    pub weight: f32,
    pub leaves: Vec<LiveClipLeaf<'a>>,
    pub inner: Vec<LiveOverlayLayer<'a>>,
}

/// Under this an eased weight is not kept as it is (bits `0x358637be`, just over 1e-6).
const SMALL_WEIGHT: f32 = f32::from_bits(0x3586_37be);

/// The weight a node has `dtime` seconds on and the time it still has to reach its goal. It
/// moves a straight share of the way each step; a node whose parent has no weight goes to
/// its goal at once.
///
/// TODO(fact): BO2's own step is open (QUESTIONS.md 66); this one matches MW2's.
fn goal_weight_step(state: &NodeClock, dtime: f32, parent_has_weight: bool) -> (f32, f32) {
    let arrives_now = !parent_has_weight || state.blend_left <= dtime;
    if arrives_now {
        return (state.target_w, 0.0);
    }
    let travelled = (state.target_w - state.weight) * dtime / state.blend_left;
    let eased = state.weight + travelled;
    let kept = if eased < SMALL_WEIGHT {
        state.target_w * 0.001
    } else {
        eased
    };
    (kept, state.blend_left - dtime)
}

impl AnimGraphInstance {
    /// Every node at rest, the roots at full weight.
    pub fn new(blueprint: Arc<AnimGraphBlueprint>) -> Self {
        let states = blueprint
            .nodes
            .iter()
            .map(|node| NodeClock::resting_at(if node.owner.is_some() { 0.0 } else { 1.0 }))
            .collect();
        Self { node_states: states, blueprint }
    }

    pub fn set_state(&mut self, id: GraphSlot, next: NodeClock) -> Result<(), AnimGraphFault> {
        let at = usize::from(id.0);
        if !next.is_finite() {
            return Err(AnimGraphFault::NanInNode { node: at });
        }
        let missing = AnimGraphFault::ParentMissing {
            node_ix: at,
            parent_ix: at,
        };
        self.node_states.get_mut(at).ok_or(missing)?.clone_from(&next);
        Ok(())
    }

    /// Moves the tree on `step_s` seconds: weights first, parents before children, then each
    /// leaf that still has weight along its clip.
    pub fn refresh(&mut self, step_s: f32) -> Result<(), AnimGraphFault> {
        if !step_s.is_finite() || step_s < 0.0 {
            return Err(AnimGraphFault::NanInNode { node: 0 });
        }
        for at in 0..self.node_states.len() {
            let parent_has_weight = self.blueprint.nodes[at]
                .owner
                .map_or(true, |id| self.node_states[usize::from(id.0)].weight != 0.0);
            let node_state = &mut self.node_states[at];
            let (w_now, t_goal) = goal_weight_step(node_state, step_s, parent_has_weight);
            *node_state = NodeClock {
                weight: w_now,
                blend_left: t_goal,
                prev_t: node_state.time,
                prev_laps: node_state.laps,
                ..*node_state
            };
        }
        let weighted = self
            .blueprint
            .nodes
            .iter()
            .zip(self.node_states.iter_mut())
            .filter(|(_, st)| st.weight != 0.0);
        for (entry, node_state) in weighted {
            if let AnimGraphNodeType::Leaf { clip, .. } = &entry.kind {
                let per_second = clip.plays_per_second();
                let wraps = clip.looping;
                let (since, cycles) = (node_state.prev_t, node_state.laps);
                if let Some(advanced) =
                    advance_leaf_time(since, cycles, node_state.rate, per_second, step_s, wraps)
                {
                    node_state.time = advanced.0;
                    node_state.laps = advanced.1;
                }
            }
        }
        Ok(())
    }

    /// The leaves that pose the model and the additive layers laid on them, each weighted
    /// by every node above it. Branches with no weight are left out.
    pub(crate) fn active_pose(&self) -> (Vec<LiveClipLeaf<'_>>, Vec<LiveOverlayLayer<'_>>) {
        let graph_nodes = &self.blueprint.nodes;
        let mut children_of: Vec<Vec<usize>> = vec![vec![]; graph_nodes.len()];
        for (at, node_def) in graph_nodes.iter().enumerate() {
            if let Some(parent) = node_def.owner {
                children_of[usize::from(parent.0)].push(at);
            }
        }
        let (mut clips_on, mut overlays) = (vec![], vec![]);
        let roots = graph_nodes
            .iter()
            .enumerate()
            .filter(|(_, nd)| nd.owner.is_none())
            .map(|(at, _)| at);
        for root in roots {
            self.gather(root, 1.0, &children_of, &mut clips_on, &mut overlays);
        }
        (clips_on, overlays)
    }

    /// Adds what `at` and its subtree pose to `out_leaves` and `out_layers`; `above` is the weight
    /// of everything over it.
    fn gather<'a>(
        &'a self,
        at: usize,
        above: f32,
        kids: &[Vec<usize>],
        out_leaves: &mut Vec<LiveClipLeaf<'a>>,
        out_layers: &mut Vec<LiveOverlayLayer<'a>>,
    ) {
        let state = &self.node_states[at];
        let weight = above * state.weight;
        let weightless = weight <= 0.0;
        if weightless {
            return;
        }
        let kind = &self.blueprint.nodes[at].kind;
        if let AnimGraphNodeType::Leaf { clip: cut, parts: part_mask } = kind {
            let parts = part_mask.as_ref();
            out_leaves.push(LiveClipLeaf {
                pieces: parts,
                weight,
                time: cut.duration() * state.time,
                reel: cut,
            });
            return;
        }
        if matches!(kind, AnimGraphNodeType::Blend) {
            for &kid in &kids[at] {
                self.gather(kid, weight, kids, out_leaves, out_layers);
            }
            return;
        }
        // An additive node: its children make a layer of their own.
        let mut layer = LiveOverlayLayer {
            inner: Vec::new(),
            leaves: Vec::new(),
            weight,
        };
        for &kid in &kids[at] {
            self.gather(kid, 1.0, kids, &mut layer.leaves, &mut layer.inner);
        }
        if layer.leaves.len() + layer.inner.len() > 0 {
            out_layers.push(layer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootShift;

    /// A looping clip two seconds long.
    fn reel(name: &str) -> Arc<SkeletalClip> {
        Arc::new(SkeletalClip {
            name: name.into(),
            fps: 10.0,
            frame_total: 20,
            looping: true,
            tracks: Vec::new(),
            notifies: Vec::new(),
            root_motion_shift: RootShift::Default,
        })
    }

    fn node(owner: Option<u16>, kind: AnimGraphNodeType) -> AnimGraphNodeSpec {
        AnimGraphNodeSpec {
            owner: owner.map(GraphSlot),
            kind,
        }
    }

    fn leaf(owner: u16, name: &str) -> AnimGraphNodeSpec {
        node(Some(owner), AnimGraphNodeType::Leaf { clip: reel(name), parts: None })
    }

    fn weighted(weight: f32) -> NodeClock {
        NodeClock {
            weight,
            target_w: weight,
            ..Default::default()
        }
    }

    #[test]
    fn a_tree_lists_parents_first() {
        let err = |nodes| AnimGraphBlueprint::new(nodes).unwrap_err();
        assert_eq!(err(Vec::new()), AnimGraphFault::Empty);
        assert_eq!(
            err(vec![node(None, AnimGraphNodeType::Blend), leaf(5, "a")]),
            AnimGraphFault::ParentMissing { node_ix: 1, parent_ix: 5 }
        );
        assert_eq!(
            err(vec![node(Some(1), AnimGraphNodeType::Blend), leaf(0, "a")]),
            AnimGraphFault::ParentAfterChild { node_ix: 0, parent_ix: 1 }
        );
        assert_eq!(
            err(vec![
                node(None, AnimGraphNodeType::Blend),
                leaf(0, "a"),
                leaf(1, "b")
            ]),
            AnimGraphFault::LeafWithChildren { node: 1 }
        );
        let tree = AnimGraphBlueprint::new(vec![node(None, AnimGraphNodeType::Blend), leaf(0, "a")]);
        assert_eq!(tree.unwrap().nodes().len(), 2);
    }

    fn runtime(nodes: Vec<AnimGraphNodeSpec>) -> AnimGraphInstance {
        AnimGraphInstance::new(Arc::new(AnimGraphBlueprint::new(nodes).unwrap()))
    }

    #[test]
    fn roots_start_at_full_weight() {
        let tree = runtime(vec![node(None, AnimGraphNodeType::Blend), leaf(0, "a")]);
        assert_eq!(tree.node_states[0], weighted(1.0));
        assert_eq!(tree.node_states[1], NodeClock::default());
    }

    #[test]
    fn states_must_be_finite_and_in_the_tree() {
        let mut tree = runtime(vec![node(None, AnimGraphNodeType::Blend)]);
        let bad = NodeClock {
            rate: f32::NAN,
            ..Default::default()
        };
        assert_eq!(
            tree.set_state(GraphSlot(0), bad),
            Err(AnimGraphFault::NanInNode { node: 0 })
        );
        assert_eq!(
            tree.set_state(GraphSlot(3), weighted(1.0)),
            Err(AnimGraphFault::ParentMissing { node_ix: 3, parent_ix: 3 })
        );
        assert_eq!(
            tree.refresh(-1.0),
            Err(AnimGraphFault::NanInNode { node: 0 })
        );
    }

    #[test]
    fn weights_ease_to_their_goal() {
        let mut tree = runtime(vec![node(None, AnimGraphNodeType::Blend), leaf(0, "a")]);
        let easing = NodeClock {
            target_w: 1.0,
            blend_left: 0.5,
            ..Default::default()
        };
        tree.set_state(GraphSlot(1), easing).unwrap();
        tree.refresh(0.25).unwrap();
        assert_eq!(
            (tree.node_states[1].weight, tree.node_states[1].blend_left),
            (0.5, 0.25)
        );
        tree.refresh(0.25).unwrap();
        assert_eq!(
            (tree.node_states[1].weight, tree.node_states[1].blend_left),
            (1.0, 0.0)
        );
        // A weight too small to keep is set to a thousandth of the goal.
        let faint = NodeClock {
            target_w: 0.001,
            blend_left: 1000.0,
            ..Default::default()
        };
        tree.set_state(GraphSlot(1), faint).unwrap();
        tree.refresh(0.001).unwrap();
        assert_eq!(tree.node_states[1].weight, 0.001 * 0.001);
    }

    #[test]
    fn under_a_weightless_parent_the_goal_is_reached_at_once() {
        let mut tree = runtime(vec![
            node(None, AnimGraphNodeType::Blend),
            node(Some(0), AnimGraphNodeType::Blend),
            leaf(1, "a"),
        ]);
        let easing = NodeClock {
            target_w: 1.0,
            blend_left: 10.0,
            ..Default::default()
        };
        tree.set_state(GraphSlot(2), easing).unwrap();
        tree.refresh(0.1).unwrap();
        assert_eq!(
            (tree.node_states[2].weight, tree.node_states[2].blend_left),
            (1.0, 0.0)
        );
    }

    #[test]
    fn leaves_with_weight_play() {
        let mut tree = runtime(vec![
            node(None, AnimGraphNodeType::Blend),
            leaf(0, "on"),
            leaf(0, "off"),
        ]);
        tree.set_state(GraphSlot(1), weighted(1.0)).unwrap();
        tree.refresh(0.5).unwrap();
        // A quarter of the two-second clip.
        assert_eq!((tree.node_states[1].prev_t, tree.node_states[1].time), (0.0, 0.25));
        assert_eq!(tree.node_states[2].time, 0.0);
        tree.refresh(1.75).unwrap();
        assert_eq!(
            (tree.node_states[1].time, tree.node_states[1].laps),
            (0.125, 1)
        );
        assert_eq!(tree.node_states[1].prev_laps, 0);
    }

    #[test]
    fn the_active_pose_multiplies_weights_down_the_tree() {
        let mut tree = runtime(vec![
            node(None, AnimGraphNodeType::Blend),
            node(Some(0), AnimGraphNodeType::Blend),
            leaf(1, "walk"),
            leaf(1, "run"),
            node(Some(0), AnimGraphNodeType::Additive),
            leaf(4, "lean"),
            leaf(0, "idle"),
        ]);
        let set = |tree: &mut AnimGraphInstance, node, weight| {
            tree.set_state(GraphSlot(node), weighted(weight)).unwrap();
        };
        set(&mut tree, 1, 0.5);
        set(&mut tree, 2, 0.25);
        set(&mut tree, 3, 0.75);
        set(&mut tree, 4, 0.5);
        set(&mut tree, 5, 1.0);
        tree.set_state(
            GraphSlot(3),
            NodeClock {
                time: 0.5,
                ..weighted(0.75)
            },
        )
        .unwrap();
        let (live_clips, layers) = tree.active_pose();
        let named: Vec<_> = live_clips
            .iter()
            .map(|l| (l.reel.name.as_str(), l.weight, l.time))
            .collect();
        // "idle" has no weight and is left out; "run" is halfway, one second in.
        assert_eq!(named, [("walk", 0.125, 0.0), ("run", 0.375, 1.0)]);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].weight, 0.5);
        assert_eq!(layers[0].leaves[0].reel.name, "lean");
        assert_eq!(layers[0].leaves[0].weight, 1.0);
        assert!(layers[0].inner.is_empty());
        // An additive node whose leaves have no weight lays nothing.
        set(&mut tree, 5, 0.0);
        assert!(tree.active_pose().1.is_empty());
    }
}
