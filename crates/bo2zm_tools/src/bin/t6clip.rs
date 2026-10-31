//! bo2zm: measure a Black Ops II map's collision, read-only.
//! `t6clip [zone.ff] [x y z ...]` (default Nuketown, at its player start)
//! counts brush contents bits, then lists every brush whose player-sized hull
//! holds each point: the brushes a player standing there would be inside.

use std::collections::BTreeMap;
use std::path::PathBuf;

use asset_t6::{ClipBrushRef, ClipRef, capture_zone};

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("zm_nuked.ff")
}

/// the legacy standing player box.
const BODY_HALF: f32 = 15.0;
const BODY_BOX_LOWER: [f32; 3] = [-BODY_HALF, -BODY_HALF, 0.0];
const BODY_BOX_UPPER: [f32; 3] = [BODY_HALF, BODY_HALF, 70.0];

/// Every plane of a brush: the six box planes, then the extra sides.
fn planes(b: &ClipBrushRef) -> Vec<[f32; 4]> {
    let mut p = vec![
        [1.0, 0.0, 0.0, b.hi_corner[0]],
        [-1.0, 0.0, 0.0, -b.lo_corner[0]],
        [0.0, 1.0, 0.0, b.hi_corner[1]],
        [0.0, -1.0, 0.0, -b.lo_corner[1]],
        [0.0, 0.0, 1.0, b.hi_corner[2]],
        [0.0, 0.0, -1.0, -b.lo_corner[2]],
    ];
    p.extend(b.sides.iter().map(|&(n, d, _)| [n[0], n[1], n[2], d]));
    p
}

/// Whether the box at `origin` overlaps the brush: every plane pushed out by
/// the box's reach along its normal (its nearest corner, so a box above a
/// brush's top is not counted) must still have the origin behind it.
fn box_inside(b: &ClipBrushRef, spot: [f32; 3]) -> bool {
    planes(b).iter().all(|p| {
        let reach: f32 = -(0..3)
            .map(|i| {
                if p[i] < 0.0 {
                    p[i] * BODY_BOX_UPPER[i]
                } else {
                    p[i] * BODY_BOX_LOWER[i]
                }
            })
            .sum::<f32>();
        p[0] * spot[0] + p[1] * spot[1] + p[2] * spot[2] - (p[3] + reach) < 0.0
    })
}

/// The brushes listed under one leaf brush node (0 = none), by the same
/// visit rule the engine uses: brush lists at positive counts, else the
/// node after a negative count and each nonzero child offset.
fn leaf_brushes(clip: &ClipRef, root: i32) -> Vec<u16> {
    let mut out = Vec::new();
    if root <= 0 {
        return out;
    }
    let mut pending = vec![root as usize];
    while let Some(index) = pending.pop() {
        let Some(node) = clip.leafbrush_nodes.get(index) else {
            continue;
        };
        if node.leaf_solid_total > 0 {
            out.extend_from_slice(&node.brushes);
            continue;
        }
        if node.leaf_solid_total < 0 {
            pending.push(index + 1);
        }
        for off in node.child_offset {
            if off != 0 {
                pending.push(index + usize::from(off));
            }
        }
    }
    out
}

/// A run that already said why on its own.
struct Failed;

/// How many of `contents` have each single bit set, keyed by that bit.
fn contents_histogram(contents: impl IntoIterator<Item = u32>) -> BTreeMap<u32, usize> {
    let mut bits = BTreeMap::new();
    for c in contents {
        for bit in (0..32).map(|n| 1u32 << n).filter(|bit| c & bit != 0) {
            *bits.entry(bit).or_default() += 1;
        }
    }
    bits
}

/// Per node and per leaf, whether the walk from node 0 got there.
struct Reached {
    nodes: Vec<bool>,
    leaves: Vec<bool>,
}

fn walk_tree(children: &[[i16; 2]], leaf_count: usize) -> Reached {
    let mut node_seen = vec![false; children.len()];
    let mut leaf_hit = vec![false; leaf_count];
    let mut pending = vec![0i32];
    while let Some(c) = pending.pop() {
        if c < 0 {
            if let Some(hit) = leaf_hit.get_mut((-1 - c) as usize) {
                *hit = true;
            }
            continue;
        }
        let i = c as usize;
        match node_seen.get_mut(i) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        pending.extend(children[i].map(i32::from));
    }
    Reached { nodes: node_seen, leaves: leaf_hit }
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let mut args = std::env::args().skip(1).peekable();
    let path = match args.peek() {
        Some(a) if a.ends_with(".ff") => PathBuf::from(args.next().unwrap_or_default()),
        _ => default_zone(),
    };
    let nums: Vec<f32> = args.filter_map(|a| a.parse().ok()).collect();
    let capture = match capture_zone(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6clip: {e}");
            return Err(Failed);
        }
    };
    // T6CLIP_RAY=<model>@x,y[;x,y...]: where a straight-up line at (x, y) in
    // that model's own space crosses its collision triangles, bottom to top
    // (the bus floor, its ceiling and roof top).
    if let Some((want, spots)) = std::env::var("T6CLIP_RAY")
        .ok()
        .and_then(|v| v.split_once('@').map(|(m, s)| (m.to_owned(), s.to_owned())))
    {
        for m in capture.xmodels.iter().filter(|m| m.name == want) {
            for spot in spots.split(';') {
                let xy: Vec<f32> = spot.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                let [x, y] = xy[..] else { continue };
                let mut found = vec![];
                for c in &m.collision_faces {
                    for t in &c.triangle_list {
                        let [plane, s, tv] = *t;
                        if plane[2].abs() < 1e-4 {
                            continue;
                        }
                        let z = (plane[3] - plane[0] * x - plane[1] * y) / plane[2];
                        let p = [x, y, z];
                        let d = |v: [f32; 4]| v[0] * p[0] + v[1] * p[1] + v[2] * p[2] - v[3];
                        let (a, bb) = (d(s), d(tv));
                        if a >= 0.0 && bb >= 0.0 && a + bb <= 1.0 {
                            found.push((z, plane[2], c.face_property_bits));
                        }
                    }
                }
                found.sort_by(|a, b| a.0.total_cmp(&b.0));
                println!("{} at {x},{y}: {:?}", m.name, found);
            }
        }
        return Ok(());
    }
    if let Ok(out) = std::env::var("T6CLIP_ENTS") {
        let text: String = capture.map_ents.concat();
        if let Err(e) = std::fs::write(&out, text) {
            eprintln!("t6clip: {out}: {e}");
        }
    }
    let Some(clip) = capture.clip.as_ref() else {
        eprintln!("t6clip: {} has no clipMap", path.to_string_lossy());
        return Err(Failed);
    };

    let bits = contents_histogram(clip.brushes.iter().map(|b| b.contents as u32));
    println!("{} brushes; contents bits:", clip.brushes.len());
    for (bit, n) in &bits {
        println!("   {bit:#010x}  {n}");
    }
    if let Some(want) = std::env::var("T6CLIP_CONTENTS")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
    {
        for (i, b) in clip.brushes.iter().enumerate() {
            if (b.contents as u32) & want != 0 {
                println!(
                    "   contents brush {i}: {:#010x}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
                    b.contents as u32,
                    b.lo_corner[0],
                    b.lo_corner[1],
                    b.lo_corner[2],
                    b.hi_corner[0],
                    b.hi_corner[1],
                    b.hi_corner[2]
                );
            }
        }
        let mut leaves = 0;
        for n in clip.box_trees.iter().filter(|n| n.sub_element_total == 0) {
            let Some(m) = clip.materials.get(n.material_slot as usize) else {
                continue;
            };
            if (m.pack_presence_bits as u32) & want == 0 {
                continue;
            }
            let Some(&(triangle_tally, first)) = clip.partitions.get(n.u as usize) else {
                continue;
            };
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for t in first as usize..first as usize + triangle_tally as usize {
                for k in 0..3 {
                    let Some(v) = clip.tri_index_list.get(t * 3 + k).and_then(|&i| clip.vertex_points.get(i as usize)) else {
                        continue;
                    };
                    for a in 0..3 {
                        lo[a] = lo[a].min(v[a]);
                        hi[a] = hi[a].max(v[a]);
                    }
                }
            }
            leaves += 1;
            if leaves <= 40 {
                println!(
                    "   contents tris (material {}, {:#x}): {triangle_tally} tris, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
                    n.material_slot, m.pack_presence_bits as u32, lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
                );
            }
        }
        println!("   contents tri leaves: {leaves}");
    }
    let mut mats: BTreeMap<u32, usize> = BTreeMap::default();
    for m in &clip.materials {
        *mats.entry(m.pack_presence_bits as u32).or_default() += 1;
    }
    println!("{} clip materials; content flags:", clip.materials.len());
    for (flags, n) in &mats {
        println!("   {flags:#010x}  {n}");
    }
    // Slide ground (surface flag 0x2, BO2's slide surface): the clip
    // materials and brush faces that carry it.
    let slide_mats = clip.materials.iter().filter(|m| m.surface_flags as u32 & 2 != 0).count();
    let slide_faces: usize = clip
        .brushes
        .iter()
        .map(|b| {
            b.axial_sflags.iter().flatten().filter(|&&f| f as u32 & 2 != 0).count()
                + b.sides.iter().filter(|s| s.2 as u32 & 2 != 0).count()
        })
        .sum();
    println!("slide ground (surface flag 0x2): {slide_mats} materials, {slide_faces} brush faces");
    for (i, m) in clip.materials.iter().enumerate() {
        if m.surface_flags as u32 & 2 != 0 {
            let triangle_list: u32 = clip
                .box_trees
                .iter()
                .filter(|t| t.material_slot as usize == i && t.sub_element_total == 0)
                .filter_map(|t| clip.partitions.get(t.u as usize))
                .map(|&(tri_tally, _)| u32::from(tri_tally))
                .sum();
            println!(
                "   slide material {i} {:?}: surface type {}, contents {:#x}, {triangle_list} mesh triangles",
                m.name,
                (m.surface_flags as u32 >> 20) & 0x3f,
                m.pack_presence_bits as u32
            );
        }
    }
    for (i, b) in clip.brushes.iter().enumerate() {
        let n = b.axial_sflags.iter().flatten().filter(|&&f| f as u32 & 2 != 0).count()
            + b.sides.iter().filter(|s| s.2 as u32 & 2 != 0).count();
        if n > 0 {
            println!(
                "   slide brush {i}: {n} faces, contents {:#x}, mins {:?} maxs {:?}, axial sflags {:x?}",
                b.contents as u32, b.lo_corner, b.hi_corner, b.axial_sflags
            );
        }
    }
    // `T6CLIP_MATS=1`: every clip material, with its surface type (bits 20..25).
    if std::env::var_os("T6CLIP_MATS").is_some() {
        for (i, m) in clip.materials.iter().enumerate() {
            println!(
                "   material {i}: surface type {} contents {:#x} {:?}",
                (m.surface_flags as u32 >> 20) & 0x3f,
                m.pack_presence_bits as u32,
                m.name
            );
        }
    }
    // Foliage (contents 0x2): the materials, and the mesh triangles under each.
    for (i, m) in clip.materials.iter().enumerate() {
        if m.pack_presence_bits as u32 & 2 != 0 {
            let triangle_list: u32 = clip
                .box_trees
                .iter()
                .filter(|t| t.material_slot as usize == i && t.sub_element_total == 0)
                .filter_map(|t| clip.partitions.get(t.u as usize))
                .map(|&(tri_tally, _)| u32::from(tri_tally))
                .sum();
            println!("   foliage material {:?}: {triangle_list} triangles", m.name);
        }
    }

    // T6CLIP_BRUSHES=357,358: every plane of those brushes.
    if let Ok(list) = std::env::var("T6CLIP_BRUSHES") {
        for i in list.split(',').filter_map(|s| s.trim().parse::<usize>().ok()) {
            let Some(b) = clip.brushes.get(i) else { continue };
            println!("brush {i}: {:#010x}", b.contents as u32);
            for p in planes(b) {
                println!("   plane ({:.4}, {:.4}, {:.4}) {:.3}", p[0], p[1], p[2], p[3]);
            }
        }
    }
    let mut points: Vec<[f32; 3]> = nums.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    // T6CLIP_FIND=<text>: every map entity with a value holding that text,
    // all its keys; for a brush entity also its model's box placed at its
    // origin, and whether a player box at each point is inside that box or
    // inside one of the model's own brushes (which brush model a trigger
    // uses, where it sits, and whether a point is really in it).
    if let Ok(want) = std::env::var("T6CLIP_FIND") {
        for text in &capture.map_ents {
            for e in asset_t6::parse_entities(text) {
                if !e.fields.iter().any(|(_, v)| v.contains(want.as_str())) {
                    continue;
                }
                let kv: Vec<String> = e.fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
                println!("ent {}", kv.join(" "));
                let model = e.fields.iter().find(|(k, _)| k == "model").and_then(|(_, v)| {
                    v.strip_prefix('*')?.parse::<usize>().ok()
                });
                let (Some(n), Some(m)) = (model, model.and_then(|n| clip.sub_models.get(n))) else {
                    continue;
                };
                let o = e.vec3("origin").unwrap_or([0.0; 3]);
                let list = leaf_brushes(clip, m.leaf.brush_leaf_link);
                let lo: [f32; 3] = std::array::from_fn(|i| o[i] + m.lo_corner[i]);
                let hi: [f32; 3] = std::array::from_fn(|i| o[i] + m.hi_corner[i]);
                println!(
                    "   *{n}: {} brushes (map info {}), box [{:.0} {:.0} {:.0}]..[{:.0} {:.0} {:.0}]",
                    list.len(),
                    m.shares_map_info,
                    lo[0],
                    lo[1],
                    lo[2],
                    hi[0],
                    hi[1],
                    hi[2]
                );
                for p in &points {
                    let in_box = (0..3).all(|i| {
                        p[i] + BODY_BOX_UPPER[i] >= lo[i] && p[i] + BODY_BOX_LOWER[i] <= hi[i]
                    });
                    let local: [f32; 3] = std::array::from_fn(|i| p[i] - o[i]);
                    let in_brush = list
                        .iter()
                        .filter_map(|&b| clip.brushes.get(usize::from(b)))
                        .any(|b| box_inside(b, local));
                    println!(
                        "      at [{:.0} {:.0} {:.0}]: in box {in_box}, in a brush {in_brush}",
                        p[0], p[1], p[2]
                    );
                }
            }
        }
    }
    if points.is_empty() {
        for text in &capture.map_ents {
            for e in asset_t6::parse_entities(text) {
                if e.classname() == "info_player_start"
                    && let Some(o) = e.vec3("origin")
                {
                    points.push(o);
                    points.push([o[0], o[1], o[2] + 228.0]);
                }
            }
        }
    }
    // Leaves the tree reaches from node 0; a negative child c is leaf -1-c.
    let (n_nodes, n_leaves) = (clip.nodes.len(), clip.leaves.len());
    let children: Vec<[i16; 2]> = clip.nodes.iter().map(|n| n.children).collect();
    let Reached { nodes: seen_nodes, leaves: reach } = walk_tree(&children, n_leaves);
    println!(
        "tree reaches {} of {n_nodes} nodes and {} of {n_leaves} leaves",
        seen_nodes.iter().filter(|v| **v).count(),
        reach.iter().filter(|v| **v).count(),
    );
    let n_brushes = clip.brushes.len();
    let mut world = vec![false; n_brushes];
    let mut world_reached = vec![false; n_brushes];
    for (li, leaf) in clip.leaves.iter().enumerate() {
        for b in leaf_brushes(clip, leaf.brush_leaf_link) {
            if let Some(w) = world.get_mut(usize::from(b)) {
                *w = true;
            }
            if reach[li]
                && let Some(w) = world_reached.get_mut(usize::from(b))
            {
                *w = true;
            }
        }
    }
    println!(
        "brushes listed by reached leaves: {}; by any leaf: {}",
        world_reached.iter().filter(|v| **v).count(),
        world.iter().filter(|v| **v).count()
    );
    let mut owner: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, m) in clip.sub_models.iter().enumerate() {
        for b in leaf_brushes(clip, m.leaf.brush_leaf_link) {
            owner.entry(usize::from(b)).or_default().push(i);
        }
    }
    let n_models = clip.sub_models.len();
    let sharing = clip.sub_models.iter().filter(|m| m.shares_map_info).count();
    let listed = world.iter().filter(|w| **w).count();
    println!(
        "tree: {n_nodes} nodes, {n_leaves} leaves, {} leaf brush nodes, {n_models} brush models ({sharing} share the map's info); {listed} brushes listed by leaves",
        clip.leafbrush_nodes.len(),
    );
    if let Some(m) = clip.sub_models.first() {
        println!(
            "model 0: {} brushes, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            leaf_brushes(clip, m.leaf.brush_leaf_link).len(),
            m.lo_corner[0],
            m.lo_corner[1],
            m.lo_corner[2],
            m.hi_corner[0],
            m.hi_corner[1],
            m.hi_corner[2]
        );
    }
    // T6CLIP_COLUMN=1: what a vertical line through each point meets, from
    // its z down: brushes (the span the line is inside) and collision
    // triangles (the height and the material's contents).
    if std::env::var_os("T6CLIP_COLUMN").is_some() {
        for p in &points {
            println!("column at [{:.1}, {:.1}] below {:.1}:", p[0], p[1], p[2]);
            for (i, b) in clip.brushes.iter().enumerate() {
                let (mut lo, mut hi) = (f32::MIN, f32::MAX);
                let mut miss = false;
                for q in planes(b) {
                    let off = q[0] * p[0] + q[1] * p[1] - q[3];
                    if q[2].abs() < 1e-6 {
                        miss |= off > 0.0;
                    } else if q[2] > 0.0 {
                        hi = hi.min(-off / q[2]);
                    } else {
                        lo = lo.max(-off / q[2]);
                    }
                }
                if !miss && lo < hi && lo <= p[2] {
                    println!(
                        "   brush {i} ({}): {:#010x} z {lo:.2}..{hi:.2}",
                        if world_reached[i] { "world" } else if world[i] { "entity leaf" } else { "no leaf" },
                        b.contents as u32
                    );
                }
            }
            for n in clip.box_trees.iter().filter(|n| n.sub_element_total == 0) {
                let Some(&(triangle_tally, first)) = clip.partitions.get(n.u as usize) else {
                    continue;
                };
                let flags = clip.materials.get(n.material_slot as usize).map_or(0, |m| m.pack_presence_bits as u32);
                for t in first as usize..first as usize + triangle_tally as usize {
                    let v: Vec<[f32; 3]> = (0..3)
                        .filter_map(|k| clip.tri_index_list.get(t * 3 + k).and_then(|&i| clip.vertex_points.get(i as usize)).copied())
                        .collect();
                    if v.len() < 3 {
                        continue;
                    }
                    let d = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                    let (s0, s1, s2) = (d(v[0], v[1]), d(v[1], v[2]), d(v[2], v[0]));
                    let inside = (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0);
                    let area = s0 + s1 + s2;
                    if !inside || area.abs() < 1e-6 {
                        continue;
                    }
                    let z = (s1 * v[0][2] + s2 * v[1][2] + s0 * v[2][2]) / area;
                    if z <= p[2] {
                        println!("   tri {t} (material {}, {flags:#010x}) z {z:.2}", n.material_slot);
                    }
                }
            }
        }
    }
    for p in points {
        let found: Vec<usize> = clip
            .brushes
            .iter()
            .enumerate()
            .filter(|(_key, b)| box_inside(b, p))
            .map(|(i, _)| i)
            .collect();
        println!(
            "player box at [{:.1}, {:.1}, {:.1}]: inside {} brushes",
            p[0],
            p[1],
            p[2],
            found.len()
        );
        for &i in found.iter().take(20) {
            let b = &clip.brushes[i];
            let whose = format!(
                "{}{}",
                match (world_reached[i], world[i]) {
                    (true, _) => "WORLD WALL",
                    (false, true) => "entity leaf only",
                    (false, false) => "no leaf",
                },
                owner
                    .get(&i)
                    .map_or_else(String::new, |m| format!(", brush model {m:?}"))
            );
            println!(
                "   brush {i} ({whose}): contents {:#010x}, {} extra sides, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
                b.contents as u32,
                b.sides.len(),
                b.lo_corner[0],
                b.lo_corner[1],
                b.lo_corner[2],
                b.hi_corner[0],
                b.hi_corner[1],
                b.hi_corner[2]
            );
            println!(
                "      table contents {:#x?}",
                clip.solid_content_flags.get(i).map(|c| *c as u32)
            );
            println!(
                "      axial cflags {:x?} sflags {:x?}; side sflags {:x?}",
                b.axial_cflags,
                b.axial_sflags,
                b.sides.iter().map(|s| s.2).collect::<Vec<_>>()
            );
        }
    }
    let mut same = 0;
    let mut differ: BTreeMap<(u32, u32, bool), usize> = BTreeMap::default();
    for (i, b) in clip.brushes.iter().enumerate() {
        let t = clip.solid_content_flags.get(i).copied().unwrap_or(-1);
        if t == b.contents {
            same += 1;
        } else {
            *differ
                .entry((b.contents as u32, t as u32, owner.contains_key(&i)))
                .or_default() += 1;
        }
    }
    println!(
        "table vs brush contents: {n_brushes} brushes, {} table entries, {same} same",
        clip.solid_content_flags.len()
    );
    for ((b, t, owned), n) in differ {
        println!("   brush {b:#010x} table {t:#010x} model-owned {owned}: {n}");
    }
    let owned: usize = owner.len();
    let both = owner.keys().filter(|&&i| world[i]).count();
    println!("{owned} brushes owned by brush models; {both} of them also in world leaves");
    // T6CLIP_MODELS=183,1: what those brush models are made of (brushes, or
    // collision triangles under their own AABB trees).
    let picked: Vec<usize> = std::env::var("T6CLIP_MODELS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    for &i in &picked {
        let Some(m) = clip.sub_models.get(i) else {
            println!("   model {i}: none");
            continue;
        };
        let list = leaf_brushes(clip, m.leaf.brush_leaf_link);
        let first = usize::from(m.leaf.first_collision_box_slot);
        let trees = &clip.box_trees[first.min(clip.box_trees.len())
            ..(first + usize::from(m.leaf.collision_box_total)).min(clip.box_trees.len())];
        println!(
            "   model {i}: shares map info {}, {} brushes, {} aabb roots from {first}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            m.shares_map_info,
            list.len(),
            m.leaf.collision_box_total,
            m.lo_corner[0],
            m.lo_corner[1],
            m.lo_corner[2],
            m.hi_corner[0],
            m.hi_corner[1],
            m.hi_corner[2]
        );
        for &b in list.iter().take(6) {
            let b = &clip.brushes[usize::from(b)];
            println!(
                "      brush contents {:#x}, {} extra sides, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}], axial sflags {:x?}, side sflags {:x?}",
                b.contents as u32,
                b.sides.len(),
                b.lo_corner[0],
                b.lo_corner[1],
                b.lo_corner[2],
                b.hi_corner[0],
                b.hi_corner[1],
                b.hi_corner[2],
                b.axial_sflags,
                b.sides.iter().map(|s| s.2).collect::<Vec<_>>()
            );
        }
        for t in trees.iter().take(4) {
            let mat = clip.materials.get(usize::from(t.material_slot));
            println!(
                "      aabb origin {:?} half {:?} material {} {:?} contents {:#x} children {}",
                t.origin,
                t.half_extent,
                t.material_slot,
                mat.map_or("?", |m| m.name.as_str()),
                mat.map_or(0, |m| m.pack_presence_bits as u32),
                t.sub_element_total
            );
        }
        let in_world = clip.leaves.iter().any(|l| {
            let lo = usize::from(l.first_collision_box_slot);
            l.collision_box_total > 0 && lo <= first && first < lo + usize::from(l.collision_box_total)
        });
        println!("      root listed by a world leaf: {in_world}");
        // A straight line down through the model's middle, in its own space,
        // through BO2's `CM_BoxTrace` on the model (as the height lock reads a
        // ceiling).
        if let Ok(map) = asset_world::t6_clip_map(clip)
            && map.cmodels.get(i).is_some_and(|c| c.leaf.collision_box_total > 0)
        {
            let mid = [(m.lo_corner[0] + m.hi_corner[0]) * 0.5, (m.lo_corner[1] + m.hi_corner[1]) * 0.5];
            let start = [mid[0], mid[1], m.hi_corner[2] + 1000.0];
            let end = [mid[0], mid[1], m.lo_corner[2] - 1000.0];
            let mut marks = vec![0; map.partitions.len()];
            let mut check = clipmap_t6::CheckCount::new(&mut marks);
            let tr = map.box_trace(&mut check, start, end, [0.0; 3], [0.0; 3], i as u32, u32::MAX);
            println!(
                "      line down the middle: fraction {:.4}, hit z {:.1}",
                tr.fraction,
                start[2] + (end[2] - start[2]) * tr.fraction
            );
        }
    }
    // T6CLIP_SFLAGS=8: how many brushes carry each surface flag bit, and the first ones with
    // the asked bits (where the map's ladders are).
    let wanted_sflags = std::env::var("T6CLIP_SFLAGS")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
    if let Some(want) = wanted_sflags {
        let mut sbits: BTreeMap<u32, usize> = BTreeMap::default();
        let mut shown = 0;
        for (i, b) in clip.brushes.iter().enumerate() {
            let axial = b.axial_sflags.iter().flatten().map(|f| *f as u32);
            let all = axial.chain(b.sides.iter().map(|s| s.2 as u32)).fold(0, |a, f| a | f);
            for bit in (0..32).filter(|bit| all & (1 << bit) != 0) {
                *sbits.entry(1 << bit).or_default() += 1;
            }
            if all & want != 0 && shown < 60 {
                shown += 1;
                println!(
                    "   brush {i}: contents {:#010x}, sflags {all:#x}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}], models {:?}",
                    b.contents as u32,
                    b.lo_corner[0],
                    b.lo_corner[1],
                    b.lo_corner[2],
                    b.hi_corner[0],
                    b.hi_corner[1],
                    b.hi_corner[2],
                    owner.get(&i)
                );
            }
        }
        for m in clip.materials.iter().filter(|m| m.surface_flags as u32 & want != 0) {
            println!(
                "   clip material {}: surface flags {:#x}, contents {:#x}",
                m.name, m.surface_flags as u32, m.pack_presence_bits as u32
            );
        }
        println!("brushes by surface flag bit:");
        for (bit, n) in &sbits {
            println!("   {bit:#010x}  {n}");
        }
    }
    for (i, m) in clip.sub_models.iter().enumerate().take(12) {
        let list = leaf_brushes(clip, m.leaf.brush_leaf_link);
        println!(
            "   model {i}: {} brushes {:?}.., contents {:#x}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            list.len(),
            &list[..list.len().min(4)],
            m.leaf.solid_content_flags as u32,
            m.lo_corner[0],
            m.lo_corner[1],
            m.lo_corner[2],
            m.hi_corner[0],
            m.hi_corner[1],
            m.hi_corner[2]
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_counts_each_set_bit() {
        let h = contents_histogram([0b101, 0b100, 0]);
        assert_eq!(h.get(&1), Some(&1));
        assert_eq!(h.get(&4), Some(&2));
        assert_eq!(h.len(), 2);
        let top = contents_histogram([0x8000_0000]);
        assert_eq!(top.get(&0x8000_0000), Some(&1));
    }

    #[test]
    fn walk_reaches_only_what_hangs_off_node_zero() {
        // node 0 -> (node 1, leaf 0); node 1 -> (leaf 1, leaf 1); node 2 is orphaned.
        let children = [[1, -1], [-2, -2], [-3, -3]];
        let Reached { nodes, leaves } = walk_tree(&children, 3);
        assert_eq!(nodes, [true, true, false]);
        assert_eq!(leaves, [true, true, false]);
    }

    #[test]
    fn walk_ignores_children_out_of_range() {
        let Reached { nodes, leaves } = walk_tree(&[[7, -9]], 1);
        assert_eq!(nodes, [true]);
        assert_eq!(leaves, [false]);
    }
}
