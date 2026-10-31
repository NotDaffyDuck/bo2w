//! bo2zm fix list 2: what the map's placed models (`clipMap_t`
//! `staticModelList`) give bullets and thrown things to hit: per model, how
//! many are placed, the instance and model contents, the collision LOD and
//! the collision surfaces (triangles, contents, surface type). Read-only.
//!
//! `t6props [zone.ff]` (default Nuketown). `T6PROPS_NEAR=x,y,z,r` lists
//! the placements within r units of a point.

use std::collections::BTreeMap;
use std::path::PathBuf;
use asset_t6::ZoneCapture;

fn default_zone() -> PathBuf {
    bo2zm_tools::zone_file("zm_nuked.ff")
}

/// A run that already said why on its own.
struct Failed;

/// `1,2,x,3` -> the entries that are numbers.
fn parse_indices(list: &str) -> Vec<usize> {
    list.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

/// `x,y,z,r`: a point and a radius, or nothing when it is not exactly four numbers.
fn parse_near(spec: &str) -> Option<[f32; 4]> {
    let n: Vec<f32> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    <[f32; 4]>::try_from(n).ok()
}

/// `T6PROPS_MODEL=<substring>`: every model of the zone whose name contains
/// it: bounds, contents, collision LOD and collision surfaces.
fn list_models(capture: &ZoneCapture, want: &str) {
    for m in capture.xmodels.iter().filter(|m| m.name.contains(want)) {
        println!(
            "model {}: bounds {:?}..{:?} radius {} contents {:#x} coll lod {} surfs {}",
            m.name,
            m.lo_corner,
            m.hi_corner,
            m.radius,
            m.contents,
            m.collision_detail_tier,
            m.collision_faces.len()
        );
        println!("  lighting origin offset {:?} range {}", m.lighting_origin_offset, m.lighting_origin_range);
        for k in m.materials.iter().flatten() {
            let mat = &capture.materials[k.index];
            let ts = mat.method_set.map_or("-".to_owned(), |t| capture.technique_sets[t.index].name.clone());
            println!("  material {} techset {ts} sort {}", mat.name, mat.sort_rank);
        }
        for c in &m.collision_faces {
            println!(
                "  coll surf: {} tris, {:?}..{:?}, contents {:#x}, surf flags {:#x}",
                c.triangle_list.len(),
                c.lo_corner,
                c.hi_corner,
                c.contents,
                c.face_property_bits
            );
        }
        // Its collision maps' brushes (what `spawncollision` pieces are
        // solid with, and to what: their contents).
        for b in &m.collmap_brushes {
            println!(
                "  collmap brush: {:?}..{:?}, contents {:#x}, {} extra sides",
                b.lo_corner,
                b.hi_corner,
                b.contents,
                b.sides.len()
            );
        }
    }
}

/// `T6PROPS_MATFLAGS=<substring>`: every material of the zone whose name
/// contains it (decals too), with its game flags and surface type bits.
fn list_material_flags(capture: &ZoneCapture, want: &str) {
    for m in capture.materials.iter().filter(|m| m.name.contains(want)) {
        println!(
            "material {}: game flags {:#x} type bits {:#x} surface flags {:#x} sort {}",
            m.name, m.game_flags, m.facet_type_bits, m.surf_bits, m.sort_rank
        );
    }
}

/// `T6PROPS_PRESET=<substring>`: the zone's physics presets whose name contains it.
fn list_presets(capture: &ZoneCapture, want: &str) {
    for p in capture.phys_presets.iter().filter(|p| p.name.contains(want)) {
        println!("{p:?}");
    }
}

/// The modes that need only the capture, in the order they are tried.
const CAPTURE_QUERIES: [(&str, fn(&ZoneCapture, &str)); 3] = [
    ("T6PROPS_MODEL", list_models),
    ("T6PROPS_MATFLAGS", list_material_flags),
    ("T6PROPS_PRESET", list_presets),
];

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let path = std::env::args()
        .nth(1)
        .map_or_else(default_zone, PathBuf::from);
    let capture = match asset_t6::capture_zone(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t6props: {e}");
            return Err(Failed);
        }
    };
    if let Some((query, want)) = CAPTURE_QUERIES
        .iter()
        .find_map(|(var, query)| Some((query, std::env::var(var).ok()?)))
    {
        query(&capture, &want);
        return Ok(());
    }
    let Some(clip) = capture.clip.as_ref() else {
        eprintln!("t6props: no clipMap");
        return Err(Failed);
    };
    // T6PROPS_DYN=<substring>: the loose props (clipmap dynents) whose model
    // contains it: kind, health, physics preset (and its numbers), contents.
    if let Ok(want) = std::env::var("T6PROPS_DYN") {
        for (i, d) in clip
            .debris_pieces
            .iter()
            .enumerate()
            .filter(|(_key, d)| d.model.contains(want.as_str()))
        {
            let preset = capture.phys_presets.iter().find(|p| p.name == d.physics_template);
            let model_preset = capture
                .xmodels
                .iter()
                .find(|m| m.name.trim_start_matches(',') == d.model.trim_start_matches(','))
                .and_then(|m| m.physics_template)
                .and_then(|k| capture.phys_presets.get(k.index));
            println!(
                "dyn {i} kind {} model {} at {:?} health {} flags {:#x} contents {:#x} brush {} phys brush {} preset {:?} {:?} model preset {:?}",
                d.kind,
                d.model,
                d.origin,
                d.health,
                d.flags,
                d.contents,
                d.solid_model,
                d.physics_solid_mesh,
                d.physics_template,
                preset,
                model_preset
            );
        }
        return Ok(());
    }
    // T6PROPS_CMODEL=<n>[,<n>...]: those submodels' bounds and brush count.
    if let Ok(list) = std::env::var("T6PROPS_CMODEL") {
        for n in parse_indices(&list) {
            if let Some(m) = clip.sub_models.get(n) {
                println!(
                    "cmodel {n}: mins {:?} maxs {:?} radius {} leaf brush node {} shares map info {}",
                    m.lo_corner, m.hi_corner, m.radius, m.leaf.brush_leaf_link, m.shares_map_info
                );
            }
        }
        println!("cmodels: {}", clip.sub_models.len());
        if let Some(world) = capture.world.as_ref() {
            let statics: std::collections::HashSet<u16> =
                world.sorted_facet_index.iter().copied().collect();
            println!(
                "gfx brush models: {}, static surfaces {} of {}",
                world.solid_models.len(),
                world.sorted_facet_index.len(),
                world.surfaces.len()
            );
            for n in parse_indices(&list) {
                if let Some((head, count, bounds)) = world.solid_models.get(n) {
                    let in_static = (*head..head + count)
                        .filter(|i| statics.contains(&(*i as u16)))
                        .count();
                    println!(
                        "gfx brush model {n}: surfaces {head}..{} ({count}), {in_static} in the static list, bounds {bounds:?}",
                        head + count
                    );
                }
            }
        }
    }
    println!(
        "clip static models: {} (header count {}), world static models: {}",
        clip.prop_set.len(),
        clip.static_model_count,
        capture.world.as_ref().map_or(0, |w| w.prop_set.len())
    );
    let near = std::env::var("T6PROPS_NEAR").ok().and_then(|v| parse_near(&v));
    // name -> (placed, instance contents set, model)
    let mut per_model: BTreeMap<String, (usize, Vec<u32>, Option<&asset_t6::XModelRef>)> =
        BTreeMap::new();
    let mut unresolved = 0;
    for sm in &clip.prop_set {
        let model = sm.model.map(|k| &capture.xmodels[k.index]);
        if model.is_none() {
            unresolved += 1;
        }
        let name = model.map_or("<none>".to_owned(), |m| m.name.clone());
        if let Some([x, y, z, r]) = near {
            let d = ((sm.origin[0] - x).powi(2)
                + (sm.origin[1] - y).powi(2)
                + (sm.origin[2] - z).powi(2))
            .sqrt();
            if d <= r {
                println!(
                    "near: {name} at {:?} contents {:#x} abs {:?}..{:?} dist {d:.0}",
                    sm.origin.map(|v| v.round()),
                    sm.contents,
                    sm.absmin.map(|v| v.round()),
                    sm.absmax.map(|v| v.round())
                );
                // T6PROPS_EXACT=1: the exact place, the inverse axis and the
                // drawn mesh's own bounds (LOD 0 vertices, model space).
                if std::env::var_os("T6PROPS_EXACT").is_some() {
                    let mut lo = [f32::MAX; 3];
                    let mut hi = [f32::MIN; 3];
                    for s in model.map(|m| m.lod0.as_slice()).unwrap_or_default() {
                        for v in s.vertex_points.chunks_exact(32) {
                            for (i, c) in v[..12].chunks_exact(4).enumerate() {
                                let f = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                                lo[i] = lo[i].min(f);
                                hi[i] = hi[i].max(f);
                            }
                        }
                    }
                    println!(
                        "      origin {:?} inv axis {:?} mesh {lo:?}..{hi:?}",
                        sm.origin, sm.reverse_scaled_basis
                    );
                    // The faces in the middle (|x| < 20, |z| < 6): each
                    // vertex's y there, rounded to a tenth, with counts.
                    let mut ys: BTreeMap<i32, usize> = BTreeMap::default();
                    for s in model.map(|m| m.lod0.as_slice()).unwrap_or_default() {
                        for v in s.vertex_points.chunks_exact(32) {
                            let f =
                                |o: usize| f32::from_le_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
                            if f(0).abs() < 20.0 && f(8).abs() < 6.0 {
                                *ys.entry((f(4) * 10.0).round() as i32).or_default() += 1;
                            }
                        }
                    }
                    println!("      middle vertex y (tenths: count) {ys:?}");
                    let mats: Vec<String> = model
                        .map(|m| {
                            m.materials
                                .iter()
                                .map(|k| {
                                    k.map_or("-".to_owned(), |k| {
                                        let mt = &capture.materials[k.index];
                                        let state = mt
                                            .state_words
                                            .first()
                                            .map(|b| asset_t6::DrawState::decode(*b));
                                        format!(
                                            "{} sort {} state0 {state:?}",
                                            mt.name, mt.sort_rank
                                        )
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    println!("      materials {mats:?}");
                    println!(
                        "      lods {:?}",
                        model.map(|m| m.detail_tiers.clone()).unwrap_or_default()
                    );
                    // T6PROPS_TRIS=1: every LOD 0 triangle (model space).
                    if std::env::var_os("T6PROPS_TRIS").is_some() {
                        for (si, s) in model
                            .map(|m| m.lod0.as_slice())
                            .unwrap_or_default()
                            .iter()
                            .enumerate()
                        {
                            let p = |i: u16| -> [f32; 3] {
                                let o = usize::from(i) * 32;
                                std::array::from_fn(|k| {
                                    let b = &s.vertex_points[o + k * 4..o + k * 4 + 4];
                                    (f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * 10.0).round()
                                        / 10.0
                                })
                            };
                            for t in s.indices.chunks_exact(3) {
                                println!(
                                    "      surf {si} tri {:?} {:?} {:?}",
                                    p(t[0]),
                                    p(t[1]),
                                    p(t[2])
                                );
                            }
                        }
                    }
                }
            }
        }
        let e = per_model.entry(name).or_insert((0, vec![], model));
        e.0 += 1;
        if !e.1.contains(&sm.contents) {
            e.1.push(sm.contents);
        }
    }
    println!("unresolved model pointers: {unresolved}");
    let mut with_coll = 0;
    let mut placed_with_coll = 0;
    let mut placed = 0;
    for (name, (count, contents, model)) in &per_model {
        placed += count;
        let Some(m) = model else {
            println!("{name}: x{count} contents {contents:x?} NO MODEL");
            continue;
        };
        let triangle_list: usize = m.collision_faces.iter().map(|s| s.triangle_list.len()).sum();
        if triangle_list > 0 && m.collision_detail_tier >= 0 {
            with_coll += 1;
            placed_with_coll += count;
        }
        let surfs: Vec<String> = m
            .collision_faces
            .iter()
            .take(4)
            .map(|s| {
                format!(
                    "[{} tris c={:#x} sf={:#x} (type {}) bone {} {:?}..{:?}]",
                    s.triangle_list.len(),
                    s.contents,
                    s.face_property_bits,
                    (s.face_property_bits >> 20) & 0x1f,
                    s.joint_position,
                    s.lo_corner.map(|v| (v * 10.0).round() / 10.0),
                    s.hi_corner.map(|v| (v * 10.0).round() / 10.0)
                )
            })
            .collect();
        println!(
            "{name}: x{count} inst contents {contents:x?} model contents {:#x} collLod {} collSurfs {} tris {triangle_list} radius {:.0} {}",
            m.contents,
            m.collision_detail_tier,
            m.collision_faces.len(),
            m.radius,
            surfs.join(" ")
        );
    }
    println!(
        "models: {} placed kinds, {with_coll} with collision; placements {placed}, {placed_with_coll} with collision",
        per_model.len()
    );
    // T6PROPS_MAT=<substring>: world surfaces drawn with matching materials
    // (bounds, surface type) and the clip materials of that name.
    if let (Ok(want), Some(world)) = (std::env::var("T6PROPS_MAT"), capture.world.as_ref()) {
        let mut groups: BTreeMap<String, Vec<&asset_t6::WorldSurface>> = BTreeMap::new();
        for srf in &world.surfaces {
            let Some(k) = srf.material else { continue };
            let m = &capture.materials[k.index];
            if m.name.contains(want.as_str()) {
                groups.entry(m.name.clone()).or_default().push(srf);
            }
        }
        for (name, list) in &groups {
            let m = capture.materials.iter().find(|m| &m.name == name);
            println!(
                "world material {name}: {} surfaces, surface flags {:#x} (type {}) game flags {:#x} type bits {:#x}",
                list.len(),
                m.map_or(0, |m| m.surf_bits),
                m.map_or(0, |m| (m.surf_bits >> 20) & 0x1f),
                m.map_or(0, |m| m.game_flags),
                m.map_or(0, |m| m.facet_type_bits)
            );
            for srf in list.iter().take(12) {
                println!(
                    "   bounds {:?}..{:?} tris {}",
                    srf.bounds[0].map(|v| v.round()),
                    srf.bounds[1].map(|v| v.round()),
                    srf.tri_tally
                );
            }
        }
        if let Some(clip) = capture.clip.as_ref() {
            for m in clip
                .materials
                .iter()
                .filter(|m| m.name.contains(want.as_str()))
            {
                println!(
                    "clip material {}: surface flags {:#x} (type {}) contents {:#x}",
                    m.name,
                    m.surface_flags,
                    (m.surface_flags >> 20) & 0x1f,
                    m.pack_presence_bits
                );
            }
        }
    }
    // T6PROPS_AT=x,y,z,r: world surfaces whose bounds come within r of a
    // point, with their material's game flags and surface type bits.
    if let (Ok(at), Some(world)) = (std::env::var("T6PROPS_AT"), capture.world.as_ref()) {
        let v: Vec<f32> = at
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        if let [x, y, z, r] = v[..] {
            for srf in &world.surfaces {
                let [lo, hi] = srf.bounds;
                let d2: f32 = [x, y, z]
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (lo[i] - p).max(0.0).max(p - hi[i]).powi(2))
                    .sum();
                if d2 > r * r {
                    continue;
                }
                let Some(k) = srf.material else { continue };
                let m = &capture.materials[k.index];
                println!(
                    "surface at {:?}..{:?} tris {} material {} type {} game flags {:#x} type bits {:#x}",
                    lo.map(|v| v.round()),
                    hi.map(|v| v.round()),
                    srf.tri_tally,
                    m.name,
                    (m.surf_bits >> 20) & 0x1f,
                    m.game_flags,
                    m.facet_type_bits
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_skip_what_is_not_a_number() {
        assert_eq!(parse_indices("1, 2,x,,30"), [1, 2, 30]);
        assert!(parse_indices("").is_empty());
        assert!(parse_indices("-1").is_empty());
    }

    #[test]
    fn near_needs_exactly_four_numbers() {
        assert_eq!(parse_near("1,2.5, -3,100"), Some([1.0, 2.5, -3.0, 100.0]));
        assert_eq!(parse_near("1,2,3"), None);
        assert_eq!(parse_near("1,2,3,4,5"), None);
        assert_eq!(parse_near("1,2,three,4"), None);
    }
}
