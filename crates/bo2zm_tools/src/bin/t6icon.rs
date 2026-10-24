//! bo2zm: where Black Ops II keeps a HUD icon's pixels, read-only.
//! `t6icon <material>...` looks each material up in Nuketown's zones
//! (common_zm, zm_nuked, patch_zm) and prints its colour map's image: size,
//! levels, and whether its pixels sit in an image pack or in the zone.
//! `T6ICON_ZONES=zm_highrise,zm_highrise_patch` looks in those zones instead.
//! A name that is a model prints its LOD0 surfaces: material, vertex count
//! and the texcoord range (half2 @20 of each 32-byte vertex).

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{ImageSource, PackSet, capture_zone};

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let dir = Path::new(ZONES);
    let packs = match PackSet::open_dir(dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("packs: {e}");
            return ExitCode::FAILURE;
        }
    };
    let zones: Vec<String> = std::env::var("T6ICON_ZONES").map_or_else(
        |_| {
            ["common_zm", "zm_nuked", "patch_zm"]
                .map(String::from)
                .to_vec()
        },
        |v| v.split(',').map(String::from).collect(),
    );
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        // T6ICON_LIST=1: every material the zone's capture holds.
        if std::env::var_os("T6ICON_LIST").is_some() {
            println!("{zone}: {} materials, {} images", capture.materials.len(), capture.images.len());
            for m in &capture.materials {
                println!("  {}", m.name);
            }
        }
        for x in capture
            .xmodels
            .iter()
            .filter(|x| names.iter().any(|n| n == &x.name))
        {
            println!("{zone} model {} surfaces {}", x.name, x.lod0.len());
            for (i, surf) in x.lod0.iter().enumerate() {
                let material = surf
                    .material
                    .and_then(|k| capture.materials.get(k.index))
                    .map_or("?", |m| m.name.as_str());
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for v in surf.verts.chunks_exact(32) {
                    for c in 0..2 {
                        let h = u16::from_le_bytes([v[20 + 2 * c], v[21 + 2 * c]]);
                        let f = f16_to_f32(h);
                        lo[c] = lo[c].min(f);
                        hi[c] = hi[c].max(f);
                    }
                }
                println!(
                    "  surf {i} {material} verts {} flags {:#x} uv {:.3},{:.3}..{:.3},{:.3}",
                    surf.vert_count, surf.flags, lo[0], lo[1], hi[0], hi[1]
                );
            }
        }
        for m in capture
            .materials
            .iter()
            .filter(|m| names.iter().any(|n| n == &m.name))
        {
            let techniques = m
                .technique_set
                .and_then(|k| capture.technique_sets.get(k.index))
                .map_or("", |t| t.name.as_str());
            println!("{zone} {} technique {techniques}", m.name);
            for t in &m.textures {
                let Some(img) = t.image.and_then(|k| capture.images.get(k.index)) else {
                    continue;
                };
                let source = match packs.locate(img) {
                    Some(ImageSource::Pack(i, entry)) => {
                        // What the pack hands over: the decoded size and format.
                        let iwi = packs.packs[i].read(entry).ok().and_then(|b| {
                            ipak_t6::parse_iwi(&b).ok().map(|w| {
                                // Distinct 8-byte blocks in the top level: a blank
                                // or placeholder level has a handful.
                                let top = w.level(0);
                                let distinct: std::collections::HashSet<&[u8]> =
                                    top.chunks(8).collect();
                                format!(
                                    "{:?} {}x{} levels {} bytes {} total {} distinct blocks {}",
                                    w.format,
                                    w.width,
                                    w.height,
                                    w.levels,
                                    top.len(),
                                    b.len(),
                                    distinct.len()
                                )
                            })
                        });
                        format!("pack {i} ({})", iwi.as_deref().unwrap_or("unreadable"))
                    }
                    Some(ImageSource::Embedded) => {
                        let e = img.embedded.as_ref().unwrap();
                        format!(
                            "zone dxgi {} levels {} {} bytes",
                            e.dxgi_format,
                            e.level_count,
                            e.data.len()
                        )
                    }
                    Some(ImageSource::Empty) => "empty".to_owned(),
                    None => "nowhere".to_owned(),
                };
                println!(
                    "  slot {:08x} semantic {} image {} {}x{} levels {} streamed {} -> {source}",
                    t.name_hash,
                    t.semantic,
                    img.name,
                    img.width,
                    img.height,
                    img.level_count,
                    img.streamed_parts
                );
            }
        }
    }
    ExitCode::SUCCESS
}

/// An IEEE half float.
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f32::from(h & 0x3ff);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}
