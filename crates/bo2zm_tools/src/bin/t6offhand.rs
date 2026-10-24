//! bo2mp: what a Black Ops II grenade or piece of equipment's definition says
//! about throwing it, read-only. `t6offhand [weapon...]` looks each weapon up
//! in the multiplayer zones (`T6OFFHAND_ZONES` to change them) and prints its
//! offhand slot, the hold / throw / raise / drop times, the cook fuse, the
//! detonator and plant flags and the throw speeds. With no names it prints
//! every grenade and mine. `t6offhand --facts NAME...` prints instead one
//! `name<TAB>variant hex<TAB>def hex` row per weapon (the last zone wins), the
//! dump `gun_trace` reads.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::capture_zone;
use fastfile_t6::layout::WeaponDef as d;

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    let facts = names.first().is_some_and(|a| a == "--facts");
    if facts {
        names.remove(0);
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let dir = Path::new(ZONES);
    let zones: Vec<String> = std::env::var("T6OFFHAND_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for w in capture.weapons.iter().filter(|w| !w.def.is_empty()) {
            let kind = w.def_i32(d::weapType);
            // 1 = grenade, 2 = mine (the two kinds the offhand path takes).
            let wanted = if names.is_empty() { kind == 1 || kind == 2 } else { names.iter().any(|n| n == &w.name) };
            if !wanted {
                continue;
            }
            if facts {
                let row = format!("{}	{}	{}", w.name, hex(&w.variant), hex(&w.def));
                match rows.iter_mut().find(|(n, _)| n == &w.name) {
                    Some(r) => r.1 = row,
                    None => rows.push((w.name.clone(), row)),
                }
                continue;
            }
            println!(
                "{zone} {} type {} class {} offhandClass {} slot {}",
                w.name,
                kind,
                w.def_i32(d::weapClass),
                w.def_i32(d::offhandClass),
                w.def_i32(d::offhandSlot),
            );
            println!(
                "  holdFire {}ms fire {}ms fireDelay {}ms quickRaise {}ms quickDrop {}ms raise {}ms drop {}ms firstRaise {}ms",
                w.def_i32(d::iHoldFireTime),
                w.def_i32(d::iFireTime),
                w.def_i32(d::iFireDelay),
                w.def_i32(d::quickRaiseTime),
                w.def_i32(d::quickDropTime),
                w.def_i32(d::iRaiseTime),
                w.def_i32(d::iDropTime),
                w.def_i32(d::iFirstRaiseTime),
            );
            println!(
                "  fuse {}ms cookOffHold {} holdButtonToThrow {} cancelable {} holdUse {} plantable {} detonator {} timedDet {} detTime {}ms detDelay {}ms",
                w.def_i32(d::fuseTime),
                w.def_u8(d::bCookOffHold),
                w.def_u8(d::holdButtonToThrow),
                w.def_u8(d::offhandHoldIsCancelable),
                w.def_u8(d::isHoldUseGrenade),
                w.def_u8(d::plantable),
                w.def_u8(d::hasDetonator),
                w.def_u8(d::timedDetonation),
                w.def_i32(d::iDetonateTime),
                w.def_i32(d::iDetonateDelay),
            );
            println!(
                "  speed {} up {} forward {}",
                w.def_i32(d::iProjectileSpeed),
                w.def_i32(d::iProjectileSpeedUp),
                w.def_i32(d::iProjectileSpeedForward),
            );
        }
    }
    // Rows follow the order the names were given (gun_trace numbers them from 1).
    rows.sort_by_key(|(n, _)| names.iter().position(|x| x == n).unwrap_or(usize::MAX));
    for (_, row) in &rows {
        println!("{row}");
    }
    ExitCode::SUCCESS
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
