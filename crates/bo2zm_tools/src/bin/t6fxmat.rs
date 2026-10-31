//! bo2zm: the materials a Black Ops II map's placed effects draw with,
//! read-only. `t6fxmat [bo2 root]` reads Nuketown's zones and scripts, walks
//! every effect placed at load (and the effects they spawn), and prints per
//! material its technique set, blend, colour map and pixel format, then a
//! count per (technique, blend, format). `FXMAT_MAP=<map>`: another map's.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use asset_t6::{
    DrawState, FxEffectRef, FxVisualRef, ImageRef, ImageSource, MapScriptFacts, MaterialRef, PackSet, ZoneCapture,
    capture_zone,
};

const FX_ZONES: [&str; 8] = [
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
    "zm_nuked",
    "zm_nuked_patch",
    "code_pre_gfx_zm",
];
const COLOR_MAP_HASH: u32 = 0xa0ab_1041;

/// A material one element draws, with that element's type.
#[derive(Debug, PartialEq)]
struct Drawn {
    drawn_mat: String,
    by_elem_type: u8,
}

/// What one effect draws and which effects it spawns.
#[derive(Debug, PartialEq)]
struct Uses {
    drawn: Vec<Drawn>,
    children: Vec<String>,
}

fn uses(e: &FxEffectRef) -> Uses {
    let mut mats = vec![];
    let mut children = vec![];
    for el in &e.elems {
        for v in &el.visuals {
            match v {
                FxVisualRef::Material(m) => mats.push(Drawn { drawn_mat: m.clone(), by_elem_type: el.elem_type() }),
                FxVisualRef::Effect(child) => children.push(child.clone()),
                _ => {}
            }
        }
        let spawned = [&el.effect_upon_strike, &el.effect_upon_demise, &el.effect_spawned];
        children.extend(spawned.into_iter().filter(|child| !child.is_empty()).cloned());
    }
    Uses { drawn: mats, children }
}

/// Every effect reached from `start` (and the effects those spawn), and per
/// material the `effect#t<type>` users that draw it.
fn reach<'a>(
    start: Vec<String>,
    effect_of: impl Fn(&str) -> Option<&'a FxEffectRef>,
) -> (BTreeSet<String>, BTreeMap<String, BTreeSet<String>>) {
    let mut todo = start;
    let mut met = BTreeSet::default();
    let mut mats: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    while let Some(name) = todo.pop() {
        if !met.insert(name.clone()) {
            continue;
        }
        let Some(e) = effect_of(&name) else { continue };
        let Uses { drawn, children } = uses(e);
        for Drawn { drawn_mat, by_elem_type } in drawn {
            mats.entry(drawn_mat).or_default().insert(format!("{name}#t{by_elem_type}"));
        }
        todo.extend(children);
    }
    (met, mats)
}

/// The material's colour map image when it has one, else its first; a `,` name
/// stands for the real image of that name in some zone.
fn colour_image<'a>(caps: &'a [ZoneCapture], c: &'a ZoneCapture, mat: &MaterialRef) -> Option<&'a ImageRef> {
    let tex = mat.textures.iter().find(|t| t.label_digest == COLOR_MAP_HASH).or(mat.textures.first());
    let img = tex.and_then(|t| t.image).map(|k| &c.images[k.index])?;
    let real = img.name.strip_prefix(',').and_then(|real| caps.iter().find_map(|cc| cc.images.iter().find(|x| x.name == real)));
    Some(real.unwrap_or(img))
}

/// The draw state of the highest technique slot (6 down to 0) that has one.
fn draw_state(mat: &MaterialRef) -> Option<(usize, DrawState)> {
    (0..=6).rev().find_map(|t| {
        let entry = *mat.state_bits_entry.get(t)?;
        mat.state_words.get(usize::from(entry)).map(|b| (t, DrawState::decode(*b)))
    })
}

fn blend_text(state: Option<(usize, DrawState)>) -> String {
    state.map_or("no state".to_owned(), |(t, s)| {
        format!("slot{t} {}/{} op{} at{:?}", s.src_blend, s.dst_blend, s.blend_op, s.alpha_test)
    })
}

/// The pixel format of an image: read from its pack, or where else its pixels are.
fn pixel_format(packs: &PackSet, img: Option<&ImageRef>) -> String {
    let Some(i) = img else { return "-".to_owned() };
    match packs.locate(i) {
        Some(ImageSource::Pack(p, entry)) => packs.packs[p]
            .read(entry)
            .ok()
            .and_then(|b| ipak_t6::parse_iwi(&b).ok().map(|iwi| format!("{:?}", iwi.format)))
            .unwrap_or_else(|| "unreadable".to_owned()),
        Some(ImageSource::Embedded) => "embedded".to_owned(),
        Some(ImageSource::Empty) => "empty".to_owned(),
        None => "missing".to_owned(),
    }
}

/// The technique-set family: its name up to the first underscore, a leading comma dropped.
fn family(method_set: &str) -> &str {
    method_set.trim_start_matches(',').split('_').next().unwrap_or("")
}

/// Up to `n` of the items, joined with a comma and a space.
fn first_joined<'a>(items: impl IntoIterator<Item = &'a String>, n: usize) -> String {
    items.into_iter().take(n).cloned().collect::<Vec<_>>().join(", ")
}

/// The material's printed row and the (family, blend, format) combo it counts under.
fn describe(caps: &[ZoneCapture], packs: &PackSet, m: &str, users: &BTreeSet<String>) -> Option<(String, String)> {
    let (c, mat) = caps.iter().find_map(|c| c.materials.iter().find(|x| x.name == m && !x.textures.is_empty()).map(|x| (c, x)))?;
    let ts = mat.method_set.map_or("-".to_owned(), |t| c.technique_sets[t.index].name.clone());
    let img = colour_image(caps, c, mat);
    let format = pixel_format(packs, img);
    let blend = blend_text(draw_state(mat));
    let row = format!("{m} | {ts} | {blend} | {} {format} | users {}", img.map_or("-", |i| i.name.as_str()), first_joined(users, 2));
    Some((row, format!("{} {blend} {format}", family(&ts))))
}

fn run(root: PathBuf) -> Result<(), String> {
    let dir = root.join("zone").join("all");
    let map = std::env::var("FXMAT_MAP").unwrap_or_else(|_| "zm_nuked".to_owned());
    let zones = FX_ZONES.map(|z| z.replace("zm_nuked", &map));
    let mut caps: Vec<ZoneCapture> = Vec::new();
    for z in &zones {
        match capture_zone(&dir.join(format!("{z}.ff"))) {
            Ok(c) => caps.push(c),
            Err(e) => eprintln!("{z}: {e}"),
        }
    }
    let packs = PackSet::open_dir(&dir).map_err(|_| "no image packs".to_owned())?;
    let facts = MapScriptFacts::read(&map, |name| {
        let want = format!("script:{name}");
        caps.iter().rev().find_map(|c| c.raw_files.iter().find(|(n, _)| *n == want).map(|(_, b)| b.clone()))
    });
    let fx: HashMap<&str, &FxEffectRef> =
        caps.iter().flat_map(|c| &c.fx).filter(|e| !e.name.starts_with(',') && !e.name.is_empty()).map(|e| (e.name.as_str(), e)).collect();
    // Effects placed at load, and every effect they spawn.
    let placed = facts.placed.iter().filter(|p| p.kind != "exploder");
    let mut start: Vec<String> = placed.filter_map(|p| facts.effects.get(&p.fx_key).cloned()).collect();
    // FXMAT_SEED=<name part>: the effects so named instead (the ones a weapon or a hit plays,
    // which no map places).
    if let Ok(seed) = std::env::var("FXMAT_SEED") {
        start = fx.keys().filter(|n| n.contains(seed.as_str())).map(|n| (*n).to_owned()).collect();
    }
    let (met, mut mats) = reach(start, |name| fx.get(name).copied());
    // A model element draws with its model's mats.
    for name in &met {
        let Some(e) = fx.get(name.as_str()) else { continue };
        let models = e.elems.iter().flat_map(|el| &el.visuals).filter_map(|v| match v {
            FxVisualRef::Model(model) => Some(model),
            _ => None,
        });
        for model in models {
            let found = caps.iter().find_map(|c| c.xmodels.iter().find(|x| x.name == *model).map(|x| (c, x)));
            let Some((c, x)) = found else { continue };
            for k in x.materials.iter().flatten() {
                let users = mats.entry(c.materials[k.index].name.clone()).or_default();
                users.insert(format!("{name}#model {model}"));
            }
        }
    }
    println!("effects reached {reached}, materials {total}", reached = met.len(), total = mats.len());
    let mut combos: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (m, users) in &mats {
        match describe(&caps, &packs, m, users) {
            Some((row, combo)) => {
                println!("{row}");
                combos.entry(combo).or_default().push(m.clone());
            }
            None => println!("{m}: NOT FOUND"),
        }
    }
    println!("\n== combos");
    combos.iter().for_each(|(k, v)| println!("{:>3} {k}: {}", v.len(), first_joined(v, 3)));
    Ok(())
}

fn main() -> std::process::ExitCode {
    let root = std::env::args().nth(1).map_or_else(bo2zm_tools::game_dir, PathBuf::from);
    let outcome = run(root);
    if let Err(e) = &outcome {
        eprintln!("{e}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use asset_t6::{DrawState, FxEffectRef, FxElemRef, FxVisualRef};
    use std::collections::HashMap;
    use super::{uses, Uses, Drawn, reach, blend_text, family, first_joined};

    fn effect(label: &str, visuals: Vec<FxVisualRef>, impact: &str) -> FxEffectRef {
        let elem = FxElemRef { visuals, effect_upon_strike: impact.into(), ..FxElemRef::default() };
        FxEffectRef { name: label.into(), elems: vec![elem], ..FxEffectRef::default() }
    }

    #[test]
    fn an_effect_uses_its_materials_and_the_effects_it_spawns() {
        let e = effect("a", vec![FxVisualRef::Material("smoke".into()), FxVisualRef::Effect("b".into()), FxVisualRef::None], "c");
        assert_eq!(uses(&e), Uses { drawn: vec![Drawn { drawn_mat: "smoke".to_owned(), by_elem_type: 0 }], children: vec!["b".to_owned(), "c".to_owned()] });
    }

    #[test]
    fn reaching_follows_spawned_effects_once_and_skips_unknown_ones() {
        let all: HashMap<&str, FxEffectRef> = HashMap::from([
            ("a", effect("a", vec![FxVisualRef::Effect("b".into())], "")),
            ("b", effect("b", vec![FxVisualRef::Material("m".into()), FxVisualRef::Effect("a".into())], "gone")),
        ]);
        let (seen, materials) = reach(vec!["a".to_owned()], |n| all.get(n));
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["a", "b", "gone"]);
        assert_eq!(materials["m"].iter().collect::<Vec<_>>(), ["b#t0"]);
    }

    #[test]
    fn a_blend_shows_the_slot_factors_op_and_alpha_test() {
        let mut s = DrawState::decode([0, 0]);
        (s.src_blend, s.dst_blend, s.blend_op, s.alpha_test) = (5, 6, 1, Some(1));
        assert_eq!(blend_text(Some((2, s))), "slot2 5/6 op1 atSome(1)");
        assert_eq!(blend_text(None), "no state");
    }

    #[test]
    fn the_family_is_the_first_word_of_the_technique_set() {
        assert_eq!(family(",lit_sm_flag"), "lit");
        assert_eq!(family("distortion"), "distortion");
        assert_eq!(family(""), "");
    }

    #[test]
    fn lists_are_cut_and_joined_with_comma_space() {
        let items = ["a", "b", "c"].map(str::to_owned);
        assert_eq!(first_joined(&items, 2), "a, b");
        assert_eq!(first_joined(&items, 9), "a, b, c");
    }
}
