//! bo2zm: print a T6 model's bones and an animation's tracks/notetracks.
//! `t6bones <zone.ff> <model or anim name>...`
#[allow(dead_code)]
#[path = "../../../render_anim/src/anim/t6_loop_seam.rs"]
mod t6_loop_seam;

/// The biggest turn of any bone in one frame as a loop plays, quarter frames apart and
/// across the wrap: (degrees per frame, bone, frame it starts at).
fn biggest_turn(clip: &xmodel_runtime::AnimClip) -> (f32, String, f32) {
    let steps = usize::from(clip.numframes) * 4;
    let at = |t: usize, i: usize| {
        clip.sample_track(t, i as f32 / 4.0 / clip.framerate).and_then(|s| s.rotation)
    };
    let mut best = (0.0f32, String::new(), 0.0f32);
    for (t, track) in clip.tracks.iter().enumerate() {
        for i in 0..steps {
            if let (Some(a), Some(b)) = (at(t, i), at(t, (i + 1) % steps)) {
                let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6bones <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
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
        for a in c.xanims.iter().filter(|a| a.looping && names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::AnimClip::from_parts(&raw_parts(a)) else {
                println!("{}: decode failed", a.name);
                continue;
            };
            let angle = |f: &[f32; 4], l: &[f32; 4]| {
                let dot = (f[0] * l[0] + f[1] * l[1] + f[2] * l[2] + f[3] * l[3]).abs().min(1.0);
                2.0 * dot.acos().to_degrees()
            };
            let mut worst = (0.0f32, String::new(), 0usize);
            // The biggest turn in one frame: (degrees per frame, track, frame).
            let mut step = (0.0f32, String::new(), 0u16);
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
                            xmodel_runtime::FrameIndices::Dense => (i as u16 - 1, i as u16),
                            xmodel_runtime::FrameIndices::Sparse(fr) => (fr[i - 1], fr[i]),
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
                a.name, a.numframes, worst.0, worst.1, worst.2, step.0, step.1, step.2
            );
        }
        return;
    }
    // T6BONES_FLIPS=1: for every anim whose name holds one of the given words, each bone whose
    // rotation keys change sign from one key to the next (the clip blends them the long way round).
    if std::env::var_os("T6BONES_FLIPS").is_some() {
        for a in c.xanims.iter().filter(|a| names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::AnimClip::from_parts(&raw_parts(a)) else {
                println!("{}: decode failed", a.name);
                continue;
            };
            for t in &clip.tracks {
                if let xmodel_runtime::Rotation::HalfKeyed(k) | xmodel_runtime::Rotation::FullKeyed(k) = &t.rotation {
                    let flips: Vec<usize> = k.values.windows(2).enumerate()
                        .filter(|(_, w)| w[0].iter().zip(w[1].iter()).map(|(x, y)| x * y).sum::<f32>() < 0.0)
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
        for a in c.xanims.iter().filter(|a| a.looping && names.iter().any(|n| a.name.contains(n.as_str()))) {
            let Ok(clip) = xmodel_runtime::AnimClip::from_parts(&raw_parts(a)) else {
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
                let worst = |cl: &xmodel_runtime::AnimClip, t: usize| {
                    let steps = usize::from(cl.numframes) * 4;
                    let at = |i: usize| cl.sample_track(t, i as f32 / 4.0 / cl.framerate);
                    let (mut r, mut rf, mut m, mut mf) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                    for i in 0..steps {
                        if let (Some(x), Some(y)) = (at(i), at((i + 1) % steps)) {
                            if let (Some(a), Some(b)) = (x.rotation, y.rotation) {
                                let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
                                let deg = 2.0 * d.acos().to_degrees() * 4.0;
                                if deg > r { r = deg; rf = i as f32 / 4.0; }
                            }
                            if let (Some(a), Some(b)) = (x.translation, y.translation) {
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
                let n = usize::from(clip.numframes);
                for (t, tr) in clip.tracks.iter().enumerate() {
                    let at = |i: usize| clip.sample_track(t, i as f32 / clip.framerate);
                    let (mut turn, mut mv) = ([0.0f32; 2], [0.0f32; 2]);
                    for i in 0..n - 1 {
                        let h = usize::from(i >= n / 2);
                        let (Some(x), Some(y)) = (at(i), at(i + 1)) else { continue };
                        if let (Some(a), Some(b)) = (x.rotation, y.rotation) {
                            let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
                            turn[h] += 2.0 * d.acos().to_degrees();
                        }
                        if let (Some(a), Some(b)) = (x.translation, y.translation) {
                            mv[h] += ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                        }
                    }
                    println!("    {:22} turn {:6.1} | {:6.1} deg   move {:6.2} | {:6.2} u", tr.name, turn[0], turn[1], mv[0], mv[1]);
                }
            }
        }
        return;
    }
    for name in names {
        if let Some(m) = c.xmodels.iter().find(|m| &m.name == name) {
            println!(
                "model {name}: {} bones ({} root), {} surfaces",
                m.bone_names.len(),
                m.num_root_bones,
                m.lod0.len()
            );
            for (i, b) in m.bone_names.iter().enumerate() {
                let parent = if i < usize::from(m.num_root_bones) {
                    -1
                } else {
                    let step = m
                        .parent_list
                        .get(i - usize::from(m.num_root_bones))
                        .copied()
                        .unwrap_or(0);
                    i as i32 - i32::from(step)
                };
                let (q, t) = m.base_mat.get(i).copied().unwrap_or_default();
                println!(
                    "  {i:3} {b:30} parent {parent:3} base q=({:.3},{:.3},{:.3},{:.3}) t=({:.2},{:.2},{:.2})",
                    q[0], q[1], q[2], q[3], t[0], t[1], t[2]
                );
            }
            // What bullets can hit: the per-bone hit boxes, the contents and the collision surfaces.
            println!(
                "  contents {:#x} coll_lod {} coll_surfs {} hit boxes {}",
                m.contents,
                m.coll_lod,
                m.coll_surfs.len(),
                m.bone_info.iter().filter(|b| b.is_some()).count()
            );
            for (i, b) in m.bone_info.iter().enumerate() {
                if let Some((lo, hi, r2)) = b {
                    println!("  hitbox bone {i} {lo:?}..{hi:?} r2 {r2:.1}");
                }
            }
            for s in &m.coll_surfs {
                println!(
                    "  collsurf bone {} contents {:#x} flags {:#x} {} tris {:?}..{:?}",
                    s.bone_idx,
                    s.contents,
                    s.surf_flags,
                    s.tris.len(),
                    s.mins,
                    s.maxs
                );
            }
            for (k, t) in m.trans.iter().enumerate() {
                println!(
                    "  trans[{k}] (bone {}) = ({:.2},{:.2},{:.2}) quat {:?}",
                    k + usize::from(m.num_root_bones),
                    t[0],
                    t[1],
                    t[2],
                    m.quats.get(k)
                );
            }
            for (i, s) in m.lod0.iter().enumerate() {
                // T6BONES_VC=1: the surface's vertex colours (min/max/mean
                // per channel) and texcoord range (half2, u low), from each
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
                    let n = usize::from(s.vert_count);
                    let (mut lo, mut hi, mut sum) = ([255u8; 4], [0u8; 4], [0u32; 4]);
                    let (mut ulo, mut uhi) = ([f32::MAX; 2], [f32::MIN; 2]);
                    for v in s.verts.chunks_exact(32).take(n) {
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
                    let mean: Vec<u32> = sum.iter().map(|x| x / n.max(1) as u32).collect();
                    println!(
                        "  surf {i}: vertex colour rgba min {lo:?} max {hi:?} mean {mean:?}; uv u {:.2}..{:.2} v {:.2}..{:.2}",
                        ulo[0], uhi[0], ulo[1], uhi[1]
                    );
                }
                // T6BONES_VERTS=<file>: append each vertex (surface, rigid
                // bone, x y z) for offline posing.
                if let Ok(path) = std::env::var("T6BONES_VERTS") {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .expect("verts file");
                    let mut bones = Vec::new();
                    for &(off, run, _, _) in &s.vert_lists {
                        bones.extend(std::iter::repeat_n(off / 64, usize::from(run)));
                    }
                    for (k, v) in s.verts.chunks_exact(32).take(usize::from(s.vert_count)).enumerate() {
                        let p = |o: usize| f32::from_le_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
                        writeln!(f, "{i} {} {} {} {}", bones.get(k).copied().unwrap_or(0), p(0), p(4), p(8)).ok();
                    }
                    for t in s.indices.chunks_exact(3) {
                        writeln!(f, "t {i} {} {} {}", t[0], t[1], t[2]).ok();
                    }
                }
                println!(
                    "  surf {i}: part bits {:08x?} rigid lists (bone offset, verts, tri offset, tris) {:?}",
                    s.part_bits, s.vert_lists
                );
                println!(
                    "  surf {i}: verts {} tris {} rigid lists {} blend {:?} material {:?}",
                    s.vert_count,
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
                .technique_set
                .map(|k| c.technique_sets[k.index].name.clone())
                .unwrap_or_default();
            println!("material {} techset {tech} sort {}", m.name, m.sort_key);
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
                    t.name_hash, t.semantic, img
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
                d.client_only
            );
            for (i, p) in d.pieces.iter().enumerate() {
                println!(
                    "  piece {i}: parent {} health {} bullet x{} hide_bones {:08x?} constraint {:?}",
                    p.parent_piece, p.health, p.bullet_damage_scale, p.hide_bones, p.constraint
                );
                for (k, st) in p.stages.iter().enumerate() {
                    if st.show_bone.is_empty() && st.break_health == 0.0 && k > 0 {
                        continue;
                    }
                    println!(
                        "    stage {k}: show `{}` break_health {} flags {:x} phys {} spawn {:?}",
                        st.show_bone, st.break_health, st.flags, st.phys_preset, st.spawn_models
                    );
                    for m in st.spawn_models.iter().filter(|m| !m.is_empty()) {
                        let preset = c
                            .xmodels
                            .iter()
                            .find(|x| x.name.trim_start_matches(',') == m)
                            .and_then(|x| x.phys_preset)
                            .and_then(|k| c.phys_presets.get(k.index));
                        println!("      {m}: preset {preset:?}");
                    }
                }
            }
        }
        if let Some(e) = c.fx.iter().find(|e| &e.name == name) {
            println!(
                "fx {name}: loop {} oneshot {} emit {} flags {:#x}",
                e.looping_count, e.one_shot_count, e.emission_count, e.flags
            );
            for (i, el) in e.elems.iter().enumerate() {
                println!(
                    "  elem {i}: type {} flags {:#010x} visuals {:?} life {}+{} spawn {} {} lighting {}",
                    el.elem_type(),
                    el.i32_at(0),
                    el.visuals,
                    el.i32_at(48),
                    el.i32_at(52),
                    el.i32_at(4),
                    el.i32_at(8),
                    el.raw[261]
                );
            }
        }
        if let Some(w) = c.weapons.iter().find(|w| &w.name == name) {
            println!("weapon {name}:");
            for (i, a) in w.xanims.iter().enumerate() {
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
        if let Some(a) = c.xanims.iter().find(|a| &a.name == name) {
            println!(
                "anim {name}: {} frames @ {} fps loop={} delta={} counts {:?} tracks {:?}",
                a.numframes, a.framerate, a.looping, a.delta, a.bone_count, a.names
            );
            println!("  notifies {:?}", a.notifies);
            println!("  loopEntryTime {} primedLength {}", a.loop_entry_time, a.primed_length);
            // T6BONES_DECODE=1: run the game's clip decoder on it and
            // print the error a silently dropped clip would have hit.
            if std::env::var_os("T6BONES_DECODE").is_some() {
                let parts = xmodel_runtime::RawXAnimParts {
                    name: a.name.clone(),
                    data_byte: a.data_byte.clone(),
                    data_short: a.data_short.clone(),
                    data_int: a.data_int.clone(),
                    random_data_byte: a.random_data_byte.clone(),
                    random_data_short: a.random_data_short.clone(),
                    random_data_int: a.random_data_int.clone(),
                    numframes: a.numframes,
                    flags: u8::from(a.looping) | (u8::from(a.delta) << 1),
                    bone_count: a.bone_count,
                    framerate: a.framerate,
                    names: a.names.clone(),
                    notifies: Vec::new(),
                    indices: a.indices.clone(),
                    delta_trans: None,
                };
                println!(
                    "  data byte {} short {} int {} rbyte {} rshort {} rint {} idx {}",
                    a.data_byte.len(),
                    a.data_short.len(),
                    a.data_int.len(),
                    a.random_data_byte.len(),
                    a.random_data_short.len(),
                    a.random_data_int.len(),
                    a.indices.len()
                );
                match xmodel_runtime::AnimClip::from_parts(&parts) {
                    Ok(clip) => {
                        println!("  decode ok");
                        // T6BONES_KEYS=bone,bone: each named bone's key frames (rotation, then
                        // translation) and its turn from frame 0 at every frame.
                        if let Ok(want) = std::env::var("T6BONES_KEYS") {
                            for t in clip.tracks.iter().filter(|t| want.split(',').any(|w| w == t.name)) {
                                let idx = |f: &xmodel_runtime::FrameIndices| match f {
                                    xmodel_runtime::FrameIndices::Dense => "dense".to_string(),
                                    xmodel_runtime::FrameIndices::Sparse(at) => format!("{at:?}"),
                                };
                                if let xmodel_runtime::Rotation::HalfKeyed(k) | xmodel_runtime::Rotation::FullKeyed(k) = &t.rotation {
                                    println!("  {} rot keys {} at {}", t.name, k.values.len(), idx(&k.frames));
                                }
                                if let xmodel_runtime::Translation::SmallKeyed(k) | xmodel_runtime::Translation::FullKeyed(k) = &t.translation {
                                    println!("  {} trans keys {} at {}", t.name, k.values.len(), idx(&k.frames));
                                }
                                let i = clip.tracks.iter().position(|x| x.name == t.name).unwrap_or(0);
                                let r0 = clip.sample_track(i, 0.0).and_then(|s| s.rotation);
                                let turns: Vec<String> = (0..=usize::from(clip.numframes))
                                    .map(|f| {
                                        let r = clip.sample_track(i, f as f32 / clip.framerate).and_then(|s| s.rotation);
                                        match (r0, r) {
                                            (Some(a), Some(b)) => {
                                                let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().min(1.0);
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
                                    std::mem::discriminant(&t.rotation), &t.translation);
                                for f in [0.0f32, 6.0, 12.0, 18.0, 24.0] {
                                    let s = clip.sample_track(i, f / clip.framerate);
                                    println!("    f{f}: {s:?}");
                                }
                            }
                        }
                    }
                    Err(e) => println!("  decode FAILED {e:?}"),
                }
            }
            // T6BONES_DELTA=1: the root motion keys (frame, x y z) and the
            // speed between keys.
            if std::env::var_os("T6BONES_DELTA").is_some() {
                let keys = &a.delta_trans;
                println!("  delta keys {}", keys.len());
                for w in keys.windows(2) {
                    let (f0, p0) = w[0];
                    let (f1, p1) = w[1];
                    let d = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2)).sqrt();
                    let dt = f32::from(f1.saturating_sub(f0)) / a.framerate.max(1.0);
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
                let parts = xmodel_runtime::RawXAnimParts {
                    name: a.name.clone(),
                    data_byte: a.data_byte.clone(),
                    data_short: a.data_short.clone(),
                    data_int: a.data_int.clone(),
                    random_data_byte: a.random_data_byte.clone(),
                    random_data_short: a.random_data_short.clone(),
                    random_data_int: a.random_data_int.clone(),
                    numframes: a.numframes,
                    flags: u8::from(a.looping) | (u8::from(a.delta) << 1),
                    bone_count: a.bone_count,
                    framerate: a.framerate,
                    names: a.names.clone(),
                    notifies: Vec::new(),
                    indices: a.indices.clone(),
                    delta_trans: None,
                };
                if let Ok(clip) = xmodel_runtime::AnimClip::from_parts(&parts) {
                    let mut total = 0;
                    for (ti, tr) in clip.tracks.iter().enumerate() {
                        let mut flips = Vec::new();
                        for f in 0..a.numframes {
                            let t0 = f32::from(f) / a.framerate.max(1.0);
                            let t1 = f32::from(f + 1) / a.framerate.max(1.0);
                            let (Some(q0), Some(q1)) = (
                                clip.sample_track(ti, t0).and_then(|s| s.rotation),
                                clip.sample_track(ti, t1).and_then(|s| s.rotation),
                            ) else {
                                continue;
                            };
                            let dot = q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3];
                            if dot < 0.0 {
                                flips.push(f);
                            }
                        }
                        if !flips.is_empty() {
                            total += flips.len();
                            println!(
                                "  flips {}: frames {:?}",
                                tr.name,
                                &flips[..flips.len().min(12)]
                            );
                        }
                    }
                    println!("  flips total {total}");
                }
            }
            if let Ok(bone) = std::env::var("T6BONES_SAMPLE") {
                let parts = xmodel_runtime::RawXAnimParts {
                    name: a.name.clone(),
                    data_byte: a.data_byte.clone(),
                    data_short: a.data_short.clone(),
                    data_int: a.data_int.clone(),
                    random_data_byte: a.random_data_byte.clone(),
                    random_data_short: a.random_data_short.clone(),
                    random_data_int: a.random_data_int.clone(),
                    numframes: a.numframes,
                    flags: u8::from(a.looping) | (u8::from(a.delta) << 1),
                    bone_count: a.bone_count,
                    framerate: a.framerate,
                    names: a.names.clone(),
                    notifies: Vec::new(),
                    indices: a.indices.clone(),
                    delta_trans: None,
                };
                match xmodel_runtime::AnimClip::from_parts(&parts) {
                    Ok(clip) => {
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
                        for f in 0..a.numframes {
                            let t = f32::from(f) / a.framerate.max(1.0);
                            let s = clip.sample_track(track, t);
                            if let Some(v) = s.and_then(|s| s.translation) {
                                println!("  {bone} frame {f:2}: trans ({:.3} {:.3} {:.3})", v[0], v[1], v[2]);
                            }
                            if let Some(q) = s.and_then(|s| s.rotation) {
                                let [x, y, z, w] = [q[0], q[1], q[2], q[3]];
                                // The bone's axes in its parent's space; each
                                // one's heading on the ground (degrees).
                                let ax = [
                                    1.0 - 2.0 * (y * y + z * z),
                                    2.0 * (x * y + w * z),
                                    2.0 * (x * z - w * y),
                                ];
                                let ay = [
                                    2.0 * (x * y - w * z),
                                    1.0 - 2.0 * (x * x + z * z),
                                    2.0 * (y * z + w * x),
                                ];
                                let az = [
                                    2.0 * (x * z + w * y),
                                    2.0 * (y * z - w * x),
                                    1.0 - 2.0 * (x * x + y * y),
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
                    Err(e) => println!("  decode: {e:?}"),
                }
            }
        }
    }
    // T6BONES_DOT=1: the first anim named against each other one: for every
    // bone both animate, the sign of the rotation dot product (first anim
    // at its middle frame, the other at its last). A negative bone cancels
    // when the two are summed at equal weight.
    if std::env::var_os("T6BONES_DOT").is_some() {
        let clips: Vec<xmodel_runtime::AnimClip> = names
            .iter()
            .filter_map(|n| c.xanims.iter().find(|a| &a.name == n))
            .filter_map(|a| {
                xmodel_runtime::AnimClip::from_parts(&xmodel_runtime::RawXAnimParts {
                    name: a.name.clone(),
                    data_byte: a.data_byte.clone(),
                    data_short: a.data_short.clone(),
                    data_int: a.data_int.clone(),
                    random_data_byte: a.random_data_byte.clone(),
                    random_data_short: a.random_data_short.clone(),
                    random_data_int: a.random_data_int.clone(),
                    numframes: a.numframes,
                    flags: u8::from(a.looping) | (u8::from(a.delta) << 1),
                    bone_count: a.bone_count,
                    framerate: a.framerate,
                    names: a.names.clone(),
                    notifies: Vec::new(),
                    indices: a.indices.clone(),
                    delta_trans: None,
                })
                .ok()
            })
            .collect();
        let Some((base, others)) = clips.split_first() else {
            return;
        };
        let rot = |clip: &xmodel_runtime::AnimClip, name: &str, time: f32| {
            let track = clip.tracks.iter().position(|t| t.name == name)?;
            clip.sample_track(track, time)?.rotation
        };
        for other in others {
            let mut negative = Vec::new();
            let mut shared = 0;
            for track in &base.tracks {
                let (Some(a), Some(b)) = (
                    rot(base, &track.name, base.duration() * 0.5),
                    rot(other, &track.name, other.duration()),
                ) else {
                    continue;
                };
                shared += 1;
                if a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3] < 0.0 {
                    negative.push(track.name.as_str());
                }
            }
            println!(
                "dot {} vs {}: {shared} shared bones, {} negative {:?}",
                base.name,
                other.name,
                negative.len(),
                negative
            );
        }
    }
}

/// The captured clip as the game's decoder takes it.
fn raw_parts(a: &asset_t6::XAnimRef) -> xmodel_runtime::RawXAnimParts {
    xmodel_runtime::RawXAnimParts {
        name: a.name.clone(),
        data_byte: a.data_byte.clone(),
        data_short: a.data_short.clone(),
        data_int: a.data_int.clone(),
        random_data_byte: a.random_data_byte.clone(),
        random_data_short: a.random_data_short.clone(),
        random_data_int: a.random_data_int.clone(),
        numframes: a.numframes,
        flags: u8::from(a.looping) | (u8::from(a.delta) << 1),
        bone_count: a.bone_count,
        framerate: a.framerate,
        names: a.names.clone(),
        notifies: Vec::new(),
        indices: a.indices.clone(),
        delta_trans: None,
    }
}
