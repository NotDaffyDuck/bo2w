//! bo2mp probe: the sound aliases whose name contains a pattern, with their bus, duck group
//! and the duck they start, for the zones given; then a count of ducks per alias prefix.
//! cargo run -p asset_t6 --example snd_alias_find -- PATTERN ZONE...
fn main() {
    let mut argv = std::env::args().skip(1);
    let pattern = argv.next().expect("pattern").to_ascii_lowercase();
    let mut alias_groups: Vec<(String, u32)> = Vec::new();
    let mut printed = 0usize;
    let mut matched = 0usize;
    let mut by_duck: std::collections::BTreeMap<String, usize> = Default::default();
    let caps: Vec<_> = argv
        .map(|zone| asset_t6::capture_zone(std::path::Path::new(&zone)).expect("zone"))
        .collect();
    // A bank's alias may name a duck another bank (another zone) defines.
    let mut all_ducks: std::collections::HashMap<u32, String> = Default::default();
    let mut duck_refs: std::collections::HashMap<u32, _> = Default::default();
    for cap in &caps {
        for bank in &cap.sound_banks {
            for d in &bank.ducks {
                all_ducks.entry(d.id).or_insert_with(|| format!("{} ({})", d.name, bank.name));
                duck_refs.entry(d.id).or_insert_with(|| d.clone());
            }
        }
    }
    let mut used: std::collections::BTreeSet<u32> = Default::default();
    for cap in &caps {
        if let Some(g) = &cap.snd_globals {
            if alias_groups.is_empty() {
                alias_groups = g.duck_groups.clone();
            }
        }
        for bank in &cap.sound_banks {
            for list in &bank.aliases {
                for a in &list.entries {
                    if !a.name.to_ascii_lowercase().contains(&pattern) {
                        continue;
                    }
                    matched += 1;
                    used.insert(a.duck);
                    let duck = all_ducks
                        .get(&a.duck)
                        .cloned()
                        .unwrap_or_else(|| if a.duck == 0 { "-".into() } else { format!("#{:08x}", a.duck) });
                    *by_duck.entry(duck.clone()).or_default() += 1;
                    if printed < 40 {
                        printed += 1;
                        println!(
                            "{} bank {} bus {} duck_group {} duck {}",
                            a.name,
                            bank.name,
                            (a.flags[0] >> 11) & 0xf,
                            a.duck_group,
                            duck
                        );
                    }
                }
            }
        }
    }
    println!("{matched} aliases match");
    for (duck, n) in by_duck {
        println!("duck {duck}: {n}");
    }
    let names: Vec<String> = alias_groups.iter().map(|(n, _)| n.clone()).collect();
    println!("duck groups: {}", names.join(" "));
    for id in used {
        let Some(d) = duck_refs.get(&id) else { continue };
        let turned: Vec<String> = d
            .attenuation
            .iter()
            .zip(&d.filter)
            .enumerate()
            .filter(|(_, (a, f))| **a < 0.999 || **f < 0.999)
            .map(|(i, (a, f))| format!("{}={a:.3}/{f:.3}", names.get(i).map_or("?", |s| s.as_str())))
            .collect();
        println!(
            "duck {}: fade_in {} fade_out {} delay {} length {} distance {}; volume/filter: {}",
            d.name,
            d.ease_on,
            d.ease_off,
            d.start_delay,
            d.length,
            d.distance,
            turned.join(" ")
        );
    }
}
