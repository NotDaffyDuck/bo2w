//! bo2mp: one Black Ops II effect part by part, read-only.
//! `t6fxdump <name part> [bo2 root]` reads the multiplayer zones (or
//! `T6FXDUMP_ZONES=a,b`) and prints, for each effect whose name contains the
//! part (and the effects it spawns): every element's type, flags, counts,
//! delays and life, its colour and size graph as stored, what it draws; then
//! each material it draws with: technique set, blend, colour map.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use asset_t6::{DrawState, FxVisualRef, ZoneCapture};
use fastfile_t6::layout::{FxElemDef, FxElemVisStateSample, FxElemVisualState};

const FX_ZONES: [&str; 5] = ["code_pre_gfx_mp", "code_post_gfx_mp", "common_mp", "patch_mp", "mp_nuketown_2020"];

/// A run that already said why on its own.
struct Failed;

/// Where the walk over effects stands: what to visit, what was visited, what they draw.
#[derive(Default)]
struct Crawl {
    pending: Vec<String>,
    visited: BTreeSet<String>,
    materials: BTreeSet<String>,
}

impl Crawl {
    /// Effects to start from, visited in name order.
    fn starting_at(mut listed: Vec<String>) -> Self {
        listed.sort_by(|l, r| r.cmp(l));
        Crawl { pending: listed, ..Crawl::default() }
    }

    fn queue(&mut self, name: &str) {
        self.pending.push(name.to_owned());
    }

    /// The next effect not visited yet, marking it visited.
    fn next_unvisited(&mut self) -> Option<String> {
        while let Some(name) = self.pending.pop() {
            if self.visited.insert(name.clone()) {
                return Some(name);
            }
        }
        None
    }
}

fn f32_in(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// One visual state of a vis sample, starting `at` bytes in.
fn vis_state_text(sample: &[u8], at: usize) -> String {
    let colour = &sample[at + FxElemVisualState::color..at + FxElemVisualState::color + 4];
    format!(
        "rgba {colour:?} size {:.1}x{:.1} scale {:.2} rot {:.2}/{:.2}",
        f32_in(sample, at + FxElemVisualState::size),
        f32_in(sample, at + FxElemVisualState::size + 4),
        f32_in(sample, at + FxElemVisualState::scale),
        f32_in(sample, at + FxElemVisualState::rotationDelta),
        f32_in(sample, at + FxElemVisualState::rotationTotal),
    )
}

/// "a+b" for the pair of ints stored at `off`.
fn int_pair(el: &asset_t6::FxElemRef, off: usize) -> String {
    format!("{}+{}", el.int_from_bytes(off), el.int_from_bytes(off + 4))
}

/// "a..b" for the pair of floats stored at `off`.
fn float_range(el: &asset_t6::FxElemRef, off: usize) -> String {
    format!("{}..{}", el.float_from_bytes(off), el.float_from_bytes(off + 4))
}

fn element_line(i: usize, el: &asset_t6::FxElemRef) -> String {
    let byte = |off: usize| el.raw.get(off).copied().unwrap_or(0);
    let fade_ms = u16::from_le_bytes([byte(FxElemDef::alphaFadeTimeMsec), byte(FxElemDef::alphaFadeTimeMsec + 1)]);
    format!(
        "  #{i} type {} flags {:#010x} spawn {} delay {} ms life {} ms alphaFade {fade_ms} ms lighting {} sort {} gravity {}+{} fade-in {} fade-out {}",
        el.elem_type(),
        el.int_from_bytes(FxElemDef::flags),
        int_pair(el, FxElemDef::spawn),
        int_pair(el, FxElemDef::spawnDelayMsec),
        int_pair(el, FxElemDef::lifeSpanMsec),
        byte(FxElemDef::lightingFrac),
        byte(FxElemDef::sortOrder),
        el.float_from_bytes(FxElemDef::gravity),
        el.float_from_bytes(FxElemDef::gravity + 4),
        float_range(el, FxElemDef::fadeInRange),
        float_range(el, FxElemDef::fadeOutRange),
    )
}

/// bo2zm: where an element spawns and how it moves: its origin, offset and angle ranges,
/// gravity, and its first and last velocity samples (local and world, base and amplitude).
fn show_motion(el: &asset_t6::FxElemRef) {
    let byte = |off: usize| el.raw.get(off).copied().unwrap_or(0);
    let r = |off: usize| [el.float_from_bytes(off), el.float_from_bytes(off + 4)];
    println!(
        "     origin {:?} {:?} {:?} offR {:?} offH {:?} angles {:?} {:?} {:?} grav {:?} velInt {}",
        r(FxElemDef::spawnOrigin),
        r(FxElemDef::spawnOrigin + 8),
        r(FxElemDef::spawnOrigin + 16),
        r(FxElemDef::spawnOffsetRadius),
        r(FxElemDef::spawnOffsetHeight),
        r(FxElemDef::spawnAngles),
        r(FxElemDef::spawnAngles + 8),
        r(FxElemDef::spawnAngles + 16),
        r(FxElemDef::gravity),
        byte(FxElemDef::velIntervalCount),
    );
    let samples: Vec<&[u8]> = el.speed_readings.chunks_exact(96).collect();
    let last = samples.len().saturating_sub(1);
    for (k, s) in samples.iter().enumerate().filter(|(k, _)| *k == 0 || *k == last) {
        let v = |at: usize| [0, 4, 8].map(|d| f32_in(s, at + d));
        println!(
            "     v{k}: local vel {:?} delta {:?} world vel {:?} delta {:?} | amp local delta {:?} world delta {:?}",
            v(0),
            v(12),
            v(24),
            v(36),
            v(60),
            v(84)
        );
    }
}

/// Prints what an element draws or plays, queueing the effects and materials it leads to.
fn show_visuals(el: &asset_t6::FxElemRef, crawl: &mut Crawl) {
    for vis in &el.visuals {
        match vis {
            FxVisualRef::None => {}
            FxVisualRef::Material(m) => {
                println!("     material {m}");
                crawl.materials.insert(m.clone());
            }
            FxVisualRef::Mark([a, b]) => {
                println!("     mark {a} | {b}");
                crawl.materials.insert(a.clone());
            }
            FxVisualRef::Model(m) => println!("     model {m}"),
            FxVisualRef::Effect(c) => {
                println!("     runs {c}");
                crawl.queue(c);
            }
            FxVisualRef::Sound(s) => println!("     sound {s}"),
            FxVisualRef::Light(_) => println!("     light"),
        }
    }
    let children = [
        ("on impact", &el.effect_upon_strike),
        ("on death", &el.effect_upon_demise),
        ("emitted", &el.effect_spawned),
    ];
    for (what, child) in children.into_iter().filter(|(_, c)| !c.is_empty()) {
        println!("     {what} {child}");
        crawl.queue(child);
    }
}

/// The zones to read: `T6FXDUMP_ZONES`, else the multiplayer set.
fn zone_names() -> Vec<String> {
    match std::env::var("T6FXDUMP_ZONES") {
        Ok(list) => list.split(',').map(str::to_owned).collect(),
        Err(_) => FX_ZONES.iter().map(|z| (*z).to_owned()).collect(),
    }
}

/// "name | technique set | blend | colour map" for one material, or why it is missing.
fn material_line(caps: &[ZoneCapture], wanted: &str) -> String {
    let found = caps
        .iter()
        .flat_map(|c| c.materials.iter().map(move |m| (c, m)))
        .find(|(_, m)| m.name == wanted && !m.textures.is_empty());
    let Some((c, mat)) = found else {
        return format!("{wanted}: NOT FOUND");
    };
    let set = mat
        .method_set
        .map_or_else(|| String::from("-"), |t| c.technique_sets[t.index].name.clone());
    let state = (0..=6usize).rev().find_map(|stage| {
        let entry = *mat.state_bits_entry.get(stage)?;
        let bits = mat.state_words.get(usize::from(entry))?;
        Some((stage, DrawState::decode(*bits)))
    });
    let blend = match state {
        Some((stage, s)) => format!("slot{stage} {}/{} op{} at{:?}", s.src_blend, s.dst_blend, s.blend_op, s.alpha_test),
        None => "no state".to_owned(),
    };
    let colour_map = mat.textures.iter().find(|t| t.label_digest == 0xa0ab_1041).or(mat.textures.first());
    let image = match colour_map.and_then(|t| t.image) {
        Some(k) => c.images[k.index].name.as_str(),
        None => "-",
    };
    format!("{wanted} | {set} | {blend} | {image}")
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(want) = argv.first() else {
        eprintln!("usage: t6fxdump <effect name part> [bo2 root]");
        return Err(Failed);
    };
    let root = argv.get(1).map_or_else(bo2zm_tools::game_dir, PathBuf::from);
    let dir = root.join("zone").join("all");
    let caps: Vec<ZoneCapture> = zone_names()
        .iter()
        .filter_map(|z| {
            asset_t6::capture_zone(&dir.join(format!("{z}.ff")))
                .inspect_err(|err| eprintln!("{z}: {err}"))
                .ok()
        })
        .collect();
    let fx: HashMap<String, _> = caps
        .iter()
        .flat_map(|c| c.fx.iter())
        .filter(|f| !f.name.starts_with(',') && !f.name.is_empty())
        .map(|f| (f.name.clone(), f))
        .collect();
    let mut crawl = Crawl::starting_at(fx.keys().filter(|n| n.contains(want.as_str())).cloned().collect());
    while let Some(name) = crawl.next_unvisited() {
        let Some(f) = fx.get(&name) else {
            println!("\n== {name}: NOT FOUND");
            continue;
        };
        println!(
            "\n== {name}: flags {:#06x} looping {} one-shot {} emitted {} looping life {} ms, non-looping life {} ms",
            f.flags, f.repeat_total, f.single_play_total, f.spawn_total, f.loop_lifetime_millis, f.msec_non_looping_life
        );
        for (i, el) in f.elems.iter().enumerate() {
            println!("{shown}", shown = element_line(i, el));
            show_motion(el);
            for (k, sample) in el.visibility_readings.chunks_exact(FxElemVisStateSample::SIZE).enumerate() {
                println!(
                    "     vis {k}: base {} | amp {}",
                    vis_state_text(sample, FxElemVisStateSample::base),
                    vis_state_text(sample, FxElemVisStateSample::amplitude)
                );
            }
            show_visuals(el, &mut crawl);
        }
    }
    println!("\n== materials");
    crawl.materials.iter().for_each(|m| println!("{}", material_line(&caps, m)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crawl_visits_in_name_order_and_never_twice() {
        let mut crawl = Crawl::starting_at(vec!["b".into(), "c".into(), "a".into()]);
        assert_eq!(crawl.next_unvisited().as_deref(), Some("a"));
        crawl.queue("b");
        crawl.queue("z");
        assert_eq!(crawl.next_unvisited().as_deref(), Some("z"));
        assert_eq!(crawl.next_unvisited().as_deref(), Some("b"));
        assert_eq!(crawl.next_unvisited().as_deref(), Some("c"));
        assert_eq!(crawl.next_unvisited(), None);
    }

    #[test]
    fn floats_are_read_little_endian_at_an_offset() {
        let mut bytes = vec![0u8; 3];
        bytes.extend_from_slice(&2.5f32.to_le_bytes());
        assert_eq!(f32_in(&bytes, 3), 2.5);
    }

    #[test]
    fn vis_state_reads_colour_size_scale_and_rotation() {
        let mut sample = vec![0u8; 48];
        sample[FxElemVisualState::color..FxElemVisualState::color + 4].copy_from_slice(&[1, 2, 3, 4]);
        for (off, val) in [(FxElemVisualState::size, 8.0f32), (FxElemVisualState::size + 4, 2.0), (FxElemVisualState::scale, 0.5), (FxElemVisualState::rotationDelta, 1.0), (FxElemVisualState::rotationTotal, -1.0)] {
            sample[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
        assert_eq!(
            vis_state_text(&sample, 0),
            "rgba [1, 2, 3, 4] size 8.0x2.0 scale 0.50 rot 1.00/-1.00"
        );
    }
}
