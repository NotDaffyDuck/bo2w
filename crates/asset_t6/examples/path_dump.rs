//! bo2mp probe: print a map's path nodes as its zone holds them, one line each:
//! `index type spawnflags x y z targetname target animscript | node:dist[:neg] ...`.
//! cargo run -p asset_t6 --example path_dump -- ZONE
fn main() {
    let zone = std::env::args().nth(1).expect("zone path");
    let capture = asset_t6::capture_zone(std::path::Path::new(&zone)).expect("zone");
    for (i, n) in capture.path_nodes.iter().enumerate() {
        let links: Vec<String> = n
            .links
            .iter()
            .map(|l| format!("{}:{:.0}{}", l.0, l.1, if l.2 { ":neg" } else { "" }))
            .collect();
        let or_dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };
        println!(
            "{i} {} {:#x} {:.0} {:.0} {:.0} {} {} {} | {}",
            n.ty,
            n.spawnflags,
            n.origin[0],
            n.origin[1],
            n.origin[2],
            or_dash(&n.target_key),
            or_dash(&n.target),
            or_dash(&n.animscript),
            links.join(" ")
        );
    }
}
