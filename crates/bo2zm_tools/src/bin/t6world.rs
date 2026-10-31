//! bo2zm: measure a Black Ops II map's world geometry, read-only.
//! `t6world [zone.ff]` (default Nuketown) captures the GfxWorld and reports
//! vertex streams, surfaces, materials and static models, and tests where
//! positions sit inside a world vertex: every surface's vertices must fall
//! inside that surface's own bounds.

use std::collections::BTreeMap;
use std::path::PathBuf;

use asset_t6::{WorldRef, capture_zone};

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("zm_nuked.ff")
}

fn float_from_bytes(b: &[u8], at: usize) -> Option<f32> {
    b.get(at..at + 4)
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `x,y,z,r` as four numbers (blanks around each allowed); anything else is not a probe.
fn point_and_radius(spec: &str) -> Option<[f32; 4]> {
    let nums: Vec<f32> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    <[f32; 4]>::try_from(nums).ok()
}

/// How many entities of each class a map-entities string holds.
fn class_counts(ents: &str) -> BTreeMap<&str, usize> {
    ents.lines()
        .filter_map(|line| line.trim().strip_prefix("\"classname\" "))
        .fold(BTreeMap::new(), |mut counts, rest| {
            *counts.entry(rest.trim_matches('"')).or_default() += 1;
            counts
        })
}

/// Every `{ ... }` entity block (each line ending in a newline) whose text holds `want`.
fn blocks_containing(ents: &str, want: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut block = String::new();
    for line in ents.lines() {
        block.push_str(line);
        block.push('\n');
        if line.trim() == "}" {
            let done = std::mem::take(&mut block);
            found.extend(done.contains(want).then_some(done));
        }
    }
    found
}

/// The leaf a descent ends in: `child_of` gives a node's child toward the point
/// (a negative number is a leaf, stored complemented); a missing node is `usize::MAX`.
fn leaf_index(child_of: impl Fn(i32) -> Option<i32>) -> usize {
    let path = std::iter::successors(Some(0i32), |&num| if num >= 0 { child_of(num) } else { None });
    match path.last() {
        Some(end) if end < 0 => (!end) as usize,
        _ => usize::MAX,
    }
}

fn main() -> std::process::ExitCode {
    let path = std::env::args()
        .nth(1)
        .map_or_else(default_zone, PathBuf::from);
    let outcome = run(&path);
    if let Err(msg) = &outcome {
        eprintln!("{msg}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

/// The breakable windows' clip brushes as the game links them (box, tag, the leaves that
/// list them, the leaf the pane's middle stands in).
fn print_glass_clip(capture: &asset_t6::ZoneCapture) {
    let Some(c) = &capture.clip else { return };
    let Some((_first, models)) = asset_world::build_t6_glass(&capture.glasses) else { return };
    let mut clip = match asset_world::build_t6_clip_collision(c) {
        Ok(clip) => clip,
        Err(e) => {
            println!("glassclip: {e:?}");
            return;
        }
    };
    println!("glassclip {:?}", asset_world::attach_t6_glass_brushes(&mut clip, &models));
    let list = |first: u32, num: u16| {
        clip.brush_leaf_indices.get(first as usize..first as usize + usize::from(num)).unwrap_or(&[])
    };
    let leaf_at = |p: [f32; 3]| {
        leaf_index(|num| {
            let n = clip.nodes.get(num as usize)?;
            let d = n.plane[0] * p[0] + n.plane[1] * p[1] + n.plane[2] * p[2] - n.plane[3];
            Some(if d >= 0.0 { n.children[0] } else { n.children[1] })
        })
    };
    for (pane, place) in models.iter().enumerate() {
        let model = place.solid_model;
        let Some(m) = clip.cmodels.get(model as usize) else { continue };
        let at = leaf_at(place.origin);
        println!(
            "pane {pane} *{model} cmodel {:?}..{:?} origin {:?} angles {:?} in leaf {at}",
            m.lo, m.hi, place.origin, place.angles
        );
        for &b in list(m.initial_convex_solid, m.solid_volume_total) {
            let Some(br) = clip.brushes.get(usize::from(b)) else { continue };
            let p = &br.planes;
            let leaves: Vec<usize> = (0..clip.leaves.len())
                .filter(|&i| list(clip.leaves[i].initial_convex_solid, clip.leaves[i].solid_volume_total).contains(&b))
                .collect();
            println!(
                "  brush {b} enc={} contents={:#x} box [{},{},{}]..[{},{},{}] planes={} leaves={leaves:?}",
                br.glass_hit_code, br.contents, -p[1][3], -p[3][3], -p[5][3], p[0][3], p[2][3], p[4][3], p.len()
            );
        }
    }
}

/// The cell tree against the portals: a point just past a portal (on the
/// side the walk looks through it from) should sit in the portal's far cell,
/// one just short of it in the cell that owns the portal.
fn print_cell_check(w: &WorldRef, spec: &str) {
    let cells_total = w.cell_bounds.len();
    let holding = |p: [f32; 3]| dpvs_t6::cell_holding(&w.dpvs_planes, &w.dpvs_nodes, cells_total, p);
    let near: Vec<f32> = spec.split(',').filter_map(|x| x.parse().ok()).collect();
    if let [x, y, z] = near[..] {
        let at = holding([x, y, z]);
        println!("cells: {cells_total}; [{x},{y},{z}] is in {at:?}");
        if let Some(c) = at.and_then(|c| w.cell_bounds.get(c)) {
            println!("  bounds {:?}..{:?}", c.lo_corner, c.hi_corner);
            for (i, p) in c.portals.iter().enumerate() {
                let d = p.plane[0] * x + p.plane[1] * y + p.plane[2] * z + p.plane[3];
                println!("  portal {i} -> cell {} plane {:?} eye_dist {d:.2} verts {:?}", p.cell, p.plane, p.vertices);
            }
        }
        return;
    }
    let (mut ok, mut bad) = (0u32, 0u32);
    for (owner, cell) in w.cell_bounds.iter().enumerate() {
        for (i, p) in cell.portals.iter().enumerate() {
            let n = [p.plane[0], p.plane[1], p.plane[2]];
            let k = p.vertices.len().max(1) as f32;
            let mid = p.vertices.iter().fold([0.0f32; 3], |a, v| [a[0] + v[0] / k, a[1] + v[1] / k, a[2] + v[2] / k]);
            // The middle, and each corner pulled a fifth of the way in.
            let spots = std::iter::once(mid).chain(p.vertices.iter().map(|v| {
                [v[0] + (mid[0] - v[0]) * 0.2, v[1] + (mid[1] - v[1]) * 0.2, v[2] + (mid[2] - v[2]) * 0.2]
            }));
            for s in spots {
                for step in [0.25f32, 1.0, 4.0, 16.0] {
                    // The walk queues a portal whose plane distance is negative,
                    // so the owner is on the negative side, the far cell on the positive.
                    let past = [s[0] + n[0] * step, s[1] + n[1] * step, s[2] + n[2] * step];
                    let short = [s[0] - n[0] * step, s[1] - n[1] * step, s[2] - n[2] * step];
                    for (spot, want, side) in [(past, usize::from(p.cell), "past"), (short, owner, "short")] {
                        let got = holding(spot);
                        if got == Some(want) {
                            ok += 1;
                        } else {
                            bad += 1;
                            if bad <= 400 {
                                println!(
                                    "cellcheck: cell {owner} portal {i} -> {} {side} {step} at [{:.1},{:.1},{:.1}]: tree says {got:?}, want {want}",
                                    p.cell, spot[0], spot[1], spot[2]
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!("cellcheck: {cells_total} cells, {ok} samples agree, {bad} disagree");
}

/// One camera's portal walk on the map's own cells (a 60 degree tall, 16:9
/// view, no bevels).
fn print_walk(w: &WorldRef, spec: &str) {
    let v: Vec<f32> = spec.split(',').filter_map(|x| x.parse().ok()).collect();
    let [x, y, z, yaw, near] = v[..] else { return };
    let mut cells = Vec::new();
    let (mut portals, mut verts) = (Vec::new(), Vec::new());
    for c in &w.cell_bounds {
        let first = portals.len();
        for p in &c.portals {
            portals.push(dpvs_t6::Portal {
                plane: p.plane,
                cell: p.cell,
                vertex_first: verts.len() as u32,
                vertex_tally: p.vertices.len() as u32,
                hull_axes_dir: p.hull_axes_dir,
            });
            verts.extend_from_slice(&p.vertices);
        }
        cells.push(dpvs_t6::Cell {
            portal_start: first as u32,
            portal_opening_total: (portals.len() - first) as u32,
            ..Default::default()
        });
    }
    let world = dpvs_t6::DpvsWorld {
        cells: &cells,
        portals: &portals,
        vertex_points: &verts,
        planes: &w.dpvs_planes,
        nodes: &w.dpvs_nodes,
    };
    let eye = [x, y, z];
    let (s, c) = yaw.to_radians().sin_cos();
    let fwd = glam::Vec3::new(c, s, 0.0);
    let right = glam::Vec3::new(s, -c, 0.0);
    let up = glam::Vec3::Z;
    let at = glam::Vec3::from(eye);
    let (ty, tx) = (30f32.to_radians().tan(), 30f32.to_radians().tan() * 16.0 / 9.0);
    let plane = |n: glam::Vec3, p: glam::Vec3| {
        let n = n.normalize();
        [n.x, n.y, n.z, -n.dot(p)]
    };
    let planes = [
        plane(fwd, at + fwd * near.abs()),
        plane(-fwd, at + fwd * 8000.0),
        plane((fwd - right * tx).cross(up) * -1.0, at),
        plane((fwd + right * tx).cross(up), at),
        plane((fwd - up * ty).cross(right), at),
        plane((fwd + up * ty).cross(right) * -1.0, at),
    ];
    // Each side plane must have the view's middle in front.
    let planes = planes.map(|p| {
        let mid = at + fwd * 100.0;
        if p[0] * mid.x + p[1] * mid.y + p[2] * mid.z + p[3] < 0.0 { [-p[0], -p[1], -p[2], -p[3]] } else { p }
    });
    // A negative near walks as the game does: the start cell tests portals
    // against the plane through the eye, its children keep the near plane.
    let mut start_planes = planes;
    if near < 0.0 {
        start_planes[0] = dpvs_t6::facing_plane(fwd.to_array(), eye);
    }
    let params = dpvs_t6::WalkParams {
        eye,
        view_plane: dpvs_t6::facing_plane(fwd.to_array(), eye),
        planes: &start_planes,
        near: Some(planes[0]),
        far: Some(planes[1]),
        walk_limit: 0,
        bevels: None,
        force_invisible: None,
        occluders: &[],
    };
    let Some(start) = world.cell_holding(eye) else {
        println!("walk: {eye:?} is outside every cell");
        return;
    };
    let mut scratch = dpvs_t6::TraversalScratchpad::default();
    let tally = dpvs_t6::visit_portals(&world, start, &params, &mut scratch);
    let seen: Vec<usize> = (0..cells.len()).filter(|&c| scratch.cell_visible(c)).collect();
    println!("walk: eye {eye:?} yaw {yaw} near {near}: in cell {start}, sees {seen:?} (clip skips {})", tally.skipped_by_clip_total);
}

fn run(path: &std::path::Path) -> Result<(), String> {
    let capture = capture_zone(path).map_err(|e| format!("t6world: {e}"))?;
    let world = capture
        .world
        .as_ref()
        .ok_or_else(|| format!("t6world: {} has no GfxWorld", path.to_string_lossy()))?;
    report(&capture, world);
    if let Some(c) = &capture.clip {
        let sided = c.brushes.iter().filter(|b| !b.sides.is_empty()).count();
        println!(
            "clip {}: {} brushes ({} with extra sides), {} materials, {} verts, {} tris, {} partitions, {} aabb nodes, {} static models",
            c.name,
            c.brushes.len(),
            sided,
            c.materials.len(),
            c.vertex_points.len(),
            c.tri_index_list.len() / 3,
            c.partitions.len(),
            c.box_trees.len(),
            c.static_model_count
        );
    }
    // T6WORLD_CELLS=1: every portal, sampled just past and just short of its
    // plane inside its winding, against the cell tree; =x,y,z: the cell
    // there and its portals.
    if let Ok(spec) = std::env::var("T6WORLD_CELLS") {
        print_cell_check(world, &spec);
    }
    // T6WORLD_WALK=x,y,z,yaw,near: the cells the portal walk reaches from
    // there, level, looking along yaw, with the near plane that far ahead
    // (negative: the start cell uses the eye plane, as the game does).
    if let Ok(spec) = std::env::var("T6WORLD_WALK") {
        print_walk(world, &spec);
    }
    // T6WORLD_DYN=1: the clipmap's loose and breakable props.
    if std::env::var_os("T6WORLD_DYN").is_some()
        && let Some(c) = &capture.clip
    {
        for (i, d) in c.debris_pieces.iter().enumerate() {
            println!(
                "dyn {i} kind={} model={} destroyed={} fx={} snd={:#x} phys={} hp={} flags={:#x} contents={:#x} brush={}/{} origin={:?} quat={:?}",
                d.kind, d.model, d.destroyed_model, d.destroy_effect, d.destroy_sound, d.physics_template,
                d.health, d.flags, d.contents, d.solid_model, d.physics_solid_mesh, d.origin, d.rotation
            );
        }
    }
    // T6WORLD_FOLIAGE=1: the placed models that wear foliage-like technique
    // sets (tree canopies, alpha-tested or blended lit leaves) or have a
    // plant-like name: per model and technique set, how many and where.
    if std::env::var_os("T6WORLD_FOLIAGE").is_some() {
        let plant = ["tree", "foliage", "leaf", "leaves", "bush", "grass", "ivy", "plant", "palm", "fern", "shrub", "vine", "hedge", "pine", "branch", "flower", "weed"];
        let mut rows: BTreeMap<(String, String), (usize, Vec<[f32; 3]>)> = BTreeMap::new();
        for sm in &world.prop_set {
            let Some(k) = sm.model else { continue };
            let Some(m) = capture.xmodels.get(k.index) else { continue };
            let lname = m.name.to_ascii_lowercase();
            for mk in m.materials.iter().flatten() {
                let tech = capture
                    .material(*mk)
                    .and_then(|mat| mat.method_set)
                    .and_then(|t| capture.technique_sets.get(t.index))
                    .map_or_else(|| "?".to_owned(), |t| t.name.clone());
                let leafy = tech.contains("treecanopy")
                    || tech.split('_').any(|t| t.starts_with("t0c") || t.starts_with("b0c"))
                    || plant.iter().any(|p| lname.contains(p));
                if !leafy {
                    continue;
                }
                let e = rows.entry((m.name.clone(), tech)).or_default();
                e.0 += 1;
                if e.1.len() < 3 {
                    e.1.push(sm.origin);
                }
            }
        }
        for ((model, tech), (n, at)) in &rows {
            let at: Vec<String> = at.iter().map(|p| format!("{:.0},{:.0},{:.0}", p[0], p[1], p[2])).collect();
            println!("foliage {n:>4} {model} | {tech} | {}", at.join(" "));
        }
        // T6WORLD_FOLIAGE=all: also every canopy placement, and the world
        // surfaces that wear a canopy technique set.
        if std::env::var("T6WORLD_FOLIAGE").is_ok_and(|v| v == "all") {
            let canopy = |mk: &asset_t6::AssetIdentity| {
                capture
                    .material(*mk)
                    .and_then(|mat| mat.method_set)
                    .and_then(|t| capture.technique_sets.get(t.index))
                    .is_some_and(|t| t.name.contains("treecanopy"))
            };
            for sm in &world.prop_set {
                let Some(m) = sm.model.and_then(|k| capture.xmodels.get(k.index)) else { continue };
                if m.materials.iter().flatten().any(canopy) {
                    println!(
                        "canopy {:.0} {:.0} {:.0} {:.0} {}",
                        sm.origin[0], sm.origin[1], sm.origin[2], m.radius * sm.scale, m.name
                    );
                }
            }
            for surf in &world.surfaces {
                if surf.material.as_ref().is_some_and(canopy) {
                    println!(
                        "worldcanopy {:.0} {:.0} {:.0} .. {:.0} {:.0} {:.0}",
                        surf.bounds[0][0], surf.bounds[0][1], surf.bounds[0][2], surf.bounds[1][0], surf.bounds[1][1], surf.bounds[1][2]
                    );
                }
            }
        }
    }
    // T6WORLD_NEAR=x,y,z,r: the placed models within r of the point, with
    // each material's technique set.
    // T6WORLD_XMODEL=<name part>: each matching model's materials and
    // technique sets (the sky model is not placed, so NEAR misses it).
    if let Ok(want) = std::env::var("T6WORLD_XMODEL") {
        for m in capture.xmodels.iter().filter(|m| m.name.contains(&want)) {
            for mk in m.materials.iter().flatten() {
                let mat = capture.material(*mk);
                let tech = mat
                    .and_then(|mat| mat.method_set)
                    .and_then(|t| capture.technique_sets.get(t.index))
                    .map_or("?", |t| t.name.as_str());
                println!("xmodel {} | {}={tech}", m.name, mat.map_or("?", |mat| mat.name.as_str()));
            }
        }
    }
    if let Ok(spec) = std::env::var("T6WORLD_NEAR") {
        if let Some([x, y, z, r]) = point_and_radius(&spec) {
            for sm in &world.prop_set {
                let d = ((sm.origin[0] - x).powi(2) + (sm.origin[1] - y).powi(2) + (sm.origin[2] - z).powi(2)).sqrt();
                if d > r {
                    continue;
                }
                let Some(m) = sm.model.and_then(|k| capture.xmodels.get(k.index)) else { continue };
                let mats: Vec<String> = m
                    .materials
                    .iter()
                    .flatten()
                    .map(|mk| {
                        let mat = capture.material(*mk);
                        let tech = mat
                            .and_then(|mat| mat.method_set)
                            .and_then(|t| capture.technique_sets.get(t.index))
                            .map_or("?", |t| t.name.as_str());
                        format!("{}={tech}", mat.map_or("?", |mat| mat.name.as_str()))
                    })
                    .collect();
                println!(
                    "near {d:.0} {} at {:.0},{:.0},{:.0} scale {:.2} | {}",
                    m.name, sm.origin[0], sm.origin[1], sm.origin[2], sm.scale, mats.join(" ")
                );
            }
        }
    }
    // T6WORLD_SURF=x,y,z,r: the world surfaces whose bounds come within r of
    // the point, with material, technique set, flags and probe.
    if let Ok(spec) = std::env::var("T6WORLD_SURF") {
        if let Some([x, y, z, r]) = point_and_radius(&spec) {
            for (i, surf) in world.surfaces.iter().enumerate() {
                let (lo, hi) = (surf.bounds[0], surf.bounds[1]);
                let d = (0..3)
                    .map(|k| ([x, y, z][k] - [x, y, z][k].clamp(lo[k], hi[k])).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if d > r {
                    continue;
                }
                let mat = surf.material.and_then(|mk| capture.material(mk));
                let tech = mat
                    .and_then(|mat| mat.method_set)
                    .and_then(|t| capture.technique_sets.get(t.index))
                    .map_or("?", |t| t.name.as_str());
                println!(
                    "surf {i} d {d:.0} tris {} flags {:#x} lmap {} probe {} {:.0},{:.0},{:.0} .. {:.0},{:.0},{:.0} | {}={tech}",
                    surf.tri_tally, surf.flags, surf.lightbake_index, surf.env_probe_index,
                    lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], mat.map_or("?", |mat| mat.name.as_str())
                );
            }
        }
    }
    // T6WORLD_GLASS=1: the breakable windows (where each stands, which way it faces).
    if std::env::var_os("T6WORLD_GLASS").is_some() {
        for (i, g) in capture.glasses.iter().enumerate() {
            println!(
                "glass {i} def={} hp={} brush={} min={:?} max={:?} normal={:?} index={} origin={:?} angles={:?} outline={}",
                g.definition_name, g.hit_point_ceiling, g.solid_model, g.absmin, g.absmax, g.axis[2], g.index, g.origin, g.angles,
                g.outline.len()
            );
            // T6WORLD_GLASS=2: also what its shards are cut from (a pane replays offline).
            if std::env::var("T6WORLD_GLASS").as_deref() == Ok("2") {
                println!(
                    "glassshape {i} outline={:?} axis={:?} outline_origin={:?} planar={} thickness={} uv_scale={} shard={}..{} life={} max_shards={} pools={}/{}",
                    g.outline, g.axis, g.outline_origin, g.is_planar, g.pane_thickness, g.uv_scale, g.min_shard_size,
                    g.max_shard_size, g.shard_life_probability, g.max_shards, g.pool_shards, g.pool_physics
                );
            }
        }
    }
    // T6WORLD_GLASSMAT=1: the materials the windows name and wear (in this zone or only
    // referenced, with or without a technique set), and their brush surfaces' own.
    if std::env::var_os("T6WORLD_GLASSMAT").is_some() {
        for m in capture.materials.iter().filter(|m| m.name.contains("glass")) {
            println!(
                "glassmat {:?} techset={} textures={} sort={}",
                m.name,
                m.method_set.is_some(),
                m.textures.len(),
                m.sort_rank
            );
        }
        if let Some(w) = &capture.world {
            let mut own: BTreeMap<String, usize> = BTreeMap::default();
            for g in &capture.glasses {
                let Some(&(lo, count, _)) = w.solid_models.get(g.solid_model as usize) else {
                    continue;
                };
                for s in w.surfaces.iter().skip(lo as usize).take(count as usize) {
                    let name = s
                        .material
                        .and_then(|k| capture.material(k))
                        .map_or_else(|| "(none)".to_owned(), |m| m.name.clone());
                    *own.entry(name).or_default() += 1;
                }
            }
            println!("glass brush surfaces' own materials: {own:?}");
        }
    }
    // T6WORLD_GLASSCLIP=1: each window's clip brushes as the game links them.
    if std::env::var_os("T6WORLD_GLASSCLIP").is_some() {
        print_glass_clip(&capture);
    }
    // T6WORLD_ENTS=<file>: the entity strings, written out whole.
    if let Some(out) = std::env::var_os("T6WORLD_ENTS") {
        let _ = std::fs::write(out, capture.map_ents.concat());
    }
    for (i, ents) in capture.map_ents.iter().enumerate() {
        println!("map ents {i}: {} chars", ents.len());
        for (k, n) in class_counts(ents) {
            println!("   {n:>5}  {k}");
        }
        // T6WORLD_ENTS=<text>: print every entity block that contains it.
        if let Ok(want) = std::env::var("T6WORLD_ENTS") {
            blocks_containing(ents, &want).iter().for_each(|block| println!("{block}"));
        }
    }
    Ok(())
}

fn report(capture: &asset_t6::ZoneCapture, w: &WorldRef) {
    println!(
        "world {}: bounds {:?} .. {:?}; sky model `{}`",
        w.name, w.lo_corner, w.hi_corner, w.sky_box_model
    );
    println!(
        "vertices {}: vd0 {} bytes ({:.3} per vertex), vd1 {} bytes ({:.3} per vertex); indices {}",
        w.vertex_count,
        w.vd0.len(),
        w.vd0.len() as f64 / w.vertex_count.max(1) as f64,
        w.vd1.len(),
        w.vd1.len() as f64 / w.vertex_count.max(1) as f64,
        w.indices.len()
    );
    let triangle_list: usize = w.surfaces.iter().map(|s| s.tri_tally as usize).sum();
    let with_mat = w.surfaces.iter().filter(|s| s.material.is_some()).count();
    println!(
        "surfaces {} ({} triangles), {} with a resolved material; static models {} ({} resolved); lightmaps {}",
        w.surfaces.len(),
        triangle_list,
        with_mat,
        w.prop_set.len(),
        w.prop_set.iter().filter(|m| m.model.is_some()).count(),
        w.light_atlases.len()
    );
    println!(
        "capture: {} materials, {} techsets, {} xmodels, {} images; unresolved asset refs {}",
        capture.materials.len(),
        capture.technique_sets.len(),
        capture.xmodels.len(),
        capture.images.len(),
        capture.unresolved_refs
    );
    // T6WORLD_SMODEL=<substring>: every drawn static model placement whose
    // model name contains it (place, scale, flags, cull distance).
    if let Ok(want) = std::env::var("T6WORLD_SMODEL") {
        for (i, sm) in w.prop_set.iter().enumerate() {
            let name = sm.model.map_or("<none>", |k| capture.xmodels[k.index].name.as_str());
            if name.contains(want.as_str()) {
                println!(
                    "smodel {i}: {name} at {:?} scale {} axis {:?} flags {:#x} cull {} light idx {} colors {}",
                    sm.origin, sm.scale, sm.axis, sm.flags, sm.hide_dist, sm.key_light_index, sm.vertex_color_slot
                );
            }
        }
    }

    // vd0: which stride and position offset put every surface's vertices
    // inside its bounds?
    let s = &w.surfaces;
    for stride in [12usize, 16, 20, 24, 28, 32, 36, 40, 44, 48] {
        for pos_off in [0usize, 4, 8, 12] {
            if pos_off + 12 > stride {
                continue;
            }
            let (mut inside, mut total) = (0usize, 0usize);
            for surf in s.iter().take(400) {
                for v in 0..surf.vertex_count as usize {
                    let base = surf.first_vertex as usize + v;
                    let at = base * stride + pos_off;
                    let (Some(x), Some(y), Some(z)) = (
                        float_from_bytes(&w.vd0, at),
                        float_from_bytes(&w.vd0, at + 4),
                        float_from_bytes(&w.vd0, at + 8),
                    ) else {
                        continue;
                    };
                    total += 1;
                    let eps = 0.5;
                    if (0..3).all(|i| {
                        let c = [x, y, z][i];
                        c >= surf.lo_corner[i] - eps && c <= surf.hi_corner[i] + eps
                    }) {
                        inside += 1;
                    }
                }
            }
            if total > 0 && inside * 100 / total >= 50 {
                println!(
                    "vd0 stride {stride} position at +{pos_off}: {inside}/{total} vertices inside their surface bounds (first_vertex indexing)"
                );
            }
        }
    }
    // Same test, addressing by vertexDataOffset0 instead of firstVertex.
    for pos_off in [0usize, 4, 8, 12] {
        let (mut inside, mut total) = (0usize, 0usize);
        for surf in s.iter().take(400) {
            let at = surf.vertex_data_offset0 as usize + pos_off;
            let (Some(x), Some(y), Some(z)) = (
                float_from_bytes(&w.vd0, at),
                float_from_bytes(&w.vd0, at + 4),
                float_from_bytes(&w.vd0, at + 8),
            ) else {
                continue;
            };
            total += 1;
            if (0..3).all(|i| {
                let c = [x, y, z][i];
                c >= surf.lo_corner[i] - 0.5 && c <= surf.hi_corner[i] + 0.5
            }) {
                inside += 1;
            }
        }
        if total > 0 {
            println!(
                "vd0 by vertexDataOffset0, position at +{pos_off}: {inside}/{total} first vertices inside bounds"
            );
        }
    }
    let mut offs: BTreeMap<i32, (usize, i64, i64)> = BTreeMap::new();
    for surf in s {
        let e = offs
            .entry(surf.vertex_data_offset0)
            .or_insert((0, i64::MAX, 0));
        e.0 += 1;
        e.1 = e.1.min(surf.first_vertex as i64);
        e.2 = e.2.max(surf.first_vertex as i64 + surf.vertex_count as i64);
    }
    println!("vertexDataOffset0 groups: {}", offs.len());
    for (off, (n, lo, hi)) in offs.iter().take(12) {
        println!("   offset {off:>9}: {n:>5} surfaces, firstVertex {lo}..{hi}");
    }
    for (i, surf) in s.iter().enumerate().take(8) {
        println!(
            "   surf {i}: off0 {} off1 {} first {} n {} tris {} base {} flags {:#x} lmap {}",
            surf.vertex_data_offset0,
            surf.vertex_data_offset1,
            surf.first_vertex,
            surf.vertex_count,
            surf.tri_tally,
            surf.base_index,
            surf.flags,
            surf.lightbake_index
        );
    }
    // Hypothesis: vertex = vd0[vertexDataOffset0 + index * stride], index
    // from the surface's own triangles (u16, relative to its group).
    for stride in [32usize, 36, 40] {
        for pos_off in [0usize, 4, 8, 12, 16, 20, 24] {
            if pos_off + 12 > stride {
                continue;
            }
            let (mut inside, mut total, mut out_of_range) = (0usize, 0usize, 0usize);
            let mut zero_bounds = 0usize;
            for surf in s {
                let (lo, hi) = (surf.bounds[0], surf.bounds[1]);
                if lo == [0.0; 3] && hi == [0.0; 3] {
                    zero_bounds += 1;
                    continue;
                }
                let first = surf.base_index as usize;
                let count = surf.tri_tally as usize * 3;
                for &idx in w.indices.get(first..first + count).unwrap_or(&[]) {
                    let at = surf.vertex_data_offset0 as usize + idx as usize * stride + pos_off;
                    let (Some(x), Some(y), Some(z)) = (
                        float_from_bytes(&w.vd0, at),
                        float_from_bytes(&w.vd0, at + 4),
                        float_from_bytes(&w.vd0, at + 8),
                    ) else {
                        out_of_range += 1;
                        continue;
                    };
                    total += 1;
                    if (0..3).all(|i| {
                        let c = [x, y, z][i];
                        c >= lo[i] - 0.5 && c <= hi[i] + 0.5
                    }) {
                        inside += 1;
                    }
                }
            }
            if total > 0 && inside * 100 / total >= 20 {
                println!(
                    "vd0 group-relative, stride {stride}, position at +{pos_off}: {inside}/{total} triangle corners inside GfxSurface bounds ({out_of_range} past the end, {zero_bounds} surfaces without bounds)"
                );
            }
        }
    }
    // Which packed field is the normal, and how is it encoded? Compare each
    // candidate decoding with the face normal of the triangle it belongs to.
    fn sext10(v: u32) -> f32 {
        let v = (v & 0x3ff) as i32;
        let v = if v >= 512 { v - 1024 } else { v };
        v as f32 / 511.0
    }
    let decoders: [(&str, fn(u32) -> [f32; 3]); 9] = [
        ("10:10:10 signed zyx", |p| {
            [sext10(p >> 20), sext10(p >> 10), sext10(p)]
        }),
        ("octahedral snorm16", |p| {
            let x = (p & 0xffff) as u16 as i16 as f32 / 32767.0;
            let y = (p >> 16) as u16 as i16 as f32 / 32767.0;
            let z = 1.0 - x.abs() - y.abs();
            let (x, y) = if z < 0.0 {
                ((1.0 - y.abs()) * x.signum(), (1.0 - x.abs()) * y.signum())
            } else {
                (x, y)
            };
            let l = (x * x + y * y + z * z).sqrt().max(1e-9);
            [x / l, y / l, z / l]
        }),
        ("octahedral unorm16", |p| {
            let x = (p & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
            let y = (p >> 16) as f32 / 65535.0 * 2.0 - 1.0;
            let z = 1.0 - x.abs() - y.abs();
            let (x, y) = if z < 0.0 {
                ((1.0 - y.abs()) * x.signum(), (1.0 - x.abs()) * y.signum())
            } else {
                (x, y)
            };
            let l = (x * x + y * y + z * z).sqrt().max(1e-9);
            [x / l, y / l, z / l]
        }),
        ("11:11:10 signed", |p| {
            let s11 = |v: u32| {
                let v = (v & 0x7ff) as i32;
                (if v >= 1024 { v - 2048 } else { v }) as f32 / 1023.0
            };
            let s10 = |v: u32| sext10(v);
            [s11(p), s11(p >> 11), s10(p >> 22)]
        }),
        ("bytes+scale", |p| {
            let b = p.to_le_bytes();
            let sc = (b[3] as f32 + 192.0) / 32385.0;
            [
                (b[0] as f32 - 127.0) * sc,
                (b[1] as f32 - 127.0) * sc,
                (b[2] as f32 - 127.0) * sc,
            ]
        }),
        ("10:10:10 signed", |p| {
            [sext10(p), sext10(p >> 10), sext10(p >> 20)]
        }),
        ("10:10:10 unsigned", |p| {
            let f = |v: u32| (v & 0x3ff) as f32 / 1023.0 * 2.0 - 1.0;
            [f(p), f(p >> 10), f(p >> 20)]
        }),
        ("8:8:8 snorm", |p| {
            let b = p.to_le_bytes();
            [
                b[0] as i8 as f32 / 127.0,
                b[1] as i8 as f32 / 127.0,
                b[2] as i8 as f32 / 127.0,
            ]
        }),
        ("8:8:8 unorm", |p| {
            let b = p.to_le_bytes();
            [
                b[0] as f32 / 127.5 - 1.0,
                b[1] as f32 / 127.5 - 1.0,
                b[2] as f32 / 127.5 - 1.0,
            ]
        }),
    ];
    let read_pos = |at: usize| -> [f32; 3] {
        [
            float_from_bytes(&w.vd0, at).unwrap_or(0.0),
            float_from_bytes(&w.vd0, at + 4).unwrap_or(0.0),
            float_from_bytes(&w.vd0, at + 8).unwrap_or(0.0),
        ]
    };
    for field in [20usize, 24, 28, 32] {
        for (name, dec) in decoders {
            let (mut n, mut sum_dot, mut sum_abs, mut sum_len) = (0usize, 0f64, 0f64, 0f64);
            for surf in s.iter().take(3000) {
                let first = surf.base_index as usize;
                for t in 0..surf.tri_tally as usize {
                    let idx = &w.indices[first + t * 3..first + t * 3 + 3];
                    let at = |i: u16| surf.vertex_data_offset0 as usize + i as usize * 36;
                    let (a, b, c) = (
                        read_pos(at(idx[0])),
                        read_pos(at(idx[1])),
                        read_pos(at(idx[2])),
                    );
                    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let f = [
                        e1[1] * e2[2] - e1[2] * e2[1],
                        e1[2] * e2[0] - e1[0] * e2[2],
                        e1[0] * e2[1] - e1[1] * e2[0],
                    ];
                    let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
                    if fl < 1e-3 {
                        continue;
                    }
                    let f = [f[0] / fl, f[1] / fl, f[2] / fl];
                    let raw = u32::from_le_bytes(
                        w.vd0[at(idx[0]) + field..at(idx[0]) + field + 4]
                            .try_into()
                            .unwrap(),
                    );
                    let v = dec(raw);
                    let vl = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    if vl < 1e-6 {
                        continue;
                    }
                    let d = (v[0] * f[0] + v[1] * f[1] + v[2] * f[2]) / vl;
                    n += 1;
                    sum_dot += d as f64;
                    sum_abs += d.abs() as f64;
                    sum_len += vl as f64;
                }
            }
            if n > 0 && (sum_abs / n as f64 > 0.8 || (sum_len / n as f64 - 1.0).abs() < 0.02) {
                println!(
                    "field +{field} as {name:<18}: mean dot with face normal {:+.3}, mean |dot| {:.3}, mean length {:.3} ({n} triangles)",
                    sum_dot / n as f64,
                    sum_abs / n as f64,
                    sum_len / n as f64
                );
            }
        }
    }
    // Which half of the uv_semantic pair at +20 is u? The tangent (+28) runs
    // along increasing u: compare it with dP/du solved per triangle.
    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0f32 } else { 1.0 };
        let e = ((bits >> 10) & 0x1f) as i32;
        let m = (bits & 0x3ff) as f32;
        if e == 0 {
            sign * m * 2f32.powi(-24)
        } else if e == 31 {
            f32::NAN
        } else {
            sign * (1.0 + m / 1024.0) * 2f32.powi(e - 15)
        }
    }
    for (label, lo_is_u) in [("u = low half", true), ("u = high half", false)] {
        let (mut n, mut agree) = (0usize, 0f64);
        for surf in s.iter().take(3000) {
            let first = surf.base_index as usize;
            for t in 0..surf.tri_tally as usize {
                let idx = &w.indices[first + t * 3..first + t * 3 + 3];
                let at = |i: u16| surf.vertex_data_offset0 as usize + i as usize * 36;
                let p: Vec<[f32; 3]> = idx.iter().map(|&i| read_pos(at(i))).collect();
                let uv: Vec<[f32; 2]> = idx
                    .iter()
                    .map(|&i| {
                        let raw =
                            u32::from_le_bytes(w.vd0[at(i) + 20..at(i) + 24].try_into().unwrap());
                        let (lo, hi) = (half(raw as u16), half((raw >> 16) as u16));
                        if lo_is_u { [lo, hi] } else { [hi, lo] }
                    })
                    .collect();
                let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
                let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
                let (du1, dv1) = (uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]);
                let (du2, dv2) = (uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]);
                let det = du1 * dv2 - du2 * dv1;
                if det.abs() < 1e-8 {
                    continue;
                }
                let r = 1.0 / det;
                let dpdu = [
                    (e1[0] * dv2 - e2[0] * dv1) * r,
                    (e1[1] * dv2 - e2[1] * dv1) * r,
                    (e1[2] * dv2 - e2[2] * dv1) * r,
                ];
                let l = (dpdu[0] * dpdu[0] + dpdu[1] * dpdu[1] + dpdu[2] * dpdu[2]).sqrt();
                if !l.is_finite() || l < 1e-6 {
                    continue;
                }
                let raw =
                    u32::from_le_bytes(w.vd0[at(idx[0]) + 28..at(idx[0]) + 32].try_into().unwrap());
                let tan = [sext10(raw), sext10(raw >> 10), sext10(raw >> 20)];
                let d = (tan[0] * dpdu[0] + tan[1] * dpdu[1] + tan[2] * dpdu[2]) / l;
                n += 1;
                agree += d as f64;
            }
        }
        println!(
            "texcoord {label}: mean cos(tangent, dP/du) {:+.3} over {n} triangles",
            agree / n.max(1) as f64
        );
    }
    for (si, surf) in [(0usize, &s[0]), (6, &s[6])] {
        println!(
            "surface {si}: bounds {:?} .. {:?}, group at vd0 {:#x}, vd1 {:#x}",
            surf.lo_corner, surf.hi_corner, surf.vertex_data_offset0, surf.vertex_data_offset1
        );
        for v in 0..3usize {
            let at = surf.vertex_data_offset0 as usize + v * 36;
            let rec = &w.vd0[at..at + 36];
            let floats: Vec<String> = rec
                .chunks_exact(4)
                .map(|c| format!("{:.3}", f32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                .collect();
            let hex: Vec<String> = rec
                .chunks_exact(4)
                .map(|c| format!("{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3]))
                .collect();
            println!("   v{v} f32: {}", floats.join(" "));
            println!("   v{v} hex: {}", hex.join(" "));
            let at1 = surf.vertex_data_offset1 as usize + v * 8;
            if let Some(r1) = w.vd1.get(at1..at1 + 8) {
                println!("   v{v} vd1: {:02x?}", r1);
            }
        }
    }
    let first = &s[0];
    println!(
        "surface 0: first_vertex {} vertex_data_offset0 {} vertex_data_offset1 {} vertices {} tris {} base_index {}",
        first.first_vertex,
        first.vertex_data_offset0,
        first.vertex_data_offset1,
        first.vertex_count,
        first.tri_tally,
        first.base_index
    );
    let mut sets: BTreeMap<String, usize> = BTreeMap::default();
    for surf in s {
        let name = surf
            .material
            .and_then(|k| capture.material(k))
            .and_then(|m| m.method_set)
            .and_then(|k| capture.technique_sets.get(k.index))
            .map_or_else(
                || "?".to_owned(),
                |t| format!("{} (vf {})", t.name, t.level_vertex_format),
            );
        *sets.entry(name).or_default() += 1;
    }
    println!("world surfaces by technique set: {}", sets.len());
    let mut ordered: Vec<_> = sets.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, n) in ordered.iter().take(25) {
        println!("   {n:>5}  {name}");
    }
    // T6WORLD_SURFS=a..b: list those surfaces (the ids the drawsurf
    // diagnostic prints) with their material, technique set and bounds.
    if let Some((a, b)) = std::env::var("T6WORLD_SURFS")
        .ok()
        .and_then(|r| {
            r.split_once("..")
                .map(|(a, b)| (a.parse::<usize>(), b.parse::<usize>()))
        })
        .and_then(|(a, b)| Some((a.ok()?, b.ok()?)))
    {
        for (i, surf) in s.iter().enumerate().take(b + 1).skip(a) {
            let mat = surf.material.and_then(|k| capture.material(k));
            let set = mat
                .and_then(|m| m.method_set)
                .and_then(|k| capture.technique_sets.get(k.index))
                .map_or("?", |t| t.name.as_str());
            // Its first triangle corner's position.
            let corner = w.indices.get(surf.base_index as usize).and_then(|&v| {
                let at = surf.vertex_data_offset0 as usize + v as usize * 36;
                Some([
                    float_from_bytes(&w.vd0, at)?,
                    float_from_bytes(&w.vd0, at + 4)?,
                    float_from_bytes(&w.vd0, at + 8)?,
                ])
            });
            println!(
                "surf {i}: {} [{set}] tris {} bounds {:?} .. {:?} corner {corner:?}",
                mat.map_or("?", |m| m.name.as_str()),
                surf.tri_tally,
                surf.lo_corner,
                surf.hi_corner
            );
            // T6WORLD_FACING=x,y,z: how many of its triangles have the
            // point on the side of cross(b-a, c-a) (the same winding as a
            // floor seen from above, when the floor counts "toward").
            if let Some(p) = std::env::var("T6WORLD_FACING").ok().and_then(|v| {
                let n: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                (n.len() == 3).then(|| [n[0], n[1], n[2]])
            }) {
                let pos = |v: u16| {
                    let at = surf.vertex_data_offset0 as usize + v as usize * 36;
                    Some([
                        float_from_bytes(&w.vd0, at)?,
                        float_from_bytes(&w.vd0, at + 4)?,
                        float_from_bytes(&w.vd0, at + 8)?,
                    ])
                };
                let (mut toward, mut away) = (0u32, 0u32);
                for t in 0..surf.tri_tally as usize {
                    let at = surf.base_index as usize + t * 3;
                    let (Some(a), Some(b), Some(c)) = (
                        w.indices.get(at).and_then(|&v| pos(v)),
                        w.indices.get(at + 1).and_then(|&v| pos(v)),
                        w.indices.get(at + 2).and_then(|&v| pos(v)),
                    ) else {
                        continue;
                    };
                    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let n = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let d = n[0] * (p[0] - a[0]) + n[1] * (p[1] - a[1]) + n[2] * (p[2] - a[2]);
                    if d > 0.0 {
                        toward += 1;
                    } else {
                        away += 1;
                    }
                }
                println!("   facing {p:?}: toward {toward} away {away}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{point_and_radius, class_counts, blocks_containing, leaf_index, float_from_bytes};

    #[test]
    fn a_probe_needs_exactly_four_numbers() {
        assert_eq!(point_and_radius("1, 2,3 ,4.5"), Some([1.0, 2.0, 3.0, 4.5]));
        assert_eq!(point_and_radius("1,2,3"), None);
        assert_eq!(point_and_radius("1,2,3,4,5"), None);
        assert_eq!(point_and_radius("1,x,3,4"), None);
    }

    #[test]
    fn classes_are_counted_per_name() {
        let ents = "{\n\"classname\" \"worldspawn\"\n}\n{\n  \"classname\" \"light\"\n}\n{\n\"classname\" \"light\"\n\"origin\" \"1 2 3\"\n}";
        let counts: Vec<_> = class_counts(ents).into_iter().collect();
        assert_eq!(counts, [("light", 2), ("worldspawn", 1)]);
    }

    #[test]
    fn blocks_come_out_whole_with_a_newline_per_line() {
        let ents = "{\n\"a\" \"1\"\n}\n{\n\"a\" \"2\"\n\"b\" \"9\"\n}\n";
        assert_eq!(blocks_containing(ents, "\"b\""), ["{\n\"a\" \"2\"\n\"b\" \"9\"\n}\n"]);
        assert_eq!(blocks_containing(ents, "\"a\"").len(), 2);
        assert!(blocks_containing(ents, "zzz").is_empty());
    }

    #[test]
    fn a_descent_ends_at_the_complemented_leaf() {
        // node 0 -> node 1 -> leaf 4 (stored as !4).
        let tree = |n: i32| match n {
            0 => Some(1),
            1 => Some(!4),
            _ => None,
        };
        assert_eq!(leaf_index(tree), 4);
        assert_eq!(leaf_index(|n| (n == 0).then_some(7)), usize::MAX);
        assert_eq!(leaf_index(|_| None), usize::MAX);
    }

    #[test]
    fn floats_are_read_little_endian_inside_bounds_only() {
        let bytes = 1.5f32.to_le_bytes();
        assert_eq!(float_from_bytes(&bytes, 0), Some(1.5));
        assert_eq!(float_from_bytes(&bytes, 1), None);
    }
}
