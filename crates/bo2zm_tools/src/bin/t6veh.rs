//! bo2mp: print a T6 vehicle def's drive and engine values.
//! `t6veh <zone.ff> <vehicle name>...`
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6veh <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    for v in c.vehicles.iter().filter(|v| names.is_empty() || names.contains(&v.name)) {
        println!("{v:#?}");
    }
}
