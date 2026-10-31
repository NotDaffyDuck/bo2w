//! bo2zm: Black Ops II clipMap_t -> CollisionClipData. Not part of the upstream engine.
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
    AabbTree, CModel, PlaneEquation, ClipMap, ClipMaterial, ClipRepository, Leaf, Node, Partition, StaticModelStore, StaticWorld,
    StoreError, StoredBrush, StoredCollSurf, StoredLeafBrushNode, StoredStaticModel, StoredXModel, ModelHitTriangle,
};
use trace_t6::{Brush, BrushSide, Plane};

use crate::clip_error::SolidsTraceFault;

use crate::clip_flags::{t5_axial_plane_flags, t5_surface_flags};
use crate::clip_leafbrush::{LEAFBRUSH_NODE_MAX_DEPTH, leafbrush_node_visits, leafbrush_range};
use crate::clip_tables::{
    CollisionSolid, ClipLeafRow, ClipNodeRow, CollisionSubmodel, CollisionClipData, ClipMaterialRow,
};

fn truncated() -> SolidsTraceFault {
    SolidsTraceFault::Truncated
}

/// Append the brush list under one leaf's brush node (0 = none) and return
/// its range in `listed`.
fn append_leaf_brushes(
    clip: &ClipRef,
    leaf: &ClipLeafRef,
    listed: &mut Vec<u16>,
) -> Result<(u32, u16), SolidsTraceFault> {
    let before = Vec::len(listed);
    if let Some(root) = usize::try_from(leaf.brush_leaf_link).ok().filter(|&root| root > 0) {
        gather_node_brushes(clip, root, 0, listed)?;
    }
    leafbrush_range(before, listed.len())
}

/// The brushes of the node at `at` and of every node its chain reaches,
/// last-listed branch first, `depth` nodes down from the leaf's own.
fn gather_node_brushes(
    clip: &ClipRef,
    at: usize,
    hops: usize,
    out: &mut Vec<u16>,
) -> Result<(), SolidsTraceFault> {
    (hops <= LEAFBRUSH_NODE_MAX_DEPTH).then_some(()).ok_or_else(truncated)?;
    let entry = clip.leafbrush_nodes.get(at).ok_or_else(truncated)?;
    if entry.leaf_solid_total > 0 {
        out.extend_from_slice(&entry.brushes);
        return Ok(());
    }
    let [jump_a, jump_b] = entry.child_offset.map(usize::from);
    leafbrush_node_visits(at, entry.leaf_solid_total, jump_a, jump_b)
        .into_iter()
        .rev()
        .try_for_each(|next| gather_node_brushes(clip, next, hops + 1, out))
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
    pub solid_model: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// Which of the map's windows this is (its index in the capture's).
    pub glass: usize,
}

fn dot_product(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(x, y)| x * y).fold(0.0, |sum, term| sum + term)
}

/// A window's frame: model space -> world is origin + x*axis[0] + y*axis[1] + z*axis[2].
struct PaneFrame {
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
}

impl PaneFrame {
    fn of(place: &T6GlassPlacement) -> Self {
        Self { origin: place.origin, axes: game_math::euler_to_rows(place.angles) }
    }

    /// `v` (model space) turned to the world's axes, no shift.
    fn turn(&self, v: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|k| self.axes.iter().zip(v).fold(0.0, |sum, (axis, part)| sum + part * axis[k]))
    }

    fn world_point(&self, pt: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|k| self.axes.iter().zip(pt).fold(self.origin[k], |sum, (axis, part)| sum + part * axis[k]))
    }

    /// The side plane n.x <= d in model space is n'.w <= d + n'.origin in the
    /// world, n' the normal turned by the axis.
    fn world_plane(&self, side: [f32; 4]) -> [f32; 4] {
        let [facing_x, facing_y, facing_z, d] = side;
        let turned = self.turn([facing_x, facing_y, facing_z]);
        let offset = turned.iter().zip(self.origin).fold(d, |sum, (t, o)| sum + t * o);
        [turned[0], turned[1], turned[2], offset]
    }

    /// The world box around a model box's eight corners.
    fn level_bounds(&self, (box_lo, box_hi): ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
        (0..8u32)
            .map(|corner| self.world_point(std::array::from_fn(|k| if corner >> k & 1 == 1 { box_hi[k] } else { box_lo[k] })))
            .fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), pt| {
                (std::array::from_fn(|k| lo[k].min(pt[k])), std::array::from_fn(|k| hi[k].max(pt[k])))
            })
    }
}

/// Move a window brush into the world and tag it; the world box it now fills.
fn plant_glass_brush(brush: &mut CollisionSolid, frame: &PaneFrame, tag: u16) -> ([f32; 3], [f32; 3]) {
    brush.glass_hit_code = tag;
    let p = &brush.planes;
    let model_box = ([-p[1][3], -p[3][3], -p[5][3]], [p[0][3], p[2][3], p[4][3]]);
    brush.planes.iter_mut().for_each(|plane| *plane = frame.world_plane(*plane));
    frame.level_bounds(model_box)
}

/// Every leaf slot the box `lo..hi` reaches, in the order a depth-first walk
/// of the tree (second child pushed last, visited first) meets them. A map
/// with no tree is one leaf.
fn leaf_slots_reached(nodes: &[ClipNodeRow], lo: [f32; 3], hi: [f32; 3]) -> Result<Vec<usize>, SolidsTraceFault> {
    let Some(_) = nodes.first() else {
        return Ok(vec![0]);
    };
    let centre: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
    let extent: [f32; 3] = std::array::from_fn(|k| (hi[k] - lo[k]) * 0.5 + 0.125);
    let (mut pending, mut slots, mut budget) = (vec![0i32], Vec::new(), 1usize << 20);
    while let Some(cursor) = pending.pop() {
        budget = budget.checked_sub(1).ok_or_else(truncated)?;
        if cursor < 0 {
            slots.push((!cursor) as usize);
            continue;
        }
        let Some(split) = nodes.get(cursor as usize) else { continue };
        let normal = [split.plane[0], split.plane[1], split.plane[2]];
        let dist = dot_product(normal, centre) - split.plane[3];
        let reach = dot_product(extent, normal.map(f32::abs));
        let reaches = [dist + reach >= 0.0, dist - reach < 0.0];
        pending.extend(split.children.iter().zip(reaches).filter(|&(_, yes)| yes).map(|(&child, _)| child));
    }
    Ok(slots)
}

/// The brush lists `ranges` name in `table`, one list per range.
fn lists_in(table: &[u16], ranges: impl Iterator<Item = (u32, u16)>) -> Vec<Vec<u16>> {
    ranges
        .map(|(start, count)| {
            let from = start as usize;
            table.get(from..from + usize::from(count)).unwrap_or_default().to_vec()
        })
        .collect()
}

/// Lay `lists` end to end in `merged`, writing each one's range into its slot.
fn pack_lists<'s>(
    lists: &[Vec<u16>],
    slots: impl Iterator<Item = (&'s mut u32, &'s mut u16)>,
    merged: &mut Vec<u16>,
) -> Result<(), SolidsTraceFault> {
    for (list, (first, count)) in lists.iter().zip(slots) {
        let before = merged.len();
        merged.extend_from_slice(list);
        (*first, *count) = leafbrush_range(before, merged.len())?;
    }
    Ok(())
}

/// codmp: Black Ops II's windows stand until they break. Each window's
/// brushes belong to its brush model (`*N`), which no world leaf lists, so
/// nothing was stopped by them. Move each brush to where its window stands
/// (`placements[pane]`), tag it with its pane (`glass_hit_code` = pane + 1,
/// what a trace asks the glass state about) and list it in every world leaf
/// its box reaches.
pub fn attach_t6_glass_brushes(
    out: &mut CollisionClipData,
    placements: &[T6GlassPlacement],
) -> Result<T6GlassClipCensus, SolidsTraceFault> {
    let mut census = T6GlassClipCensus::default();
    let mut leaf_lists = lists_in(&out.brush_leaf_indices, out.leaves.iter().map(|lf| (lf.initial_convex_solid, lf.solid_volume_total)));
    let model_lists = lists_in(&out.brush_leaf_indices, out.cmodels.iter().map(|cm| (cm.initial_convex_solid, cm.solid_volume_total)));
    // Panes are tagged 1, 2, ..; the tag is a u16, so the 65536th window ends the walk.
    for (tag, place) in (1..=u16::MAX).zip(placements) {
        let members = match place.solid_model {
            0 => None,
            model => model_lists.get(model as usize),
        };
        let Some(members) = members else {
            census.missing_models += 1;
            continue;
        };
        census.panes += 1;
        let frame = PaneFrame::of(place);
        for &brush_id in members {
            let Some(solid) = out.brushes.get_mut(usize::from(brush_id)).filter(|b| b.planes.len() >= 6) else {
                continue;
            };
            if solid.glass_hit_code != 0 {
                census.shared_brushes += 1;
                continue;
            }
            if census.brushes == 0 {
                census.contents = solid.contents;
            }
            census.brushes += 1;
            let (lo, hi) = plant_glass_brush(solid, &frame, tag);
            for slot in leaf_slots_reached(&out.nodes, lo, hi)? {
                let Some(list) = leaf_lists.get_mut(slot).filter(|l| !l.contains(&brush_id)) else {
                    continue;
                };
                list.push(brush_id);
                census.leaf_links += 1;
            }
        }
    }
    if census.brushes > 0 {
        let mut merged = Vec::with_capacity(out.brush_leaf_indices.len() + census.leaf_links);
        pack_lists(&leaf_lists, out.leaves.iter_mut().map(|lf| (&mut lf.initial_convex_solid, &mut lf.solid_volume_total)), &mut merged)?;
        pack_lists(&model_lists, out.cmodels.iter_mut().map(|cm| (&mut cm.initial_convex_solid, &mut cm.solid_volume_total)), &mut merged)?;
        out.brush_leaf_indices = merged;
    }
    Ok(census)
}

/// The flags of the material named "caulk" (a map without one has none to skip).
fn caulk_surface_flags(clip: &ClipRef) -> u32 {
    let named_caulk = clip.materials.iter().find(|mat| mat.name == "caulk");
    named_caulk.map_or(u32::MAX, |mat| mat.surface_flags as u32)
}

pub fn build_t6_clip_collision(clip: &ClipRef) -> Result<CollisionClipData, SolidsTraceFault> {
    let has_geometry = [clip.brushes.len(), clip.tri_index_list.len()].iter().any(|&n| n > 0);
    has_geometry.then_some(()).ok_or(SolidsTraceFault::NoBrushTables)?;
    // bo2mp: BO2's player collision skips caulk faces when it picks the face
    // a brush hit reports.
    let caulk = caulk_surface_flags(clip);
    let mut brush_leaf_indices = Vec::new();
    let leaves = clip
        .leaves
        .iter()
        .map(|leaf| {
            let (start, count) = append_leaf_brushes(clip, leaf, &mut brush_leaf_indices)?;
            Ok(ClipLeafRow {
                initial_convex_solid: start,
                solid_volume_total: count,
                first_collision_box_slot: leaf.first_collision_box_slot,
                collision_box_total: leaf.collision_box_total,
            })
        })
        .collect::<Result<Vec<_>, SolidsTraceFault>>()?;
    let (cmodels, cmodel_mesh_roots): (Vec<_>, Vec<_>) = clip
        .sub_models
        .iter()
        .map(|model| {
            let (start, count) = if model.shares_map_info {
                append_leaf_brushes(clip, &model.leaf, &mut brush_leaf_indices)?
            } else {
                leafbrush_range(brush_leaf_indices.len(), brush_leaf_indices.len())?
            };
            let collision_geometry = CollisionSubmodel {
                initial_convex_solid: start,
                solid_volume_total: count,
                radius: model.radius,
                lo: model.lo_corner, hi: model.hi_corner,
            };
            Ok((collision_geometry, (model.leaf.first_collision_box_slot, model.leaf.collision_box_total)))
        })
        .collect::<Result<Vec<_>, SolidsTraceFault>>()?
        .into_iter()
        .unzip();
    Ok(CollisionClipData {
        brushes: clip.brushes.iter().map(|brush| t6_brush(brush, caulk)).collect(),
        materials: clip
            .materials
            .iter()
            .map(|mat| ClipMaterialRow {
                pack_presence_bits: mat.pack_presence_bits as u32,
                face_bits: t5_surface_flags(mat.surface_flags as u32),
                name: mat.name.clone(),
            })
            .collect(),
        nodes: clip.nodes.iter().map(|node| ClipNodeRow { children: node.children.map(i32::from), plane: node.plane }).collect(),
        leaves,
        brush_leaf_indices,
        cmodels,
        cmodel_mesh_roots,
        // bo2mp: the clip map BO2's traces read comes from the same tables; a
        // map whose tables point outside themselves does not load.
        t6: Some(t6_clip_map(clip).map_err(SolidsTraceFault::Store)?),
        ..Default::default()
    })
}

/// bo2mp: `cLeaf_s` as BO2's traces read it.
fn t6_leaf(src: &ClipLeafRef) -> Leaf {
    let mut leaf = Leaf::default();
    leaf.cluster = src.cluster;
    leaf.brush_leaf_link = src.brush_leaf_link;
    (leaf.mins, leaf.maxs) = (src.lo_corner, src.hi_corner);
    (leaf.terrain_contents, leaf.solid_content_flags) = (src.ground_mask as u32, src.solid_content_flags as u32);
    (leaf.first_coll_aabb, leaf.collision_box_total) = (src.first_collision_box_slot, src.collision_box_total);
    leaf
}

fn as_u32_sides(rows: [[i32; 3]; 2]) -> [[u32; 3]; 2] {
    rows.map(|row| row.map(|v| v as u32))
}

/// The pooled brush ids and the nodes over them, each node pointing at where its
/// own brushes start in the pool.
fn t6_leaf_brush_tables(clip: &ClipRef) -> Result<(Vec<u16>, Vec<StoredLeafBrushNode>), StoreError> {
    let mut pool: Vec<u16> = Vec::new();
    let mut nodes = Vec::with_capacity(clip.leafbrush_nodes.len());
    for (index, node) in clip.leafbrush_nodes.iter().enumerate() {
        let pool_start = pool.len() as u32;
        if node.leaf_solid_total > 0 {
            if node.brushes.len() != node.leaf_solid_total as usize {
                return Err(StoreError { table: "leaf_brush_nodes", index });
            }
            pool.extend(node.brushes.iter().copied());
        }
        nodes.push(StoredLeafBrushNode {
            initial_convex_solid: pool_start,
            contents: node.contents as u32,
            leaf_solid_total: node.leaf_solid_total,
            child_offset: node.child_offset, range: node.range,
            dist: node.plane_dist, axis: node.axis,
        });
    }
    Ok((pool, nodes))
}

/// The brushes and the pooled sides they index.
fn t6_brush_tables(clip: &ClipRef) -> Result<(Vec<StoredBrush>, Vec<BrushSide>), StoreError> {
    t6_brush_rows(&clip.brushes)
}

/// `brushes` as `cbrush_t` rows, their sides in one table.
fn t6_brush_rows(brushes: &[ClipBrushRef]) -> Result<(Vec<StoredBrush>, Vec<BrushSide>), StoreError> {
    let mut sides: Vec<BrushSide> = Vec::new();
    let brushes = brushes
        .iter()
        .enumerate()
        .map(|(index, b)| {
            if b.side_cflags.len() != b.sides.len() {
                return Err(StoreError { table: "brushes", index });
            }
            let first_side = sides.len() as u32;
            for (&(normal, dist, sflags), &cflags) in b.sides.iter().zip(&b.side_cflags) {
                let plane = Plane { normal, plane_offset: dist };
                sides.push(BrushSide { plane, cflags: cflags as u32, sflags: sflags as u32 });
            }
            Ok(StoredBrush {
                first_side,
                side_count: b.sides.len() as u32,
                mins: b.lo_corner,
                maxs: b.hi_corner,
                contents: b.contents as u32,
                axial_cflags: as_u32_sides(b.axial_cflags),
                axial_sflags: as_u32_sides(b.axial_sflags),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((brushes, sides))
}

/// bo2mp: `clip`'s tables as BO2's traces read them, with BO2's own surface
/// flags (no T5 translation). A brush model that does not share the map's
/// tables gets no brushes, as `build_t6_clip_collision` gives it.
pub fn t6_clip_store(clip: &ClipRef) -> Result<ClipRepository, StoreError> {
    let (leaf_brushes, leaf_brush_nodes) = t6_leaf_brush_tables(clip)?;
    let (brush_rows, side_rows) = t6_brush_tables(clip)?;
    let node = |n: &asset_t6::ClipNodeRef| Node {
        plane: PlaneEquation {
            normal: [n.plane[0], n.plane[1], n.plane[2]],
            dist: n.plane[3],
            kind: n.plane_type,
            signbits: n.plane_signbits,
        },
        children: n.children };
    let collision_geometry = |m: &asset_t6::ClipCmodelRef| {
        let mut leaf = t6_leaf(&m.leaf);
        if !m.shares_map_info {
            leaf = Leaf { solid_content_flags: 0, brush_leaf_link: 0, ..leaf };
        }
        CModel { mins: m.lo_corner, maxs: m.hi_corner, radius: m.radius, leaf }
    };
    Ok(ClipRepository {
        nodes: clip.nodes.iter().map(node).collect(),
        leafs: clip.leaves.iter().map(t6_leaf).collect(),
        materials: clip
            .materials
            .iter()
            .map(|m| ClipMaterial { surface_flags: m.surface_flags as u32, pack_presence_bits: m.pack_presence_bits as u32 })
            .collect(),
        aabb_trees: clip
            .box_trees
            .iter()
            .map(|t| AabbTree {
                u: t.u as u32,
                origin: t.origin,
                half_size: t.half_extent,
                sub_element_total: t.sub_element_total,
                material_index: t.material_slot,
            })
            .collect(),
        partitions: clip.partitions.iter().map(|&(tri_tally, leading_triangle)| Partition { tri_tally, leading_triangle }).collect(),
        vertex_points: clip.vertex_points.clone(),
        tri_indices: clip.tri_index_list.chunks_exact(3).filter_map(|t| <[u16; 3]>::try_from(t).ok()).collect(),
        tri_edge_is_walkable: clip.tri_edge_is_walkable.clone(),
        cmodels: clip.sub_models.iter().map(collision_geometry).collect(),
        box_brush_node: u32::try_from(clip.box_leaf_brush_node).ok().filter(|&i| i > 0),
        leaf_brushes,
        leaf_brush_nodes,
        brushes: brush_rows,
        brush_sides: side_rows,
        ..ClipRepository::default()
    })
}

/// The value kept under `name` in `cache`, built (and filed) by `build` the
/// first time that name is asked for.
fn kept_under<T: Copy>(
    cache: &std::sync::Mutex<Vec<(String, T)>>,
    name: &str,
    build: impl FnOnce() -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let mut filed = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(value) = filed.iter().find_map(|(kept, value)| (kept == name).then_some(value)) {
        return Ok(*value);
    }
    let value = build()?;
    filed.push((name.to_owned(), value));
    Ok(value)
}

/// bo2mp: the BO2 clip map of `clip`, built and checked the first time its
/// map name is asked for and kept for the rest of the run.
pub fn t6_clip_map(clip: &ClipRef) -> Result<&'static ClipMap<'static>, StoreError> {
    static KEPT: std::sync::Mutex<Vec<(String, &'static ClipMap<'static>)>> = std::sync::Mutex::new(Vec::new());
    kept_under(&KEPT, &clip.name, || {
        let store = t6_clip_store(clip)?;
        store.check()?;
        Ok(store.leak())
    })
}

/// bo2mp: the placed models of `clip` (`staticModelList`: cars, fences,
/// trees) with the collision of the models they place (`xmodels`, the
/// zone's models), checked, filed over the map's world model the first
/// time its map name is asked for and kept for the rest of the run, as
/// `out.t6_static`. Returns how many models are placed.
pub fn attach_t6_static_world(
    out: &mut CollisionClipData,
    clip: &ClipRef,
    xmodels: &[XModelRef],
) -> Result<usize, StoreError> {
    let world = t6_static_world(clip, |i| {
        let key = clip.prop_set[i].model?;
        xmodels
            .get(key.index)
            .map(|m| (key.index, i32::from(m.collision_detail_tier), m.collision_faces.as_slice()))
    })?;
    Ok(out.t6_static.insert(world).models.len())
}

/// The kept static world of `clip`, each placement's model from `coll` by
/// placement index (xmodel index, collision LOD, surfaces).
fn t6_static_world<'a>(
    clip: &ClipRef,
    coll: impl Fn(usize) -> Option<(usize, i32, &'a [XModelCollSurfRef])>,
) -> Result<&'static StaticWorld, StoreError> {
    static KEPT: std::sync::Mutex<Vec<(String, &'static StaticWorld)>> = std::sync::Mutex::new(Vec::new());
    kept_under(&KEPT, &clip.name, || {
        let store = t6_static_store(clip, coll);
        store.check()?;
        Ok(store.leak(t6_clip_map(clip)?))
    })
}

/// File one model's collision surfaces and their triangles; the model's index.
fn push_xmodel(store: &mut StaticModelStore, collision_detail_tier: i32, surfs: &[XModelCollSurfRef]) -> u32 {
    let index = store.xmodels.len() as u32;
    let lead_skin = store.surfs.len() as u32;
    for s in surfs {
        let leading_triangle = store.triangle_list.len() as u32;
        let triangle_list = s.triangle_list.iter().map(|&[plane, surface_tangent, surface_bitangent]| ModelHitTriangle { plane, surface_tangent, surface_bitangent });
        store.triangle_list.extend(triangle_list);
        store.surfs.push(StoredCollSurf {
            leading_triangle,
            tri_tally: s.triangle_list.len() as u32,
            mins: s.lo_corner,
            maxs: s.hi_corner,
            joint_position: s.joint_position,
            contents: s.contents,
            face_property_bits: s.face_property_bits,
        });
    }
    store.xmodels.push(StoredXModel { collision_detail_tier, lead_skin, facet_amount: surfs.len() as u32 });
    index
}

/// `clip`'s static models as a store, each placed model's collision added
/// once, the first time a placement names it.
fn t6_static_store<'a>(
    clip: &ClipRef,
    coll: impl Fn(usize) -> Option<(usize, i32, &'a [XModelCollSurfRef])>,
) -> StaticModelStore {
    let mut store = StaticModelStore::default();
    let mut filed_at = std::collections::HashMap::new();
    for (index, sm) in clip.prop_set.iter().enumerate() {
        let xmodel = coll(index).map(|(key, collision_detail_tier, surfs)| {
            *filed_at.entry(key).or_insert_with(|| push_xmodel(&mut store, collision_detail_tier, surfs))
        });
        store.models.push(StoredStaticModel {
            xmodel,
            contents: sm.contents,
            origin: sm.origin,
            reverse_scaled_basis: sm.reverse_scaled_basis,
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
fn t6_brush(b: &ClipBrushRef, caulk: u32) -> CollisionSolid {
    // Per axis: the +1 face at the box's top, then the -1 face at its bottom.
    let face = |axis: usize, sign: f32, bound: f32| {
        let mut plane = [0.0f32; 4];
        plane[axis] = sign;
        plane[3] = bound;
        plane
    };
    let mut planes: Vec<[f32; 4]> = (0..3)
        .flat_map(|axis| [face(axis, 1.0, b.hi_corner[axis]), face(axis, -1.0, -b.lo_corner[axis])])
        .collect();
    planes.extend(b.sides.iter().map(|&([facing_x, facing_y, facing_z], dist, _)| [facing_x, facing_y, facing_z, dist]));

    let axial_flags: [u32; 6] = std::array::from_fn(|i| t5_surface_flags(b.axial_sflags[i / 3][i % 3] as u32));
    let mut side_flags = t5_axial_plane_flags(axial_flags).to_vec();
    side_flags.extend(b.sides.iter().map(|&(_, _, sflags)| t5_surface_flags(sflags as u32)));

    let hit_surface_flags = movement_t6::gjk::brush_surface_flags(
        as_u32_sides(b.axial_sflags),
        b.sides.iter().map(|&(normal, _, sflags)| (normal[2], sflags as u32)),
        caulk,
    );
    CollisionSolid {
        hit_surface_flags: t5_surface_flags(hit_surface_flags),
        vertex_points: b.vertex_points.clone(),
        glass_hit_code: 0u16,
        plane_material_flags: side_flags,
        contents: b.contents as u32,
        planes,
    }
}

/// bo2mp: the collision-map brushes of xmodel `name` as BO2's traces read
/// them, built the first time that name is asked for and kept for the rest
/// of the run. `SV_ClipHandleForEntity` hands an entity whose model has a
/// collmap to `CM_TempBrushModel`, which traces these as the temporary
/// brush model (handle 0x4000).
fn t6_collmap_brushes(name: &str, brushes: &[ClipBrushRef]) -> Result<&'static [Brush<'static>], StoreError> {
    static KEPT: std::sync::Mutex<Vec<(String, &'static [Brush<'static>])>> = std::sync::Mutex::new(Vec::new());
    kept_under(&KEPT, name, || {
        let (rows, sides) = t6_brush_rows(brushes)?;
        let sides: &'static [BrushSide] = Box::leak(sides.into_boxed_slice());
        let views = rows.iter().map(|b| {
            let first = b.first_side as usize;
            Brush {
                lower: b.mins,
                upper: b.maxs,
                contents: b.contents,
                sides: &sides[first..first + b.side_count as usize],
                axial_cflags: b.axial_cflags,
                axial_sflags: b.axial_sflags,
            }
        });
        Ok(&*Box::leak(views.collect::<Vec<_>>().into_boxed_slice()))
    })
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
    out: &mut CollisionClipData,
    models: impl IntoIterator<Item = (&'a str, &'a [ClipBrushRef])>,
) -> T6ModelClipCensus {
    let mut census = T6ModelClipCensus::default();
    let mut added: usize = 0;
    for (name, brushes) in models {
        if brushes.is_empty() || out.model_cmodels.contains_key(name) {
            continue;
        }
        let initial_convex_solid = out.brushes.len();
        let room = initial_convex_solid + brushes.len() <= usize::from(u16::MAX) && u32::try_from(out.cmodels.len()).is_ok();
        if !room {
            census.skipped += 1;
            continue;
        }
        let first_listed = out.brush_leaf_indices.len();
        let (mins, maxs) = brushes.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), brush| {
            (
                std::array::from_fn(|axis| lo[axis].min(brush.lo_corner[axis])),
                std::array::from_fn(|axis| hi[axis].max(brush.hi_corner[axis])),
            )
        });
        out.brushes.extend(brushes.iter().map(|brush| t6_brush(brush, u32::MAX)));
        out.brush_leaf_indices.extend((initial_convex_solid..initial_convex_solid + brushes.len()).map(|id| id as u16));
        let radius = mins.iter().zip(&maxs).map(|(lo, hi)| lo.abs().max(hi.abs()).powi(2)).sum::<f32>().sqrt();
        let handle = out.cmodels.len() as u32;
        out.model_cmodels.insert(name.to_owned(), handle);
        // A model whose brushes BO2 could not read has none to trace, as it has no collmap.
        if let Ok(collmap) = t6_collmap_brushes(name, brushes) {
            out.t6_model_brushes.insert(handle, collmap);
        }
        let placed = CollisionSubmodel { solid_volume_total: brushes.len() as u16, initial_convex_solid: first_listed as u32, radius, lo: mins, hi: maxs };
        out.cmodels.push(placed);
        census.brushes += brushes.len();
        added += 1;
    }
    census.models = added;
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
            solid_content_flags: 1,
            brush_leaf_link: 1,
            lo_corner: [-1000.0; 3],
            hi_corner: [1000.0; 3],
            ..ClipLeafRef::default()
        };
        ClipRef {
            name: "clip_t6 test floor".to_string(),
            brushes: vec![ClipBrushRef {
                vertex_points: vec![],
                side_cflags: vec![],
                sides: vec![],
                axial_sflags: [[0; 3]; 2],
                axial_cflags: [[1; 3]; 2],
                contents: 1,
                lo_corner: [-64.0, -64.0, -16.0],
                hi_corner: [64.0, 64.0, 0.0],
            }],
            materials: vec![ClipMaterialRef {
                name: "floor".to_string(),
                surface_flags: Default::default(),
                pack_presence_bits: 1,
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
                    leaf_solid_total: 0,
                    brushes: Vec::new(),
                    child_offset: [0; 2],
                    axis: 0,
                    contents: Default::default(),
                    plane_dist: 0.0,
                    range: 0.0,
                },
                ClipLeafBrushNodeRef {
                    leaf_solid_total: 1,
                    brushes: vec![0],
                    child_offset: [0; 2],
                    axis: 0,
                    contents: 1,
                    plane_dist: 0.0,
                    range: 0.0,
                },
            ],
            sub_models: vec![ClipCmodelRef {
                lo_corner: [-64.0, -64.0, -16.0],
                hi_corner: [64.0, 64.0, 0.0],
                radius: 100.0,
                leaf,
                shares_map_info: true,
            }],
            ..ClipRef::default()
        }
    }

    /// `sweep_box` and BO2's trace on the map's own store agree on fraction,
    /// normal and start solid.
    fn agree(start: [f32; 3], stop: [f32; 3], lo: [f32; 3], hi: [f32; 3]) {
        let clip = floor_clip();
        let old = build_t6_clip_collision(&clip).expect("old build").sweep_box(start, stop, lo, hi, 1);
        let store = t6_clip_store(&clip).expect("store");
        store.check().expect("check");
        let mut marks = vec![0u16; store.partitions.len()];
        let new = store.with_map(|map| map.box_trace(&mut CheckCount::new(&mut marks), start, stop, lo, hi, 0, 1));
        assert!(
            (old.fraction - new.fraction).abs() < 1e-3,
            "{start:?} -> {stop:?}: old {} new {}",
            old.fraction,
            new.fraction
        );
        assert_eq!(old.began_inside, new.starts_inside, "{start:?} -> {stop:?}");
        if new.fraction < 1.0 {
            assert_eq!(old.normal, new.normal, "{start:?} -> {stop:?}");
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
            leaf_solid_total: 1,
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
        bad.leaves[0].brush_leaf_link = 2;
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
            triangle_list: vec![[[-1.0, 0.0, 0.0, 0.0], [0.0, 0.1, 0.0, 0.0], [0.0, 0.0, 0.1, 0.0]]],
            lo_corner: [-1.0; 3],
            hi_corner: [10.0; 3],
            joint_position: 0,
            contents: 1,
            face_property_bits: 0x50,
        }];
        let placed = |x: f32| ClipStaticModelRef {
            model: Option::default(),
            contents: 1,
            origin: [x, 0.0, -8.0],
            reverse_scaled_basis: clipmap_t6::reverse_scaled_basis([0.0; 3], 1.0),
            absmin: [x - 4.0, -4.0, -12.0],
            absmax: [x + 4.0, 4.0, -4.0],
        };
        let mut clip = floor_clip();
        clip.name = "clip_t6 test static models".into();
        clip.prop_set = vec![placed(40.0), placed(0.0), placed(-40.0)];
        // Placements 0 and 2 place xmodel 3; placement 1 places none.
        let coll = |i: usize| (i != 1).then_some((3, 0, &wall[..]));

        let store = t6_static_store(&clip, coll);
        assert_eq!((store.models.len(), store.xmodels.len(), store.surfs.len(), store.triangle_list.len()), (3, 1, 1, 1));
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
        let check_map = |map: &str| -> Result<(), String> {
            let path = std::path::Path::new(&dir).join(format!("{map}.ff"));
            let capture = asset_t6::capture_zone(&path).unwrap_or_else(|e| panic!("{map}: {e}"));
            let clip = capture.clip.as_ref().unwrap_or_else(|| panic!("{map}: no clipMap"));
            let store = t6_clip_store(clip).map_err(|e| format!("{map}: {e}; {}", refused_row(clip, e)))?;
            store.check().map_err(|e| {
                let bad_rows = every_bad(&store);
                format!("{map}: {e}; {}; every bad row: {bad_rows}", refused_row(clip, e))
            })?;
            let mut out = CollisionClipData::default();
            attach_t6_static_world(&mut out, clip, &capture.xmodels)
                .map(drop)
                .map_err(|e| format!("{map} static world: {e}"))
        };
        let problems: Vec<String> = ["mp_nuketown_2020", "mp_la"].into_iter().filter_map(|map| check_map(map).err()).collect();
        assert_eq!(problems, Vec::<String>::new());
    }

    /// How many rows of each table break `ClipRepository::check`'s rule, and the
    /// first of each, so one run shows every table that is read wrong.
    fn every_bad(s: &ClipRepository) -> String {
        let mut out = Vec::new();
        let mut note = |table: &str, bad: Vec<usize>| {
            if let Some(&first) = bad.first() {
                out.push(format!("{table} {} (first {first})", bad.len()));
            }
        };
        let rows = |ok: &dyn Fn(usize) -> bool, len: usize| (0..len).filter(|&i| !ok(i)).collect::<Vec<_>>();
        let leaf_ok = |l: &Leaf| {
            usize::from(l.first_coll_aabb) + usize::from(l.collision_box_total) <= s.aabb_trees.len()
                && l.brush_leaf_link >= 0
                && ((l.brush_leaf_link as usize) < s.leaf_brush_nodes.len()
                    || (l.brush_leaf_link == 0 && l.solid_content_flags == 0))
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
                    if n.leaf_solid_total > 0 {
                        n.initial_convex_solid as usize + n.leaf_solid_total as usize <= s.leaf_brushes.len()
                    } else {
                        n.axis < 3
                            && n.child_offset.iter().all(|&o| i + usize::from(o) < lbn)
                            && (n.leaf_solid_total == 0 || i + 1 < lbn)
                    }
                },
                lbn,
            ),
        );
        let box_run = s
            .box_brush_node
            .and_then(|i| s.leaf_brush_nodes.get(i as usize))
            .map(|n| n.initial_convex_solid as usize..n.initial_convex_solid as usize + n.leaf_solid_total.max(0) as usize);
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
                        && if t.sub_element_total > 0 {
                            t.u as usize + usize::from(t.sub_element_total) <= s.aabb_trees.len()
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
                    p.leading_triangle >= 0 && p.leading_triangle as usize + usize::from(p.tri_tally) <= s.tri_indices.len()
                },
                s.partitions.len(),
            ),
        );
        note(
            "tri_indices",
            rows(&|i| s.tri_indices[i].iter().all(|&v| usize::from(v) < s.vertex_points.len()), s.tri_indices.len()),
        );
        if s.tri_edge_is_walkable.len() * 8 < s.tri_indices.len() * 3 {
            note("tri_edge_is_walkable", vec![s.tri_edge_is_walkable.len()]);
        }
        out.join(", ")
    }

    /// Where the out-of-range leaf brushes come from: the nodes holding them,
    /// and how their lists sit against BO2's own `brush_leaf_indices` table.
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
            .map(|l| l.brush_leaf_link)
            .chain(clip.sub_models.iter().map(|m| m.leaf.brush_leaf_link))
            .filter(|&n| n > 0)
            .map(|n| n as usize)
            .collect();
        while let Some(i) = pending.pop() {
            let Some(n) = clip.leafbrush_nodes.get(i) else { continue };
            if std::mem::replace(&mut reachable[i], true) {
                continue;
            }
            if n.leaf_solid_total <= 0 {
                if n.leaf_solid_total < 0 {
                    pending.push(i + 1);
                }
                pending.extend(n.child_offset.iter().filter(|&&o| o != 0).map(|&o| i + usize::from(o)));
            }
        }
        let mut bad_reachable = 0;
        let mut bad_contents0 = 0;
        for (i, n) in clip.leafbrush_nodes.iter().enumerate() {
            if n.leaf_solid_total <= 0 {
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
                    n.leaf_solid_total,
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
            clip.nodes.len(), clip.leaves.len(), clip.leafbrush_nodes.len(),
            clip.brushes.len(), clip.box_trees.len(), clip.partitions.len(),
            clip.materials.len(), clip.tri_index_list.len() / 3, clip.vertex_points.len(),
            clip.tri_edge_is_walkable.len(), clip.sub_models.len(),
        );
        let row = match e.table {
            "nodes" => format!("{:?}", clip.nodes.get(i).map(|n| n.children)),
            "leafs" => format!("{:?}", clip.leaves.get(i)),
            "cmodels" => format!(
                "{:?}",
                clip.sub_models.get(i).map(|m| (m.shares_map_info, m.leaf.clone()))
            ),
            "leaf_brush_nodes" => format!(
                "{:?}; with contents 0: {}",
                clip.leafbrush_nodes.get(i).map(|n| (
                    n.leaf_solid_total,
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
                clip.box_trees.get(i).map(|t| (t.material_slot, t.sub_element_total, t.u))
            ),
            "partitions" => format!("{:?}", clip.partitions.get(i)),
            _ => String::new(),
        };
        format!("row {row}; {sizes}")
    }
}
