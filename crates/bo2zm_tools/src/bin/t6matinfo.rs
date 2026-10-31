//! bo2zm: print Black Ops II materials' constants and textures, read-only.
//! `t6matinfo <zone.ff> <material name>...` (`~text` lists the materials
//! whose name or technique set has `text`; `%text` the localized strings
//! whose key or text has it).
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = argv.split_first() else {
        eprintln!("usage: t6matinfo <zone.ff> <material name>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    let techset = |m: &asset_t6::MaterialRef| {
        m.method_set
            .and_then(|k| c.technique_sets.get(k.index))
            .map_or("", |t| t.name.as_str())
    };
    for wanted in names {
        // `%text`: list the localized strings whose key or text has it.
        if let Some(part) = wanted.strip_prefix('%') {
            let part = part.to_lowercase();
            for (key, text) in &c.localize {
                if key.to_lowercase().contains(&part) || text.to_lowercase().contains(&part) {
                    println!("{key} = {text}");
                }
            }
            continue;
        }
        // `~text`: list the materials whose wanted or technique set has it.
        if let Some(part) = wanted.strip_prefix('~') {
            let part = part.to_lowercase();
            for m in &c.materials {
                let set = techset(m);
                if m.name.to_lowercase().contains(&part) || set.to_lowercase().contains(&part) {
                    println!("{} | {set} | {} textures", m.name, m.textures.len());
                }
            }
            continue;
        }
        let Some(m) = c.materials.iter().find(|m| &m.name == wanted) else {
            println!("{wanted}: not in the zone");
            continue;
        };
        println!("material {} (sort {}, technique set {})", m.name, m.sort_rank, techset(m));
        for (hash, cname, v) in &m.constants {
            println!("  const {hash:08x} {cname:<20} {v:?}");
        }
        for t in &m.textures {
            let image = t.image.and_then(|k| c.images.get(k.index));
            match image {
                Some(i) => println!(
                    "  tex {:08x} sem {} sampler {} image {} {}x{} type {} embedded {}",
                    t.label_digest, t.semantic, t.texture_sampling_mode, i.name, i.width, i.height, i.map_kind,
                    i.embedded.as_ref().map_or(0, |e| e.data.len())
                ),
                None => println!("  tex {:08x} sem {} image {:?}", t.label_digest, t.semantic, t.image),
            }
        }
        println!("  state bits {:?}", m.state_words);
    }
}
