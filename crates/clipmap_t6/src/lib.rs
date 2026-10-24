//! bo2mp: Black Ops II's clip map, written from BO2's own order and numbers:
//! the map tree (`cNode_t`) down to its leafs (`cLeaf_s`), the leaf brush
//! node tree that holds a leaf's brushes (`cLeafBrushNode_s`), the terrain
//! mesh (`CollisionAabbTree`, `CollisionPartition` and their triangles) and
//! the brush models (`cmodel_t`) and the temporary box and brush models,
//! with `CM_Trace`, `CM_BoxTrace`, `CM_PointContents` and the sight traces
//! `CM_BoxSightTrace` and `CM_SightTracePoint` over them, the world sectors
//! (`cm_world_t`) that file the linked entities and static models, and the
//! point traces through the static models' collision triangles.
//!
//! The trace setup and the sweep against one brush come from the trace
//! crate. It depends on no game crate, so another project can use it as a
//! path dependency.
//!
//! Not here yet: the other traces' primitive lists.

#![no_std]
#![forbid(unsafe_code)]
// The negated float tests are BO2's own: they keep its NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

extern crate alloc;

mod contents;
mod mesh;
mod sectors;
mod sight;
mod static_store;
mod staticmodels;
mod store;
mod temp;
mod tree;

pub use contents::{
    LeafList, angles_to_axis, box_leafnums, box_on_plane_side, point_contents, point_leafnum,
    transformed_point_contents,
};
pub use mesh::{closest_point_on_tri, does_capsule_intersect_triangle, segment_distance_sq};
pub use sectors::{
    LINKED_ENTITIES, LinkedEntities, SECTOR_SPLIT_MIN, StaticModel, SvEntity, WORLD_SECTORS, World, WorldSector,
};
pub use sight::SightPrim;
pub use staticmodels::{
    XMODEL_TRACE_SKIN, XMODEL_TRI_EPSILON, XModelColl, XModelCollSurf, XModelCollTri, inv_scaled_axis,
};
pub use static_store::{StaticHit, StaticModelStore, StaticWorld, StoredCollSurf, StoredStaticModel, StoredXModel};
pub use store::{ClipStore, StoreError, StoredBrush, StoredLeafBrushNode};
pub use temp::TempModels;
pub use tree::{box_brushes, intersect_extents_aabb};

use trace_t6::{Brush, Trace, TraceWork};

/// The handle of the temporary box model (`CM_TempBoxModel`), which BO2
/// traces as a capsule.
pub const CAPSULE_MODEL_HANDLE: u32 = 0x3fff;
/// The handle of the temporary brush model (`CM_TempBrushModel`).
pub const TEMP_BRUSH_MODEL_HANDLE: u32 = 0x4000;
/// Leafs `CM_PositionTest` looks at, at most.
pub const POSITION_TEST_MAX_LEAFS: usize = 1024;

/// `cplane_s`: a plane, which axis it lies on (3 for none) and the signs of
/// its normal.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CPlane {
    pub normal: [f32; 3],
    pub dist: f32,
    pub kind: u8,
    pub signbits: u8,
}

impl CPlane {
    /// A plane with its axis and sign bits worked out.
    #[must_use]
    pub fn new(normal: [f32; 3], dist: f32) -> Self {
        let kind = if normal[0] == 1.0 {
            0
        } else if normal[1] == 1.0 {
            1
        } else if normal[2] == 1.0 {
            2
        } else {
            3
        };
        let mut signbits = 0;
        for (i, n) in normal.iter().enumerate() {
            if *n < 0.0 {
                signbits |= 1 << i;
            }
        }
        Self {
            normal,
            dist,
            kind,
            signbits,
        }
    }
}

/// `cNode_t`: a split of the map tree. A child below zero is leaf
/// `-1 - child`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Node {
    pub plane: CPlane,
    pub children: [i16; 2],
}

/// `cLeaf_s`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Leaf {
    pub first_coll_aabb: u16,
    pub coll_aabb_count: u16,
    pub brush_contents: u32,
    pub terrain_contents: u32,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub leaf_brush_node: i32,
    pub cluster: i16,
}

/// `cLeafBrushNode_s`. With a positive count it holds that many brushes;
/// otherwise it splits on `axis` at `dist` (widened by `range`) and its
/// children are `child_offset` nodes further on. A negative count also
/// means the next node holds brushes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LeafBrushNode<'a> {
    pub axis: u8,
    pub leaf_brush_count: i16,
    pub contents: u32,
    pub brushes: &'a [u16],
    pub dist: f32,
    pub range: f32,
    pub child_offset: [u16; 2],
}

/// `CollisionAabbTree`: a node of a terrain mesh's box tree. With no
/// children, `u` is its partition; otherwise its first child.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AabbTree {
    pub origin: [f32; 3],
    pub material_index: u16,
    pub child_count: u16,
    pub half_size: [f32; 3],
    pub u: u32,
}

/// `CollisionPartition`: a run of mesh triangles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Partition {
    pub tri_count: u8,
    pub first_tri: i32,
}

/// `ClipMaterial`'s flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipMaterial {
    pub surface_flags: u32,
    pub content_flags: u32,
}

/// `cmodel_t`: a brush model and the leaf that holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CModel {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub radius: f32,
    pub leaf: Leaf,
}

/// The parts of `clipMap_t` the traces read. Model 0 is the world.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClipMap<'a> {
    pub nodes: &'a [Node],
    pub leafs: &'a [Leaf],
    pub leaf_brush_nodes: &'a [LeafBrushNode<'a>],
    pub brushes: &'a [Brush<'a>],
    pub materials: &'a [ClipMaterial],
    pub aabb_trees: &'a [AabbTree],
    pub partitions: &'a [Partition],
    pub verts: &'a [[f32; 3]],
    pub tri_indices: &'a [[u16; 3]],
    /// Three bits a triangle, one an edge: a sphere stopped by that edge
    /// may stand on it.
    pub tri_edge_is_walkable: &'a [u8],
    pub cmodels: &'a [CModel],
    /// The trace thread's temporary box and brush list.
    pub temp: TempModels<'a>,
}

/// BO2's trace check count: a partition already looked at during this
/// trace is skipped. `partitions` has one mark for each map partition.
#[derive(Debug)]
pub struct CheckCount<'b> {
    pub global: u16,
    pub partitions: &'b mut [u16],
}

impl<'b> CheckCount<'b> {
    #[must_use]
    pub fn new(partitions: &'b mut [u16]) -> Self {
        Self {
            global: 0,
            partitions,
        }
    }

    /// The first look at partition `p` this trace.
    pub(crate) fn first_look(&mut self, p: usize) -> bool {
        if self.partitions[p] == self.global {
            return false;
        }
        self.partitions[p] = self.global;
        true
    }
}

impl ClipMap<'_> {
    /// `CM_ClipHandleToModel`: a brush model, or the temporary box model for
    /// either temporary handle.
    #[must_use]
    pub fn brush_model(&self, model: u32) -> Option<&CModel> {
        match self.cmodels.get(model as usize) {
            None if model == CAPSULE_MODEL_HANDLE || model == TEMP_BRUSH_MODEL_HANDLE => Some(&self.temp.box_model),
            found => found,
        }
    }

    /// `CM_Trace`: sweep the box `mins`..`maxs` from `start` to `end`
    /// through `model` (0 for the world). `trace` carries the best hit so
    /// far in and out.
    #[allow(clippy::too_many_arguments)]
    pub fn trace(
        &self,
        check: &mut CheckCount<'_>,
        trace: &mut Trace,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        model: u32,
        contents: u32,
    ) {
        let mut tw = TraceWork::new(start, end, mins, maxs, contents);
        check.global = check.global.wrapping_add(1);
        let old_sflags = trace.sflags;
        let old_fraction = trace.fraction;
        trace.sflags = u32::MAX;
        if tw.is_position_test() {
            tw.is_point = false;
            if model == 0 {
                contents::position_test(self, check, &tw, trace);
            } else if model == CAPSULE_MODEL_HANDLE {
                if self.temp.box_contents & contents != 0 {
                    temp::test_capsule_in_capsule(&tw, &self.temp, trace);
                }
            } else if model == TEMP_BRUSH_MODEL_HANDLE {
                if !trace.allsolid {
                    temp::test_in_temp_brush(&tw, &self.temp, trace);
                }
            } else if let Some(cm) = self.brush_model(model) {
                let leaf = &cm.leaf;
                if !trace.allsolid
                    && (leaf.brush_contents & contents == 0
                        || !tree::test_in_leaf_brush_node(self, &tw, leaf, trace))
                    && leaf.terrain_contents & contents != 0
                {
                    mesh::mesh_test_in_leaf(self, check, &tw, leaf, trace);
                }
            }
        } else if model == 0 {
            let s = tw.extents.start;
            let e = tw.extents.end;
            let p1 = [s[0], s[1], s[2], 0.0];
            let p2 = [e[0], e[1], e[2], trace.fraction];
            tree::trace_through_tree(self, check, &tw, 0, p1, p2, trace);
        } else if model == CAPSULE_MODEL_HANDLE {
            if self.temp.box_contents & contents != 0 {
                temp::trace_capsule_through_capsule(&tw, &self.temp, trace);
            }
        } else if model == TEMP_BRUSH_MODEL_HANDLE {
            temp::trace_through_temp_brush(&tw, &self.temp, trace);
        } else if let Some(cm) = self.brush_model(model) {
            tree::trace_through_leaf(self, check, &tw, &cm.leaf, trace);
        }
        if !trace.walkable && !trace.startsolid {
            trace.walkable = 0.7 <= trace.normal[2];
        }
        if !(trace.fraction < old_fraction) {
            trace.sflags = old_sflags;
        }
    }

    /// `CM_BoxTrace`: a fresh trace through `model`.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn box_trace(
        &self,
        check: &mut CheckCount<'_>,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        model: u32,
        contents: u32,
    ) -> Trace {
        let mut trace = Trace {
            fraction: 1.0,
            bone_index: 0xfe,
            ..Trace::default()
        };
        self.trace(check, &mut trace, start, end, mins, maxs, model, contents);
        trace
    }

    pub(crate) fn tri_verts(&self, tri: usize) -> [[f32; 3]; 3] {
        let [a, b, c] = self.tri_indices[tri];
        [
            self.verts[a as usize],
            self.verts[b as usize],
            self.verts[c as usize],
        ]
    }

    pub(crate) fn edge_is_walkable(&self, bit: usize) -> bool {
        self.tri_edge_is_walkable[bit >> 3] & (1 << (bit & 7)) != 0
    }
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Vec3NormalizeTo`: the unit vector and the length (a zero vector stays
/// zero).
pub(crate) fn normalize_to(v: [f32; 3]) -> ([f32; 3], f32) {
    let len = libm::sqrtf(dot(v, v));
    let div = if 0.0 <= -len { 1.0 } else { len };
    (scale(v, 1.0 / div), len)
}

#[cfg(test)]
mod tests;
