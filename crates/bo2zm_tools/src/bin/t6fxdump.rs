//! bo2zm: one Black Ops II effect's elements, read-only. `t6fxdump <effect
//! name> [bo2 root]` prints per element its type, flags, life, spawn, fade
//! ranges and each visual state sample's colour and size (base / amplitude).

use std::path::PathBuf;
use std::process::ExitCode;

use fastfile_t6::layout::FxElemDef as e;

const DEFAULT_BO2: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II";
const ZONES: [&str; 4] = ["common_zm", "patch_zm", "zm_nuked", "zm_nuked_patch"];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(want) = args.next() else {
        eprintln!("usage: t6fxdump <effect name> [bo2 root]");
        return ExitCode::FAILURE;
    };
    let root = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_BO2.to_owned()));
    let dir = root.join("zone").join("all");
    // T6FXDUMP_ZONES=a,b: read these zones instead (another map's effects).
    let zones: Vec<String> = std::env::var("T6FXDUMP_ZONES").map_or_else(
        |_| ZONES.iter().map(|z| (*z).to_owned()).collect(),
        |v| v.split(',').map(str::to_owned).collect(),
    );
    for z in &zones {
        let Ok(c) = asset_t6::capture_zone(&dir.join(format!("{z}.ff"))) else { continue };
        for fx in c.fx.iter().filter(|f| f.name == want) {
            println!("== {z}: {} ({} elems)", fx.name, fx.elems.len());
            for (n, el) in fx.elems.iter().enumerate() {
                let b = |o: usize| el.raw.get(o).copied().unwrap_or(0);
                println!(
                    "#{n} type {} flags {:#010x} life {}+{} spawn {} {} delay {}+{} fadeIn {:?} fadeOut {:?} visInt {} visuals {:?}",
                    b(e::elemType),
                    el.i32_at(e::flags),
                    el.i32_at(e::lifeSpanMsec),
                    el.i32_at(e::lifeSpanMsec + 4),
                    el.i32_at(e::spawn),
                    el.i32_at(e::spawn + 4),
                    el.i32_at(e::spawnDelayMsec),
                    el.i32_at(e::spawnDelayMsec + 4),
                    [el.f32_at(e::fadeInRange), el.f32_at(e::fadeInRange + 4)],
                    [el.f32_at(e::fadeOutRange), el.f32_at(e::fadeOutRange + 4)],
                    b(e::visStateIntervalCount),
                    el.visuals.iter().take(2).collect::<Vec<_>>(),
                );
                let r = |o: usize| [el.f32_at(o), el.f32_at(o + 4)];
                println!(
                    "   origin {:?} {:?} {:?} offR {:?} offH {:?} angles {:?} {:?} {:?} grav {:?} velInt {}",
                    r(e::spawnOrigin), r(e::spawnOrigin + 8), r(e::spawnOrigin + 16),
                    r(e::spawnOffsetRadius), r(e::spawnOffsetHeight),
                    r(e::spawnAngles), r(e::spawnAngles + 8), r(e::spawnAngles + 16),
                    r(e::gravity), b(e::velIntervalCount),
                );
                let vs: Vec<&[u8]> = el.vel_samples.chunks(96).collect();
                for (k, s) in vs.iter().enumerate().filter(|(k, _)| *k == 0 || *k + 1 == vs.len()) {
                    let v = |o: usize| [0, 4, 8].map(|d| f32::from_le_bytes([s[o + d], s[o + d + 1], s[o + d + 2], s[o + d + 3]]));
                    println!(
                        "   v{k}: local vel {:?} delta {:?} world vel {:?} delta {:?} | amp local delta {:?} world delta {:?}",
                        v(0), v(12), v(24), v(36), v(60), v(84)
                    );
                }
                for (k, s) in el.vis_samples.chunks(48).enumerate() {
                    let f = |o: usize| f32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
                    println!(
                        "   s{k}: col {:?}/{:?} size {:.0},{:.0}/{:.0},{:.0} scale {:.2}/{:.2}",
                        &s[0..4], &s[24..28], f(12), f(16), f(36), f(40), f(20), f(44)
                    );
                }
            }
        }
    }
    ExitCode::SUCCESS
}
