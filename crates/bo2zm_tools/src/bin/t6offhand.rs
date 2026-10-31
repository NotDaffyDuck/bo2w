//! bo2mp: what a Black Ops II grenade or piece of equipment's definition says
//! about throwing it, read-only. `t6offhand [weapon...]` looks each weapon up
//! in the multiplayer zones (`T6OFFHAND_ZONES` to change them) and prints its
//! offhand slot, the hold / throw / raise / drop times, the cook fuse, the
//! detonator and plant flags and the throw speeds. With no names it prints
//! every grenade and mine. `t6offhand --facts NAME...` prints instead one
//! `name<TAB>variant hex<TAB>def hex` row per weapon (the last zone wins), the
//! dump `gun_trace` reads.

use std::path::Path;

use asset_t6::{WeaponRef, capture_zone};
use fastfile_t6::layout::WeaponDef;

/// The zones to read: the comma list in `T6OFFHAND_ZONES`, else the multiplayer ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    setting.map_or_else(
        || ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |list| list.split(',').map(str::to_owned).collect(),
    )
}

/// The command line: `--facts` first, then the weapon names.
#[derive(Debug, PartialEq)]
struct CmdLine {
    facts: bool,
    names: Vec<String>,
}

fn parse_args(argv: Vec<String>) -> CmdLine {
    match argv.split_first() {
        Some((flag, tail)) if flag == "--facts" => CmdLine { facts: true, names: tail.to_vec() },
        _ => CmdLine { facts: false, names: argv },
    }
}

/// Whether a weapon is taken: by name when names were given, else every
/// grenade (type 1) and mine (type 2), the two kinds the offhand path takes.
fn is_wanted(names: &[String], kind: i32, wanted: &str) -> bool {
    names.first().map_or(matches!(kind, 1 | 2), |_| names.iter().any(|n| n == wanted))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Keep one row per weapon: a later zone's row replaces the earlier one in place.
fn upsert(rows: &mut Vec<(String, String)>, name: &str, row: String) {
    match rows.iter_mut().find(|(n, _)| n == name) {
        Some(found) => found.1 = row,
        None => rows.push((name.to_owned(), row)),
    }
}

/// Rows in the order the names were given (gun_trace numbers them from 1).
fn in_argument_order(mut rows: Vec<(String, String)>, names: &[String]) -> Vec<String> {
    rows.sort_by_key(|(n, _)| names.iter().position(|x| x == n).unwrap_or(usize::MAX));
    rows.into_iter().map(|(_, row)| row).collect()
}

fn print_weapon(zone: &str, w: &WeaponRef) {
    println!(
        "{zone} {} type {} class {} offhandClass {} slot {}",
        w.name,
        w.def_i32(WeaponDef::weapType),
        w.def_i32(WeaponDef::weapClass),
        w.def_i32(WeaponDef::offhandClass),
        w.def_i32(WeaponDef::offhandSlot),
    );
    println!(
        "  holdFire {}ms fire {}ms fireDelay {}ms quickRaise {}ms quickDrop {}ms raise {}ms drop {}ms firstRaise {}ms",
        w.def_i32(WeaponDef::iHoldFireTime),
        w.def_i32(WeaponDef::iFireTime),
        w.def_i32(WeaponDef::iFireDelay),
        w.def_i32(WeaponDef::quickRaiseTime),
        w.def_i32(WeaponDef::quickDropTime),
        w.def_i32(WeaponDef::iRaiseTime),
        w.def_i32(WeaponDef::iDropTime),
        w.def_i32(WeaponDef::iFirstRaiseTime),
    );
    println!(
        "  fuse {}ms cookOffHold {} holdButtonToThrow {} cancelable {} holdUse {} plantable {} detonator {} timedDet {} detTime {}ms detDelay {}ms",
        w.def_i32(WeaponDef::fuseTime),
        w.def_u8(WeaponDef::bCookOffHold),
        w.def_u8(WeaponDef::holdButtonToThrow),
        w.def_u8(WeaponDef::offhandHoldIsCancelable),
        w.def_u8(WeaponDef::isHoldUseGrenade),
        w.def_u8(WeaponDef::plantable),
        w.def_u8(WeaponDef::hasDetonator),
        w.def_u8(WeaponDef::timedDetonation),
        w.def_i32(WeaponDef::iDetonateTime),
        w.def_i32(WeaponDef::iDetonateDelay),
    );
    println!(
        "  speed {} up {} forward {}",
        w.def_i32(WeaponDef::iProjectileSpeed),
        w.def_i32(WeaponDef::iProjectileSpeedUp),
        w.def_i32(WeaponDef::iProjectileSpeedForward),
    );
}

fn main() {
    let CmdLine { facts, names } = parse_args(std::env::args().skip(1).collect());
    let mut rows: Vec<(String, String)> = vec![];
    let dir = &bo2zm_tools::zones_dir();
    for zone in zone_list(std::env::var("T6OFFHAND_ZONES").ok()) {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        let wanted = capture.weapons.iter().filter(|w| !w.def.is_empty() && is_wanted(&names, w.def_i32(WeaponDef::weapType), &w.name));
        for w in wanted {
            if facts {
                upsert(&mut rows, &w.name, format!("{}\t{}\t{}", w.name, hex(&w.variant), hex(&w.def)));
            } else {
                print_weapon(&zone, w);
            }
        }
    }
    in_argument_order(rows, &names).iter().for_each(|row| println!("{row}"));
}

#[cfg(test)]
mod tests {
    use super::{zone_list, parse_args, CmdLine, is_wanted, upsert, in_argument_order, hex};

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn zones_come_from_the_setting_or_the_three_multiplayer_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp", "patch_mp"]);
        assert_eq!(zone_list(Some("q".into())), ["q"]);
    }

    #[test]
    fn the_facts_flag_is_only_taken_in_first_place() {
        assert_eq!(parse_args(strings(&["--facts", "a"])), CmdLine { facts: true, names: strings(&["a"]) });
        assert_eq!(parse_args(strings(&["a", "--facts"])), CmdLine { facts: false, names: strings(&["a", "--facts"]) });
        assert_eq!(parse_args(vec![]), CmdLine { facts: false, names: vec![] });
    }

    #[test]
    fn no_names_takes_grenades_and_mines_and_names_take_exactly_those() {
        assert!(is_wanted(&[], 1, "frag"));
        assert!(is_wanted(&[], 2, "claymore"));
        assert!(!is_wanted(&[], 0, "an94"));
        assert!(is_wanted(&strings(&["an94"]), 0, "an94"));
        assert!(!is_wanted(&strings(&["an94"]), 1, "frag"));
    }

    #[test]
    fn a_later_zone_replaces_the_row_in_place_and_rows_follow_argument_order() {
        let mut rows = vec![];
        upsert(&mut rows, "b", "b-old".into());
        upsert(&mut rows, "a", "a1".into());
        upsert(&mut rows, "b", "b-new".into());
        assert_eq!(in_argument_order(rows.clone(), &strings(&["a", "b"])), ["a1", "b-new"]);
        assert_eq!(in_argument_order(rows, &[]), ["b-new", "a1"]);
    }

    #[test]
    fn bytes_print_as_two_lower_case_hex_digits() {
        assert_eq!(hex(&[1, 171]), "01ab");
    }
}
