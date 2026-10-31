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

use asset_t6::{ImageRef, PackSet, WorldRef, ZoneCapture, capture_zone};

const HELP_BANNER: &str = "usage: t6imgslot <bo2 root> <zone>[,<zone>...] <image>[,<image>...]";

/// The command line: the BO2 folder, the zones and the image names asked for.
#[derive(Debug, PartialEq)]
struct Cli {
    zone_dir: PathBuf,
    zones: Vec<String>,
    names: Vec<String>,
}

fn comma_list(list: &str) -> Vec<String> {
    list.split(',').map(str::to_owned).collect()
}

/// `None` unless the program name and all three arguments are there.
fn read_flags(args: &[String]) -> Option<Cli> {
    let [_, root, zones, names, ..] = args else { return None };
    Some(Cli { zone_dir: PathBuf::from(root).join("zone").join("all"), zones: comma_list(zones), names: comma_list(names) })
}

/// How many bytes of an embedded image are printed: all up to 512, else the first 16.
fn preview_len(len: usize) -> usize {
    if len <= 512 { len } else { 16 }
}

/// An embedded image's format, level count, flags, size and leading bytes.
fn embedded_text(i: &ImageRef) -> String {
    i.embedded.as_ref().map_or(String::new(), |e| {
        format!(
            " embedded dxgi {} levels {} flags {:#x} {} bytes, data {:02x?}",
            e.dxgi_format,
            e.tier_total,
            e.flags,
            e.data.len(),
            &e.data[..preview_len(e.data.len())]
        )
    })
}

/// Whether an image was asked for by name; `*embedded` asks for every image held in the zone itself.
fn is_wanted(names: &[String], img: &ImageRef) -> bool {
    let asked = |name: &str| names.iter().any(|n| n == name);
    asked(img.name.trim_start_matches(',')) || (asked("*embedded") && img.embedded.is_some())
}

/// A 128x128 BC3 or BC3-sRGB cube is the probe image nobody needs to hear about.
fn is_standard_probe(img: Option<&ImageRef>) -> bool {
    img.is_some_and(|i| i.width == 128 && i.height == 128 && i.embedded.as_ref().is_some_and(|e| matches!(e.dxgi_format, 77 | 78)))
}

fn image_lines(zone: &str, cap: &ZoneCapture, packs: Option<&PackSet>, names: &[String]) -> Vec<String> {
    let wanted = cap.images.iter().filter(|img| is_wanted(names, img));
    wanted
        .map(|img| {
            format!(
                "{zone} image {} type {} {}x{}x{} levels {} streamed {} source {:?}{}",
                img.name,
                img.map_kind,
                img.width,
                img.height,
                img.depth,
                img.tier_total,
                img.streamed_parts,
                packs.and_then(|p| p.locate(img)),
                embedded_text(img)
            )
        })
        .collect()
}

/// How many world surfaces draw each material (by material index).
fn surfaces_per_material(cap: &ZoneCapture) -> HashMap<usize, usize> {
    let mut counts = HashMap::new();
    for k in cap.world.iter().flat_map(|w| &w.surfaces).filter_map(|s| s.material) {
        *counts.entry(k.index).or_default() += 1;
    }
    counts
}

/// Each texture slot that uses a wanted image; a listed name inside a
/// material's name prints every slot of it.
fn material_lines(zone: &str, cap: &ZoneCapture, names: &[String]) -> Vec<String> {
    let on_world = surfaces_per_material(cap);
    let mut found = Vec::with_capacity(cap.materials.len());
    for (mi, m) in cap.materials.iter().enumerate() {
        let ts = m.method_set.and_then(|k| cap.technique_sets.get(k.index)).map_or("", |t| t.name.as_str());
        let surfaces = on_world.get(&mi).copied().unwrap_or(0);
        let named = names.iter().any(|n| m.name.contains(n.as_str()));
        if named && m.textures.is_empty() {
            found.push(format!("{zone} material {} techset {ts} no texture slots, world surfaces {surfaces}", m.name));
        }
        let slots = m.textures.iter().filter_map(|t| Some((t, t.image.and_then(|k| cap.images.get(k.index))?)));
        for (t, img) in slots.filter(|(_, img)| named || is_wanted(names, img)) {
            found.push(format!(
                "{zone} material {} techset {ts} slot {:#010x} semantic {} sampler {:#x} image {} world surfaces {surfaces}",
                m.name, t.label_digest, t.semantic, t.texture_sampling_mode, img.name
            ));
        }
    }
    found
}

/// Every reflection probe that is not a standard cube, with the surfaces that use it.
fn probe_lines(zone: &str, cap: &ZoneCapture, world: &WorldRef) -> Vec<String> {
    let probes = world.probe_list.iter().enumerate();
    let odd = probes.map(|(k, p)| (k, p, p.image.and_then(|ik| cap.images.get(ik.index)))).filter(|&(_, _, img)| !is_standard_probe(img));
    odd.map(|(k, p, img)| {
        let users = world.surfaces.iter().filter(|s| s.env_probe_index == k as i8).count();
        format!(
            "{zone} probe {k} at {:?} image {} {}x{} levels {}{} surfaces {users}",
            p.origin,
            img.map_or("none", |i| i.name.as_str()),
            img.map_or(0, |i| i.width),
            img.map_or(0, |i| i.height),
            img.map_or(0, |i| i.tier_total),
            img.map_or(String::new(), embedded_text)
        )
    })
    .collect()
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(Cli { zone_dir: all, zones, names }) = read_flags(&args) else {
        eprintln!("{HELP_BANNER}");
        return std::process::ExitCode::FAILURE;
    };
    let packs = PackSet::open_dir(&all).ok();
    for zone in &zones {
        let cap = match capture_zone(&all.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                println!("{zone}: {e}");
                continue;
            }
        };
        let probes = cap.world.as_ref().map(|w| probe_lines(zone, &cap, w)).unwrap_or_default();
        let sections = [image_lines(zone, &cap, packs.as_ref(), &names), material_lines(zone, &cap, &names), probes];
        sections.iter().flatten().for_each(|line| println!("{line}"));
    }
    std::process::ExitCode::from(0)
}

#[cfg(test)]
mod tests {
    use asset_t6::{EmbeddedImage, ImageRef};
    use std::path::PathBuf;
    use super::{read_flags, Cli, comma_list, preview_len, is_wanted, is_standard_probe};

    fn image(name: &str, embedded: Option<u32>, size: u16) -> ImageRef {
        let embedded = embedded.map(|dxgi_format| EmbeddedImage { dxgi_format, tier_total: 1, flags: 0, data: vec![0; 4] });
        ImageRef {
            name: name.into(),
            label_digest: 0,
            map_kind: 0,
            height: size,
            width: size,
            depth: 1,
            tier_total: 1,
            streamed_parts: 0,
            part_hash: 0,
            embedded,
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_command_line_needs_a_root_zones_and_images() {
        assert_eq!(read_flags(&words(&["t6imgslot", "r", "a,b", "x"])), Some(Cli {
            zone_dir: PathBuf::from("r").join("zone").join("all"),
            zones: words(&["a", "b"]),
            names: words(&["x"]),
        }));
        assert_eq!(read_flags(&words(&["t6imgslot", "r", "a"])), None);
        assert_eq!(comma_list("a,,b"), ["a", "", "b"]);
    }

    #[test]
    fn small_embedded_images_print_whole_and_big_ones_sixteen_bytes() {
        assert_eq!((preview_len(0), preview_len(512), preview_len(513)), (0, 512, 16));
    }

    #[test]
    fn an_image_is_wanted_by_name_without_its_leading_comma_or_as_embedded() {
        let names = words(&["pic"]);
        assert!(is_wanted(&names, &image(",pic", None, 8)));
        assert!(!is_wanted(&names, &image("other", Some(77), 8)));
        let every_embedded = words(&["*embedded"]);
        assert!(is_wanted(&every_embedded, &image("other", Some(77), 8)));
        assert!(!is_wanted(&every_embedded, &image("other", None, 8)));
    }

    #[test]
    fn only_a_128_bc3_embedded_cube_is_the_standard_probe() {
        assert!(is_standard_probe(Some(&image("p", Some(77), 128))));
        assert!(is_standard_probe(Some(&image("p", Some(78), 128))));
        assert!(!is_standard_probe(Some(&image("p", Some(71), 128))));
        assert!(!is_standard_probe(Some(&image("p", Some(77), 64))));
        assert!(!is_standard_probe(Some(&image("p", None, 128))));
        assert!(!is_standard_probe(None));
    }
}
