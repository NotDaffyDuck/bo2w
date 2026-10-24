//! The collision tables a map's clip data is read into.

use std::collections::HashMap;

use crate::MapTriggerHull;

/// One convex brush.
#[derive(Clone, Debug)]
pub struct ClipBrush {
    /// The bounding planes, each as a normal and a distance.
    pub planes: Vec<[f32; 4]>,
    pub contents: u32,
    /// The surface flags of each plane, in plane order.
    pub plane_surface_flags: Vec<u32>,
    pub glass_encoded: u16,
    /// The brush's corner points. Only the Black Ops II player sweep reads
    /// them, so a map from anywhere else leaves this empty.
    pub verts: Vec<[f32; 3]>,
    /// The surface flags a hit on this brush reports. Black Ops II reports
    /// one face for the whole brush; other maps leave this at zero.
    pub hit_surface_flags: u32,
}

/// An inner node of the collision tree: a splitting plane and the node on
/// each side. A negative child `!n` stands for leaf `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipBspNode {
    pub plane: [f32; 4],
    pub children: [i32; 2],
}

/// A leaf of the collision tree: the run of `leafbrushes` it owns and the run
/// of the mesh's AABB trees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClipBspLeaf {
    pub first_brush: u32,
    pub num_brushes: u16,
    pub first_coll_aabb_index: u16,
    pub coll_aabb_count: u16,
}

/// A brush model: the world's box plus the brushes that make up each moving
/// or triggered piece.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClipCmodel {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub radius: f32,
    pub first_brush: u32,
    pub num_brushes: u16,
}

/// A material's collision-relevant flags, by the order the map lists them.
#[derive(Clone, Debug, Default)]
pub struct ClipMapMaterial {
    pub name: String,
    pub surface_flags: u32,
    pub content_flags: u32,
}

/// Everything a map says about what is solid.
#[derive(Clone, Debug, Default)]
pub struct ClipCollision {
    pub brushes: Vec<ClipBrush>,
    pub nodes: Vec<ClipBspNode>,
    pub leaves: Vec<ClipBspLeaf>,
    /// Brush numbers, which leaves index with `first_brush`/`num_brushes`.
    pub leafbrushes: Vec<u16>,
    pub materials: Vec<ClipMapMaterial>,
    pub cmodels: Vec<ClipCmodel>,
    /// For each brush model, its collision-triangle trees as (first AABB node,
    /// count). Black Ops II only: its helicopter height-lock ceilings are
    /// meshes of this kind, with no brushes.
    pub cmodel_mesh_roots: Vec<(u16, u16)>,
    /// The trigger hulls of each brush model.
    pub trigger_models: Vec<Vec<MapTriggerHull>>,
    /// The brush model built for an xmodel that carries brushes of its own
    /// (Black Ops II's cars), by model name. An entity showing that model is
    /// solid as that brush model.
    pub model_cmodels: HashMap<String, u32>,
    /// The map's own clip map, which Black Ops II's world traces read.
    pub t6: Option<&'static clipmap_t6::ClipMap<'static>>,
    /// The placed static models filed in a world of their own, which Black
    /// Ops II's bullet and script traces read.
    pub t6_static: Option<&'static clipmap_t6::StaticWorld>,
}
