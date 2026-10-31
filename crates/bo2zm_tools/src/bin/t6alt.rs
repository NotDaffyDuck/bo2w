//! bo2mp: a weapon's switch timing and animation names, read-only.
//! `t6alt <weapon>...` prints each weapon's raise/drop/alt-raise/alt-drop
//! times and the ADS and alt animation slots from the zones in `T6ICON_ZONES`
//! (default common_mp,common_patch_mp). With no names, lists every weapon
//! whose name contains `dualoptic` or has an alt weapon with that tag.

use std::path::Path;

use asset_t6::{WeaponRef, XAnimRef, ZoneCapture, capture_zone};
use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};

/// Animation slots printed under each weapon: ADS and alt animations.
const ANIM_SLOTS: [usize; 8] = [0x17, 0x19, 0x1A, 0x1B, 0x55, 0x56, 0x57, 0x58];

/// The zones to read: the comma list in `T6ICON_ZONES`, else the two common ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    match setting {
        Some(list) => list.split(',').map(str::to_owned).collect(),
        None => vec!["common_mp".to_owned(), "common_patch_mp".to_owned()],
    }
}

/// Whether a weapon is listed: by name when names were given, else the dual-optic and HAMR guns.
fn is_hit(names: &[String], weapon: &str) -> bool {
    let by_default = weapon.contains("dualoptic") || weapon.starts_with("hamr");
    names.first().map_or(by_default, |_| names.iter().any(|n| n == weapon))
}

fn anim_ms(frames: u16, fps: f32) -> f64 {
    f64::from(frames) / f64::from(fps) * 1000.0
}

/// `name@time` for each notetrack, the 0..1 time scaled to the animation's length.
fn note_list(notifies: &[(String, f32)], ms: f64) -> String {
    let notes: Vec<String> = notifies.iter().map(|(k, t)| format!("{k}@{:.0}ms", f64::from(*t) * ms)).collect();
    notes.join(",")
}

fn anim_suffix(a: &XAnimRef) -> String {
    let ms = anim_ms(a.frame_total, a.frame_rate_hz);
    format!(" [{} frames @ {} fps = {ms:.0} ms; notes {}]", a.frame_total, a.frame_rate_hz, note_list(&a.notifies, ms))
}

/// What follows an animation's name: its length and notes, or that the zone lacks it.
fn anim_note(anims: &[XAnimRef], wanted: &str) -> String {
    anims
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(wanted))
        .map_or_else(|| " [anim not in this zone]".to_owned(), anim_suffix)
}

fn print_weapon(zone: &str, capture: &ZoneCapture, w: &WeaponRef) {
    println!(
        "{zone} {}: alt='{}' variant[altRaise {} adsIn {} adsOut {}] def[drop {} raise {} altDrop {} firstRaise {} emptyRaise {} quickDrop {} quickRaise {} fire {}] type {} class {} offhandSlot {} inVehicle {} noDropsOrRaises {}",
        w.name,
        w.alt_weapon_name,
        w.var_i32(WeaponVariantDef::iAltRaiseTime),
        w.var_i32(WeaponVariantDef::iAdsTransInTime),
        w.var_i32(WeaponVariantDef::iAdsTransOutTime),
        w.def_i32(WeaponDef::iDropTime),
        w.def_i32(WeaponDef::iRaiseTime),
        w.def_i32(WeaponDef::iAltDropTime),
        w.def_i32(WeaponDef::iFirstRaiseTime),
        w.def_i32(WeaponDef::iEmptyRaiseTime),
        w.def_i32(WeaponDef::quickDropTime),
        w.def_i32(WeaponDef::quickRaiseTime),
        w.def_i32(WeaponDef::iFireTime),
        w.def_i32(WeaponDef::weapType),
        w.def_i32(WeaponDef::weapClass),
        w.def_i32(WeaponDef::offhandSlot),
        w.def_u8(WeaponDef::bCanUseInVehicle),
        w.def_u8(WeaponDef::bNoDropsOrRaises),
    );
    println!("    ammo='{}' clip='{}' clipSize {} startAmmo {} maxAmmo {} sharedAmmoCap {}",
        w.ammo_name, w.collision_label, w.var_i32(WeaponVariantDef::iClipSize), w.def_i32(WeaponDef::iStartAmmo), w.def_i32(WeaponDef::iMaxAmmo), w.def_i32(WeaponDef::iSharedAmmoCap));
    ANIM_SLOTS
        .iter()
        .filter_map(|&slot| w.animation_refs.get(slot).filter(|n| !n.is_empty()).map(|n| (slot, n)))
        .for_each(|(slot, n)| println!("    xanim 0x{slot:02X} = {n}{}", anim_note(&capture.animation_refs, n)));
}

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    for zone in zone_list(std::env::var("T6ICON_ZONES").ok()) {
        let capture = match capture_zone(&bo2zm_tools::zones_dir().join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        capture
            .weapons
            .iter()
            .filter(|w| is_hit(&names, &w.name))
            .for_each(|w| print_weapon(&zone, &capture, w));
    }
}

#[cfg(test)]
mod tests {
    use super::{zone_list, is_hit, anim_ms, note_list};

    #[test]
    fn zones_come_from_the_setting_or_the_two_common_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp"]);
        assert_eq!(zone_list(Some("a,b".into())), ["a", "b"]);
    }

    #[test]
    fn no_names_lists_dual_optic_and_hamr_guns() {
        assert!(is_hit(&[], "dualoptic_acr_mp"));
        assert!(is_hit(&[], "hamr_mp"));
        assert!(!is_hit(&[], "an94_mp"));
        assert!(is_hit(&["an94_mp".to_owned()], "an94_mp"));
        assert!(!is_hit(&["an94_mp".to_owned()], "hamr_mp"));
    }

    #[test]
    fn notetrack_times_scale_to_the_animation_length() {
        let ms = anim_ms(30, 30.0);
        assert_eq!(ms, 1000.0);
        assert_eq!(note_list(&[("a".into(), 0.5), ("b".into(), 1.0)], ms), "a@500ms,b@1000ms");
        assert_eq!(note_list(&[], ms), "");
    }
}
