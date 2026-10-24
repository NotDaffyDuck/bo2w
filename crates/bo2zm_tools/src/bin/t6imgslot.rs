//! bo2mp maps: where named images' pixels are and which material slots use
//! them, read-only. `t6imgslot <bo2 root> <zone>[,<zone>...] <image>[,<image>...]`
//! prints, per zone, each named image (size, levels, streamed parts, where
//! its pixels are, an embedded image's DXGI format and bytes: all
//! of them up to 512), each
//! material texture slot that uses one (with how many world surfaces draw
//! that material), and every reflection probe that is not a 128x128 BC3 cube.
//! The image name `*embedded` stands for every image held in the zone itself;
//! a name found inside a material's name prints all of that material's slots.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use asset_t6::{PackSet, capture_zone};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: t6imgslot <bo2 root> <zone>[,<zone>...] <image>[,<image>...]");
        return ExitCode::FAILURE;
    }
    let all = PathBuf::from(&args[1]).join("zone").join("all");
    let names: Vec<&str> = args[3].split(',').collect();
    let packs = PackSet::open_dir(&all).ok();
    for zone in args[2].split(',') {
        let cap = match capture_zone(&all.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                println!("{zone}: {e}");
                continue;
            }
        };
        let embedded = |i: &asset_t6::ImageRef| {
            i.embedded.as_ref().map_or(String::new(), |e| {
                format!(
                    " embedded dxgi {} levels {} flags {:#x} {} bytes, data {:02x?}",
                    e.dxgi_format,
                    e.level_count,
                    e.flags,
                    e.data.len(),
                    &e.data[..e.data.len().min(if e.data.len() <= 512 { 512 } else { 16 })]
                )
            })
        };
        // `*embedded` in the image list: every image whose pixels are in the
        // zone itself.
        let wanted = |img: &asset_t6::ImageRef| {
            names.contains(&img.name.trim_start_matches(','))
                || (names.contains(&"*embedded") && img.embedded.is_some())
        };
        for img in &cap.images {
            if !wanted(img) {
                continue;
            }
            let src = packs.as_ref().and_then(|p| p.locate(img));
            println!(
                "{zone} image {} type {} {}x{}x{} levels {} streamed {} source {:?}{}",
                img.name,
                img.map_type,
                img.width,
                img.height,
                img.depth,
                img.level_count,
                img.streamed_parts,
                src,
                embedded(img)
            );
        }
        let mut on_world: HashMap<usize, usize> = HashMap::new();
        if let Some(world) = &cap.world {
            for s in &world.surfaces {
                if let Some(k) = s.material {
                    *on_world.entry(k.index).or_default() += 1;
                }
            }
        }
        for (mi, m) in cap.materials.iter().enumerate() {
            let ts = m
                .technique_set
                .and_then(|k| cap.technique_sets.get(k.index))
                .map_or("", |t| t.name.as_str());
            // A listed name inside a material's name prints every slot of it.
            let named = names.iter().any(|n| m.name.contains(n));
            if named && m.textures.is_empty() {
                println!("{zone} material {} techset {ts} no texture slots, world surfaces {}", m.name, on_world.get(&mi).copied().unwrap_or(0));
            }
            for t in &m.textures {
                let Some(img) = t.image.and_then(|k| cap.images.get(k.index)) else {
                    continue;
                };
                if wanted(img) || named {
                    println!(
                        "{zone} material {} techset {ts} slot {:#010x} semantic {} sampler {:#x} image {} world surfaces {}",
                        m.name,
                        t.name_hash,
                        t.semantic,
                        t.sampler_state,
                        img.name,
                        on_world.get(&mi).copied().unwrap_or(0)
                    );
                }
            }
        }
        if let Some(world) = &cap.world {
            for (k, p) in world.reflection_probes.iter().enumerate() {
                let img = p.image.and_then(|k| cap.images.get(k.index));
                let cube = img.and_then(|i| i.embedded.as_ref()).is_some_and(|e| {
                    matches!(e.dxgi_format, 77 | 78) && img.is_some_and(|i| i.width == 128 && i.height == 128)
                });
                if !cube {
                    let users = world
                        .surfaces
                        .iter()
                        .filter(|s| s.reflection_probe_index == k as i8)
                        .count();
                    println!(
                        "{zone} probe {k} at {:?} image {} {}x{} levels {}{} surfaces {users}",
                        p.origin,
                        img.map_or("none", |i| i.name.as_str()),
                        img.map_or(0, |i| i.width),
                        img.map_or(0, |i| i.height),
                        img.map_or(0, |i| i.level_count),
                        img.map_or(String::new(), embedded)
                    );
                }
            }
        }
    }
    ExitCode::SUCCESS
}
