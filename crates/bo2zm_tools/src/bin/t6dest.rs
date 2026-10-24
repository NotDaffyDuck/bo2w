//! bo2mp: print a T6 zone's destructible defs (pieces, health, damage scales, stages).
//! `t6dest <zone.ff> [name...]` (no names: every def). Read-only.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6dest <zone.ff> [name...]");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    for d in c.destructibles.iter().filter(|d| names.is_empty() || names.contains(&d.name)) {
        println!("{d:#?}");
    }
}
