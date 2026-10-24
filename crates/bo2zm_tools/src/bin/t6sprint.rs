//! bo2mp: a weapon's movement-state gun times, read-only (spec aid for
//! docs/gun/SPRINT.md). `t6sprint <weapon>...` prints sprint, dive, slide,
//! crawl and low-ready times plus the movement animation lengths from the
//! zones in `T6ICON_ZONES` (default common_mp,common_patch_mp).
//! With `T6SPRINT_DUMP=<file>` it also writes the weapons as a gun_trace facts
//! dump (`name<TAB>variant hex<TAB>def hex`, later zones win), in argument order.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::capture_zone;
use fastfile_t6::layout::WeaponDef as d;

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

/// Animation file slots: sprint in/loop/out (+empty), crawl in..out, dive
/// in/loop/out (+empty), slide in, mantle, camera sprint/dive/mantle.
const SLOTS: [(usize, &str); 23] = [
    (32, "sprint_in"),
    (33, "sprint_loop"),
    (34, "sprint_out"),
    (35, "sprint_empty_in"),
    (37, "sprint_empty_out"),
    (38, "lowready_in"),
    (44, "crawl_in"),
    (45, "crawl_forward"),
    (46, "crawl_back"),
    (47, "crawl_right"),
    (48, "crawl_left"),
    (49, "crawl_out"),
    (65, "dtp_in"),
    (66, "dtp_loop"),
    (67, "dtp_out"),
    (68, "dtp_empty_in"),
    (70, "dtp_empty_out"),
    (71, "slide_in"),
    (72, "mantle"),
    (73, "camera_sprint_loop"),
    (74, "camera_dtp_in"),
    (76, "camera_dtp_out"),
    (77, "camera_mantle"),
];

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let zones: Vec<String> = std::env::var("T6ICON_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let mut blobs: Vec<Option<(Vec<u8>, Vec<u8>)>> = vec![None; names.len()];
    for zone in &zones {
        let capture = match capture_zone(&Path::new(ZONES).join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for w in &capture.weapons {
            let Some(i) = names.iter().position(|n| n == &w.name) else {
                continue;
            };
            blobs[i] = Some((w.variant.clone(), w.def.clone()));
            println!(
                "{zone} {}: sprint[in {} loop {} out {}] lowready[in {} loop {} out {}] dtp[in {} loop {} out {}] crawl[in {} fwd {} back {} right {} left {} outFire {} out {}] slideIn {} sprintDurationScale {} blocksProne {}",
                w.name,
                w.def_i32(d::sprintInTime),
                w.def_i32(d::sprintLoopTime),
                w.def_i32(d::sprintOutTime),
                w.def_i32(d::lowReadyInTime),
                w.def_i32(d::lowReadyLoopTime),
                w.def_i32(d::lowReadyOutTime),
                w.def_i32(d::dtpInTime),
                w.def_i32(d::dtpLoopTime),
                w.def_i32(d::dtpOutTime),
                w.def_i32(d::crawlInTime),
                w.def_i32(d::crawlForwardTime),
                w.def_i32(d::crawlBackTime),
                w.def_i32(d::crawlRightTime),
                w.def_i32(d::crawlLeftTime),
                w.def_i32(d::crawlOutFireTime),
                w.def_i32(d::crawlOutTime),
                w.def_i32(d::slideInTime),
                w.def_f32(d::sprintDurationScale),
                w.def_u8(d::blocksProne),
            );
            for (slot, label) in SLOTS {
                if let Some(n) = w.xanims.get(slot).filter(|n| !n.is_empty()) {
                    let len = capture
                        .xanims
                        .iter()
                        .find(|a| a.name.eq_ignore_ascii_case(n))
                        .map(|a| format!(" [{:.0} ms]", f64::from(a.numframes) / f64::from(a.framerate) * 1000.0))
                        .unwrap_or_default();
                    println!("    {label} = {n}{len}");
                }
            }
        }
    }
    if let Ok(path) = std::env::var("T6SPRINT_DUMP") {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut text = String::new();
        for (name, blob) in names.iter().zip(&blobs) {
            let Some((v, d)) = blob else {
                eprintln!("{name}: not in the zones");
                return ExitCode::FAILURE;
            };
            text.push_str(&format!("{name}	{}	{}
", hex(v), hex(d)));
        }
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
