//! bo2mp probe: print BO2 destructible defs (DestructibleDef) whose name holds
//! DGREP: each piece's health and the stages it breaks through (shown bone,
//! health share, timer, notify, effect, sound, loop sound).
//! cargo run -p asset_t6 --example destructible_dump -- ZONE...
use asset_t6::capture::ZoneCapture;
use asset_transport::{T6ZoneMemory, open_zone};

fn main() {
    let grep = std::env::var("DGREP").unwrap_or_default().to_ascii_lowercase();
    for path in std::env::args().skip(1) {
        let Ok(image) = open_zone(&path) else { continue };
        let Ok(header) = image.t6_header() else { continue };
        let mut memory = T6ZoneMemory::for_header(&header);
        let Ok(mut stream) = memory.stream(&image.bytes) else { continue };
        let Ok(table) = fastfile_t6::open_asset_table(&mut stream) else { continue };
        let mut cap = ZoneCapture::default();
        println!("== {path}");
        if let Err(e) = fastfile_t6::Walker::new(&mut stream, &mut cap).walk_assets(&table) {
            eprintln!("{path}: walk failed: {e}");
        }
        for d in cap.destructibles.iter().filter(|d| d.name.to_ascii_lowercase().contains(&grep)) {
            println!("def {} model {} pieces {}", d.name, d.model, d.pieces.len());
            for (k, p) in d.pieces.iter().enumerate() {
                println!("  piece {k}: health {} parent {} bullet {} explosive {}", p.health, p.parent_piece, p.bullet_damage_scale, p.explosive_damage_scale);
                for (j, st) in p.stages.iter().enumerate() {
                    if st.show_bone.is_empty() && st.break_notify.is_empty() && st.loop_sound.is_empty() && st.break_health == 0.0 {
                        continue;
                    }
                    println!("    stage {j}: bone {:?} health {} time {} notify {:?} fx {:?} sound {:?} loop {:?}",
                        st.show_bone, st.break_health, st.max_time, st.break_notify, st.break_effect, st.break_sound, st.loop_sound);
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
