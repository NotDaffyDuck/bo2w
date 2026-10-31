//! bo2mp gun brain window 6: what a Black Ops II weapon's definition says
//! about melee, read-only. `t6melee [grep]` prints, for every weapon in the
//! multiplayer zones (`T6MELEE_ZONES` to change them) whose name holds
//! `grep`: melee damage, swing time and hit delay, lunge time, hit delay and
//! range, quick raise / drop times, bayonet and use-as-melee flags, and which
//! melee animation slots it fills (CLOSE_COMBAT, MELEE1-3, empty, lunge, lunge empty).

use std::path::Path;

use asset_t6::{PackSet, WeaponRef, capture_zone};
use fastfile_t6::layout::WeaponDef;

/// The zones to read: the comma list in `T6MELEE_ZONES`, else the multiplayer ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    setting.map_or_else(
        || ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |list| list.split(',').map(str::to_owned).collect(),
    )
}

/// Animation slots 9..=15: CLOSE_COMBAT, MELEE1, MELEE2, MELEE3, MELEE_EMPTY,
/// MELEE_CHARGE, MELEE_CHARGE_EMPTY; a dash for each the weapon leaves empty.
fn melee_anims(animation_refs: &[String]) -> String {
    let names: Vec<&str> = (9..=15).map(|i| animation_refs.get(i).map_or("-", |s| if s.is_empty() { "-" } else { s })).collect();
    names.join(" | ")
}

/// Whether a weapon's name holds the (lower-case) search text.
fn name_matches(name: &str, grep: &str) -> bool {
    name.to_ascii_lowercase().contains(grep)
}

fn print_weapon(zone: &str, w: &WeaponRef) {
    println!(
        "{zone} {}: type {} class {} meleeDamage {} meleeTime {} meleeDelay {} chargeTime {} chargeDelay {} chargeRange {} quickRaise {} quickDrop {} bayonet {} useAsMelee {}",
        w.name,
        w.def_i32(WeaponDef::weapType),
        w.def_i32(WeaponDef::weapClass),
        w.def_i32(WeaponDef::iMeleeDamage),
        w.def_i32(WeaponDef::iMeleeTime),
        w.def_i32(WeaponDef::iMeleeDelay),
        w.def_i32(WeaponDef::meleeChargeTime),
        w.def_i32(WeaponDef::meleeChargeDelay),
        w.def_f32(WeaponDef::meleeChargeRange),
        w.def_i32(WeaponDef::quickRaiseTime),
        w.def_i32(WeaponDef::quickDropTime),
        w.def_u8(WeaponDef::bHasBayonet),
        w.def_u8(WeaponDef::bUseAsMelee),
    );
    println!("  anims {}", melee_anims(&w.animation_refs));
}

fn run(grep: &str) -> Result<(), String> {
    let dir = &bo2zm_tools::zones_dir();
    PackSet::open_dir(dir).map_err(|e| format!("packs: {e}"))?;
    for zone in zone_list(std::env::var("T6MELEE_ZONES").ok()) {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        capture
            .weapons
            .iter()
            .filter(|w| name_matches(&w.name, grep) && !w.def.is_empty())
            .for_each(|w| print_weapon(&zone, w));
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let grep = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let outcome = run(&grep);
    if let Err(e) = &outcome {
        eprintln!("{e}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use super::{zone_list, melee_anims, name_matches};

    #[test]
    fn zones_come_from_the_setting_or_the_three_multiplayer_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp", "patch_mp"]);
        assert_eq!(zone_list(Some("x,y".into())), ["x", "y"]);
    }

    #[test]
    fn missing_and_empty_melee_slots_print_a_dash() {
        let mut anims = vec![String::new(); 12];
        anims[9] = "knife".into();
        anims[11] = "knife3".into();
        assert_eq!(melee_anims(&anims), "knife | - | knife3 | - | - | - | -");
        assert_eq!(melee_anims(&[]), "- | - | - | - | - | - | -");
    }

    #[test]
    fn the_search_is_a_lower_case_substring_of_the_name() {
        assert!(name_matches("Knife_Ballistic_MP", "ballistic"));
        assert!(name_matches("an94_mp", ""));
        assert!(!name_matches("an94_mp", "knife"));
    }
}
