//! bo2mp probe: print BO2 localized strings whose key holds LOC_GREP, and
//! material names holding MAT_GREP (and their draw states).
//! cargo run -p asset_t6 --example loc_dump -- ZONE...
fn main() {
    let grep = std::env::var("LOC_GREP").unwrap_or_default().to_ascii_uppercase();
    let mat = std::env::var("MAT_GREP").unwrap_or_default().to_ascii_lowercase();
    for zone in std::env::args().skip(1) {
        let cap = asset_t6::capture_zone(std::path::Path::new(&zone)).expect("zone");
        if !grep.is_empty() {
            for (k, v) in &cap.localize {
                if k.to_ascii_uppercase().contains(&grep) {
                    println!("{k}\t{v:?}");
                }
            }
        }
        if !mat.is_empty() {
            for m in &cap.materials {
                if m.name.to_ascii_lowercase().contains(&mat) {
                    println!("material {}", m.name);
                    for t in 0..36 {
                        if let Some(d) = m.draw_state(t) {
                            println!("  tech {t}: src {} dst {} op {} atest {:?}", d.src_blend, d.dst_blend, d.blend_op, d.alpha_test);
                        }
                    }
                }
            }
        }
    }
}
