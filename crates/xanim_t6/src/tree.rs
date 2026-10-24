//! An XAnim tree: blend, additive and leaf nodes. Each node eases its weight toward a goal,
//! and each leaf with weight moves along its clip, once per frame.

use std::sync::Arc;

use dobj_t6::PartBits;

use crate::{AnimClip, advance_leaf_time};

/// A node's place in its tree's node list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XAnimNodeId(pub u16);

#[derive(Clone, Debug)]
pub enum XAnimNodeKind {
    /// Its children's poses, mixed by weight.
    Blend,
    /// Its children's poses laid on top of the pose under it.
    Additive,
    /// A clip, on the given parts (every part when `None`).
    Leaf {
        clip: Arc<AnimClip>,
        parts: Option<PartBits>,
    },
}

#[derive(Clone, Debug)]
pub struct XAnimNodeDefinition {
    /// `None` for a root.
    pub parent: Option<XAnimNodeId>,
    pub kind: XAnimNodeKind,
}

/// A tree's shape: its nodes, each listed after its parent.
#[derive(Clone, Debug)]
pub struct XAnimTreeDefinition {
    nodes: Vec<XAnimNodeDefinition>,
}

/// Why a tree or a state was refused. Nodes are given by index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XAnimTreeError {
    Empty,
    TooManyNodes { count: usize },
    ParentOutOfRange { node: usize, parent: usize },
    ParentNotBeforeChild { node: usize, parent: usize },
    LeafHasChildren { node: usize },
    NonFiniteState { node: usize },
}

impl core::fmt::Display for XAnimTreeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for XAnimTreeError {}

impl XAnimTreeDefinition {
    pub fn new(nodes: Vec<XAnimNodeDefinition>) -> Result<Self, XAnimTreeError> {
        if nodes.is_empty() {
            return Err(XAnimTreeError::Empty);
        }
        if nodes.len() > usize::from(u16::MAX) {
            return Err(XAnimTreeError::TooManyNodes { count: nodes.len() });
        }
        for (node, definition) in nodes.iter().enumerate() {
            let Some(XAnimNodeId(parent)) = definition.parent else {
                continue;
            };
            let parent = usize::from(parent);
            if parent >= nodes.len() {
                return Err(XAnimTreeError::ParentOutOfRange { node, parent });
            }
            if parent >= node {
                return Err(XAnimTreeError::ParentNotBeforeChild { node, parent });
            }
            if matches!(nodes[parent].kind, XAnimNodeKind::Leaf { .. }) {
                return Err(XAnimTreeError::LeafHasChildren { node: parent });
            }
        }
        Ok(Self { nodes })
    }

    pub fn nodes(&self) -> &[XAnimNodeDefinition] {
        &self.nodes
    }
}

/// One node's playback state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XAnimNodeState {
    /// How far a leaf is through its clip, 0..1.
    pub time: f32,
    /// `time` at the start of the last update.
    pub old_time: f32,
    /// How many times a looping leaf has wrapped.
    pub cycle_count: i16,
    pub old_cycle_count: i16,
    /// Seconds left to reach `goal_weight`.
    pub goal_time: f32,
    pub goal_weight: f32,
    pub weight: f32,
    /// Playback speed; 1 is the clip's own.
    pub rate: f32,
}

impl Default for XAnimNodeState {
    fn default() -> Self {
        Self {
            time: 0.0,
            old_time: 0.0,
            cycle_count: 0,
            old_cycle_count: 0,
            goal_time: 0.0,
            goal_weight: 0.0,
            weight: 0.0,
            rate: 1.0,
        }
    }
}

impl XAnimNodeState {
    fn is_finite(&self) -> bool {
        [
            self.time,
            self.old_time,
            self.goal_time,
            self.goal_weight,
            self.weight,
            self.rate,
        ]
        .iter()
        .all(|v| v.is_finite())
    }
}

/// A tree playing: its shape and a state per node.
#[derive(Clone, Debug)]
pub struct XAnimTreeRuntime {
    definition: Arc<XAnimTreeDefinition>,
    states: Vec<XAnimNodeState>,
}

/// A leaf that poses the model this frame.
pub(crate) struct ActiveXAnimLeaf<'a> {
    pub clip: &'a AnimClip,
    /// Seconds into the clip.
    pub time: f32,
    /// Its weight times every weight above it.
    pub weight: f32,
    pub parts: Option<&'a PartBits>,
}

/// An additive node with weight: its own leaves and the additive nodes under it, laid on
/// the pose below.
pub(crate) struct ActiveAdditiveLayer<'a> {
    pub weight: f32,
    pub leaves: Vec<ActiveXAnimLeaf<'a>>,
    pub inner: Vec<ActiveAdditiveLayer<'a>>,
}

/// Under this an eased weight is not kept as it is (bits `0x358637be`, just over 1e-6).
const SMALL_WEIGHT: f32 = f32::from_bits(0x3586_37be);

/// The weight a node has `dtime` seconds on and the time it still has to reach its goal. It
/// moves a straight share of the way each step; a node whose parent has no weight goes to
/// its goal at once.
///
/// TODO(fact): BO2's own step is open (QUESTIONS.md 66); this one matches MW2's.
fn goal_weight_step(state: &XAnimNodeState, dtime: f32, parent_has_weight: bool) -> (f32, f32) {
    if !parent_has_weight || state.goal_time <= dtime {
        return (state.goal_weight, 0.0);
    }
    let mut weight = (state.goal_weight - state.weight) * dtime / state.goal_time + state.weight;
    if weight < SMALL_WEIGHT {
        weight = state.goal_weight * 0.001;
    }
    (weight, state.goal_time - dtime)
}

impl XAnimTreeRuntime {
    /// Every node at rest, the roots at full weight.
    pub fn new(definition: Arc<XAnimTreeDefinition>) -> Self {
        let states = definition
            .nodes
            .iter()
            .map(|node| {
                let weight = if node.parent.is_none() { 1.0 } else { 0.0 };
                XAnimNodeState {
                    weight,
                    goal_weight: weight,
                    ..Default::default()
                }
            })
            .collect();
        Self { definition, states }
    }

    pub fn set_state(
        &mut self,
        node: XAnimNodeId,
        state: XAnimNodeState,
    ) -> Result<(), XAnimTreeError> {
        let index = usize::from(node.0);
        if !state.is_finite() {
            return Err(XAnimTreeError::NonFiniteState { node: index });
        }
        let slot = self
            .states
            .get_mut(index)
            .ok_or(XAnimTreeError::ParentOutOfRange {
                node: index,
                parent: index,
            })?;
        *slot = state;
        Ok(())
    }

    /// Moves the tree on `dtime` seconds: weights first, parents before children, then each
    /// leaf that still has weight along its clip.
    pub fn update(&mut self, dtime: f32) -> Result<(), XAnimTreeError> {
        if !dtime.is_finite() || dtime < 0.0 {
            return Err(XAnimTreeError::NonFiniteState { node: 0 });
        }
        for node in 0..self.states.len() {
            let parent_has_weight = match self.definition.nodes[node].parent {
                None => true,
                Some(XAnimNodeId(parent)) => self.states[usize::from(parent)].weight != 0.0,
            };
            let state = &mut self.states[node];
            (state.weight, state.goal_time) = goal_weight_step(state, dtime, parent_has_weight);
            state.old_time = state.time;
            state.old_cycle_count = state.cycle_count;
        }
        for (node, state) in self.definition.nodes.iter().zip(&mut self.states) {
            let XAnimNodeKind::Leaf { clip, .. } = &node.kind else {
                continue;
            };
            if state.weight == 0.0 {
                continue;
            }
            if let Some((time, cycle_count)) = advance_leaf_time(
                state.old_time,
                state.cycle_count,
                state.rate,
                clip.frequency(),
                dtime,
                clip.looping,
            ) {
                state.time = time;
                state.cycle_count = cycle_count;
            }
        }
        Ok(())
    }

    /// The leaves that pose the model and the additive layers laid on them, each weighted
    /// by every node above it. Branches with no weight are left out.
    pub(crate) fn active_pose(&self) -> (Vec<ActiveXAnimLeaf<'_>>, Vec<ActiveAdditiveLayer<'_>>) {
        let nodes = &self.definition.nodes;
        let mut children = vec![Vec::new(); nodes.len()];
        for (node, definition) in nodes.iter().enumerate() {
            if let Some(XAnimNodeId(parent)) = definition.parent {
                children[usize::from(parent)].push(node);
            }
        }
        let mut leaves = Vec::new();
        let mut layers = Vec::new();
        for (root, definition) in nodes.iter().enumerate() {
            if definition.parent.is_none() {
                self.collect(root, 1.0, &children, &mut leaves, &mut layers);
            }
        }
        (leaves, layers)
    }

    fn collect<'a>(
        &'a self,
        node: usize,
        incoming: f32,
        children: &[Vec<usize>],
        leaves: &mut Vec<ActiveXAnimLeaf<'a>>,
        layers: &mut Vec<ActiveAdditiveLayer<'a>>,
    ) {
        let state = &self.states[node];
        let weight = incoming * state.weight;
        if weight <= 0.0 {
            return;
        }
        match &self.definition.nodes[node].kind {
            XAnimNodeKind::Leaf { clip, parts } => leaves.push(ActiveXAnimLeaf {
                clip,
                time: clip.duration() * state.time,
                weight,
                parts: parts.as_ref(),
            }),
            XAnimNodeKind::Blend => {
                for &child in &children[node] {
                    self.collect(child, weight, children, leaves, layers);
                }
            }
            XAnimNodeKind::Additive => {
                let mut layer = ActiveAdditiveLayer {
                    weight,
                    leaves: Vec::new(),
                    inner: Vec::new(),
                };
                for &child in &children[node] {
                    self.collect(child, 1.0, children, &mut layer.leaves, &mut layer.inner);
                }
                if !layer.leaves.is_empty() || !layer.inner.is_empty() {
                    layers.push(layer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Translation;

    /// A looping clip two seconds long.
    fn clip(name: &str) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: name.into(),
            framerate: 10.0,
            numframes: 20,
            looping: true,
            tracks: Vec::new(),
            notifies: Vec::new(),
            delta_translation: Translation::Default,
        })
    }

    fn node(parent: Option<u16>, kind: XAnimNodeKind) -> XAnimNodeDefinition {
        XAnimNodeDefinition {
            parent: parent.map(XAnimNodeId),
            kind,
        }
    }

    fn leaf(parent: u16, name: &str) -> XAnimNodeDefinition {
        node(
            Some(parent),
            XAnimNodeKind::Leaf {
                clip: clip(name),
                parts: None,
            },
        )
    }

    fn weighted(weight: f32) -> XAnimNodeState {
        XAnimNodeState {
            weight,
            goal_weight: weight,
            ..Default::default()
        }
    }

    #[test]
    fn a_tree_lists_parents_first() {
        let err = |nodes| XAnimTreeDefinition::new(nodes).unwrap_err();
        assert_eq!(err(Vec::new()), XAnimTreeError::Empty);
        assert_eq!(
            err(vec![node(None, XAnimNodeKind::Blend), leaf(5, "a")]),
            XAnimTreeError::ParentOutOfRange { node: 1, parent: 5 }
        );
        assert_eq!(
            err(vec![node(Some(1), XAnimNodeKind::Blend), leaf(0, "a")]),
            XAnimTreeError::ParentNotBeforeChild { node: 0, parent: 1 }
        );
        assert_eq!(
            err(vec![
                node(None, XAnimNodeKind::Blend),
                leaf(0, "a"),
                leaf(1, "b")
            ]),
            XAnimTreeError::LeafHasChildren { node: 1 }
        );
        let tree = XAnimTreeDefinition::new(vec![node(None, XAnimNodeKind::Blend), leaf(0, "a")]);
        assert_eq!(tree.unwrap().nodes().len(), 2);
    }

    fn runtime(nodes: Vec<XAnimNodeDefinition>) -> XAnimTreeRuntime {
        XAnimTreeRuntime::new(Arc::new(XAnimTreeDefinition::new(nodes).unwrap()))
    }

    #[test]
    fn roots_start_at_full_weight() {
        let tree = runtime(vec![node(None, XAnimNodeKind::Blend), leaf(0, "a")]);
        assert_eq!(tree.states[0], weighted(1.0));
        assert_eq!(tree.states[1], XAnimNodeState::default());
    }

    #[test]
    fn states_must_be_finite_and_in_the_tree() {
        let mut tree = runtime(vec![node(None, XAnimNodeKind::Blend)]);
        let bad = XAnimNodeState {
            rate: f32::NAN,
            ..Default::default()
        };
        assert_eq!(
            tree.set_state(XAnimNodeId(0), bad),
            Err(XAnimTreeError::NonFiniteState { node: 0 })
        );
        assert_eq!(
            tree.set_state(XAnimNodeId(3), weighted(1.0)),
            Err(XAnimTreeError::ParentOutOfRange { node: 3, parent: 3 })
        );
        assert_eq!(
            tree.update(-1.0),
            Err(XAnimTreeError::NonFiniteState { node: 0 })
        );
    }

    #[test]
    fn weights_ease_to_their_goal() {
        let mut tree = runtime(vec![node(None, XAnimNodeKind::Blend), leaf(0, "a")]);
        let easing = XAnimNodeState {
            goal_weight: 1.0,
            goal_time: 0.5,
            ..Default::default()
        };
        tree.set_state(XAnimNodeId(1), easing).unwrap();
        tree.update(0.25).unwrap();
        assert_eq!(
            (tree.states[1].weight, tree.states[1].goal_time),
            (0.5, 0.25)
        );
        tree.update(0.25).unwrap();
        assert_eq!(
            (tree.states[1].weight, tree.states[1].goal_time),
            (1.0, 0.0)
        );
        // A weight too small to keep is set to a thousandth of the goal.
        let faint = XAnimNodeState {
            goal_weight: 0.001,
            goal_time: 1000.0,
            ..Default::default()
        };
        tree.set_state(XAnimNodeId(1), faint).unwrap();
        tree.update(0.001).unwrap();
        assert_eq!(tree.states[1].weight, 0.001 * 0.001);
    }

    #[test]
    fn under_a_weightless_parent_the_goal_is_reached_at_once() {
        let mut tree = runtime(vec![
            node(None, XAnimNodeKind::Blend),
            node(Some(0), XAnimNodeKind::Blend),
            leaf(1, "a"),
        ]);
        let easing = XAnimNodeState {
            goal_weight: 1.0,
            goal_time: 10.0,
            ..Default::default()
        };
        tree.set_state(XAnimNodeId(2), easing).unwrap();
        tree.update(0.1).unwrap();
        assert_eq!(
            (tree.states[2].weight, tree.states[2].goal_time),
            (1.0, 0.0)
        );
    }

    #[test]
    fn leaves_with_weight_play() {
        let mut tree = runtime(vec![
            node(None, XAnimNodeKind::Blend),
            leaf(0, "on"),
            leaf(0, "off"),
        ]);
        tree.set_state(XAnimNodeId(1), weighted(1.0)).unwrap();
        tree.update(0.5).unwrap();
        // A quarter of the two-second clip.
        assert_eq!((tree.states[1].old_time, tree.states[1].time), (0.0, 0.25));
        assert_eq!(tree.states[2].time, 0.0);
        tree.update(1.75).unwrap();
        assert_eq!(
            (tree.states[1].time, tree.states[1].cycle_count),
            (0.125, 1)
        );
        assert_eq!(tree.states[1].old_cycle_count, 0);
    }

    #[test]
    fn the_active_pose_multiplies_weights_down_the_tree() {
        let mut tree = runtime(vec![
            node(None, XAnimNodeKind::Blend),
            node(Some(0), XAnimNodeKind::Blend),
            leaf(1, "walk"),
            leaf(1, "run"),
            node(Some(0), XAnimNodeKind::Additive),
            leaf(4, "lean"),
            leaf(0, "idle"),
        ]);
        let set = |tree: &mut XAnimTreeRuntime, node, weight| {
            tree.set_state(XAnimNodeId(node), weighted(weight)).unwrap();
        };
        set(&mut tree, 1, 0.5);
        set(&mut tree, 2, 0.25);
        set(&mut tree, 3, 0.75);
        set(&mut tree, 4, 0.5);
        set(&mut tree, 5, 1.0);
        tree.set_state(
            XAnimNodeId(3),
            XAnimNodeState {
                time: 0.5,
                ..weighted(0.75)
            },
        )
        .unwrap();
        let (leaves, layers) = tree.active_pose();
        let named: Vec<_> = leaves
            .iter()
            .map(|l| (l.clip.name.as_str(), l.weight, l.time))
            .collect();
        // "idle" has no weight and is left out; "run" is halfway, one second in.
        assert_eq!(named, [("walk", 0.125, 0.0), ("run", 0.375, 1.0)]);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].weight, 0.5);
        assert_eq!(layers[0].leaves[0].clip.name, "lean");
        assert_eq!(layers[0].leaves[0].weight, 1.0);
        assert!(layers[0].inner.is_empty());
        // An additive node whose leaves have no weight lays nothing.
        set(&mut tree, 5, 0.0);
        assert!(tree.active_pose().1.is_empty());
    }
}
