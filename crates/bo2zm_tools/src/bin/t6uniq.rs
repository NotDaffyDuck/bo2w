//! bo2mp: each Black Ops II multiplayer gun's attachment-unique overrides, read-only.
//! `t6uniq [weapon...]` prints, per gun, the def/variant first raise, alt raise and drop,
//! reload ammo add, start add and segmented flag, then every `au_<gun>_<attachment>` whose
//! value for one of them is not 0. BO2's getters (BG_GetFirstRaiseTime etc.) take the first
//! attachment unique that differs from the gun's base one, else the def's. Zones as t6scope
//! (`T6UNIQ_ZONES` to change them). Also lists the uniques not named `au_...`.
//! `T6UNIQ_LOC=1` instead prints each `au_` whose hit-location table differs from its gun's
//! base unique `au_<gun>_none` (BG_GetWeaponHitLocationMultiplier: only those change a hit).

use std::path::Path;

use asset_t6::{AttachmentUniqueRef, PackSet, WeaponRef, ZoneCapture, capture_zone};
use fastfile_t6::layout::{WeaponAttachmentUnique, WeaponDef, WeaponVariantDef};

/// The zones to read: the comma list in `T6UNIQ_ZONES`, else the multiplayer ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    setting.map_or_else(
        || ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |list| list.split(',').map(str::to_owned).collect(),
    )
}

/// Whether a multiplayer gun is listed: every one when no names were given.
fn is_listed(names: &[String], weapon: &str) -> bool {
    weapon.ends_with("_mp") && (names.is_empty() || names.iter().any(|n| n == weapon))
}

/// The attachment uniques named something other than `au_...`.
fn odd_names(uniques: &[AttachmentUniqueRef]) -> Vec<&str> {
    uniques.iter().map(|a| a.name.as_str()).filter(|n| !n.starts_with("au_")).collect()
}

/// `index:gun->unique` for each hit location where the unique's table differs
/// from the gun's (a location the gun's table lacks reads NaN).
fn location_diff(unique: &[f32], gun: &[f32]) -> Vec<String> {
    let differing = unique.iter().enumerate().filter(|&(i, x)| gun.get(i) != Some(x));
    differing.map(|(i, x)| format!("{i}:{}->{x}", gun.get(i).copied().unwrap_or(f32::NAN))).collect()
}

/// The values a unique can override, in print order: first raise, alt raise, alt drop,
/// reload ammo add, reload start add, segmented-reload flag.
fn override_values(au: &AttachmentUniqueRef) -> [i32; 6] {
    [
        au.int_from_bytes(WeaponAttachmentUnique::iFirstRaiseTime),
        au.int_from_bytes(WeaponAttachmentUnique::iAltRaiseTime),
        au.int_from_bytes(WeaponAttachmentUnique::iAltDropTime),
        au.int_from_bytes(WeaponAttachmentUnique::iReloadAmmoAdd),
        au.int_from_bytes(WeaponAttachmentUnique::iReloadStartAdd),
        i32::from(au.bytes.get(WeaponAttachmentUnique::bSegmentedReload).copied().unwrap_or(0)),
    ]
}

/// The uniques of gun `prefix` (`au_<gun>_`) whose hit-location table differs from the base one.
fn location_lines(zone: &str, capture: &ZoneCapture, w: &WeaponRef, lead: &str) -> Vec<String> {
    let none = format!("{lead}none");
    let gun = capture
        .attachment_uniques
        .iter()
        .find(|a| a.name == none)
        .and_then(|a| a.location_damage.as_deref())
        .unwrap_or(&[]);
    capture
        .attachment_uniques
        .iter()
        .filter(|a| a.name.starts_with(lead))
        .filter_map(|au| au.location_damage.as_deref().map(|t| (au, location_diff(t, gun))))
        .filter(|(_key, diff)| !diff.is_empty())
        .map(|(au, diff)| format!("{zone} {} {}: {}", w.name, au.name, diff.join(" ")))
        .collect()
}

/// The uniques of gun `prefix` that change at least one value, one line each.
fn override_lines(capture: &ZoneCapture, lead: &str) -> Vec<String> {
    let overriding = capture.attachment_uniques.iter().filter(|a| a.name.starts_with(lead));
    overriding
        .map(|au| (au, override_values(au)))
        .filter(|(_key, vals)| vals.iter().any(|&x| x != 0))
        .map(|(au, vals)| format!("    {} {:?}", au.name, vals))
        .collect()
}

fn gun_lines(zone: &str, capture: &ZoneCapture, w: &WeaponRef, loc_only: bool) -> Vec<String> {
    let prefix = format!("au_{}_", w.name.trim_end_matches("_mp"));
    if loc_only {
        return location_lines(zone, capture, w, &prefix);
    }
    let first = format!(
        "{zone} {} first {} altRaise {} altDrop {} ammoAdd {} startAdd {} segmented {}",
        w.name,
        w.def_i32(WeaponDef::iFirstRaiseTime),
        w.var_i32(WeaponVariantDef::iAltRaiseTime),
        w.def_i32(WeaponDef::iAltDropTime),
        w.def_i32(WeaponDef::iReloadAmmoAdd),
        w.def_i32(WeaponDef::iReloadStartAdd),
        w.def_u8(WeaponDef::bSegmentedReload),
    );
    std::iter::once(first).chain(override_lines(capture, &prefix)).collect()
}

fn run(names: &[String]) -> Result<(), String> {
    let dir = &bo2zm_tools::zones_dir();
    PackSet::open_dir(dir).map_err(|e| format!("packs: {e}"))?;
    let loc_only = std::env::var_os("T6UNIQ_LOC").is_some();
    for zone in zone_list(std::env::var("T6UNIQ_ZONES").ok()) {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        let uniques = &capture.attachment_uniques;
        println!("{zone}: {} uniques, not au_: {:?}", uniques.len(), odd_names(uniques));
        capture
            .weapons
            .iter()
            .filter(|w| is_listed(names, &w.name))
            .flat_map(|w| gun_lines(&zone, &capture, w, loc_only))
            .for_each(|line| println!("{line}"));
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let outcome = run(&names);
    if let Err(e) = &outcome {
        eprintln!("{e}");
    }
    std::process::ExitCode::from(u8::from(outcome.is_err()))
}

#[cfg(test)]
mod tests {
    use asset_t6::AttachmentUniqueRef;
    use super::{zone_list, is_listed, odd_names, location_diff, override_values};

    fn unique(name: &str) -> AttachmentUniqueRef {
        AttachmentUniqueRef { name: String::from(name), ..AttachmentUniqueRef::default() }
    }

    #[test]
    fn zones_come_from_the_setting_or_the_three_multiplayer_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp", "patch_mp"]);
        assert_eq!(zone_list(Some("a,b".into())), ["a", "b"]);
    }

    #[test]
    fn only_multiplayer_guns_are_listed_and_names_pick_exactly() {
        let ak = ["ak47_mp".to_owned()];
        assert!(is_listed(&[], "an94_mp"));
        assert!(!is_listed(&[], "an94_zm"));
        assert!(is_listed(&ak, "ak47_mp"));
        assert!(!is_listed(&ak, "an94_mp"));
    }

    #[test]
    fn uniques_not_named_au_are_the_odd_ones() {
        let all = [unique("au_ak47_none"), unique("weird"), unique("au_x_ir")];
        assert_eq!(odd_names(&all), ["weird"]);
    }

    #[test]
    fn a_location_diff_shows_gun_then_unique_and_reads_nan_past_the_gun_table() {
        assert_eq!(location_diff(&[1.0, 2.0, 3.0], &[1.0, 1.5]), ["1:1.5->2", "2:NaN->3"]);
        assert!(location_diff(&[1.0], &[1.0]).is_empty());
    }

    #[test]
    fn an_empty_unique_overrides_nothing() {
        assert_eq!(override_values(&unique("au_x_none")), [0; 6]);
    }
}
