//! bo2zm: where Black Ops II keeps a HUD icon's pixels, read-only.
//! `t6icon <material>...` looks each material up in Nuketown's zones
//! (common_zm, zm_nuked, patch_zm) and prints its colour map's image: size,
//! levels, and whether its pixels sit in an image pack or in the zone.
//! `T6ICON_ZONES=common_mp,ui_mp` looks in those zones instead;
//! `T6ICON_PNG=<dir>` also writes each pack image's top level there as
//! `<image>.png` (RGBA), and its material's constants are printed.
//! A name that is a model prints its LOD0 surfaces: material, vertex count
//! and the uv_semantic range (half2 @20 of each 32-byte vertex).

use std::path::Path;

use asset_t6::{ImageSource, PackSet, capture_zone};

/// Write the pack entry's top level as `<dir>/<name>.png`, when a directory was asked for.
fn save_pack_image(
    packs: &PackSet,
    (pack, entry): (usize, ipak_t6::IndexEntry),
    png_dir: Option<&str>,
    name: &str,
) {
    let Some(dir) = png_dir else { return };
    let Ok(bytes) = packs.packs[pack].read(entry) else { return };
    if let Ok(iwi) = ipak_t6::parse_iwi(&bytes) {
        write_png(dir, name, &iwi);
    }
}

/// `T6ICON_BYNAME`: look each name up in the packs by its hash alone.
fn report_by_name(packs: &PackSet, wanted: &[String], png_dir: Option<&str>) {
    let all_packs = std::env::var_os("T6ICON_ALLPACKS").is_some();
    for n in wanted {
        if all_packs {
            let h = ipak_t6::hash_name(n.trim_start_matches(','));
            for (p, e) in packs.packs.iter().flat_map(|p| p.find_name(h).into_iter().map(move |e| (p, e))) {
                println!("  {n}: pack {} key {:x} size {}", p.name, e.lookup_key, e.stored_len);
            }
        }
        let Some(found) = packs.locate_name(n) else {
            println!("{n}: in no pack");
            continue;
        };
        println!("{n}: pack {} size {}", packs.packs[found.0].name, found.1.stored_len);
        save_pack_image(packs, found, png_dir, n);
    }
}

/// One zone's materials among `names`: technique, constants, and where each texture's pixels are.
fn report_zone(packs: &PackSet, zone: &str, wanted: &[String], png_dir: Option<&str>) {
    let capture = match capture_zone(&bo2zm_tools::zones_dir().join(format!("{zone}.ff"))) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{zone}: {e}");
            return;
        }
    };
    // T6ICON_LIST=1: every material the zone's capture holds.
    if std::env::var_os("T6ICON_LIST").is_some() {
        println!("{zone}: {} materials, {} images", capture.materials.len(), capture.images.len());
        capture.materials.iter().for_each(|m| println!("  {}", m.name));
    }
    for x in capture.xmodels.iter().filter(|x| wanted.contains(&x.name)) {
        println!("{zone} model {} surfaces {}", x.name, x.lod0.len());
        for (i, surf) in x.lod0.iter().enumerate() {
            let material = surf
                .material
                .and_then(|k| capture.materials.get(k.index))
                .map_or("?", |m| m.name.as_str());
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in surf.vertex_points.chunks_exact(32) {
                for c in 0..2 {
                    let uv = f16_to_f32(u16::from_le_bytes([v[20 + 2 * c], v[21 + 2 * c]]));
                    lo[c] = lo[c].min(uv);
                    hi[c] = hi[c].max(uv);
                }
            }
            println!(
                "  surf {i} {material} verts {} flags {:#x} uv {:.3},{:.3}..{:.3},{:.3}",
                surf.vertex_tally, surf.flags, lo[0], lo[1], hi[0], hi[1]
            );
        }
    }
    for m in capture.materials.iter().filter(|m| wanted.contains(&m.name)) {
        let techniques = m
            .method_set
            .and_then(|k| capture.technique_sets.get(k.index))
            .map_or("", |t| t.name.as_str());
        println!("{zone} {} technique {techniques} constants {:?}", m.name, m.constants);
        // T6ICON_STATES=1: the draw state of every technique slot the material has.
        if std::env::var_os("T6ICON_STATES").is_some() {
            for (t, d) in (0..36).filter_map(|t| m.draw_state(t).map(|d| (t, d))) {
                println!("  tech {t}: src {} dst {} op {} atest {:?} rgb {} a {}", d.src_blend, d.dst_blend, d.blend_op, d.alpha_test, d.color_write_rgb, d.color_write_alpha);
            }
        }
        let images = m
            .textures
            .iter()
            .filter_map(|t| Some((t, t.image.and_then(|k| capture.images.get(k.index))?)));
        for (t, img) in images {
            let source = match packs.locate(img) {
                Some(ImageSource::Pack(i, entry)) => {
                    save_pack_image(packs, (i, entry), png_dir, &img.name);
                    format!("pack {i} ({})", pack_image_facts(packs, (i, entry)))
                }
                Some(ImageSource::Embedded) => {
                    let e = img.embedded.as_ref().unwrap();
                    format!(
                        "zone dxgi {} levels {} {} bytes",
                        e.dxgi_format,
                        e.tier_total,
                        e.data.len()
                    )
                }
                Some(ImageSource::Empty) => "empty".to_owned(),
                None => "nowhere".to_owned(),
            };
            println!(
                "  slot {:08x} semantic {} image {} (hash {:08x}, of name {:08x}) {}x{} levels {} streamed {} -> {source}",
                t.label_digest,
                t.semantic,
                img.name,
                img.label_digest,
                ipak_t6::hash_name(img.name.trim_start_matches(',')),
                img.width,
                img.height,
                img.tier_total,
                img.streamed_parts
            );
        }
    }
}

/// bo2zm: what a pack hands over for an image: the decoded size and format, and how many
/// distinct 8-byte blocks its top level has (a blank or placeholder level has a handful).
fn pack_image_facts(packs: &PackSet, (pack, entry): (usize, ipak_t6::IndexEntry)) -> String {
    let Ok(bytes) = packs.packs[pack].read(entry) else {
        return "unreadable".to_owned();
    };
    let Ok(w) = ipak_t6::parse_iwi(&bytes) else {
        return "unreadable".to_owned();
    };
    let top = w.level(0);
    let distinct: std::collections::HashSet<&[u8]> = top.chunks(8).collect();
    format!(
        "{:?} {}x{} levels {} bytes {} total {} distinct blocks {}",
        w.format,
        w.texel_width,
        w.height,
        w.levels,
        top.len(),
        bytes.len(),
        distinct.len()
    )
}

/// Copy a decoded 4x4 RGBA `tile` into `rgba` (w x h pixels) as block (`bx`, `by`), clipped at the edges.
fn blit_tile(rgba: &mut [u8], (w, h): (usize, usize), tile: &[u8; 64], (bx, by): (usize, usize)) {
    let (x0, y0) = (bx * 4, by * 4);
    let span = 4.min(w.saturating_sub(x0)) * 4;
    for (row, line) in tile.chunks_exact(16).enumerate().take(h.saturating_sub(y0)) {
        let d = ((y0 + row) * w + x0) * 4;
        rgba[d..d + span].copy_from_slice(&line[..span]);
    }
}

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let packs = PackSet::open_dir(&bo2zm_tools::zones_dir()).unwrap_or_else(|e| {
        eprintln!("packs: {e}");
        std::process::exit(1)
    });
    let zones: Vec<String> = std::env::var("T6ICON_ZONES").map_or_else(
        |_| ["common_zm", "zm_nuked", "patch_zm"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let png_dir = std::env::var("T6ICON_PNG").ok();
    let png_dir = png_dir.as_deref();
    // T6ICON_BYNAME=1: each name as an image name, looked up in the packs
    // by its hash alone (how a `,name` reference finds its pixels).
    if std::env::var_os("T6ICON_BYNAME").is_some() {
        report_by_name(&packs, &names, png_dir);
        return;
    }
    for zone in &zones {
        report_zone(&packs, zone, &names, png_dir);
    }
}

/// An image's top level as an RGBA PNG (BC1/BC2/BC3 and RGBA8 only).
fn write_png(dir: &str, name: &str, iwi: &ipak_t6::IwiImage<'_>) {
    use ipak_t6::IwiFormat;
    let (w, h) = (iwi.texel_width as usize, iwi.height as usize);
    let data = iwi.level(0);
    let (block, decode): (usize, fn(&[u8], &mut [u8], usize)) = match iwi.format {
        IwiFormat::Dxt1 => (8, bcdec_rs::bc1),
        IwiFormat::Dxt3 => (16, bcdec_rs::bc2),
        IwiFormat::Dxt5 => (16, bcdec_rs::bc3),
        IwiFormat::Rgba8 => (0, |_, _, _| {}),
        other => {
            println!("  (not written: {other:?})");
            return;
        }
    };
    let mut rgba = vec![0u8; w * h * 4];
    if block == 0 {
        let n = rgba.len().min(data.len());
        rgba[..n].copy_from_slice(&data[..n]);
    } else {
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        let mut decoded = [0u8; 64];
        let blocks = (0..bh).flat_map(|by| (0..bw).map(move |bx| (bx, by)));
        for (bx, by) in blocks {
            let at = (by * bw + bx) * block;
            let Some(src) = data.get(at..at + block) else { continue };
            decode(src, &mut decoded, 16);
            blit_tile(&mut rgba, (w, h), &decoded, (bx, by));
        }
    }
    let path = Path::new(dir).join(format!("{name}.png"));
    if let Ok(file) = std::fs::File::create(&path) {
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut wr) = enc.write_header() {
            let _ = wr.write_image_data(&rgba);
        }
        println!("  wrote {} ({:?})", path.to_string_lossy(), iwi.format);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_of(v: u8) -> [u8; 64] {
        let mut t = [0u8; 64];
        for (i, b) in t.iter_mut().enumerate() {
            *b = v + (i / 4) as u8;
        }
        t
    }

    #[test]
    fn a_whole_tile_lands_in_its_block() {
        let mut img = vec![0u8; 8 * 8 * 4];
        blit_tile(&mut img, (8, 8), &tile_of(1), (1, 1));
        assert_eq!(img[(4 * 8 + 4) * 4], 1);
        assert_eq!(img[(7 * 8 + 7) * 4 + 3], 16);
        assert_eq!(img[(3 * 8 + 4) * 4], 0);
        assert_eq!(img[(4 * 8 + 3) * 4], 0);
    }

    #[test]
    fn a_tile_at_the_edge_is_clipped_not_wrapped() {
        // 6x5 image: block (1, 1) keeps columns 4..6 and row 4 only.
        let mut img = vec![0u8; 6 * 5 * 4];
        blit_tile(&mut img, (6, 5), &tile_of(1), (1, 1));
        assert_eq!(img[(4 * 6 + 4) * 4], 1);
        assert_eq!(img[(4 * 6 + 5) * 4], 2);
        assert_eq!(img[(3 * 6 + 5) * 4], 0);
        assert_eq!(img.iter().filter(|b| **b != 0).count(), 8);
    }
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
