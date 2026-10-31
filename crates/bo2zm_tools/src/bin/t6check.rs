//! bo2zm: readiness checks for a Black Ops II map, read-only.
//! `t6check [BO2 folder]` walks the nine Nuketown Zombies zones and checks
//! that every image they use has pixels: in the zone or in a pack, decoding
//! to an IWI whose size matches the zone's description of it.
//! `t6check <BO2 folder> <zone>...` checks those zones instead (found in the
//! install or the `BO2MP_T6_EXTRA` folders) and lists every missing image.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use asset_t6::{ImageRef, ImageSource, PackSet, capture_zone};

const NUKETOWN_ZONES: [&str; 9] = [
    "code_pre_gfx_zm",
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "ui_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
    "zm_nuked",
    "zm_nuked_patch",
];

/// How many of each kind of problem are listed.
const LISTED: usize = 12;

/// The ways an image can fail the check, in the order they are reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Missing,
    SizeMismatch,
    UnreadableBytes,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Self::Missing => "MISSING",
            Self::SizeMismatch => "SIZE MISMATCH",
            Self::UnreadableBytes => "DECODE FAILED",
        }
    }
}

/// What looking one image up found: where its pixels are, and what is wrong with them.
struct Checked {
    origin: Option<String>,
    problem: Option<(Kind, String)>,
}

fn iwi_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    let iwi = ipak_t6::parse_iwi(bytes).map_err(|e| e.to_string())?;
    Ok((iwi.texel_width, iwi.height))
}

/// A problem when the pack's picture is not the size the zone describes.
fn size_problem(image: &ImageRef, (w, h): (u32, u32)) -> Option<(Kind, String)> {
    let described = (u32::from(image.width), u32::from(image.height));
    (described != (w, h)).then(|| {
        let message = format!("{name} zone {zw}x{zh} pack {w}x{h}", name = image.name, zw = image.width, zh = image.height);
        (Kind::SizeMismatch, message)
    })
}

fn check_image(packs: &PackSet, zone: &str, image: &ImageRef) -> Checked {
    match packs.locate(image) {
        Some(ImageSource::Pack(i, entry)) => {
            let pack = &packs.packs[i];
            let problem = match pack.read(entry).and_then(|bytes| iwi_size(&bytes)) {
                Ok(size) => size_problem(image, size),
                Err(e) => Some((Kind::UnreadableBytes, format!("{}: {e}", image.name))),
            };
            Checked { origin: Some(pack.name.clone()), problem }
        }
        Some(ImageSource::Embedded) => Checked { origin: Some("in the zone".into()), problem: None },
        Some(ImageSource::Empty) => Checked { origin: Some("no pixels (runtime image)".into()), problem: None },
        None => Checked { origin: None, problem: Some((Kind::Missing, format!("{} ({zone})", image.name))) },
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok { "ALL IMAGES OK" } else { "IMAGE CHECK FAILED" }
}

fn run(root: &Path, zones: &[String]) -> Result<bool, String> {
    let clock = Instant::now();
    let dir = root.join("zone").join("all");
    let packs = PackSet::open_dir(&dir)?;
    let listing: Vec<String> = packs.packs.iter().map(|p| format!("{} ({})", p.name, p.len())).collect();
    println!("packs: {}", listing.join(", "));

    let mut images_total = 0usize;
    let mut by_source: BTreeMap<String, usize> = BTreeMap::default();
    let mut problems: BTreeMap<Kind, Vec<String>> = BTreeMap::new();
    let names: Vec<&str> = match zones.is_empty() {
        true => NUKETOWN_ZONES.to_vec(),
        false => zones.iter().map(String::as_str).collect(),
    };
    // Zones named on the command line list every problem.
    let listed = if zones.is_empty() { LISTED } else { usize::MAX };
    for zone in names {
        let path = asset_transport::t6_extra::file_in(&dir, &format!("{zone}.ff"));
        let capture = capture_zone(&path)?;
        println!("{zone}: {} images", capture.images.len());
        if std::env::var_os("T6CHECK_DUMP").is_some() {
            dump(&capture);
        }
        images_total += capture.images.len();
        for Checked { origin, problem } in capture.images.iter().map(|image| check_image(&packs, zone, image)) {
            if let Some(pack) = origin {
                *by_source.entry(pack).or_default() += 1;
            }
            if let Some((sort, message)) = problem {
                problems.entry(sort).or_default().push(message);
            }
        }
    }
    println!("images: {images_total}");
    by_source.iter().for_each(|(k, n)| println!("   {n:>6}  {k}"));
    for (sort, list) in &problems {
        println!("{}: {}", sort.tag(), list.len());
        list.iter().take(listed).for_each(|item| println!("   {item}"));
    }
    let ok = problems.is_empty();
    println!("{} in {:.2?}", verdict(ok), clock.elapsed());
    Ok(ok)
}

/// `T6CHECK_DUMP=1`: every model's materials and every material's technique set, sort key
/// and images, one line each.
fn dump(c: &asset_t6::ZoneCapture) {
    let mat = |k: &Option<asset_t6::AssetIdentity>| {
        k.and_then(|k| c.materials.get(k.index)).map_or("?", |m| m.name.as_str())
    };
    for i in &c.images {
        if let Some(e) = &i.embedded {
            println!(
                "  embedded {} {}x{} dxgi={} levels={} flags={:#x} bytes={}",
                i.name,
                i.width,
                i.height,
                e.dxgi_format,
                e.tier_total,
                e.flags,
                e.data.len()
            );
        }
    }
    for m in &c.xmodels {
        let mats: Vec<&str> = m.materials.iter().map(mat).collect();
        println!("  xmodel {} [{}]", m.name, mats.join(" "));
    }
    for m in &c.materials {
        let ts = m
            .method_set
            .and_then(|k| c.technique_sets.get(k.index))
            .map_or("?", |t| t.name.as_str());
        let imgs: Vec<String> = m
            .textures
            .iter()
            .map(|t| {
                let n = t.image.and_then(|k| c.images.get(k.index)).map_or("?", |i| i.name.as_str());
                format!("{}:{n}", t.semantic)
            })
            .collect();
        println!("  material {} ts={ts} sort={} [{}]", m.name, m.sort_rank, imgs.join(" "));
    }
}

fn main() -> std::process::ExitCode {
    let root = std::env::args().nth(1).map_or_else(bo2zm_tools::game_dir, PathBuf::from);
    let zones: Vec<String> = std::env::args().skip(2).collect();
    let ok = run(&root, &zones).unwrap_or_else(|e| {
        eprintln!("t6check: {e}");
        false
    });
    std::process::ExitCode::from(u8::from(!ok))
}

#[cfg(test)]
mod tests {
    use asset_t6::ImageRef;
    use super::{size_problem, Kind, iwi_size, verdict};

    fn image(width: u16, height: u16) -> ImageRef {
        ImageRef {
            name: "img".into(),
            label_digest: 0,
            map_kind: 0,
            width,
            height,
            depth: 1,
            tier_total: 1,
            streamed_parts: 0,
            part_hash: 0,
            embedded: None,
        }
    }

    #[test]
    fn a_pack_picture_of_the_described_size_is_fine() {
        assert_eq!(size_problem(&image(64, 32), (64, 32)), None);
    }

    #[test]
    fn a_pack_picture_of_another_size_names_both_sizes() {
        let (kind, message) = size_problem(&image(64, 32), (128, 32)).unwrap();
        assert_eq!(kind, Kind::SizeMismatch);
        assert_eq!(message, "img zone 64x32 pack 128x32");
    }

    #[test]
    fn problems_are_reported_missing_then_size_then_decode() {
        let mut kinds = [Kind::UnreadableBytes, Kind::Missing, Kind::SizeMismatch];
        kinds.sort();
        assert_eq!(kinds.map(Kind::tag), ["MISSING", "SIZE MISMATCH", "DECODE FAILED"]);
    }

    #[test]
    fn bytes_that_are_no_picture_fail_to_decode() {
        assert!(iwi_size(&[]).is_err());
    }

    #[test]
    fn the_closing_line_follows_the_outcome() {
        assert_eq!(verdict(true), "ALL IMAGES OK");
        assert_eq!(verdict(false), "IMAGE CHECK FAILED");
    }
}
