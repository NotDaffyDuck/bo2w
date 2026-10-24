//! bo2mp probe: count the sound aliases per bus (`SndAliasFlags` flags0 bits 11-14), with a few
//! names each, for the zones given.
//! cargo run -p asset_t6 --example snd_bus_dump -- ZONE...
fn main() {
    const BUS: [&str; 9] = ["reverb", "fx", "voice", "pfutz", "hdrfx", "ui", "music", "movie", "reference"];
    let mut count = [0usize; 16];
    let mut names: Vec<Vec<String>> = vec![Vec::new(); 16];
    for zone in std::env::args().skip(1) {
        let cap = asset_t6::capture_zone(std::path::Path::new(&zone)).expect("zone");
        for bank in &cap.sound_banks {
            for list in &bank.aliases {
                for a in &list.entries {
                    let bus = ((a.flags[0] >> 11) & 0xf) as usize;
                    count[bus] += 1;
                    if names[bus].len() < 12 && !names[bus].contains(&a.name) {
                        names[bus].push(a.name.clone());
                    }
                }
            }
        }
    }
    for (b, n) in count.iter().enumerate() {
        if *n > 0 {
            println!("{} {}: {}", BUS.get(b).copied().unwrap_or("?"), n, names[b].join(" "));
        }
    }
}
