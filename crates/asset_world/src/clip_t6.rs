//! bo2zm: Black Ops II clipMap_t -> ClipCollision. Not part of the upstream engine.
//!
//! T6 brushes and collision triangles follow T5's shapes (box + extra side
//! planes; triangles in partitions under an AABB tree), so this mirrors the
//! T5 conversion: six axial planes from the box with the axial side flags,
//! then every extra side; surface flags translated the T5 way. The collision
//! tree comes over too, as T5's does: the world's walls are only the brushes
//! its leaves list. Brush models (doors, triggers, the zombie mode's volumes)
//! keep their brushes in the same table, reachable only through `cmodels`;
//! testing every brush instead walls the player inside those volumes.

use asset_t6::{ClipBrushRef, ClipLeafRef, ClipRef, XModelCollSurfRef, XModelRef};
use clipmap_t6::{
    AabbTree, CModel, CPlane, ClipMap, ClipMaterial, ClipStore, Leaf, Node, Partition, StaticModelStore, StaticWorld,
    StoreError, StoredBrush, StoredCollSurf, StoredLeafBrushNode, StoredStaticModel, StoredXModel, XModelCollTri,
};
use trace_t6::{BrushSide, Plane};

use crate::clip_error::ClipCollisionError;
use crate::clip_flags::{t5_axial_plane_flags, t5_surface_flags};
use crate::clip_leafbrush::{LEAFBRUSH_NODE_MAX_DEPTH, leafbrush_node_visits, leafbrush_range};
use crate::clip_tables::{
    ClipBrush, ClipBspLeaf, ClipBspNode, ClipCmodel, ClipCollision, ClipMapMaterial,
};

/// Append the brush list under one leaf's brush node (0 = none) and return
/// its range in `out`.
fn append_leaf_brushes(
    clip: &ClipRef,
    leaf: &ClipLeafRef,
    out: &mut Vec<u16>,
) -> Result<(u32, u16), ClipCollisionError> {
    let first_len = out.len();
    if leaf.leaf_brush_node > 0 {
        let mut pending = vec![(leaf.leaf_brush_node as usize, 0)];
        while let Some((index, depth)) = pending.pop() {
            if depth > LEAFBRUSH_NODE_MAX_DEPTH {
                return Err(ClipCollisionError::Truncated);
            }
            let node = clip
                .leafbrush_nodes
                .get(index)
                .ok_or(ClipCollisionError::Truncated)?;
            if node.leaf_brush_count > 0 {
                out.extend_from_slice(&node.brushes);
            } else {
                pending.extend(
                    leafbrush_node_visits(
                        index,
                        node.leaf_brush_count,
                        usize::from(node.child_offset[0]),
                        usize::from(node.child_offset[1]),
                    )
                    .into_iter()
                    .map(|i| (i, depth + 1)),
                );
            }
        }
    }
    leafbrush_range(first_len, out.len())
}

/// codmp: what `attach_t6_glass_brushes` did, for the load report.
#[derive(Clone, Copy, Debug, Default)]
pub struct T6GlassClipCensus {
    pub panes: usize,
    pub brushes: usize,
    pub leaf_links: usize,
    pub missing_models: usize,
    /// Contents of the first window brush (what masks stop at it).
    pub contents: u32,
    /// Brushes a second window's model listed again (left where the first put them).
    pub shared_brushes: usize,
}

/// codmp: where one window's brush model stands (`Glass::brushModel`,
/// `origin`, `angles`). The model's brushes are in its own space;
/// `GlassSv_PointTrace` and `GlassSv_ClipMoveTrace` meet them through
/// `CM_TransformedBoxTrace` at this origin, turned by these angles.
#[derive(Clone, Copy, Debug, Default)]
pub struct T6GlassPlacement {
    pub brush_model: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// Which of the map's windows this is (its index in the capture's).
    pub glass: usize,
}

/// codmp: Black Ops II's windows stand until they break. Each window's
/// brushes belong to its brush model (`*N`), which no world leaf lists, so
/// nothing was stopped by them. Move each brush to where its window stands
/// (`placements[pane]`), tag it with its pane (`glass_encoded` = pane + 1,
/// what a trace asks the glass state about) and list it in every world leaf
/// its box reaches.
pub fn attach_t6_glass_brushes(
    out: &mut ClipCollision,
    placements: &[T6GlassPlacement],
) -> Result<T6GlassClipCensus, ClipCollisionError> {
    let mut census = T6GlassClipCensus::default();
    let range = |first: u32, num: u16| first as usize..first as usize + usize::from(num);
    let mut leaf_lists: Vec<Vec<u16>> = out
        .leaves
        .iter()
        .map(|l| out.leafbrushes.get(range(l.first_brush, l.num_brushes)).unwrap_or(&[]).to_vec())
        .collect();
    let model_lists: Vec<Vec<u16>> = out
        .cmodels
        .iter()
        .map(|m| out.leafbrushes.get(range(m.first_brush, m.num_brushes)).unwrap_or(&[]).to_vec())
        .collect();
    for (pane, place) in placements.iter().enumerate() {
        let Ok(encoded) = u16::try_from(pane + 1) else {
            break;
        };
        let model = place.brush_model;
        let Some(list) = model_lists.get(model as usize).filter(|_| model != 0) else {
            census.missing_models += 1;
            continue;
        };
        census.panes += 1;
        // `AnglesToAxis`: model space -> world is origin + x*axis[0] + y*axis[1] + z*axis[2].
        let axis = game_math::angles_to_axis(place.angles);
        let o = place.origin;
        let to_world = |l: [f32; 3]| {
            [0, 1, 2].map(|j| o[j] + l[0] * axis[0][j] + l[1] * axis[1][j] + l[2] * axis[2][j])
        };
        for &b in list {
            let Some(brush) = out.brushes.get_mut(usize::from(b)) else {
                continue;
            };
            if brush.planes.len() < 6 {
                continue;
            }
            if brush.glass_encoded != 0 {
                census.shared_brushes += 1;
                continue;
            }
            brush.glass_encoded = encoded;
            if census.brushes == 0 {
                census.contents = brush.contents;
            }
            census.brushes += 1;
            let p = &brush.planes;
            let local_mins = [-p[1][3], -p[3][3], -p[5][3]];
            let local_maxs = [p[0][3], p[2][3], p[4][3]];
            // Each side plane n.x <= d in model space is n'.w <= d + n'.origin in the
            // world, n' the normal turned by the axis.
            for plane in &mut brush.planes {
                let n = [0, 1, 2]
                    .map(|j| plane[0] * axis[0][j] + plane[1] * axis[1][j] + plane[2] * axis[2][j]);
                let d = plane[3] + n[0] * o[0] + n[1] * o[1] + n[2] * o[2];
                *plane = [n[0], n[1], n[2], d];
            }
            // The world box around the model box's eight corners.
            let mut mins = [f32::MAX; 3];
            let mut maxs = [f32::MIN; 3];
            for corner in 0..8 {
                let pick = |i: usize| if corner >> i & 1 == 0 { local_mins[i] } else { local_maxs[i] };
                let w = to_world([pick(0), pick(1), pick(2)]);
                for j in 0..3 {
                    mins[j] = mins[j].min(w[j]);
                    maxs[j] = maxs[j].max(w[j]);
                }
            }
            let center = [0, 1, 2].map(|i| (mins[i] + maxs[i]) * 0.5);
            let half = [0, 1, 2].map(|i| (maxs[i] - mins[i]) * 0.5 + 0.125);
            let mut pending = vec![0i32];
            let mut steps = 0usize;
            while let Some(num) = pending.pop() {
                steps += 1;
                if steps > 1 << 20 {
                    return Err(ClipCollisionError::Truncated);
                }
                if out.nodes.is_empty() || num < 0 {
                    let leaf = if out.nodes.is_empty() { 0 } else { (!num) as usize };
                    if let Some(l) = leaf_lists.get_mut(leaf) {
                        if !l.contains(&b) {
                            l.push(b);
                            census.leaf_links += 1;
                        }
                    }
                    continue;
                }
                let Some(node) = out.nodes.get(num as usize) else {
                    continue;
                };
                let n = [node.plane[0], node.plane[1], node.plane[2]];
                let d = n[0] * center[0] + n[1] * center[1] + n[2] * center[2] - node.plane[3];
                let r = half[0] * n[0].abs() + half[1] * n[1].abs() + half[2] * n[2].abs();
                if d + r >= 0.0 {
                    pending.push(node.children[0]);
                }
                if d - r < 0.0 {
                    pending.push(node.children[1]);
                }
            }
        }
    }
    if census.brushes == 0 {
        return Ok(census);
    }
    let mut leafbrushes = Vec::with_capacity(out.leafbrushes.len() + census.leaf_links);
    for (leaf, list) in out.leaves.iter_mut().zip(&leaf_lists) {
        let first = leafbrushes.len();
        leafbrushes.extend_from_slice(list);
        (leaf.first_brush, leaf.num_brushes) = leafbrush_range(first, leafbrushes.len())?;
    }
    for (m, list) in out.cmodels.iter_mut().zip(&model_lists) {
        let first = leafbrushes.len();
        leafbrushes.extend_from_slice(list);
        (m.first_brush, m.num_brushes) = leafbrush_range(first, leafbrushes.len())?;
    }
    out.leafbrushes = leafbrushes;
    Ok(census)
}

pub fn build_t6_clip_collision(clip: &ClipRef) -> Result<ClipCollision, ClipCollisionError> {
    if clip.brushes.is_empty() && clip.tri_indices.is_empty() {
        return Err(ClipCollisionError::MissingTables);
    }
    let mut out = ClipCollision {
        brushes: Vec::with_capacity(clip.brushes.len()),
        ..ClipCollision::default()
    };
    // bo2mp: BO2's player collision skips caulk faces when it picks the face
    // a brush hit reports; caulk is the flags of the material named "caulk"
    // (a map without one has no face to skip).
    let caulk = clip
        .materials
        .iter()
        .find(|m| m.name == "caulk")
        .map_or(u32::MAX, |m| m.surface_flags as u32);
    out.brushes.extend(clip.brushes.iter().map(|b| t6_brush(b, caulk)));

    for m in &clip.materials {
        out.materials.push(ClipMapMaterial {
            name: m.name.clone(),
            surface_flags: t5_surface_flags(m.surface_flags as u32),
            content_flags: m.content_flags as u32,
        });
    }

    for n in &clip.nodes {
        out.nodes.push(ClipBspNode {
            plane: n.plane,
            children: [i32::from(n.children[0]), i32::from(n.children[1])],
        });
    }
    for leaf in &clip.leaves {
        let (first_brush, num_brushes) = append_leaf_brushes(clip, leaf, &mut out.leafbrushes)?;
        out.leaves.push(ClipBspLeaf {
            first_brush,
            num_brushes,
            first_coll_aabb_index: leaf.first_coll_aabb_index,
            coll_aabb_count: leaf.coll_aabb_count,
        });
    }
    for m in &clip.cmodels {
        let (first_brush, num_brushes) = if m.shares_map_info {
            append_leaf_brushes(clip, &m.leaf, &mut out.leafbrushes)?
        } else {
            leafbrush_range(out.leafbrushes.len(), out.leafbrushes.len())?
        };
        out.cmodels.push(ClipCmodel {
            mins: m.mins,
            maxs: m.maxs,
            radius: m.radius,
            first_brush,
            num_brushes,
        });
        out.cmodel_mesh_roots
            .push((m.leaf.first_coll_aabb_index, m.leaf.coll_aabb_count));
    }
    // bo2mp: the clip map BO2's traces read comes from the same tables; a map
    // whose tables point outside themselves does not load.
    out.t6 = Some(t6_clip_map(clip).map_err(ClipCollisionError::Store)?);
    Ok(out)
}

/// bo2mp: `cLeaf_s` as BO2's traces read it.
fn t6_leaf(l: &ClipLeafRef) -> Leaf {
    Leaf {
        first_coll_aabb: l.first_coll_aabb_index,
        coll_aabb_count: l.coll_aabb_count,
        brush_contents: l.brush_contents as u32,
        terrain_contents: l.terrain_contents as u32,
        mins: l.mins,
        maxs: l.maxs,
        leaf_brush_node: l.leaf_brush_node,
        cluster: l.cluster,
    }
}

/// bo2mp: `clip`'s tables as BO2's traces read them, with BO2's own surface
/// flags (no T5 translation). A brush model that does not share the map's
/// tables gets no brushes, as `build_t6_clip_collision` gives it.
pub fn t6_clip_store(clip: &ClipRef) -> Result<ClipStore, StoreError> {
    let flags = |f: [[i32; 3]; 2]| f.map(|side| side.map(|v| v as u32));
    let mut store = ClipStore {
        nodes: clip
            .nodes
            .iter()
            .map(|n| Node {
                plane: CPlane {
                    normal: [n.plane[0], n.plane[1], n.plane[2]],
                    dist: n.plane[3],
                    kind: n.plane_type,
                    signbits: n.plane_signbits,
                },
                children: n.children,
            })
            .collect(),
        leafs: clip.leaves.iter().map(t6_leaf).collect(),
        materials: clip
            .materials
            .iter()
            .map(|m| ClipMaterial {
                surface_flags: m.surface_flags as u32,
                content_flags: m.content_flags as u32,
            })
            .collect(),
        aabb_trees: clip
            .aabb_trees
            .iter()
            .map(|t| AabbTree {
                origin: t.origin,
                material_index: t.material_index,
                child_count: t.child_count,
                half_size: t.half_size,
                u: t.u as u32,
            })
            .collect(),
        partitions: clip
            .partitions
            .iter()
            .map(|&(tri_count, first_tri)| Partition { tri_count, first_tri })
            .collect(),
        verts: clip.verts.clone(),
        tri_indices: clip.tri_indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
        tri_edge_is_walkable: clip.tri_edge_is_walkable.clone(),
        cmodels: clip
            .cmodels
            .iter()
            .map(|m| {
                let mut leaf = t6_leaf(&m.leaf);
                if !m.shares_map_info {
                    leaf.brush_contents = 0;
                    leaf.leaf_brush_node = 0;
                }
                CModel {
                    mins: m.mins,
                    maxs: m.maxs,
                    radius: m.radius,
                    leaf,
                }
            })
            .collect(),
        box_brush_node: u32::try_from(clip.box_leaf_brush_node).ok().filter(|&i| i > 0),
        ..ClipStore::default()
    };
    for (index, n) in clip.leafbrush_nodes.iter().enumerate() {
        let first_brush = store.leaf_brushes.len() as u32;
        if n.leaf_brush_count > 0 {
            if n.brushes.len() != n.leaf_brush_count as usize {
                return Err(StoreError {
                    table: "leaf_brush_nodes",
                    index,
                });
            }
            store.leaf_brushes.extend_from_slice(&n.brushes);
        }
        store.leaf_brush_nodes.push(StoredLeafBrushNode {
            axis: n.axis,
            leaf_brush_count: n.leaf_brush_count,
            contents: n.contents as u32,
            first_brush,
            dist: n.dist,
            range: n.range,
            child_offset: n.child_offset,
        });
    }
    for (index, b) in clip.brushes.iter().enumerate() {
        if b.side_cflags.len() != b.sides.len() {
            return Err(StoreError { table: "brushes", index });
        }
        let first_side = store.brush_sides.len() as u32;
        store
            .brush_sides
            .extend(b.sides.iter().zip(&b.side_cflags).map(|(&(normal, dist, sflags), &cflags)| BrushSide {
                plane: Plane { normal, dist },
                cflags: cflags as u32,
                sflags: sflags as u32,
            }));
        store.brushes.push(StoredBrush {
            mins: b.mins,
            maxs: b.maxs,
            contents: b.contents as u32,
            first_side,
            side_count: b.sides.len() as u32,
            axial_cflags: flags(b.axial_cflags),
            axial_sflags: flags(b.axial_sflags),
        });
    }
    Ok(store)
}

/// bo2mp: the BO2 clip map of `clip`, built and checked the first time its
/// map name is asked for and kept for the rest of the run.
pub fn t6_clip_map(clip: &ClipRef) -> Result<&'static ClipMap<'static>, StoreError> {
    static KEPT: std::sync::Mutex<Vec<(String, &'static ClipMap<'static>)>> = std::sync::Mutex::new(Vec::new());
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&(_, map)) = kept.iter().find(|(name, _)| *name == clip.name) {
        return Ok(map);
    }
    let store = t6_clip_store(clip)?;
    store.check()?;
    let map = store.leak();
    kept.push((clip.name.clone(), map));
    Ok(map)
}

/// bo2mp: the placed models of `clip` (`staticModelList`: cars, fences,
/// trees) with the collision of the models they place (`xmodels`, the
/// zone's models), checked, filed over the map's world model the first
/// time its map name is asked for and kept for the rest of the run, as
/// `out.t6_static`. Returns how many models are placed.
pub fn attach_t6_static_world(
    out: &mut ClipCollision,
    clip: &ClipRef,
    xmodels: &[XModelRef],
) -> Result<usize, StoreError> {
    let world = t6_static_world(clip, |i| {
        let key = clip.static_models[i].model?;
        let m = xmodels.get(key.index)?;
        Some((key.index, i32::from(m.coll_lod), m.coll_surfs.as_slice()))
    })?;
    out.t6_static = Some(world);
    Ok(world.models.len())
}

/// The kept static world of `clip`, each placement's model from `coll` by
/// placement index (xmodel index, collision LOD, surfaces).
fn t6_static_world<'a>(
    clip: &ClipRef,
    coll: impl Fn(usize) -> Option<(usize, i32, &'a [XModelCollSurfRef])>,
) -> Result<&'static StaticWorld, StoreError> {
    static KEPT: std::sync::Mutex<Vec<(String, &'static StaticWorld)>> = std::sync::Mutex::new(Vec::new());
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&(_, world)) = kept.iter().find(|(name, _)| *name == clip.name) {
        return Ok(world);
    }
    let store = t6_static_store(clip, coll);
    store.check()?;
    let world = store.leak(t6_clip_map(clip)?);
    kept.push((clip.name.clone(), world));
    Ok(world)
}

/// `clip`'s static models as a store, each placed model's collision added
/// once, the first time a placement names it.
fn t6_static_store<'a>(
    clip: &ClipRef,
    coll: impl Fn(usize) -> Option<(usize, i32, &'a [XModelCollSurfRef])>,
) -> StaticModelStore {
    let mut store = StaticModelStore::default();
    let mut added: Vec<(usize, u32)> = Vec::new();
    for (index, sm) in clip.static_models.iter().enumerate() {
        let xmodel = coll(index).map(|(key, coll_lod, surfs)| {
            if let Some(&(_, at)) = added.iter().find(|(k, _)| *k == key) {
                return at;
            }
            let at = store.xmodels.len() as u32;
            store.xmodels.push(StoredXModel {
                coll_lod,
                first_surf: store.surfs.len() as u32,
                surf_count: surfs.len() as u32,
            });
            for s in surfs {
                store.surfs.push(StoredCollSurf {
                    first_tri: store.tris.len() as u32,
                    tri_count: s.tris.len() as u32,
                    mins: s.mins,
                    maxs: s.maxs,
                    bone_idx: s.bone_idx,
                    contents: s.contents,
                    surf_flags: s.surf_flags,
                });
                store.tris.extend(s.tris.iter().map(|t| XModelCollTri {
                    plane: t[0],
                    svec: t[1],
                    tvec: t[2],
                }));
            }
            added.push((key, at));
            at
        });
        store.models.push(StoredStaticModel {
            xmodel,
            contents: sm.contents,
            origin: sm.origin,
            inv_scaled_axis: sm.inv_scaled_axis,
            absmin: sm.absmin,
            absmax: sm.absmax,
        });
    }
    store
}

/// One T6 brush as six axial planes from its box (with the axial side
/// flags), then every extra side.
/// `caulk`: the surface flags of the map's caulk material, the faces a hit
/// never reports (`u32::MAX`: none to skip).
fn t6_brush(b: &ClipBrushRef, caulk: u32) -> ClipBrush {
    let (mins, maxs) = (b.mins, b.maxs);
    let mut planes = Vec::with_capacity(6 + b.sides.len());
    planes.push([1.0, 0.0, 0.0, maxs[0]]);
    planes.push([-1.0, 0.0, 0.0, -mins[0]]);
    planes.push([0.0, 1.0, 0.0, maxs[1]]);
    planes.push([0.0, -1.0, 0.0, -mins[1]]);
    planes.push([0.0, 0.0, 1.0, maxs[2]]);
    planes.push([0.0, 0.0, -1.0, -mins[2]]);
    let axial = [
        b.axial_sflags[0][0] as u32,
        b.axial_sflags[0][1] as u32,
        b.axial_sflags[0][2] as u32,
        b.axial_sflags[1][0] as u32,
        b.axial_sflags[1][1] as u32,
        b.axial_sflags[1][2] as u32,
    ];
    let mut plane_surface_flags = Vec::with_capacity(planes.capacity());
    plane_surface_flags.extend_from_slice(&t5_axial_plane_flags(axial.map(t5_surface_flags)));
    for &(normal, dist, sflags) in &b.sides {
        planes.push([normal[0], normal[1], normal[2], dist]);
        plane_surface_flags.push(t5_surface_flags(sflags as u32));
    }
    let hit_surface_flags = movement_t6::gjk::brush_surface_flags(
        b.axial_sflags.map(|side| side.map(|f| f as u32)),
        b.sides.iter().map(|&(normal, _, sflags)| (normal[2], sflags as u32)),
        caulk,
    );
    ClipBrush {
        planes,
        contents: b.contents as u32,
        plane_surface_flags,
        glass_encoded: 0,
        verts: b.verts.clone(),
        hit_surface_flags: t5_surface_flags(hit_surface_flags),
    }
}

/// codmp: what `attach_t6_model_collision` did, for the load report.
#[derive(Clone, Copy, Debug, Default)]
pub struct T6ModelClipCensus {
    pub models: usize,
    pub brushes: usize,
    pub skipped: usize,
}

/// codmp: Black Ops II's cars stop a walking player with their models'
/// collision-map brushes (player and vehicle clip), not with the triangles
/// bullets hit. Each model with any becomes a brush model of its own, in
/// model space, after the map's: no world leaf lists it, so it stands only
/// where an entity showing that model links it (`model_cmodels`).
pub fn attach_t6_model_collision<'a>(
    out: &mut ClipCollision,
    models: impl IntoIterator<Item = (&'a str, &'a [ClipBrushRef])>,
) -> T6ModelClipCensus {
    let mut census = T6ModelClipCensus::default();
    for (name, brushes) in models {
        if brushes.is_empty() || out.model_cmodels.contains_key(name) {
            continue;
        }
        let first_brush = out.brushes.len();
        let first_leaf = out.leafbrushes.len();
        if first_brush + brushes.len() > usize::from(u16::MAX)
            || u32::try_from(out.cmodels.len()).is_err()
        {
            census.skipped += 1;
            continue;
        }
        let mut mins = [f32::MAX; 3];
        let mut maxs = [f32::MIN; 3];
        for (k, b) in brushes.iter().enumerate() {
            for i in 0..3 {
                mins[i] = mins[i].min(b.mins[i]);
                maxs[i] = maxs[i].max(b.maxs[i]);
            }
            out.brushes.push(t6_brush(b, u32::MAX));
            out.leafbrushes.push((first_brush + k) as u16);
        }
        let radius = (0..3)
            .map(|i| mins[i].abs().max(maxs[i].abs()).powi(2))
            .sum::<f32>()
            .sqrt();
        out.model_cmodels.insert(name.to_owned(), out.cmodels.len() as u32);
        out.cmodels.push(ClipCmodel {
            mins,
            maxs,
            radius,
            first_brush: first_leaf as u32,
            num_brushes: brushes.len() as u16,
        });
        census.models += 1;
        census.brushes += brushes.len();
    }
    census
}

#[cfg(test)]
mod tests {
    use super::*;
    use asset_t6::{
        ClipBrushRef, ClipCmodelRef, ClipLeafBrushNodeRef, ClipMaterialRef, ClipNodeRef, ClipStaticModelRef,
    };
    use clipmap_t6::CheckCount;

    // bo2mp: one solid floor brush, 128 square with its top at z = 0, in the
    // one leaf of a one-node tree. Leaf brush node 0 is the empty node the
    // old loader reads as "no brushes".
    fn floor_clip() -> ClipRef {
        let leaf = ClipLeafRef {
            brush_contents: 1,
            leaf_brush_node: 1,
            mins: [-1000.0; 3],
            maxs: [1000.0; 3],
            ..ClipLeafRef::default()
        };
        ClipRef {
            name: "clip_t6 test floor".to_string(),
            brushes: vec![ClipBrushRef {
                mins: [-64.0, -64.0, -16.0],
                maxs: [64.0, 64.0, 0.0],
                contents: 1,
                axial_cflags: [[1; 3]; 2],
                axial_sflags: [[0; 3]; 2],
                sides: Vec::new(),
                side_cflags: Vec::new(),
                verts: Vec::new(),
            }],
            materials: vec![ClipMaterialRef {
                name: "floor".to_string(),
                surface_flags: 0,
                content_flags: 1,
            }],
            nodes: vec![ClipNodeRef {
                plane: [1.0, 0.0, 0.0, 1000.0],
                children: [-1, -1],
                plane_type: 0,
                plane_signbits: 0,
            }],
            leaves: vec![leaf],
            leafbrush_nodes: vec![
                ClipLeafBrushNodeRef {
                    leaf_brush_count: 0,
                    brushes: Vec::new(),
                    child_offset: [0; 2],
                    axis: 0,
                    contents: 0,
                    dist: 0.0,
                    range: 0.0,
                },
                ClipLeafBrushNodeRef {
                    leaf_brush_count: 1,
                    brushes: vec![0],
                    child_offset: [0; 2],
                    axis: 0,
                    contents: 1,
                    dist: 0.0,
                    range: 0.0,
                },
            ],
            cmodels: vec![ClipCmodelRef {
                mins: [-64.0, -64.0, -16.0],
                maxs: [64.0, 64.0, 0.0],
                radius: 100.0,
                leaf,
                shares_map_info: true,
            }],
            ..ClipRef::default()
        }
    }

    /// `sweep_box` and BO2's trace on the map's own store agree on fraction,
    /// normal and start solid.
    fn agree(start: [f32; 3], end: [f32; 3], mins: [f32; 3], maxs: [f32; 3]) {
        let clip = floor_clip();
        let old = build_t6_clip_collision(&clip).expect("old build").sweep_box(start, end, mins, maxs, 1);
        let store = t6_clip_store(&clip).expect("store");
        store.check().expect("check");
        let mut marks = vec![0u16; store.partitions.len()];
        let new = store.with_map(|map| map.box_trace(&mut CheckCount::new(&mut marks), start, end, mins, maxs, 0, 1));
        assert!(
            (old.fraction - new.fraction).abs() < 1e-3,
            "{start:?} -> {end:?}: old {} new {}",
            old.fraction,
            new.fraction
        );
        assert_eq!(old.startsolid, new.startsolid, "{start:?} -> {end:?}");
        if new.fraction < 1.0 {
            assert_eq!(old.normal, new.normal, "{start:?} -> {end:?}");
        }
    }

    #[test]
    fn point_and_drop_traces_match_the_old_sweep() {
        let point = [0.0; 3];
        agree([0.0, 0.0, 64.0], [0.0, 0.0, -64.0], point, point);
        agree([10.0, -20.0, 32.0], [10.0, -20.0, -4.0], point, point);
        agree([200.0, 0.0, 64.0], [200.0, 0.0, -64.0], point, point);
        agree([-200.0, 0.0, -8.0], [200.0, 0.0, -8.0], point, point);
        agree([0.0, 0.0, -8.0], [0.0, 0.0, 64.0], point, point);
        agree([0.0, 0.0, 64.0], [0.0, 0.0, -64.0], [-16.0, -16.0, 0.0], [16.0, 16.0, 72.0]);
        agree([300.0, 0.0, 64.0], [300.0, 0.0, -64.0], [-16.0, -16.0, 0.0], [16.0, 16.0, 72.0]);
    }

    #[test]
    fn the_store_keeps_the_map_and_refuses_bad_indices() {
        let clip = floor_clip();
        let map = t6_clip_map(&clip).expect("kept");
        assert!(std::ptr::eq(map, t6_clip_map(&clip).expect("kept again")));
        assert_eq!(map.brushes.len(), 1);
        assert_eq!(map.leaf_brush_nodes[1].brushes, &[0]);

        // No walk reads a brush leaf's axis.
        let mut leaf_axis = clip.clone();
        leaf_axis.leafbrush_nodes[1].axis = 0xff;
        assert_eq!(t6_clip_store(&leaf_axis).and_then(|s| s.check()), Ok(()));

        let mut bad = clip.clone();
        bad.name = "clip_t6 test bad leaf brush".into();
        bad.leafbrush_nodes[1].brushes = vec![1];
        assert_eq!(
            t6_clip_store(&bad).and_then(|s| s.check()),
            Err(StoreError {
                table: "leaf_brushes",
                index: 0
            })
        );
        assert!(build_t6_clip_collision(&bad).is_err());

        // The box hull's node names the box brush, one past the map's
        // brushes, as BO2's zones store it; only that node may.
        let mut boxed = clip.clone();
        boxed.leafbrush_nodes.push(ClipLeafBrushNodeRef {
            leaf_brush_count: 1,
            brushes: vec![1],
            ..boxed.leafbrush_nodes[1].clone()
        });
        boxed.leafbrush_nodes[2].contents = 0;
        let unnamed = boxed.clone();
        boxed.box_leaf_brush_node = 2;
        assert_eq!(t6_clip_store(&boxed).and_then(|s| s.check()), Ok(()));
        assert_eq!(
            t6_clip_store(&unnamed).and_then(|s| s.check()),
            Err(StoreError {
                table: "leaf_brushes",
                index: 1
            })
        );
        let mut past_box = boxed.clone();
        past_box.leafbrush_nodes[2].brushes = vec![2];
        assert_eq!(
            t6_clip_store(&past_box).and_then(|s| s.check()),
            Err(StoreError {
                table: "leaf_brushes",
                index: 1
            })
        );
        let mut bad = clip;
        bad.leaves[0].leaf_brush_node = 2;
        assert_eq!(
            t6_clip_store(&bad).and_then(|s| s.check()),
            Err(StoreError { table: "leafs", index: 0 })
        );
    }

    /// bo2mp: a wall model placed twice and a placement with no model are
    /// kept once per map, each model's collision stored once, and a line
    /// meets the nearer placement.
    #[test]
    fn static_models_are_kept_and_traced() {
        let wall = [XModelCollSurfRef {
            tris: vec![[[-1.0, 0.0, 0.0, 0.0], [0.0, 0.1, 0.0, 0.0], [0.0, 0.0, 0.1, 0.0]]],
            mins: [-1.0; 3],
            maxs: [10.0; 3],
            bone_idx: 0,
            contents: 1,
            surf_flags: 0x50,
        }];
        let placed = |x: f32| ClipStaticModelRef {
            model: None,
            contents: 1,
            origin: [x, 0.0, -8.0],
            inv_scaled_axis: clipmap_t6::inv_scaled_axis([0.0; 3], 1.0),
            absmin: [x - 4.0, -4.0, -12.0],
            absmax: [x + 4.0, 4.0, -4.0],
        };
        let mut clip = floor_clip();
        clip.name = "clip_t6 test static models".into();
        clip.static_models = vec![placed(40.0), placed(0.0), placed(-40.0)];
        // Placements 0 and 2 place xmodel 3; placement 1 places none.
        let coll = |i: usize| (i != 1).then_some((3, 0, &wall[..]));

        let store = t6_static_store(&clip, coll);
        assert_eq!((store.models.len(), store.xmodels.len(), store.surfs.len(), store.tris.len()), (3, 1, 1, 1));
        assert_eq!(store.models[1].xmodel, None);
        let world = t6_static_world(&clip, coll).expect("kept");
        assert!(std::ptr::eq(world, t6_static_world(&clip, coll).expect("kept again")));

        let hit = world.nearest_hit([-60.0, 2.0, -6.0], [60.0, 2.0, -6.0], 1.0, 1).expect("a wall");
        assert_eq!((hit.model, hit.sflags, hit.cflags), (2, 0x50, 1));
        assert_eq!(world.nearest_hit([-60.0, 2.0, -6.0], [60.0, 2.0, -6.0], 1.0, 2), None);
    }

    // bo2mp: BO2's own maps build their clip map and static world. Runs only
    // where the zones are: T6_ZONE_DIR = BO2's zonell folder.
    #[test]
    fn real_maps_build_their_bo2_clip_map() {
        let Ok(dir) = std::env::var("T6_ZONE_DIR") else {
            return;
        };
        let mut failed = Vec::new();
        for map in ["mp_nuketown_2020", "mp_la"] {
            let path = std::path::Path::new(&dir).join(format!("{map}.ff"));
            let capture = asset_t6::capture_zone(&path).unwrap_or_else(|e| panic!("{map}: {e}"));
            let clip = capture.clip.as_ref().unwrap_or_else(|| panic!("{map}: no clipMap"));
            let store = match t6_clip_store(clip) {
                Ok(store) => store,
                Err(e) => {
                    failed.push(format!("{map}: {e}; {}", refused_row(clip, e)));
                    continue;
                }
            };
            if let Err(e) = store.check() {
                failed.push(format!("{map}: {e}; {}; every bad row: {}", refused_row(clip, e), every_bad(&store)));
                continue;
            }
            let mut out = ClipCollision::default();
            if let Err(e) = attach_t6_static_world(&mut out, clip, &capture.xmodels) {
                failed.push(format!("{map} static world: {e}"));
            }
        }
        assert!(failed.is_empty(), "{failed:?}");
    }

    /// How many rows of each table break `ClipStore::check`'s rule, and the
    /// first of each, so one run shows every table that is read wrong.
    fn every_bad(s: &ClipStore) -> String {
        let mut out = Vec::new();
        let mut note = |table: &str, bad: Vec<usize>| {
            if let Some(&first) = bad.first() {
                out.push(format!("{table} {} (first {first})", bad.len()));
            }
        };
        let rows = |ok: &dyn Fn(usize) -> bool, len: usize| (0..len).filter(|&i| !ok(i)).collect::<Vec<_>>();
        let leaf_ok = |l: &Leaf| {
            usize::from(l.first_coll_aabb) + usize::from(l.coll_aabb_count) <= s.aabb_trees.len()
                && l.leaf_brush_node >= 0
                && ((l.leaf_brush_node as usize) < s.leaf_brush_nodes.len()
                    || (l.leaf_brush_node == 0 && l.brush_contents == 0))
        };
        note(
            "nodes",
            rows(
                &|i| {
                    s.nodes[i].children.iter().all(|&c| {
                        if c < 0 {
                            ((-1 - i32::from(c)) as usize) < s.leafs.len()
                        } else {
                            (c as usize) < s.nodes.len()
                        }
                    })
                },
                s.nodes.len(),
            ),
        );
        note("leafs", rows(&|i| leaf_ok(&s.leafs[i]), s.leafs.len()));
        note("cmodels", rows(&|i| leaf_ok(&s.cmodels[i].leaf), s.cmodels.len()));
        let lbn = s.leaf_brush_nodes.len();
        note(
            "leaf_brush_nodes",
            rows(
                &|i| {
                    let n = &s.leaf_brush_nodes[i];
                    if n.leaf_brush_count > 0 {
                        n.first_brush as usize + n.leaf_brush_count as usize <= s.leaf_brushes.len()
                    } else {
                        n.axis < 3
                            && n.child_offset.iter().all(|&o| i + usize::from(o) < lbn)
                            && (n.leaf_brush_count == 0 || i + 1 < lbn)
                    }
                },
                lbn,
            ),
        );
        let box_run = s
            .box_brush_node
            .and_then(|i| s.leaf_brush_nodes.get(i as usize))
            .map(|n| n.first_brush as usize..n.first_brush as usize + n.leaf_brush_count.max(0) as usize);
        note(
            "leaf_brushes",
            rows(
                &|i| {
                    let b = usize::from(s.leaf_brushes[i]);
                    b < s.brushes.len() || (b == s.brushes.len() && box_run.as_ref().is_some_and(|r| r.contains(&i)))
                },
                s.leaf_brushes.len(),
            ),
        );
        note(
            "brushes",
            rows(
                &|i| s.brushes[i].first_side as usize + s.brushes[i].side_count as usize <= s.brush_sides.len(),
                s.brushes.len(),
            ),
        );
        note(
            "aabb_trees",
            rows(
                &|i| {
                    let t = &s.aabb_trees[i];
                    usize::from(t.material_index) < s.materials.len()
                        && if t.child_count > 0 {
                            t.u as usize + usize::from(t.child_count) <= s.aabb_trees.len()
                        } else {
                            (t.u as usize) < s.partitions.len()
                        }
                },
                s.aabb_trees.len(),
            ),
        );
        note(
            "partitions",
            rows(
                &|i| {
                    let p = &s.partitions[i];
                    p.first_tri >= 0 && p.first_tri as usize + usize::from(p.tri_count) <= s.tri_indices.len()
                },
                s.partitions.len(),
            ),
        );
        note(
            "tri_indices",
            rows(&|i| s.tri_indices[i].iter().all(|&v| usize::from(v) < s.verts.len()), s.tri_indices.len()),
        );
        if s.tri_edge_is_walkable.len() * 8 < s.tri_indices.len() * 3 {
            note("tri_edge_is_walkable", vec![s.tri_edge_is_walkable.len()]);
        }
        out.join(", ")
    }

    /// Where the out-of-range leaf brushes come from: the nodes holding them,
    /// and how their lists sit against BO2's own `leafbrushes` table.
    fn leaf_brushes_report(clip: &ClipRef) -> String {
        let nb = clip.brushes.len();
        let table = &clip.leaf_brushes;
        let base = clip.leaf_brushes_at;
        // Where a node's list starts in the table, if it lies inside it.
        let slot = |at: Option<(u8, u32)>, count: usize| -> Option<usize> {
            let ((b, o), (tb, to)) = (at?, base?);
            if b != tb || o < to || (o - to) % 2 != 0 {
                return None;
            }
            let first = ((o - to) / 2) as usize;
            (first + count <= table.len()).then_some(first)
        };
        let (mut bad_nodes, mut bad_entries, mut max_bad, mut inside, mut outside, mut same) =
            (0, 0, 0u16, 0, 0, 0);
        let mut first_bad = None;
        let mut reachable = vec![false; clip.leafbrush_nodes.len()];
        let mut pending: Vec<usize> = clip
            .leaves
            .iter()
            .map(|l| l.leaf_brush_node)
            .chain(clip.cmodels.iter().map(|m| m.leaf.leaf_brush_node))
            .filter(|&n| n > 0)
            .map(|n| n as usize)
            .collect();
        while let Some(i) = pending.pop() {
            let Some(n) = clip.leafbrush_nodes.get(i) else { continue };
            if std::mem::replace(&mut reachable[i], true) {
                continue;
            }
            if n.leaf_brush_count <= 0 {
                if n.leaf_brush_count < 0 {
                    pending.push(i + 1);
                }
                pending.extend(n.child_offset.iter().filter(|&&o| o != 0).map(|&o| i + usize::from(o)));
            }
        }
        let mut bad_reachable = 0;
        let mut bad_contents0 = 0;
        for (i, n) in clip.leafbrush_nodes.iter().enumerate() {
            if n.leaf_brush_count <= 0 {
                continue;
            }
            let at = clip.leafbrush_node_brushes_at.get(i).copied().flatten();
            match slot(at, n.brushes.len()) {
                Some(f) => {
                    inside += 1;
                    if table[f..f + n.brushes.len()] == n.brushes[..] {
                        same += 1;
                    }
                }
                None => outside += 1,
            }
            let bad: Vec<u16> = n.brushes.iter().copied().filter(|&b| usize::from(b) >= nb).collect();
            if bad.is_empty() {
                continue;
            }
            bad_nodes += 1;
            bad_entries += bad.len();
            max_bad = max_bad.max(*bad.iter().max().unwrap());
            bad_reachable += usize::from(reachable[i]);
            bad_contents0 += usize::from(n.contents == 0);
            if first_bad.is_none() {
                first_bad = Some(format!(
                    "first bad node {i}: count {} contents {:#x} brushes {:?} at {at:?} slot {:?}",
                    n.leaf_brush_count,
                    n.contents,
                    &n.brushes[..n.brushes.len().min(12)],
                    slot(at, n.brushes.len())
                ));
            }
        }
        format!(
            "bad nodes {bad_nodes} (reachable {bad_reachable}, contents 0 {bad_contents0}) entries {bad_entries} max {max_bad}; {};              table leafbrushes len {} at {base:?} max {:?} past brushes {}; brush lists inside the table {inside} (same values {same}) outside {outside}",
            first_bad.unwrap_or_default(),
            table.len(),
            table.iter().max(),
            table.iter().filter(|&&b| usize::from(b) >= nb).count(),
        )
    }

    /// The refused row as captured, with the table sizes the check holds it to.
    fn refused_row(clip: &ClipRef, e: StoreError) -> String {
        let i = e.index;
        let sizes = format!(
            "nodes {} leafs {} lbnodes {} brushes {} aabb {} partitions {} materials {} tris {} verts {} walkable {} cmodels {}",
            clip.nodes.len(),
            clip.leaves.len(),
            clip.leafbrush_nodes.len(),
            clip.brushes.len(),
            clip.aabb_trees.len(),
            clip.partitions.len(),
            clip.materials.len(),
            clip.tri_indices.len() / 3,
            clip.verts.len(),
            clip.tri_edge_is_walkable.len(),
            clip.cmodels.len(),
        );
        let row = match e.table {
            "nodes" => format!("{:?}", clip.nodes.get(i).map(|n| n.children)),
            "leafs" => format!("{:?}", clip.leaves.get(i)),
            "cmodels" => format!(
                "{:?}",
                clip.cmodels.get(i).map(|m| (m.shares_map_info, m.leaf.clone()))
            ),
            "leaf_brush_nodes" => format!(
                "{:?}; with contents 0: {}",
                clip.leafbrush_nodes.get(i).map(|n| (
                    n.leaf_brush_count,
                    n.brushes.len(),
                    n.axis,
                    n.contents,
                    n.child_offset
                )),
                clip.leafbrush_nodes.iter().filter(|n| n.contents == 0).count()
            ),
            "leaf_brushes" => leaf_brushes_report(clip),
            "brushes" => format!(
                "{:?}",
                clip.brushes.get(i).map(|b| (b.sides.len(), b.side_cflags.len()))
            ),
            "aabb_trees" => format!(
                "{:?}",
                clip.aabb_trees.get(i).map(|t| (t.material_index, t.child_count, t.u))
            ),
            "partitions" => format!("{:?}", clip.partitions.get(i)),
            _ => String::new(),
        };
        format!("row {row}; {sizes}")
    }
}
