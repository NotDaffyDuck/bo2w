//! t6portals ZONE.ff...: list a Black Ops II map's portals, read-only, to pick spots where
//! the portal walk decides what is drawn (doorways, windows).
//!
//! One line per portal:
//! `portal N cell FROM -> TO centre X Y Z verts V size W x H yaw Y`
//! where the centre is the mean of its vertices, the size is its extent along its two hull
//! axes and the yaw (degrees) is the direction its plane faces. Run from the workspace:
//! `cargo run -p dpvs_t6 --example t6portals -- "...\zone\all\mp_nuketown_2020.ff"`.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::capture_zone;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: t6portals ZONE.ff...");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    for path in &paths {
        let capture = match capture_zone(Path::new(path)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("t6portals: {path}: {e}");
                failed = true;
                continue;
            }
        };
        let Some(world) = capture.world.as_ref() else {
            eprintln!("t6portals: {path} has no GfxWorld");
            failed = true;
            continue;
        };
        let count: usize = world.cell_bounds.iter().map(|c| c.portals.len()).sum();
        println!("{path}: {} cells, {count} portals", world.cell_bounds.len());
        let mut n = 0;
        for (from, cell) in world.cell_bounds.iter().enumerate() {
            for p in &cell.portals {
                let v = p.vertices.len().max(1) as f32;
                let c: [f32; 3] =
                    core::array::from_fn(|i| p.vertices.iter().map(|q| q[i]).sum::<f32>() / v);
                let extent = |axis: [f32; 3]| {
                    let d = p.vertices.iter().map(|q| dot(*q, axis));
                    let (lo, hi) = d.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
                    if hi >= lo { hi - lo } else { 0.0 }
                };
                let (w, h) = (extent(p.hull_axis[0]), extent(p.hull_axis[1]));
                let yaw = p.plane[1].atan2(p.plane[0]).to_degrees();
                println!(
                    "portal {n} cell {from} -> {} centre {:.1} {:.1} {:.1} verts {} size {w:.0} x {h:.0} yaw {yaw:.0}",
                    p.cell,
                    c[0],
                    c[1],
                    c[2],
                    p.vertices.len()
                );
                n += 1;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
