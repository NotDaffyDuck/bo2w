//! bo2mp: a Black Ops II clip map built from the sim's own brush list, for
//! a game whose loader gave none (the tests' hand-made rooms). One leaf
//! holds every brush but the windows; each brush model gets a leaf of its
//! own over its run of `leafbrushes`.

use clipmap_t6::{CModel, CPlane, ClipStore, Leaf, Node, StoredBrush, StoredLeafBrushNode};
use trace_t6::{BrushSide, Plane};

use crate::world::SimBrush;

/// How far a brush with no face on a side of an axis reaches that way.
const OPEN_SIDE: f32 = 262_144.0;

/// The sim's surface flags (`asset_world::t5_surface_flags`) back in BO2's
/// layout: the two mantle bits move up one.
#[must_use]
pub(crate) fn t6_surface_flags(flags: u32) -> u32 {
    let mantle_on = flags & 0x0200_0000 != 0;
    let mantle_over = flags & 0x0400_0000 != 0;
    (flags & !0x1e00_0000) | if mantle_on { 0x0400_0000 } else { 0 } | if mantle_over { 0x0800_0000 } else { 0 }
}

/// `brush` as BO2 keeps one: its axis-aligned faces as its box (with their
/// flags), every other face a side.
#[must_use]
pub(crate) fn t6_brush_parts(brush: &SimBrush) -> (StoredBrush, Vec<BrushSide>) {
    let mut out = StoredBrush {
        mins: [-OPEN_SIDE; 3],
        maxs: [OPEN_SIDE; 3],
        contents: brush.contents,
        axial_cflags: [[brush.contents; 3]; 2],
        ..StoredBrush::default()
    };
    let mut sides = Vec::new();
    for (i, p) in brush.planes.iter().enumerate() {
        let sflags = t6_surface_flags(brush.plane_surface_flags.get(i).copied().unwrap_or(0));
        let normal = [p[0], p[1], p[2]];
        let axis = (0..3).find(|&a| normal[a].abs() == 1.0 && (0..3).all(|b| b == a || normal[b] == 0.0));
        match axis {
            Some(a) if normal[a] > 0.0 => {
                if p[3] < out.maxs[a] || out.maxs[a] == OPEN_SIDE {
                    out.maxs[a] = p[3];
                    out.axial_sflags[1][a] = sflags;
                }
            }
            Some(a) => {
                if -p[3] > out.mins[a] || out.mins[a] == -OPEN_SIDE {
                    out.mins[a] = -p[3];
                    out.axial_sflags[0][a] = sflags;
                }
            }
            None => sides.push(BrushSide {
                plane: Plane { normal, dist: p[3] },
                cflags: brush.contents,
                sflags,
            }),
        }
    }
    out.side_count = sides.len() as u32;
    (out, sides)
}

/// Leaf brush nodes holding `ids`, chained when there are more than one
/// node holds: each link sends the walk to its run, then on.
fn push_brush_nodes(store: &mut ClipStore, ids: &[u16], contents: u32) -> i32 {
    let first = store.leaf_brush_nodes.len() as i32;
    let chunks: Vec<&[u16]> = ids.chunks(i16::MAX as usize).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        if i + 1 < chunks.len() {
            store.leaf_brush_nodes.push(StoredLeafBrushNode {
                axis: 0,
                leaf_brush_count: -1,
                contents,
                dist: -OPEN_SIDE * 4.0,
                child_offset: [2, 2],
                ..StoredLeafBrushNode::default()
            });
        }
        store.leaf_brush_nodes.push(StoredLeafBrushNode {
            leaf_brush_count: chunk.len() as i16,
            contents,
            first_brush: store.leaf_brushes.len() as u32,
            ..StoredLeafBrushNode::default()
        });
        store.leaf_brushes.extend_from_slice(chunk);
    }
    first
}

/// A leaf over brushes `ids` of `store`, bounded by them.
fn brush_leaf(store: &mut ClipStore, ids: &[u16]) -> Leaf {
    let ids: Vec<u16> = ids.iter().copied().filter(|&b| usize::from(b) < store.brushes.len()).collect();
    if ids.is_empty() {
        return Leaf::default();
    }
    let mut mins = [f32::MAX; 3];
    let mut maxs = [-f32::MAX; 3];
    let mut contents = 0;
    for &b in &ids {
        let brush = &store.brushes[usize::from(b)];
        contents |= brush.contents;
        for a in 0..3 {
            mins[a] = mins[a].min(brush.mins[a] - 1.0);
            maxs[a] = maxs[a].max(brush.maxs[a] + 1.0);
        }
    }
    let node = push_brush_nodes(store, &ids, contents);
    Leaf {
        brush_contents: contents,
        mins,
        maxs,
        leaf_brush_node: node,
        ..Leaf::default()
    }
}

/// The clip map for `brushes` (all of them, in order, so brush ids match),
/// with `cmodels` as its brush models over `leafbrushes`. Windows
/// (`glass_encoded` set) stay out of the world leaf: they are traced on
/// their own while their pane stands.
#[must_use]
pub(crate) fn clip_store(brushes: &[SimBrush], cmodels: &[asset_world::ClipCmodel], leafbrushes: &[u16]) -> ClipStore {
    let mut store = ClipStore::default();
    for b in brushes {
        let (mut stored, sides) = t6_brush_parts(b);
        stored.first_side = store.brush_sides.len() as u32;
        store.brush_sides.extend(sides);
        store.brushes.push(stored);
    }
    let world: Vec<u16> = (0..brushes.len().min(usize::from(u16::MAX)))
        .filter(|&i| brushes[i].glass_encoded == 0)
        .map(|i| i as u16)
        .collect();
    let mut leaf = brush_leaf(&mut store, &world);
    leaf.cluster = 0;
    store.leafs.push(leaf);
    // One split the whole map lies behind: every box and line reaches leaf 0.
    store.nodes.push(Node {
        plane: CPlane::new([1.0, 0.0, 0.0], OPEN_SIDE * 4.0),
        children: [-1, -1],
    });
    store.cmodels.push(CModel {
        mins: leaf.mins,
        maxs: leaf.maxs,
        leaf,
        ..CModel::default()
    });
    for m in cmodels.iter().skip(1) {
        let first = m.first_brush as usize;
        let ids: Vec<u16> = leafbrushes.get(first..first + usize::from(m.num_brushes)).unwrap_or(&[]).to_vec();
        let leaf = brush_leaf(&mut store, &ids);
        store.cmodels.push(CModel {
            mins: m.mins,
            maxs: m.maxs,
            radius: m.radius,
            leaf,
        });
    }
    store
}
