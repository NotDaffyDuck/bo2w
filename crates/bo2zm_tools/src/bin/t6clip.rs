//! bo2zm: measure a Black Ops II map's collision, read-only.
//! `t6clip [zone.ff] [x y z ...]` (default Nuketown, at its player start)
//! counts brush contents bits, then lists every brush whose player-sized hull
//! holds each point: the brushes a player standing there would be inside.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use asset_t6::{ClipBrushRef, ClipRef, capture_zone};

const DEFAULT_ZONE: &str =
    r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all\zm_nuked.ff";

/// the legacy standing player box.
const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];

/// Every plane of a brush: the six box planes, then the extra sides.
fn planes(b: &ClipBrushRef) -> Vec<[f32; 4]> {
    let mut p = vec![
        [1.0, 0.0, 0.0, b.maxs[0]],
        [-1.0, 0.0, 0.0, -b.mins[0]],
        [0.0, 1.0, 0.0, b.maxs[1]],
        [0.0, -1.0, 0.0, -b.mins[1]],
        [0.0, 0.0, 1.0, b.maxs[2]],
        [0.0, 0.0, -1.0, -b.mins[2]],
    ];
    p.extend(b.sides.iter().map(|&(n, d, _)| [n[0], n[1], n[2], d]));
    p
}

/// Whether the box at `origin` overlaps the brush: every plane pushed out by
/// the box's reach along its normal (its nearest corner, so a box above a
/// brush's top is not counted) must still have the origin behind it.
fn box_inside(b: &ClipBrushRef, origin: [f32; 3]) -> bool {
    planes(b).iter().all(|p| {
        let reach: f32 = -(0..3)
            .map(|i| {
                if p[i] < 0.0 {
                    p[i] * PLAYER_MAXS[i]
                } else {
                    p[i] * PLAYER_MINS[i]
                }
            })
            .sum::<f32>();
        p[0] * origin[0] + p[1] * origin[1] + p[2] * origin[2] - (p[3] + reach) < 0.0
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
        if node.leaf_brush_count > 0 {
            out.extend_from_slice(&node.brushes);
            continue;
        }
        if node.leaf_brush_count < 0 {
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

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    let path = match args.peek() {
        Some(a) if a.ends_with(".ff") => PathBuf::from(args.next().unwrap_or_default()),
        _ => PathBuf::from(DEFAULT_ZONE),
    };
    let nums: Vec<f32> = args.filter_map(|a| a.parse().ok()).collect();
    let capture = match capture_zone(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6clip: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Ok(out) = std::env::var("T6CLIP_ENTS") {
        let text: String = capture.map_ents.concat();
        if let Err(e) = std::fs::write(&out, text) {
            eprintln!("t6clip: {out}: {e}");
        }
    }
    let Some(clip) = capture.clip.as_ref() else {
        eprintln!("t6clip: {} has no clipMap", path.display());
        return ExitCode::FAILURE;
    };

    let mut bits: BTreeMap<u32, usize> = BTreeMap::new();
    for b in &clip.brushes {
        for bit in 0..32 {
            if (b.contents as u32) & (1 << bit) != 0 {
                *bits.entry(1 << bit).or_default() += 1;
            }
        }
    }
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
                    b.mins[0],
                    b.mins[1],
                    b.mins[2],
                    b.maxs[0],
                    b.maxs[1],
                    b.maxs[2]
                );
            }
        }
        let mut leaves = 0;
        for n in clip.aabb_trees.iter().filter(|n| n.child_count == 0) {
            let Some(m) = clip.materials.get(n.material_index as usize) else {
                continue;
            };
            if (m.content_flags as u32) & want == 0 {
                continue;
            }
            let Some(&(tri_n, first)) = clip.partitions.get(n.u as usize) else {
                continue;
            };
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for t in first as usize..first as usize + tri_n as usize {
                for k in 0..3 {
                    let Some(v) = clip.tri_indices.get(t * 3 + k).and_then(|&i| clip.verts.get(i as usize)) else {
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
                    "   contents tris (material {}, {:#x}): {tri_n} tris, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
                    n.material_index, m.content_flags as u32, lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
                );
            }
        }
        println!("   contents tri leaves: {leaves}");
    }
    let mut mats: BTreeMap<u32, usize> = BTreeMap::new();
    for m in &clip.materials {
        *mats.entry(m.content_flags as u32).or_default() += 1;
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
            let tris: u32 = clip
                .aabb_trees
                .iter()
                .filter(|t| t.material_index as usize == i && t.child_count == 0)
                .filter_map(|t| clip.partitions.get(t.u as usize))
                .map(|&(tri_count, _)| u32::from(tri_count))
                .sum();
            println!(
                "   slide material {i} {:?}: surface type {}, contents {:#x}, {tris} mesh triangles",
                m.name,
                (m.surface_flags as u32 >> 20) & 0x3f,
                m.content_flags as u32
            );
        }
    }
    for (i, b) in clip.brushes.iter().enumerate() {
        let n = b.axial_sflags.iter().flatten().filter(|&&f| f as u32 & 2 != 0).count()
            + b.sides.iter().filter(|s| s.2 as u32 & 2 != 0).count();
        if n > 0 {
            println!(
                "   slide brush {i}: {n} faces, contents {:#x}, mins {:?} maxs {:?}, axial sflags {:x?}",
                b.contents as u32, b.mins, b.maxs, b.axial_sflags
            );
        }
    }
    // `T6CLIP_MATS=1`: every clip material, with its surface type (bits 20..25).
    if std::env::var_os("T6CLIP_MATS").is_some() {
        for (i, m) in clip.materials.iter().enumerate() {
            println!(
                "   material {i}: surface type {} contents {:#x} {:?}",
                (m.surface_flags as u32 >> 20) & 0x3f,
                m.content_flags as u32,
                m.name
            );
        }
    }
    // Foliage (contents 0x2): the materials, and the mesh triangles under each.
    for (i, m) in clip.materials.iter().enumerate() {
        if m.content_flags as u32 & 2 != 0 {
            let tris: u32 = clip
                .aabb_trees
                .iter()
                .filter(|t| t.material_index as usize == i && t.child_count == 0)
                .filter_map(|t| clip.partitions.get(t.u as usize))
                .map(|&(tri_count, _)| u32::from(tri_count))
                .sum();
            println!("   foliage material {:?}: {tris} triangles", m.name);
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
    let mut reach = vec![false; clip.leaves.len()];
    let mut stack = if clip.nodes.is_empty() {
        vec![]
    } else {
        vec![0i32]
    };
    let mut seen_nodes = vec![false; clip.nodes.len()];
    while let Some(c) = stack.pop() {
        if c < 0 {
            if let Some(r) = reach.get_mut((-1 - c) as usize) {
                *r = true;
            }
            continue;
        }
        let i = c as usize;
        if i >= clip.nodes.len() || seen_nodes[i] {
            continue;
        }
        seen_nodes[i] = true;
        stack.extend(clip.nodes[i].children.map(i32::from));
    }
    println!(
        "tree reaches {} of {} nodes and {} of {} leaves",
        seen_nodes.iter().filter(|v| **v).count(),
        clip.nodes.len(),
        reach.iter().filter(|v| **v).count(),
        clip.leaves.len()
    );
    let mut world = vec![false; clip.brushes.len()];
    let mut world_reached = vec![false; clip.brushes.len()];
    for (li, leaf) in clip.leaves.iter().enumerate() {
        for b in leaf_brushes(clip, leaf.leaf_brush_node) {
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
    for (i, m) in clip.cmodels.iter().enumerate() {
        for b in leaf_brushes(clip, m.leaf.leaf_brush_node) {
            owner.entry(usize::from(b)).or_default().push(i);
        }
    }
    println!(
        "tree: {} nodes, {} leaves, {} leaf brush nodes, {} brush models ({} share the map's info); {} brushes listed by leaves",
        clip.nodes.len(),
        clip.leaves.len(),
        clip.leafbrush_nodes.len(),
        clip.cmodels.len(),
        clip.cmodels.iter().filter(|m| m.shares_map_info).count(),
        world.iter().filter(|w| **w).count()
    );
    if let Some(m) = clip.cmodels.first() {
        println!(
            "model 0: {} brushes, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            leaf_brushes(clip, m.leaf.leaf_brush_node).len(),
            m.mins[0],
            m.mins[1],
            m.mins[2],
            m.maxs[0],
            m.maxs[1],
            m.maxs[2]
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
            for n in clip.aabb_trees.iter().filter(|n| n.child_count == 0) {
                let Some(&(tri_n, first)) = clip.partitions.get(n.u as usize) else {
                    continue;
                };
                let flags = clip.materials.get(n.material_index as usize).map_or(0, |m| m.content_flags as u32);
                for t in first as usize..first as usize + tri_n as usize {
                    let v: Vec<[f32; 3]> = (0..3)
                        .filter_map(|k| clip.tri_indices.get(t * 3 + k).and_then(|&i| clip.verts.get(i as usize)).copied())
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
                        println!("   tri {t} (material {}, {flags:#010x}) z {z:.2}", n.material_index);
                    }
                }
            }
        }
    }
    for p in points {
        let hits: Vec<usize> = clip
            .brushes
            .iter()
            .enumerate()
            .filter(|(_, b)| box_inside(b, p))
            .map(|(i, _)| i)
            .collect();
        println!(
            "player box at [{:.1}, {:.1}, {:.1}]: inside {} brushes",
            p[0],
            p[1],
            p[2],
            hits.len()
        );
        for &i in hits.iter().take(20) {
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
                b.mins[0],
                b.mins[1],
                b.mins[2],
                b.maxs[0],
                b.maxs[1],
                b.maxs[2]
            );
            println!(
                "      table contents {:#x?}",
                clip.brush_contents.get(i).map(|c| *c as u32)
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
    let mut differ: BTreeMap<(u32, u32, bool), usize> = BTreeMap::new();
    for (i, b) in clip.brushes.iter().enumerate() {
        let t = clip.brush_contents.get(i).copied().unwrap_or(-1);
        if t == b.contents {
            same += 1;
        } else {
            *differ
                .entry((b.contents as u32, t as u32, owner.contains_key(&i)))
                .or_default() += 1;
        }
    }
    println!(
        "table vs brush contents: {} brushes, {} table entries, {same} same",
        clip.brushes.len(),
        clip.brush_contents.len()
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
        let Some(m) = clip.cmodels.get(i) else {
            println!("   model {i}: none");
            continue;
        };
        let list = leaf_brushes(clip, m.leaf.leaf_brush_node);
        let first = usize::from(m.leaf.first_coll_aabb_index);
        let trees = &clip.aabb_trees[first.min(clip.aabb_trees.len())
            ..(first + usize::from(m.leaf.coll_aabb_count)).min(clip.aabb_trees.len())];
        println!(
            "   model {i}: shares map info {}, {} brushes, {} aabb roots from {first}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            m.shares_map_info,
            list.len(),
            m.leaf.coll_aabb_count,
            m.mins[0],
            m.mins[1],
            m.mins[2],
            m.maxs[0],
            m.maxs[1],
            m.maxs[2]
        );
        for &b in list.iter().take(6) {
            let b = &clip.brushes[usize::from(b)];
            println!(
                "      brush contents {:#x}, {} extra sides, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
                b.contents as u32,
                b.sides.len(),
                b.mins[0],
                b.mins[1],
                b.mins[2],
                b.maxs[0],
                b.maxs[1],
                b.maxs[2]
            );
        }
        for t in trees.iter().take(4) {
            let mat = clip.materials.get(usize::from(t.material_index));
            println!(
                "      aabb origin {:?} half {:?} material {} {:?} contents {:#x} children {}",
                t.origin,
                t.half_size,
                t.material_index,
                mat.map_or("?", |m| m.name.as_str()),
                mat.map_or(0, |m| m.content_flags as u32),
                t.child_count
            );
        }
        let in_world = clip.leaves.iter().any(|l| {
            let lo = usize::from(l.first_coll_aabb_index);
            l.coll_aabb_count > 0 && lo <= first && first < lo + usize::from(l.coll_aabb_count)
        });
        println!("      root listed by a world leaf: {in_world}");
        // A straight line down through the model's middle, in its own space,
        // through BO2's `CM_BoxTrace` on the model (as the height lock reads a
        // ceiling).
        if let Ok(map) = asset_world::t6_clip_map(clip)
            && map.cmodels.get(i).is_some_and(|c| c.leaf.coll_aabb_count > 0)
        {
            let mid = [(m.mins[0] + m.maxs[0]) * 0.5, (m.mins[1] + m.maxs[1]) * 0.5];
            let start = [mid[0], mid[1], m.maxs[2] + 1000.0];
            let end = [mid[0], mid[1], m.mins[2] - 1000.0];
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
    for (i, m) in clip.cmodels.iter().enumerate().take(12) {
        let list = leaf_brushes(clip, m.leaf.leaf_brush_node);
        println!(
            "   model {i}: {} brushes {:?}.., contents {:#x}, mins [{:.0}, {:.0}, {:.0}] maxs [{:.0}, {:.0}, {:.0}]",
            list.len(),
            &list[..list.len().min(4)],
            m.leaf.brush_contents as u32,
            m.mins[0],
            m.mins[1],
            m.mins[2],
            m.maxs[0],
            m.maxs[1],
            m.maxs[2]
        );
    }
    ExitCode::SUCCESS
}
