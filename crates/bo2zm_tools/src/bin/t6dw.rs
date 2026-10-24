//! bo2mp: a dual-wield weapon's per-hand timing, read-only.
//! `t6dw [weapon]...` prints fire, reload, raise/drop and melee times, clip
//! size, the left-hand weapon's name and the per-hand animation slots (with
//! their lengths and notetracks) from the zones in `T6ICON_ZONES` (default
//! common_mp,common_patch_mp). With no names, lists every dual-wield weapon.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::capture_zone;
use fastfile_t6::layout::{WeaponDef as d, WeaponVariantDef as v};

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

/// Animation slots worth printing for a two-gun viewmodel: right fire,
/// lastshot, reload, reload-empty, raise/drop family, then the left-hand set.
const SLOTS: [usize; 17] = [
    0x01, 0x02, 0x04, 0x06, 0x10, 0x11, 0x12, 0x17, 0x19, 0x1C, 0x1D, 0x4E, 0x4F, 0x51, 0x52, 0x53, 0x54,
];

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let zones: Vec<String> = std::env::var("T6ICON_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    for zone in &zones {
        let capture = match capture_zone(&Path::new(ZONES).join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for w in &capture.weapons {
            let left = w
                .strings
                .iter()
                .find(|(o, _)| *o == d::szDualWieldWeaponName)
                .map_or("", |(_, n)| n.as_str());
            let hit = if names.is_empty() {
                w.def_u8(d::bDualWield) != 0 || !left.is_empty()
            } else {
                names.iter().any(|n| n == &w.name)
            };
            if !hit {
                continue;
            }
            println!(
                "{zone} {}: dualWield {} left='{}' ads {} fireType {} clip {}",
                w.name,
                w.def_u8(d::bDualWield),
                left,
                w.def_u8(d::aimDownSight),
                w.def_i32(d::fireType),
                w.var_i32(v::iClipSize),
            );
            println!(
                "    fire {} lastFire {} fireDelay {} introFire {}/{} rechamber {} melee {} meleeDelay {}",
                w.def_i32(d::iFireTime),
                w.def_i32(d::iLastFireTime),
                w.def_i32(d::iFireDelay),
                w.def_i32(d::iIntroFireTime),
                w.def_i32(d::iIntroFireLength),
                w.def_i32(d::iRechamberTime),
                w.def_i32(d::iMeleeTime),
                w.def_i32(d::iMeleeDelay),
            );
            println!(
                "    ammo='{}' clipName='{}' startAmmo {} maxAmmo {} sharedAmmo {}",
                w.ammo_name,
                w.clip_name,
                w.def_i32(d::iStartAmmo),
                w.def_i32(d::iMaxAmmo),
                w.def_u8(d::sharedAmmo),
            );
            println!(
                "    reload {} reloadEmpty {} add {} emptyAdd {} | def reloadRight {} reloadLeft {} reloadEmptyLeft {}",
                w.var_i32(v::iReloadTime),
                w.var_i32(v::iReloadEmptyTime),
                w.def_i32(d::iReloadAddTime),
                w.def_i32(d::iReloadEmptyAddTime),
                w.def_i32(d::iReloadTimeRight),
                w.def_i32(d::iReloadTimeLeft),
                w.def_i32(d::iReloadEmptyTimeLeft),
            );
            println!(
                "    drop {} raise {} quickDrop {} quickRaise {} firstRaise {} emptyRaise {} emptyDrop {}",
                w.def_i32(d::iDropTime),
                w.def_i32(d::iRaiseTime),
                w.def_i32(d::quickDropTime),
                w.def_i32(d::quickRaiseTime),
                w.def_i32(d::iFirstRaiseTime),
                w.def_i32(d::iEmptyRaiseTime),
                w.def_i32(d::iEmptyDropTime),
            );
            for slot in SLOTS {
                if let Some(n) = w.xanims.get(slot).filter(|n| !n.is_empty()) {
                    let len = capture
                        .xanims
                        .iter()
                        .find(|a| a.name.eq_ignore_ascii_case(n))
                        .map(|a| {
                            let ms = f64::from(a.numframes) / f64::from(a.framerate) * 1000.0;
                            let notes: Vec<String> =
                                a.notifies.iter().map(|(k, t)| format!("{k}@{:.0}ms", f64::from(*t) * ms)).collect();
                            format!(" [{ms:.0} ms; notes {}]", notes.join(","))
                        })
                        .unwrap_or_else(|| " [anim not in this zone]".into());
                    println!("    xanim 0x{slot:02X} = {n}{len}");
                }
            }
        }
    }
    ExitCode::SUCCESS
}
