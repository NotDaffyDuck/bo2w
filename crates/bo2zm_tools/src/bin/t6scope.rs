//! bo2mp: what a Black Ops II gun's definition says about aiming down its
//! sights, read-only. `t6scope <weapon>...` (e.g. `svu_mp dsr50_mp`) looks
//! each weapon up in the multiplayer zones (`T6SCOPE_ZONES` to change them)
//! and prints its scope overlay material and picture, the overlay's size and
//! reticle kind, the ADS zoom fields, transition times, sway, hold-breath
//! flag and ADS sounds. `T6SCOPE_PNG=<dir>` also writes each overlay picture
//! (top level, RGBA) there as `<image>.png`.

use std::path::Path;

use asset_t6::{ImageSource, PackSet, capture_zone};
use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};

/// A run that already said why on its own.
struct Failed;

/// The lowest and highest x, y, z over 32-byte vertex rows (first 12 bytes of each row).
fn vertex_box(vertex_points: &[u8]) -> ([f32; 3], [f32; 3]) {
    let at = |row: &[u8], axis: usize| f32::from_le_bytes([row[axis * 4], row[axis * 4 + 1], row[axis * 4 + 2], row[axis * 4 + 3]]);
    vertex_points.chunks_exact(32).fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), row| {
        (
            std::array::from_fn(|a| lo[a].min(at(row, a))),
            std::array::from_fn(|a| hi[a].max(at(row, a))),
        )
    })
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
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let dir = &bo2zm_tools::zones_dir();
    let packs = PackSet::open_dir(dir).map_err(|e| {
        eprintln!("packs: {e}");
        Failed
    })?;
    let zones: Vec<String> = std::env::var("T6SCOPE_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let png_dir = std::env::var("T6SCOPE_PNG").ok();
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        // `T6SCOPE_LOC=zoom`: every localized string whose key or text holds
        // it (a language zone, e.g. `T6SCOPE_ZONES=../english/en_patch_mp`).
        if let Ok(pat) = std::env::var("T6SCOPE_LOC") {
            for (k, t) in &capture.localize {
                if k.to_ascii_lowercase().contains(&pat) || t.to_ascii_lowercase().contains(&pat) {
                    println!("{zone} loc {k} = {t}");
                }
            }
        }
        // `T6SCOPE_MODELS=a,b`: those models' bounds and surfaces' extents.
        if let Ok(list) = std::env::var("T6SCOPE_MODELS") {
            for m in capture.xmodels.iter().filter(|m| list.split(',').any(|n| n == m.name)) {
                println!("{zone} model {} mins {:?} maxs {:?} radius {}", m.name, m.lo_corner, m.hi_corner, m.radius);
                // Each first-LOD surface: its material and its vertices' box.
                for (i, s) in m.lod0.iter().enumerate() {
                    let (lo, hi) = vertex_box(&s.vertex_points);
                    let mat = s
                        .material
                        .and_then(|k| capture.materials.get(k.index))
                        .map_or("-", |m| m.name.as_str());
                    println!("    surf {i} `{mat}` verts {} box {lo:?} .. {hi:?}", s.vertex_tally);
                }
            }
        }
        // `T6SCOPE_ALIAS=breath`: every sound alias whose name holds it.
        if let Ok(pat) = std::env::var("T6SCOPE_ALIAS") {
            for bank in &capture.sound_banks {
                for a in bank.aliases.iter().filter(|a| a.name.to_ascii_lowercase().contains(&pat)) {
                    println!("{zone} alias {} ({} entries) bank {}", a.name, a.entries.len(), bank.name);
                }
            }
        }
        for w in capture.weapons.iter().filter(|w| {
            names.is_empty() && !w.hud_layer_surface.is_empty() || names.iter().any(|n| n == &w.name)
        }) {
            println!("{zone} {}", w.name);
            println!(
                "  overlay `{}` low `{}` reticle {} interface {} size {}x{} alphaScale {} holdBreath {} antiQuickScope {}",
                w.hud_layer_surface,
                w.overlay_material_low,
                w.def_i32(WeaponDef::overlayReticle),
                w.def_i32(WeaponDef::overlayInterface),
                w.def_f32(WeaponDef::overlayWidth),
                w.def_f32(WeaponDef::overlayHeight),
                w.var_f32(WeaponVariantDef::fOverlayAlphaScale),
                w.def_u8(WeaponDef::bHoldBreathToSteady),
                w.var_u8(WeaponVariantDef::bAntiQuickScope),
            );
            println!(
                "  zoomFov {} {} {} inFrac {} outFrac {} transIn {}ms transOut {}ms aimDownSight {} adsFireOnly {}",
                w.var_f32(WeaponVariantDef::fAdsZoomFov1),
                w.var_f32(WeaponVariantDef::fAdsZoomFov2),
                w.var_f32(WeaponVariantDef::fAdsZoomFov3),
                w.var_f32(WeaponVariantDef::fAdsZoomInFrac),
                w.var_f32(WeaponVariantDef::fAdsZoomOutFrac),
                w.var_i32(WeaponVariantDef::iAdsTransInTime),
                w.var_i32(WeaponVariantDef::iAdsTransOutTime),
                w.def_u8(WeaponDef::aimDownSight),
                w.def_u8(WeaponDef::adsFireOnly),
            );
            println!(
                "  adsIdle {} speed {} hipIdle {} adsSway max {} lerp {} pitch {} yaw {} horiz {} vert {} adsBobFactor {} adsViewBobMult {}",
                w.def_f32(WeaponDef::fAdsIdleAmount),
                w.def_f32(WeaponDef::adsIdleSpeed),
                w.def_f32(WeaponDef::fHipIdleAmount),
                w.def_f32(WeaponDef::adsSwayMaxAngle),
                w.def_f32(WeaponDef::adsSwayLerpSpeed),
                w.def_f32(WeaponDef::adsSwayPitchScale),
                w.def_f32(WeaponDef::adsSwayYawScale),
                w.var_f32(WeaponVariantDef::fAdsSwayHorizScale),
                w.var_f32(WeaponVariantDef::fAdsSwayVertScale),
                w.def_f32(WeaponDef::fAdsBobFactor),
                w.def_f32(WeaponDef::fAdsViewBobMult),
            );
            println!(
                "  sounds adsRaise `{}` adsLower `{}` adsZoom `{}` reticle centre `{}` side `{}` dof {} {}",
                w.def_string(WeaponDef::adsRaiseSoundPlayer),
                w.def_string(WeaponDef::adsLowerSoundPlayer),
                w.def_string(WeaponDef::adsZoomSound),
                w.def_material(WeaponDef::reticleCenter),
                w.def_material(WeaponDef::reticleSide),
                w.def_f32(WeaponDef::adsDofStart),
                w.def_f32(WeaponDef::adsDofEnd),
            );
            if std::env::var_os("T6SCOPE_FULL").is_some() {
                println!("  hide {:?} attach {:?}", w.concealed_bones, w.attach_view_models);
                for (i, a) in w.animation_refs.iter().enumerate().filter(|(_, a)| !a.is_empty()) {
                    print!(" {i:02x}={a}");
                }
                println!();
            }
            let base = w.name.trim_end_matches("_mp");
            let prefix = format!("au_{base}_");
            for au in capture.attachment_uniques.iter().filter(|a| a.name.starts_with(&prefix)) {
                use fastfile_t6::layout::WeaponAttachmentUnique;
                let kind = &au.name[prefix.len()..];
                println!(
                    "  {} overlay `{}` reticle {} view `{}` +`{}` ads `{}` tag `{}` ofs {:?} rot {:?} anims {} alt `{}`",
                    au.name,
                    au.hud_layer_surface,
                    au.int_from_bytes(WeaponAttachmentUnique::overlayReticle),
                    au.view_model,
                    au.view_model_additional,
                    au.view_model_ads,
                    au.view_model_tag,
                    au.vec3_at(WeaponAttachmentUnique::viewModelOffsets),
                    au.vec3_at(WeaponAttachmentUnique::viewModelRotations),
                    au.animation_refs.iter().filter(|a| !a.is_empty()).count(),
                    au.alt_weapon_name,
                );
                if std::env::var_os("T6SCOPE_FULL").is_some() {
                    println!(
                        "    point {} hide {:?} disableBaseAttachment {} disableBaseClip {} overrideOffsets {}",
                        capture
                            .attachments
                            .iter()
                            .find(|a| kind.split('+').next() == Some(a.name.as_str()))
                            .map_or(-1, |a| a.int_from_bytes(fastfile_t6::layout::WeaponAttachment::attachmentPoint)),
                        au.concealed_bones,
                        au.bytes.get(WeaponAttachmentUnique::disableBaseWeaponAttachment).copied().unwrap_or(0),
                        au.bytes.get(WeaponAttachmentUnique::disableBaseWeaponClip).copied().unwrap_or(0),
                        au.bytes.get(WeaponAttachmentUnique::overrideBaseWeaponAttachmentOffsets).copied().unwrap_or(0),
                    );
                    for (i, a) in au.animation_refs.iter().enumerate().filter(|(_, a)| !a.is_empty()) {
                        print!(" {i:02x}={a}");
                    }
                    println!();
                }
                for part in kind.split('+') {
                    if let Some(a) = capture.attachments.iter().find(|a| a.name == part) {
                        use fastfile_t6::layout::WeaponAttachment;
                        println!(
                            "    attachment {} zoomFov {} {} {} inFrac {} outFrac {} transScale {} {} adsIdleScale {} swayOverride {} adsSwayOverride {} mms {}",
                            a.name,
                            a.float_from_bytes(WeaponAttachment::fAdsZoomFov1),
                            a.float_from_bytes(WeaponAttachment::fAdsZoomFov2),
                            a.float_from_bytes(WeaponAttachment::fAdsZoomFov3),
                            a.float_from_bytes(WeaponAttachment::fAdsZoomInFrac),
                            a.float_from_bytes(WeaponAttachment::fAdsZoomOutFrac),
                            a.float_from_bytes(WeaponAttachment::fAdsTransInTimeScale),
                            a.float_from_bytes(WeaponAttachment::fAdsTransOutTimeScale),
                            a.float_from_bytes(WeaponAttachment::fAdsIdleAmountScale),
                            a.byte_field(WeaponAttachment::swayOverride),
                            a.byte_field(WeaponAttachment::adsSwayOverride),
                            a.byte_field(WeaponAttachment::mmsWeapon),
                        );
                    }
                }
                if !au.hud_layer_surface.is_empty()
                    && let Some(dir) = &png_dir
                {
                    dump_material(&capture, &packs, &au.hud_layer_surface, dir);
                }
            }
            for name in [&w.hud_layer_surface, &w.overlay_material_low] {
                let Some(m) = capture.materials.iter().find(|m| &m.name == name) else {
                    continue;
                };
                let technique = m
                    .method_set
                    .and_then(|k| capture.technique_sets.get(k.index))
                    .map_or("", |t| t.name.as_str());
                println!("  material {} technique {technique} constants {:?}", m.name, m.constants);
                for t in &m.textures {
                    let Some(img) = t.image.and_then(|k| capture.images.get(k.index)) else {
                        continue;
                    };
                    println!(
                        "    slot {:08x} semantic {} image {} {}x{}",
                        t.label_digest, t.semantic, img.name, img.width, img.height
                    );
                    if let (Some(dir), Some(ImageSource::Pack(i, entry))) = (&png_dir, packs.locate(img))
                        && let Ok(bytes) = packs.packs[i].read(entry)
                        && let Ok(iwi) = ipak_t6::parse_iwi(&bytes)
                    {
                        write_png(dir, &img.name, &iwi);
                    }
                }
            }
        }
    }
    Ok(())
}

/// A material's textures, each written as a PNG.
fn dump_material(capture: &asset_t6::ZoneCapture, packs: &PackSet, name: &str, dir: &str) {
    let Some(m) = capture.materials.iter().find(|m| m.name == name) else {
        println!("    material {name}: not in this zone");
        return;
    };
    for t in &m.textures {
        let Some(img) = t.image.and_then(|k| capture.images.get(k.index)) else {
            continue;
        };
        println!("    material {name} image {} {}x{}", img.name, img.width, img.height);
        if let Some(ImageSource::Pack(i, entry)) = packs.locate(img)
            && let Ok(bytes) = packs.packs[i].read(entry)
            && let Ok(iwi) = ipak_t6::parse_iwi(&bytes)
        {
            write_png(dir, &img.name, &iwi);
        }
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
            println!("    (not written: {other:?})");
            return;
        }
    };
    let mut rgba = vec![0u8; w * h * 4];
    if block == 0 {
        let n = rgba.len().min(data.len());
        rgba[..n].copy_from_slice(&data[..n]);
    } else {
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        let mut texels = [0u8; 4 * 4 * 4];
        for (n, src) in data.chunks(block).take(bw * bh).enumerate() {
            if src.len() < block {
                continue;
            }
            decode(src, &mut texels, 16);
            blit_tile(&mut rgba, (w, h), &texels, (n % bw, n / bw));
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
        println!("    wrote {} ({:?} {}x{})", path.to_string_lossy(), iwi.format, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_box_spans_every_row() {
        let mut vertex_points = Vec::new();
        for p in [[1.0f32, -2.0, 3.0], [-4.0, 5.0, 0.5]] {
            for c in p {
                vertex_points.extend_from_slice(&c.to_le_bytes());
            }
            vertex_points.extend_from_slice(&[0u8; 20]);
        }
        assert_eq!(vertex_box(&vertex_points), ([-4.0, -2.0, 0.5], [1.0, 5.0, 3.0]));
        assert_eq!(vertex_box(&[]), ([f32::MAX; 3], [f32::MIN; 3]));
    }

    #[test]
    fn blit_tile_clips_at_the_image_edge() {
        let tile: [u8; 64] = std::array::from_fn(|i| i as u8 + 1);
        let mut img = vec![0u8; 6 * 5 * 4];
        blit_tile(&mut img, (6, 5), &tile, (1, 1));
        // Block (1, 1) starts at pixel (4, 4): one row, two pixels fit.
        let at = (4 * 6 + 4) * 4;
        assert_eq!(&img[at..at + 8], &tile[..8]);
        assert_eq!(img.iter().filter(|&&b| b != 0).count(), 8);
    }
}
