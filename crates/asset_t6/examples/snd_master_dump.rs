//! bo2mp probe: print the sound masters (`SndMaster`) of each zone that holds the sound
//! driver globals: the menu's audio presets pick one by name.
//! cargo run -p asset_t6 --example snd_master_dump -- ZONE...
fn main() {
    for zone in std::env::args().skip(1) {
        let cap = asset_t6::capture_zone(std::path::Path::new(&zone)).expect("zone");
        let Some(g) = cap.snd_globals.as_ref() else { continue };
        println!("{zone}: globals {} masters {}", g.name, g.masters.len());
        for m in &g.masters {
            println!("{m:?}");
        }
    }
}
