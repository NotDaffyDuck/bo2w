//! bo2mp: an owned clip map. A map loader reads `clipMap_t` once into these
//! tables and keeps them for as long as the game runs; `ClipMap` borrows
//! them. Brush sides and leaf brush lists sit in one table each, and the
//! brushes and leaf brush nodes name their run of it.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;

use trace_t6::{Brush, BrushSide};

use crate::{AabbTree, CModel, ClipMap, ClipMaterial, Leaf, LeafBrushNode, Node, Partition, TempModels};

/// `cbrush_t`, with its sides as `side_count` entries of
/// `ClipRepository::brush_sides` from `first_side`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StoredBrush {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub contents: u32,
    pub first_side: u32,
    pub side_count: u32,
    pub axial_cflags: [[u32; 3]; 2],
    pub axial_sflags: [[u32; 3]; 2],
}

/// `cLeafBrushNode_s`, with its brushes (a positive count) as that many
/// entries of `ClipRepository::leaf_brushes` from `initial_convex_solid`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StoredLeafBrushNode {
    pub axis: u8,
    pub leaf_solid_total: i16,
    pub contents: u32,
    pub initial_convex_solid: u32,
    pub dist: f32,
    pub range: f32,
    pub child_offset: [u16; 2],
}

/// The tables of one map's `clipMap_t` that the traces read.
#[derive(Clone, Debug, Default)]
pub struct ClipRepository {
    pub nodes: Vec<Node>,
    pub leafs: Vec<Leaf>,
    pub leaf_brush_nodes: Vec<StoredLeafBrushNode>,
    pub leaf_brushes: Vec<u16>,
    pub brushes: Vec<StoredBrush>,
    pub brush_sides: Vec<BrushSide>,
    pub materials: Vec<ClipMaterial>,
    pub aabb_trees: Vec<AabbTree>,
    pub partitions: Vec<Partition>,
    pub vertex_points: Vec<[f32; 3]>,
    pub tri_indices: Vec<[u16; 3]>,
    pub tri_edge_is_walkable: Vec<u8>,
    pub cmodels: Vec<CModel>,
    /// The box hull's leaf brush node (`clipMap_t::box_model`'s). Its brush
    /// index is `brushes.len()`: `clipMap_t::box_brush`, which sits after
    /// the map's brushes and which `TempModels` stands in for. No leaf or
    /// model of the map reaches this node.
    pub box_brush_node: Option<u32>,
}

/// A table entry that points outside the table it names: `table` is the
/// table holding the bad entry, `index` the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreError {
    pub table: &'static str,
    pub index: usize,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "clip map {} entry {} points outside its table", self.table, self.index)
    }
}

impl ClipRepository {
    /// Every index the traces follow lands inside its table, so no trace
    /// over this map reads past one.
    pub fn check(&self) -> Result<(), StoreError> {
        let bad = |table, index| Err(StoreError { table, index });
        if self.nodes.is_empty() {
            return bad("nodes", 0);
        }
        for (i, n) in self.nodes.iter().enumerate() {
            for c in n.children {
                let ok = if c < 0 {
                    ((-1 - i32::from(c)) as usize) < self.leafs.len()
                } else {
                    (c as usize) < self.nodes.len()
                };
                if !ok {
                    return bad("nodes", i);
                }
            }
        }
        for (i, leaf) in self.leafs.iter().enumerate() {
            if !self.leaf_ok(leaf) {
                return bad("leafs", i);
            }
        }
        for (i, m) in self.cmodels.iter().enumerate() {
            if !self.leaf_ok(&m.leaf) {
                return bad("cmodels", i);
            }
        }
        let lbn = self.leaf_brush_nodes.len();
        for (i, n) in self.leaf_brush_nodes.iter().enumerate() {
            // A node with brushes is a leaf: no walk reads its axis.
            let ok = if n.leaf_solid_total > 0 {
                n.initial_convex_solid as usize + n.leaf_solid_total as usize <= self.leaf_brushes.len()
            } else {
                n.axis < 3
                    && n.child_offset.iter().all(|&o| i + usize::from(o) < lbn)
                    && (n.leaf_solid_total == 0 || i + 1 < lbn)
            };
            if !ok {
                return bad("leaf_brush_nodes", i);
            }
        }
        let box_run = self
            .box_brush_node
            .and_then(|i| self.leaf_brush_nodes.get(i as usize))
            .filter(|n| n.leaf_solid_total > 0)
            .map(|n| n.initial_convex_solid as usize..n.initial_convex_solid as usize + n.leaf_solid_total as usize);
        let in_box_run = |i: usize| box_run.as_ref().is_some_and(|r| r.contains(&i));
        if let Some(i) = self.leaf_brushes.iter().enumerate().position(|(i, &b)| {
            let b = usize::from(b);
            b > self.brushes.len() || (b == self.brushes.len() && !in_box_run(i))
        }) {
            return bad("leaf_brushes", i);
        }
        for (i, b) in self.brushes.iter().enumerate() {
            if b.first_side as usize + b.side_count as usize > self.brush_sides.len() {
                return bad("brushes", i);
            }
        }
        for (i, t) in self.aabb_trees.iter().enumerate() {
            let ok = usize::from(t.material_index) < self.materials.len()
                && if t.sub_element_total > 0 {
                    t.u as usize + usize::from(t.sub_element_total) <= self.aabb_trees.len()
                } else {
                    (t.u as usize) < self.partitions.len()
                };
            if !ok {
                return bad("aabb_trees", i);
            }
        }
        for (i, p) in self.partitions.iter().enumerate() {
            if p.leading_triangle < 0 || p.leading_triangle as usize + usize::from(p.tri_tally) > self.tri_indices.len() {
                return bad("partitions", i);
            }
        }
        if let Some(i) = self
            .tri_indices
            .iter()
            .position(|t| t.iter().any(|&v| usize::from(v) >= self.vertex_points.len()))
        {
            return bad("tri_indices", i);
        }
        if self.tri_edge_is_walkable.len() * 8 < self.tri_indices.len() * 3 {
            return bad("tri_edge_is_walkable", self.tri_edge_is_walkable.len());
        }
        Ok(())
    }

    /// A leaf with brush contents reads its leaf brush node even when that is
    /// node 0; point contents takes node 0 as no brushes.
    fn leaf_ok(&self, leaf: &Leaf) -> bool {
        usize::from(leaf.first_coll_aabb) + usize::from(leaf.collision_box_total) <= self.aabb_trees.len()
            && leaf.brush_leaf_link >= 0
            && ((leaf.brush_leaf_link as usize) < self.leaf_brush_nodes.len()
                || (leaf.brush_leaf_link == 0 && leaf.solid_content_flags == 0))
    }

    fn brush_views(&self) -> Vec<Brush<'_>> {
        self.brushes
            .iter()
            .map(|b| {
                let first = b.first_side as usize;
                Brush {
                    lower: b.mins,
                    upper: b.maxs,
                    contents: b.contents,
                    sides: self.brush_sides.get(first..first + b.side_count as usize).unwrap_or(&[]),
                    axial_cflags: b.axial_cflags,
                    axial_sflags: b.axial_sflags,
                }
            })
            .collect()
    }

    fn leaf_brush_node_views(&self) -> Vec<LeafBrushNode<'_>> {
        self.leaf_brush_nodes
            .iter()
            .map(|n| {
                let first = n.initial_convex_solid as usize;
                let count = usize::try_from(n.leaf_solid_total).unwrap_or(0);
                LeafBrushNode {
                    axis: n.axis,
                    leaf_solid_total: n.leaf_solid_total,
                    contents: n.contents,
                    brushes: self.leaf_brushes.get(first..first + count).unwrap_or(&[]),
                    dist: n.dist,
                    range: n.range,
                    child_offset: n.child_offset,
                }
            })
            .collect()
    }

    /// Run `f` over a `ClipMap` borrowing these tables.
    pub fn with_map<R>(&self, f: impl FnOnce(&ClipMap<'_>) -> R) -> R {
        let brushes = self.brush_views();
        let leaf_brush_nodes = self.leaf_brush_node_views();
        f(&self.map(&brushes, &leaf_brush_nodes))
    }

    /// Keep these tables for the rest of the run and hand back the
    /// `ClipMap` over them. Each call keeps one more copy, so a loader
    /// calls it once a map.
    #[must_use]
    pub fn leak(self) -> &'static ClipMap<'static> {
        let store: &'static Self = Box::leak(Box::new(self));
        let brushes = Box::leak(store.brush_views().into_boxed_slice());
        let leaf_brush_nodes = Box::leak(store.leaf_brush_node_views().into_boxed_slice());
        Box::leak(Box::new(store.map(brushes, leaf_brush_nodes)))
    }

    fn map<'a>(&'a self, brushes: &'a [Brush<'a>], leaf_brush_nodes: &'a [LeafBrushNode<'a>]) -> ClipMap<'a> {
        ClipMap {
            nodes: &self.nodes,
            leafs: &self.leafs,
            leaf_brush_nodes,
            brushes,
            materials: &self.materials,
            aabb_trees: &self.aabb_trees,
            partitions: &self.partitions,
            vertex_points: &self.vertex_points,
            tri_indices: &self.tri_indices,
            tri_edge_is_walkable: &self.tri_edge_is_walkable,
            cmodels: &self.cmodels,
            temp: TempModels::default(),
        }
    }
}
