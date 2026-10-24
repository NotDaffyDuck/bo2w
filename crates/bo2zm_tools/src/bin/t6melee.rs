//! bo2mp gun brain window 6: what a Black Ops II weapon's definition says
//! about melee, read-only. `t6melee [grep]` prints, for every weapon in the
//! multiplayer zones (`T6MELEE_ZONES` to change them) whose name holds
//! `grep`: melee damage, swing time and hit delay, lunge time, hit delay and
//! range, quick raise / drop times, bayonet and use-as-melee flags, and which
//! melee animation slots it fills (MELEE, MELEE1-3, empty, lunge, lunge empty).

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{PackSet, capture_zone};
use fastfile_t6::layout::WeaponDef as d;

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let grep = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let dir = Path::new(ZONES);
    if let Err(e) = PackSet::open_dir(dir) {
        eprintln!("packs: {e}");
        return ExitCode::FAILURE;
    }
    let zones: Vec<String> = std::env::var("T6MELEE_ZONES").map_or_else(
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
        for w in capture.weapons.iter().filter(|w| w.name.to_ascii_lowercase().contains(&grep)) {
            if w.def.is_empty() {
                continue;
            }
            // Animation slots 9..=15: MELEE, MELEE1, MELEE2, MELEE3,
            // MELEE_EMPTY, MELEE_CHARGE, MELEE_CHARGE_EMPTY.
            let slots: Vec<String> = (9..=15)
                .map(|i| w.xanims.get(i).filter(|s| !s.is_empty()).map_or("-".into(), |s| s.clone()))
                .collect();
            println!(
                "{zone} {}: type {} class {} meleeDamage {} meleeTime {} meleeDelay {} chargeTime {} chargeDelay {} chargeRange {} quickRaise {} quickDrop {} bayonet {} useAsMelee {}",
                w.name,
                w.def_i32(d::weapType),
                w.def_i32(d::weapClass),
                w.def_i32(d::iMeleeDamage),
                w.def_i32(d::iMeleeTime),
                w.def_i32(d::iMeleeDelay),
                w.def_i32(d::meleeChargeTime),
                w.def_i32(d::meleeChargeDelay),
                w.def_f32(d::meleeChargeRange),
                w.def_i32(d::quickRaiseTime),
                w.def_i32(d::quickDropTime),
                w.def_u8(d::bHasBayonet),
                w.def_u8(d::bUseAsMelee),
            );
            println!("  anims {}", slots.join(" | "));
        }
    }
    ExitCode::SUCCESS
}
