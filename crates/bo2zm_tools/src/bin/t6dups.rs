//! bo2mp: front-end picture names defined in more than one zone, read-only.
//! `t6dups` lists every material name that two or more of code_post_gfx_mp,
//! ui_mp, patch_mp, patch_ui_mp and common_mp define: per copy the colour
//! image (name, size, source, a hash of its stored bytes) and a hash of the
//! material's draw state, which copy the menus picked before (first zone
//! wins) and after (latest front-end zone wins, common_mp last), and
//! whether the UI scripts or string tables mention the name.

use std::collections::BTreeMap;
use std::path::Path;

use asset_t6::{ImageRef, ImageSource, MaterialRef, PackSet, ZoneCapture, capture_zone};

const ORDER: [&str; 5] = ["code_post_gfx_mp", "ui_mp", "patch_mp", "patch_ui_mp", "common_mp"];
const COLOR_MAP_HASH: u32 = 0xa0ab_1041;

fn h<T: std::hash::Hash + ?Sized>(v: &T) -> u64 {
    let mut s = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut s);
    std::hash::Hasher::finish(&s)
}

/// A copy's colour image: its name, size, where the pixels are and a hash of the stored bytes.
struct Picture {
    image: String,
    size: String,
    source: &'static str,
    bytes: u64,
}

/// One zone's copy of a material: its picture and a hash of its draw state.
struct Def {
    zone: &'static str,
    picture: Picture,
    state: u64,
}

/// Where an image's pixels are and a hash of what is stored there.
fn stored(packs: &PackSet, i: &ImageRef) -> (&'static str, u64) {
    match packs.locate(i) {
        Some(ImageSource::Pack(p, e)) => ("pack", packs.packs[p].read(e).map_or(0, |v| h(&v))),
        Some(ImageSource::Embedded) => ("zone", i.embedded.as_ref().map_or(0, |e| h(&e.data))),
        Some(ImageSource::Empty) => ("empty", 0),
        None => ("nowhere", 0),
    }
}

fn picture_of(packs: &PackSet, img: Option<&ImageRef>) -> Picture {
    let Some(i) = img else {
        return Picture { image: "-".into(), size: "-".into(), source: "no image", bytes: 0 };
    };
    let (pack_name, bytes) = stored(packs, i);
    Picture { image: i.name.clone(), size: format!("{}x{}", i.width, i.height), source: pack_name, bytes }
}

/// The material's colour map image when it has one, else its first image.
fn colour_image<'c>(c: &'c ZoneCapture, m: &MaterialRef) -> Option<&'c ImageRef> {
    let with_image = || m.textures.iter().filter(|t| t.image.is_some());
    let tex = with_image().find(|t| t.label_digest == COLOR_MAP_HASH).or_else(|| with_image().next());
    tex.and_then(|t| t.image).and_then(|k| c.images.get(k.index))
}

fn draw_state_hash(m: &MaterialRef) -> u64 {
    let consts: Vec<(&str, [u32; 4])> = m.constants.iter().map(|(_, n, v)| (n.as_str(), v.map(f32::to_bits))).collect();
    h(&(&m.state_bits_entry[..], &m.state_words, m.condition_bits, consts))
}

/// Every byte the UI scripts and string tables carry, common_mp left out.
fn ui_text(caps: &[(&'static str, ZoneCapture)]) -> Vec<u8> {
    let zones = caps.iter().filter(|(z, _)| *z != "common_mp").map(|(_, c)| c);
    zones
        .flat_map(|c| {
            let files = c.raw_files.iter().flat_map(|(_, b)| b.iter().copied());
            let cells = c.string_tables.iter().flat_map(|t| &t.cells).flat_map(|cell| cell.bytes());
            files.chain(cells)
        })
        .collect()
}

fn named_in(text: &[u8], sought: &str) -> bool {
    text.windows(sought.len()).any(|w| w == sought.as_bytes())
}

/// The copy the menus picked before (first zone wins) and after (latest
/// front-end zone wins). common_mp is last in both orders; with only common
/// copies the pick stays the first.
fn picks(copies: &[Def]) -> (&Def, &Def) {
    let first = &copies[0];
    let before = copies.iter().find(|c| c.zone != "common_mp").unwrap_or(first);
    let after = copies.iter().rev().find(|c| c.zone != "common_mp").unwrap_or(first);
    (before, after)
}

/// Whether the later pick is a different zone with different content.
fn differs(before: &Def, after: &Def) -> bool {
    let (a, b) = (&before.picture, &after.picture);
    before.zone != after.zone && (a.bytes != b.bytes || before.state != after.state || a.image != b.image)
}

fn tag(c: &Def, before: &Def, after: &Def) -> &'static str {
    match (std::ptr::eq(c, before), std::ptr::eq(c, after)) {
        (true, true) => "old+new",
        (true, false) => "old",
        (false, true) => "new",
        _ => "",
    }
}

fn drive() -> Result<(), String> {
    let dir = &bo2zm_tools::zones_dir();
    let packs = PackSet::open_dir(dir).map_err(|_| "packs".to_owned())?;
    let caps: Vec<(&'static str, ZoneCapture)> = ORDER
        .iter()
        .filter_map(|z| capture_zone(&dir.join(format!("{z}.ff"))).ok().map(|c| (*z, c)))
        .collect();
    let text = ui_text(&caps);
    let mut by_name: BTreeMap<&str, Vec<Def>> = BTreeMap::new();
    for (zone, c) in &caps {
        for m in &c.materials {
            let picture = picture_of(&packs, colour_image(c, m));
            by_name.entry(m.name.as_str()).or_default().push(Def { zone, picture, state: draw_state_hash(m) });
        }
    }
    let mut verdicts: Vec<(bool, bool)> = vec![];
    for (name, copies) in by_name.iter().filter(|(_, copies)| copies.len() >= 2) {
        let (before, after) = picks(copies);
        let (changed, used) = (differs(before, after), named_in(&text, name));
        verdicts.push((changed, used));
        println!("{name}{}{}", if changed { "  CHANGED" } else { "" }, if used { "  [named by UI]" } else { "" });
        for c in copies {
            let p = &c.picture;
            println!(
                "    {:<17} {:<8} img {} {} {} bytes {:016x} state {:016x}",
                c.zone,
                tag(c, before, after),
                p.image,
                p.size,
                p.source,
                p.bytes,
                c.state
            );
        }
    }
    let changed = verdicts.iter().filter(|(changed, _)| *changed).count();
    let used_changed = verdicts.iter().filter(|&&(changed, used)| changed && used).count();
    println!(
        "{} duplicate names; {changed} change pick with different content; {used_changed} of those named by UI scripts/tables",
        verdicts.len()
    );
    Ok(())
}

fn main() -> std::process::ExitCode {
    let outcome = drive();
    if let Err(e) = &outcome {
        eprintln!("{e}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use super::{Def, Picture, picks, tag, differs, named_in};

    fn def(zone: &'static str, image: &str, bytes: u64, state: u64) -> Def {
        Def { zone, picture: Picture { image: image.into(), size: "1x1".into(), source: "pack", bytes }, state }
    }

    #[test]
    fn the_menus_pick_the_first_zone_before_and_the_latest_front_end_zone_after() {
        let copies = [def("ui_mp", "a", 1, 1), def("patch_ui_mp", "a", 2, 1), def("common_mp", "a", 3, 1)];
        let (before, after) = picks(&copies);
        assert_eq!((before.zone, after.zone), ("ui_mp", "patch_ui_mp"));
        assert_eq!(tag(&copies[0], before, after), "old");
        assert_eq!(tag(&copies[1], before, after), "new");
        assert_eq!(tag(&copies[2], before, after), "");
    }

    #[test]
    fn only_common_copies_keep_the_first_pick_for_both() {
        let copies = [def("common_mp", "a", 1, 1), def("common_mp", "a", 2, 1)];
        let (before, after) = picks(&copies);
        assert!(std::ptr::eq(before, after) && std::ptr::eq(before, &copies[0]));
        assert_eq!(tag(&copies[0], before, after), "old+new");
        assert!(!differs(before, after));
    }

    #[test]
    fn a_change_needs_another_zone_and_other_bytes_state_or_image() {
        let base = def("ui_mp", "a", 1, 1);
        assert!(!differs(&base, &def("ui_mp", "a", 9, 9)));
        assert!(!differs(&base, &def("patch_mp", "a", 1, 1)));
        assert!(differs(&base, &def("patch_mp", "a", 2, 1)));
        assert!(differs(&base, &def("patch_mp", "a", 1, 2)));
        assert!(differs(&base, &def("patch_mp", "b", 1, 1)));
    }

    #[test]
    fn a_name_is_found_anywhere_in_the_ui_text() {
        assert!(named_in(b"setmaterial menu_logo end", "menu_logo"));
        assert!(!named_in(b"menu_log", "menu_logo"));
    }
}
