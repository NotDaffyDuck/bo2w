//! bo2mp probe: print BO2 destructible defs (DestructibleDef) whose name holds
//! DGREP: each piece's health and the stages it breaks through (shown bone,
//! health share, timer, notify, effect, sound, loop sound).
//! cargo run -p asset_t6 --example destructible_dump -- ZONE...
use asset_t6::zone_grab::ZoneCapture;
use asset_transport::T6ZoneMemory;
use asset_transport::mount_zone;

fn main() {
    let grep = std::env::var("DGREP").unwrap_or_default().to_ascii_lowercase();
    for path in std::env::args().skip(1) {
        let Ok(image) = mount_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::read_asset_directory(&mut stream) else { continue };
        let mut cap = ZoneCapture::default();
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut cap).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
        for d in cap.destructibles.iter().filter(|d| d.name.to_ascii_lowercase().contains(&grep)) {
            println!("def {} model {} pieces {}", d.name, d.model, d.pieces.len());
            for (k, p) in d.pieces.iter().enumerate() {
                println!("  piece {k}: health {} parent {} bullet {} explosive {}", p.health, p.owner_fragment, p.round_damage_multiplier, p.blast_damage_multiplier);
                for (j, st) in p.stages.iter().enumerate() {
                    if st.reveal_joint.is_empty() && st.shatter_signal.is_empty() && st.repeating_audio.is_empty() && st.shatter_hitpoints == 0.0 {
                        continue;
                    }
                    println!("    stage {j}: bone {:?} health {} time {} notify {:?} fx {:?} sound {:?} loop {:?}",
                        st.reveal_joint, st.shatter_hitpoints, st.longest_duration, st.shatter_signal, st.shatter_visual, st.shatter_audio, st.repeating_audio);
                }
            }
        }
        // The map's entities built from those defs, and where they stand.
        for ents in &cap.map_ents {
            for block in ents.split('}') {
                let key = |k: &str| block.lines().find_map(|l| l.trim().strip_prefix(&format!("\"{k}\" \"")).map(|v| v.trim_end_matches('"').to_owned()));
                if let Some(def) = key("destructibledef").filter(|d| d.to_ascii_lowercase().contains(&grep)) {
                    println!("ent {def} origin {} angles {}", key("origin").unwrap_or_default(), key("angles").unwrap_or_default());
                }
            }
        }
    }
}
