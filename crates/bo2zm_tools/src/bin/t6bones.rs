//! bo2zm: print a T6 model's bones and an animation's tracks/notetracks.
//! `t6bones <zone.ff> <model or anim name>...`
#[allow(dead_code)]
#[path = "../../../render_anim/src/motion/t6_loop_seam.rs"]
mod t6_loop_seam;

/// Dot product of two quaternions.
fn quat_dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// The rigid bone each vertex belongs to, from the surface's (bone offset, run, ..) lists.
fn rigid_bone_per_vertex(lists: &[(u16, u16, u16, u16)]) -> Vec<u16> {
    lists
        .iter()
        .flat_map(|&(offset, run, _, _)| vec![offset / 64; usize::from(run)])
        .collect()
}

/// One "surface bone x y z" line per vertex of 32-byte rows (at most `count`).
fn vertex_rows(surface: usize, vertex_points: &[u8], vertex_total: usize, bone_of: &[u16]) -> Vec<String> {
    let coord = |row: &[u8], at: usize| f32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]]);
    vertex_points
        .chunks_exact(32)
        .take(vertex_total)
        .enumerate()
        .map(|(k, row)| {
            let bone = bone_of.get(k).copied().unwrap_or(0);
            format!("{surface} {bone} {} {} {}", coord(row, 0), coord(row, 4), coord(row, 8))
        })
        .collect()
}

/// One "t surface a b c" line per triangle.
fn triangle_rows(surface: usize, indices: &[u16]) -> Vec<String> {
    indices
        .chunks_exact(3)
        .map(|t| format!("t {surface} {} {} {}", t[0], t[1], t[2]))
        .collect()
}

/// Appends the lines to the file, creating it; a failed write is not reported.
fn append_lines(path: &str, lines: &[String]) {
    use std::io::Write;
    let mut how = std::fs::OpenOptions::new();
    how.create(true).append(true);
    let mut file = how.open(path).expect("verts file");
    for entry in lines {
        writeln!(file, "{entry}").ok();
    }
}

/// Decodes the clip, saying `failure` plus the error when it will not.
fn decode_or_say(parts: &xmodel_runtime::StoredClipParts, failure: &str) -> Option<xmodel_runtime::SkeletalClip> {
    xmodel_runtime::SkeletalClip::from_parts(parts)
        .inspect_err(|e| println!("  {failure}{e:?}"))
        .ok()
}

/// Frames whose rotation sits on the far side of the next frame's (dot < 0) on one track.
fn flip_frames(clip: &xmodel_runtime::SkeletalClip, track_ix: usize, frame_total: u16, framerate: f32) -> Vec<u16> {
    let rate = framerate.max(1.0);
    let rotation_at = |f: u16| clip.lane_at(track_ix, f32::from(f) / rate).and_then(|s| s.rotation);
    (0..frame_total)
        .filter(|&f| matches!((rotation_at(f), rotation_at(f + 1)), (Some(p), Some(q)) if quat_dot(p, q) < 0.0))
        .collect()
}

/// The biggest turn of any bone in one frame as a loop plays, quarter frames apart and
/// across the wrap: (degrees per frame, bone, frame it starts at).
fn biggest_turn(clip: &xmodel_runtime::SkeletalClip) -> (f32, String, f32) {
    let steps = usize::from(clip.frame_total) * 4;
    let at = |t: usize, i: usize| {
        clip.lane_at(t, i as f32 / 4.0 / clip.fps).and_then(|s| s.rotation)
    };
    let mut best = (0.0f32, String::default(), 0.0f32);
    for (t, track) in clip.tracks.iter().enumerate() {
        for i in 0..steps {
            if let (Some(a), Some(b)) = (at(t, i), at(t, (i + 1) % steps)) {
                let dot = quat_dot(a, b).abs().min(1.0);
                let deg = 2.0 * dot.acos().to_degrees() * 4.0;
                if deg > best.0 {
                    best = (deg, track.name.clone(), i as f32 / 4.0);
                }
            }
        }
    }
    best
}

fn main() {
    let mut argv = std::env::args().skip(1);
    let Some(zone) = argv.next() else {
        eprintln!("usage: t6bones <zone.ff> <name>...");
        return;
    };
    let names: Vec<String> = argv.collect();
    let c = asset_t6::capture_zone(std::path::Path::new(&zone)).expect("capture");
    // T6BONES_LIST=1: the names of the zone's models holding one of the given words.
    if std::env::var_os("T6BONES_LIST").is_some() {
        for m in c.xmodels.iter().filter(|m| names.iter().any(|n| m.name.contains(n.as_str()))) {
            println!("{}", m.name);
        }
        return;
    }
    // T6BONES_LOOPCHECK=1: for every looping anim whose name holds one of the
    // given words, the keyed rotation track whose last key strays furthest
    // from its first (degrees). A seamless loop ends where it starts.
    if std::env::var_os("T6BONES_LOOPCHECK").is_some() {
        for a in c.animation_refs.iter().filter(|a| a.looping && names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::SkeletalClip::from_parts(&raw_parts(a)) else {
                println!("{}: decode failed", a.name);
                continue;
            };
            let angle = |f: &[f32; 4], l: &[f32; 4]| {
                let dot = (f[0] * l[0] + f[1] * l[1] + f[2] * l[2] + f[3] * l[3]).abs().min(1.0);
                2.0 * dot.acos().to_degrees()
            };
            let mut worst = (0.0f32, String::default(), 0usize);
            // The biggest turn in one frame: (degrees per frame, track, frame).
            let mut step = (0.0f32, String::default(), 0u16);
            for t in &clip.tracks {
                if let xmodel_runtime::Rotation::FullKeyed(k) | xmodel_runtime::Rotation::HalfKeyed(k) = &t.rotation
                    && let (Some(f), Some(l)) = (k.values.first(), k.values.last())
                {
                    let deg = angle(f, l);
                    if deg > worst.0 {
                        worst = (deg, t.name.clone(), k.values.len());
                    }
                    for i in 1..k.values.len() {
                        let (f0, f1) = match &k.frames {
                            xmodel_runtime::KeyframeIds::Dense => (i as u16 - 1, i as u16),
                            xmodel_runtime::KeyframeIds::Sparse(fr) => (fr[i - 1], fr[i]),
                        };
                        let per = angle(&k.values[i - 1], &k.values[i]) / f32::from(f1.saturating_sub(f0).max(1));
                        if per > step.0 {
                            step = (per, t.name.clone(), f1);
                        }
                    }
                }
            }
            println!(
                "{:50} frames {:3} ends {:5.1} deg ({} {} keys) | biggest step {:5.1} deg/frame on {} into frame {}",
                a.name, a.frame_total, worst.0, worst.1, worst.2, step.0, step.1, step.2
            );
        }
        return;
    }
    // T6BONES_FLIPS=1: for every anim whose name holds one of the given words, each bone whose
    // rotation keys change sign from one key to the next (the clip blends them the long way round).
    if std::env::var_os("T6BONES_FLIPS").is_some() {
        for a in c.animation_refs.iter().filter(|a| names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::SkeletalClip::from_parts(&raw_parts(a)) else {
                println!("{}: decode failed", a.name);
                continue;
            };
            for t in &clip.tracks {
                if let xmodel_runtime::Rotation::HalfKeyed(k) | xmodel_runtime::Rotation::FullKeyed(k) = &t.rotation {
                    let flips: Vec<usize> = k.values.windows(2).enumerate()
                        .filter(|(_key, w)| w[0].iter().zip(w[1].iter()).map(|(px, py)| px * py).sum::<f32>() < 0.0)
                        .map(|(i, _)| i + 1).collect();
                    if !flips.is_empty() {
                        println!("{:44} {:18} {} keys, sign flips at keys {:?}", a.name, t.name, k.values.len(), flips);
                    }
                }
            }
        }
        return;
    }
    // T6BONES_SEAM=1: for every looping anim whose name holds one of the given words, the
    // biggest turn in one frame as the loop plays (wrap included), BO2's own clip and then
    // the engine's smoothed one (render_anim's t6_loop_seam), with the bone it is on.
    if std::env::var_os("T6BONES_SEAM").is_some() {
        for a in c.animation_refs.iter().filter(|a| a.looping && names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::SkeletalClip::from_parts(&raw_parts(a)) else {
                println!("{}: decode failed", a.name);
                continue;
            };
            let smooth = t6_loop_seam::smooth_loop_seam(&clip, None);
            let (b, bt, bf) = biggest_turn(&clip);
            let after = smooth.as_ref().map_or("left as BO2 has it".to_owned(), |s| {
                let (d, t, f) = biggest_turn(s);
                format!("smoothed {d:5.1} deg/frame on {t} at frame {f}")
            });
            println!("{:44} BO2 {b:5.1} deg/frame on {bt:16} at frame {bf:5.2} | {after}", a.name);
            // T6BONES_SEAM=bones: each bone's worst turn and move per frame, BO2's and smoothed.
            if std::env::var("T6BONES_SEAM").as_deref() == Ok("bones") {
                let worst = |cl: &xmodel_runtime::SkeletalClip, t: usize| {
                    let steps = usize::from(cl.frame_total) * 4;
                    let at = |i: usize| cl.lane_at(t, i as f32 / 4.0 / cl.fps);
                    let (mut r, mut rf, mut m, mut mf) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                    for i in 0..steps {
                        if let (Some(px), Some(py)) = (at(i), at((i + 1) % steps)) {
                            if let (Some(a), Some(b)) = (px.rotation, py.rotation) {
                                let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
                                let deg = 2.0 * d.acos().to_degrees() * 4.0;
                                if deg > r { r = deg; rf = i as f32 / 4.0; }
                            }
                            if let (Some(a), Some(b)) = (px.move_track, py.move_track) {
                                let u = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() * 4.0;
                                if u > m { m = u; mf = i as f32 / 4.0; }
                            }
                        }
                    }
                    (r, rf, m, mf)
                };
                for (t, tr) in clip.tracks.iter().enumerate() {
                    let (r, rf, m, mf) = worst(&clip, t);
                    let after = smooth.as_ref().map_or(String::new(), |s| {
                        let (r2, rf2, m2, mf2) = worst(s, t);
                        format!(" | smoothed {r2:6.1} deg @{rf2:5.2} {m2:5.2} u @{mf2:5.2}")
                    });
                    println!("    {:22} {r:6.1} deg @{rf:5.2} {m:5.2} u @{mf:5.2}{after}", tr.name);
                }
            }
            // T6BONES_SEAM=halves: how far each bone turns and moves in each half of the loop
            // (wrap left out), so a held piece shows which arm it moves with.
            if std::env::var("T6BONES_SEAM").as_deref() == Ok("halves") {
                let n = usize::from(clip.frame_total);
                for (t, tr) in clip.tracks.iter().enumerate() {
                    let at = |i: usize| clip.lane_at(t, i as f32 / clip.fps);
                    let (mut turn, mut mv) = ([0.0f32; 2], [0.0f32; 2]);
                    for i in 0..n - 1 {
                        let h = usize::from(i >= n / 2);
                        let (Some(px), Some(py)) = (at(i), at(i + 1)) else { continue };
                        if let (Some(a), Some(b)) = (px.rotation, py.rotation) {
                            let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
                            turn[h] += 2.0 * d.acos().to_degrees();
                        }
                        if let (Some(a), Some(b)) = (px.move_track, py.move_track) {
                            mv[h] += ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                        }
                    }
                    println!("    {:22} turn {:6.1} | {:6.1} deg   move {:6.2} | {:6.2} u", tr.name, turn[0], turn[1], mv[0], mv[1]);
                }
            }
        }
        return;
    }
    for name in &names {
        if let Some(m) = c.xmodels.iter().find(|m| &m.name == name) {
            println!(
                "model {name}: {} bones ({} root), {} surfaces",
                m.joint_labels.len(),
                m.top_joint_total,
                m.lod0.len()
            );
            println!(
                "  mins {:?} maxs {:?} radius {}; collision lod {} with {} surfaces; bone boxes {:?}",
                m.lo_corner,
                m.hi_corner,
                m.radius,
                m.collision_detail_tier,
                m.collision_faces.len(),
                m.bone_info
            );
            for (i, cs) in m.collision_faces.iter().enumerate() {
                println!(
                    "  collision surface {i}: {} tris, mins {:?} maxs {:?}, bone {}, contents {:#x}, flags {:#x}",
                    cs.triangle_list.len(),
                    cs.lo_corner,
                    cs.hi_corner,
                    cs.joint_position,
                    cs.contents,
                    cs.face_property_bits
                );
                let far = cs
                    .triangle_list
                    .iter()
                    .filter(|t| t.iter().any(|r| r[0].abs() > 1000.0 || r[1].abs() > 1000.0 || r[2].abs() > 1000.0))
                    .count();
                println!("  collision surface {i}: {far} tris with a value beyond 1000");
            }
            for (i, s) in m.lod0.iter().enumerate() {
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in s.vertex_points.chunks_exact(32) {
                    for k in 0..3 {
                        let px = f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap());
                        lo[k] = lo[k].min(px);
                        hi[k] = hi[k].max(px);
                    }
                }
                println!("  surface {i} vertices span {lo:?} to {hi:?}");
                let far: Vec<[f32; 3]> = s
                    .vertex_points
                    .chunks_exact(32)
                    .map(|v| std::array::from_fn(|k| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap())))
                    .filter(|p: &[f32; 3]| p[0].abs().max(p[1].abs()) > 1000.0)
                    .collect();
                println!("  surface {i}: {} vertices beyond 1000 out: {far:?}", far.len());
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in s.vertex_points.chunks_exact(32) {
                    let p: [f32; 3] = std::array::from_fn(|k| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap()));
                    if p[0].abs().max(p[1].abs()) <= 1000.0 {
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                }
                println!("  surface {i}: the rest spans {lo:?} to {hi:?}");
            }
            for (i, b) in m.joint_labels.iter().enumerate() {
                let roots = usize::from(m.top_joint_total);
                let parent = match i.checked_sub(roots) {
                    None => -1,
                    Some(k) => i as i32 - i32::from(m.ancestor_chain.get(k).copied().unwrap_or(0)),
                };
                let (q, t) = m.foundation_surface.get(i).copied().unwrap_or_default();
                println!(
                    "  {i:3} {b:30} parent {parent:3} base q=({:.3},{:.3},{:.3},{:.3}) t=({:.2},{:.2},{:.2})",
                    q[0], q[1], q[2], q[3], t[0], t[1], t[2]
                );
            }
            // What bullets can hit: the per-bone hit boxes, the contents and the collision surfaces.
            println!(
                "  contents {:#x} coll_lod {} coll_surfs {} hit boxes {}",
                m.contents,
                m.collision_detail_tier,
                m.collision_faces.len(),
                m.bone_info.iter().filter(|b| b.is_some()).count()
            );
            for (i, b) in m.bone_info.iter().enumerate() {
                if let Some((lo, hi, r2)) = b {
                    println!("  hitbox bone {i} {lo:?}..{hi:?} r2 {r2:.1}");
                }
            }
            for s in &m.collision_faces {
                println!(
                    "  collsurf bone {} contents {:#x} flags {:#x} {} tris {:?}..{:?}",
                    s.joint_position,
                    s.contents,
                    s.face_property_bits,
                    s.triangle_list.len(),
                    s.lo_corner,
                    s.hi_corner
                );
            }
            for (k, t) in m.shift_offset.iter().enumerate() {
                println!(
                    "  trans[{k}] (bone {}) = ({:.2},{:.2},{:.2}) quat {:?}",
                    k + usize::from(m.top_joint_total),
                    t[0],
                    t[1],
                    t[2],
                    m.rotation_quaternions.get(k)
                );
            }
            for (i, s) in m.lod0.iter().enumerate() {
                // T6BONES_VC=1: the surface's vertex colours (min/max/mean
                // per channel) and uv_semantic range (half2, u low), from each
                // 32-byte packed vertex.
                if std::env::var_os("T6BONES_VC").is_some() {
                    let half = |h: u16| -> f32 {
                        let e = i32::from((h >> 10) & 0x1f);
                        let f = f32::from(h & 0x3ff) / 1024.0;
                        let v = if e == 0 {
                            f * 2f32.powi(-14)
                        } else {
                            (1.0 + f) * 2f32.powi(e - 15)
                        };
                        if h & 0x8000 != 0 { -v } else { v }
                    };
                    let n = usize::from(s.vertex_tally);
                    let (mut lo, mut hi, mut sum) = ([255u8; 4], [0u8; 4], [0u32; 4]);
                    let (mut ulo, mut uhi) = ([f32::MAX; 2], [f32::MIN; 2]);
                    for v in s.vertex_points.chunks_exact(32).take(n) {
                        for k in 0..4 {
                            lo[k] = lo[k].min(v[16 + k]);
                            hi[k] = hi[k].max(v[16 + k]);
                            sum[k] += u32::from(v[16 + k]);
                        }
                        let t = u32::from_le_bytes([v[20], v[21], v[22], v[23]]);
                        let uv = [half(t as u16), half((t >> 16) as u16)];
                        for k in 0..2 {
                            ulo[k] = ulo[k].min(uv[k]);
                            uhi[k] = uhi[k].max(uv[k]);
                        }
                    }
                    let mean: Vec<u32> = sum.iter().map(|px| px / n.max(1) as u32).collect();
                    println!(
                        "  surf {i}: vertex colour rgba min {lo:?} max {hi:?} mean {mean:?}; uv u {:.2}..{:.2} v {:.2}..{:.2}",
                        ulo[0], uhi[0], ulo[1], uhi[1]
                    );
                }
                // T6BONES_VERTS=<file>: append each vertex (surface, rigid
                // bone, px py z) for offline posing.
                if let Ok(path) = std::env::var("T6BONES_VERTS") {
                    let bones = rigid_bone_per_vertex(&s.vert_lists);
                    let mut rows = vertex_rows(i, &s.vertex_points, usize::from(s.vertex_tally), &bones);
                    rows.extend(triangle_rows(i, &s.indices));
                    append_lines(&path, &rows);
                }
                println!(
                    "  surf {i}: part bits {:08x?} rigid lists (bone offset, verts, tri offset, tris) {:?}",
                    s.piece_mask, s.vert_lists
                );
                println!(
                    "  surf {i}: verts {} tris {} rigid lists {} blend {:?} material {:?}",
                    s.vertex_tally,
                    s.indices.len() / 3,
                    s.vert_lists.len(),
                    s.blend_counts,
                    s.material.map(|k| c.materials[k.index].name.clone())
                );
            }
        }
        for m in c
            .materials
            .iter()
            .filter(|m| m.name.trim_start_matches(',') == name)
        {
            let tech = m
                .method_set
                .map(|k| c.technique_sets[k.index].name.clone())
                .unwrap_or_default();
            println!("material {} techset {tech} sort {}", m.name, m.sort_rank);
            for t in &m.textures {
                let img = t
                    .image
                    .map(|k| {
                        (
                            c.images[k.index].name.clone(),
                            c.images[k.index].width,
                            c.images[k.index].height,
                        )
                    })
                    .unwrap_or_default();
                println!(
                    "  texture hash {:#010x} semantic {} -> {:?}",
                    t.label_digest, t.semantic, img
                );
            }
            for k in &m.constants {
                println!("  constant {} = {:?}", k.1, k.2);
            }
        }
        // bo2mp: a destructible's pieces: health, hidden bones, each stage's
        // shown bone and what it spawns.
        if let Some(d) = c.destructibles.iter().find(|d| &d.name == name) {
            println!(
                "destructible {name}: model {} pristine {} pieces {} client_only {}",
                d.model,
                d.pristine_model,
                d.pieces.len(),
                d.local_machine_only
            );
            for (i, p) in d.pieces.iter().enumerate() {
                println!(
                    "  piece {i}: parent {} health {} bullet x{} hide_bones {:08x?} constraint {:?}",
                    p.owner_fragment, p.health, p.round_damage_multiplier, p.conceal_joints, p.constraint
                );
                for (k, st) in p.stages.iter().enumerate() {
                    if st.reveal_joint.is_empty() && st.shatter_hitpoints == 0.0 && k > 0 {
                        continue;
                    }
                    println!(
                        "    stage {k}: show `{}` break_health {} flags {:x} phys {} spawn {:?}",
                        st.reveal_joint, st.shatter_hitpoints, st.flags, st.physics_template, st.spawn_models
                    );
                    for m in st.spawn_models.iter().filter(|m| !m.is_empty()) {
                        let preset = c
                            .xmodels
                            .iter()
                            .find(|px| px.name.trim_start_matches(',') == m)
                            .and_then(|px| px.physics_template)
                            .and_then(|k| c.phys_presets.get(k.index));
                        println!("      {m}: preset {preset:?}");
                    }
                }
            }
        }
        if let Some(e) = c.fx.iter().find(|e| &e.name == name) {
            println!(
                "fx {name}: loop {} oneshot {} emit {} flags {:#x}",
                e.repeat_total, e.single_play_total, e.spawn_total, e.flags
            );
            for (i, el) in e.elems.iter().enumerate() {
                println!(
                    "  elem {i}: type {} flags {:#010x} visuals {:?} life {}+{} spawn {} {} lighting {}",
                    el.elem_type(),
                    el.int_from_bytes(0),
                    el.visuals,
                    el.int_from_bytes(48),
                    el.int_from_bytes(52),
                    el.int_from_bytes(4),
                    el.int_from_bytes(8),
                    el.raw[261]
                );
            }
        }
        if let Some(w) = c.weapons.iter().find(|w| &w.name == name) {
            println!("weapon {name}:");
            for (i, a) in w.animation_refs.iter().enumerate() {
                if !a.is_empty() {
                    println!("  anim slot {i:2}: {a}");
                }
            }
            for (k, v) in &w.notetrack_sounds {
                println!("  notetrack sound {k} -> {v}");
            }
            for (o, v) in &w.strings {
                println!("  def string @{o}: {v}");
            }
        }
        if let Some(a) = c.animation_refs.iter().find(|a| &a.name == name) {
            println!(
                "anim {name}: {} frames @ {} fps loop={} delta={} counts {:?} tracks {:?}",
                a.frame_total, a.frame_rate_hz, a.looping, a.delta, a.joint_total, a.names
            );
            println!("  notifies {:?}", a.notifies);
            println!("  loopEntryTime {} primedLength {}", a.loop_entry_time, a.primed_length);
            // T6BONES_DECODE=1: run the game's clip decoder on it and
            // print the error a silently dropped clip would have hit.
            if std::env::var_os("T6BONES_DECODE").is_some() {
                let parts = raw_parts(a);
                println!(
                    "  data byte {} short {} int {} rbyte {} rshort {} rint {} idx {}",
                    a.payload_u8.len(),
                    a.payload_i16.len(),
                    a.payload_i32.len(),
                    a.noise_octet.len(),
                    a.noise_word16.len(),
                    a.noise_word32.len(),
                    a.indices.len()
                );
                if let Some(clip) = decode_or_say(&parts, "decode FAILED ") {
                    {
                        println!("  decode ok");
                        // T6BONES_KEYS=bone,bone: each named bone's key frames (rotation, then
                        // translation) and its turn from frame 0 at every frame.
                        if let Ok(want) = std::env::var("T6BONES_KEYS") {
                            for t in clip.tracks.iter().filter(|t| want.split(',').any(|w| w == t.name)) {
                                let idx = |f: &xmodel_runtime::KeyframeIds| match f {
                                    xmodel_runtime::KeyframeIds::Dense => "dense".to_string(),
                                    xmodel_runtime::KeyframeIds::Sparse(at) => format!("{at:?}"),
                                };
                                if let xmodel_runtime::Rotation::HalfKeyed(k) | xmodel_runtime::Rotation::FullKeyed(k) = &t.rotation {
                                    println!("  {} rot keys {} at {}", t.name, k.values.len(), idx(&k.frames));
                                }
                                if let xmodel_runtime::RootShift::SmallKeyed(k) | xmodel_runtime::RootShift::FullKeyed(k) = &t.move_track {
                                    println!("  {} trans keys {} at {}", t.name, k.values.len(), idx(&k.frames));
                                }
                                let i = clip.tracks.iter().position(|px| px.name == t.name).unwrap_or(0);
                                let r0 = clip.lane_at(i, 0.0).and_then(|s| s.rotation);
                                let turns: Vec<String> = (0..=usize::from(clip.frame_total))
                                    .map(|f| {
                                        let r = clip.lane_at(i, f as f32 / clip.fps).and_then(|s| s.rotation);
                                        match (r0, r) {
                                            (Some(a), Some(b)) => {
                                                let d = quat_dot(a, b).abs().min(1.0);
                                                format!("{:.0}", 2.0 * d.acos().to_degrees())
                                            }
                                            _ => "-".into(),
                                        }
                                    })
                                    .collect();
                                println!("  {} turn from f0: {}", t.name, turns.join(" "));
                            }
                        }
                        // T6BONES_SAMPLE=1: every track's rotation and
                        // translation at the first, middle and last frame.
                        if std::env::var_os("T6BONES_SAMPLE").is_some() {
                            for (i, t) in clip.tracks.iter().enumerate() {
                                println!("  track {i} {} rot {:?} trans {:?}", t.name,
                                    std::mem::discriminant(&t.rotation), &t.move_track);
                                for f in [0.0f32, 6.0, 12.0, 18.0, 24.0] {
                                    let s = clip.lane_at(i, f / clip.fps);
                                    println!("    f{f}: {s:?}");
                                }
                            }
                        }
                    }
                }
            }
            // T6BONES_DELTA=1: the root motion keys (frame, px py z) and the
            // speed between keys.
            if std::env::var_os("T6BONES_DELTA").is_some() {
                let keys = &a.delta_trans;
                println!("  delta keys {}", keys.len());
                for w in keys.windows(2) {
                    let (f0, p0) = w[0];
                    let (f1, p1) = w[1];
                    let d = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2)).sqrt();
                    let dt = f32::from(f1.saturating_sub(f0)) / a.frame_rate_hz.max(1.0);
                    println!(
                        "  frame {f0:3}->{f1:3}: ({:.1} {:.1} {:.1}) speed {:.1}",
                        p1[0],
                        p1[1],
                        p1[2],
                        if dt > 0.0 { d / dt } else { 0.0 }
                    );
                }
            }
            // T6BONES_SAMPLE=<bone>: that bone's rotation each frame, as the
            // game's decoder reads it (yaw/pitch/roll in degrees).
            // T6BONES_FLIPS=1: per track, neighbouring frames whose
            // rotations sit on opposite sides (dot < 0): a plain blend between
            // them swings the bone the long way round.
            if std::env::var_os("T6BONES_FLIPS").is_some() {
                let parts = raw_parts(a);
                if let Ok(clip) = xmodel_runtime::SkeletalClip::from_parts(&parts) {
                    let flipped: Vec<(&str, Vec<u16>)> = clip
                        .tracks
                        .iter()
                        .enumerate()
                        .map(|(ti, tr)| (tr.name.as_str(), flip_frames(&clip, ti, a.frame_total, a.frame_rate_hz)))
                        .filter(|(_key, frames)| !frames.is_empty())
                        .collect();
                    for (name, frames) in &flipped {
                        println!("  flips {name}: frames {:?}", &frames[..frames.len().min(12)]);
                    }
                    let total: usize = flipped.iter().map(|(_, frames)| frames.len()).sum();
                    println!("  flips total {total}");
                }
            }
            if let Ok(bone) = std::env::var("T6BONES_SAMPLE") {
                let parts = raw_parts(a);
                if let Some(clip) = decode_or_say(&parts, "decode: ") {
                    {
                        let Some(track) = clip.tracks.iter().position(|t| t.name == bone) else {
                            println!("  no track {bone}");
                            continue;
                        };
                        // The frames the rotation keys sit on, as stored.
                        if let xmodel_runtime::Rotation::FullKeyed(k) | xmodel_runtime::Rotation::HalfKeyed(k) =
                            &clip.tracks[track].rotation
                        {
                            println!("  {bone} rotation keys {} frames {:?}", k.values.len(), k.frames);
                            // T6BONES_KEYS=1: each stored key's value, in order.
                            if std::env::var_os("T6BONES_KEYS").is_some() {
                                for (i, v) in k.values.iter().enumerate() {
                                    println!("  {bone} key {i:2}: {v:?}");
                                }
                            }
                        }
                        for f in 0..a.frame_total {
                            let t = f32::from(f) / a.frame_rate_hz.max(1.0);
                            let s = clip.lane_at(track, t);
                            if let Some(v) = s.and_then(|s| s.move_track) {
                                println!("  {bone} frame {f:2}: trans ({:.3} {:.3} {:.3})", v[0], v[1], v[2]);
                            }
                            if let Some(q) = s.and_then(|s| s.rotation) {
                                let [px, py, z, w] = [q[0], q[1], q[2], q[3]];
                                // The bone's axes in its parent's space; each
                                // one's heading on the ground (degrees).
                                let ax = [
                                    1.0 - 2.0 * (py * py + z * z),
                                    2.0 * (px * py + w * z),
                                    2.0 * (px * z - w * py),
                                ];
                                let ay = [
                                    2.0 * (px * py - w * z),
                                    1.0 - 2.0 * (px * px + z * z),
                                    2.0 * (py * z + w * px),
                                ];
                                let az = [
                                    2.0 * (px * z + w * py),
                                    2.0 * (py * z - w * px),
                                    1.0 - 2.0 * (px * px + py * py),
                                ];
                                let h = |v: [f32; 3]| v[1].atan2(v[0]).to_degrees();
                                println!(
                                    "  {bone} frame {f:2}: x ({:.2} {:.2} {:.2}) heading x {:6.1} y {:6.1} z {:6.1}",
                                    ax[0],
                                    ax[1],
                                    ax[2],
                                    h(ax),
                                    h(ay),
                                    h(az)
                                );
                            } else {
                                println!("  {bone} frame {f:2}: no rotation");
                            }
                        }
                    }
                }
            }
        }
    }
    // T6BONES_DOT=1: the first anim named against each other one: for every
    // bone both animate, the sign of the rotation dot product (first anim
    // at its middle frame, the other at its last). A negative bone cancels
    // when the two are summed at equal weight.
    if std::env::var_os("T6BONES_DOT").is_some() {
        let clips: Vec<xmodel_runtime::SkeletalClip> = names
            .iter()
            .filter_map(|n| c.animation_refs.iter().find(|a| &a.name == n))
            .filter_map(|a| {
                xmodel_runtime::SkeletalClip::from_parts(&raw_parts(a))
                .ok()
            })
            .collect();
        let Some((first_clip, others)) = clips.split_first() else {
            return;
        };
        let turn_matrix = |clip: &xmodel_runtime::SkeletalClip, name: &str, time: f32| {
            let track = clip.tracks.iter().position(|t| t.name == name)?;
            clip.lane_at(track, time)?.rotation
        };
        for other in others {
            let mut negative = Vec::new();
            let mut shared = 0;
            for track in &first_clip.tracks {
                let (Some(a), Some(b)) = (
                    turn_matrix(first_clip, &track.name, first_clip.duration() * 0.5),
                    turn_matrix(other, &track.name, other.duration()),
                ) else {
                    continue;
                };
                shared += 1;
                if quat_dot(a, b) < 0.0 {
                    negative.push(track.name.as_str());
                }
            }
            println!(
                "dot {} vs {}: {shared} shared bones, {} negative {:?}",
                first_clip.name,
                other.name,
                negative.len(),
                negative
            );
        }
    }
}

/// The captured clip as the game's decoder takes it.
fn raw_parts(a: &asset_t6::XAnimRef) -> xmodel_runtime::StoredClipParts {
    let flags = u8::from(a.looping) | (u8::from(a.delta) << 1);
    xmodel_runtime::StoredClipParts {
        flags,
        frame_total: a.frame_total,
        joint_total: a.joint_total,
        fps: a.frame_rate_hz,
        name: a.name.clone(),
        names: a.names.clone(),
        indices: a.indices.clone(),
        payload_u8: a.payload_u8.clone(),
        payload_i16: a.payload_i16.clone(),
        payload_i32: a.payload_i32.clone(),
        noise_octet: a.noise_octet.clone(),
        noise_word16: a.noise_word16.clone(),
        noise_word32: a.noise_word32.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quaternion_dot_multiplies_componentwise() {
        assert_eq!(quat_dot([1.0, 2.0, 3.0, 4.0], [1.0, 1.0, 1.0, -1.0]), 2.0);
        assert!(quat_dot([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]) < 0.0);
    }

    #[test]
    fn rigid_bones_repeat_per_vertex_run() {
        assert_eq!(rigid_bone_per_vertex(&[(128, 2, 0, 0), (64, 3, 0, 0)]), vec![2, 2, 1, 1, 1]);
        assert!(rigid_bone_per_vertex(&[]).is_empty());
    }

    #[test]
    fn vertex_and_triangle_rows() {
        let mut vertex_points = Vec::new();
        for x in [1.5f32, 2.5] {
            vertex_points.extend_from_slice(&x.to_le_bytes());
            vertex_points.extend_from_slice(&[0u8; 28]);
        }
        let rows = vertex_rows(4, &vertex_points, 2, &[7]);
        assert_eq!(rows, vec!["4 7 1.5 0 0".to_owned(), "4 0 2.5 0 0".to_owned()]);
        assert_eq!(vertex_rows(4, &vertex_points, 1, &[7]).len(), 1);
        assert_eq!(triangle_rows(2, &[0, 1, 2, 3, 4]), vec!["t 2 0 1 2".to_owned()]);
    }
}
