//! bo2mp: a weapon's own HUD materials, read-only.
//! `t6wicon <weapon>...` prints each weapon's material pointers (hudIcon @940,
//! ammoCounterIcon @956, killIcon @1632, reticles) from the zones in
//! `T6ICON_ZONES` (default common_mp,patch_mp,common_patch_mp). With no
//! names it lists every weapon whose name contains `grenade`.

use std::path::Path;

use asset_t6::{WeaponRef, ZoneCapture, capture_zone};
use fastfile_t6::layout::WeaponDef;

/// The zones to read: the comma list in `T6ICON_ZONES`, else the two common ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    setting.map_or_else(
        || vec!["common_mp".to_owned(), "common_patch_mp".to_owned()],
        |list| list.split(',').map(str::to_owned).collect(),
    )
}

/// Whether a weapon is listed: by name when names were given, else every grenade.
fn is_hit(names: &[String], weapon: &str) -> bool {
    names.first().map_or_else(|| weapon.contains("grenade"), |_| names.iter().any(|n| n == weapon))
}

fn indicator_line(zone: &str, w: &WeaponRef) -> String {
    format!(
        "{zone} {}: indicator radius {} explosion radius {} icon {:?} ratio {} | {:?}",
        w.name,
        w.def_i32(WeaponDef::iIndicatorRadius),
        w.def_i32(WeaponDef::iExplosionRadius),
        w.def_material(WeaponDef::indicatorIcon),
        w.def_i32(WeaponDef::indicatorIconRatio),
        w.materials,
    )
}

/// One zone's report: every weapon that shows the grenade indicator when `indicators`,
/// else the listed weapons' material pointers.
fn report(zone: &str, capture: &ZoneCapture, wanted: &[String], indicators: bool) -> Vec<String> {
    let lines = capture.weapons.iter().filter_map(|w| {
        if indicators {
            (w.def_u8(WeaponDef::bShowIndicator) != 0).then(|| indicator_line(zone, w))
        } else {
            is_hit(wanted, &w.name).then(|| format!("{zone} {}: {:?}", w.name, w.materials))
        }
    });
    lines.collect()
}

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    // T6WICON_IND=1: every weapon that shows the grenade indicator, its
    // radius (bShowIndicator @1685, iIndicatorRadius @1716) and pictures.
    let indicators = std::env::var_os("T6WICON_IND").is_some();
    for zone in zone_list(std::env::var("T6ICON_ZONES").ok()) {
        let capture = match capture_zone(&bo2zm_tools::zones_dir().join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        println!("zone {zone}: {} weapons", capture.weapons.len());
        report(&zone, &capture, &names, indicators).iter().for_each(|line| println!("{line}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{zone_list, is_hit};

    #[test]
    fn zones_come_from_the_setting_or_the_two_common_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp"]);
        assert_eq!(zone_list(Some("a,b".into())), ["a", "b"]);
    }

    #[test]
    fn no_names_lists_grenades_and_names_pick_exactly() {
        assert!(is_hit(&[], "frag_grenade_mp"));
        assert!(!is_hit(&[], "an94_mp"));
        assert!(is_hit(&["an94_mp".to_owned()], "an94_mp"));
        assert!(!is_hit(&["an94_mp".to_owned()], "frag_grenade_mp"));
    }
}
