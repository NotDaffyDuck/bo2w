//! bo2mp: a dual-wield weapon's per-hand timing, read-only.
//! `t6dw [weapon]...` prints fire, reload, raise/drop and melee times, clip
//! size, the left-hand weapon's name and the per-hand animation slots (with
//! their lengths and notetracks) from the zones in `T6ICON_ZONES` (default
//! common_mp,common_patch_mp). With no names, lists every dual-wield weapon.

use std::path::Path;

use asset_t6::{WeaponRef, XAnimRef, ZoneCapture, capture_zone};
use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};

/// Animation slots worth printing for a two-gun viewmodel: right fire,
/// lastshot, reload, reload-empty, raise/drop family, then the left-hand set.
/// Then the sprint in/loop/out family (full and empty) and the sprint camera.
const SLOTS: [usize; 24] = [
    0x01, 0x02, 0x04, 0x06, 0x10, 0x11, 0x12, 0x17, 0x19, 0x1C, 0x1D, 0x4E, 0x4F, 0x51, 0x52, 0x53, 0x54,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x49,
];

/// The zones to read: the comma list in `T6ICON_ZONES`, else the two common ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    match setting {
        Some(list) => list.split(',').map(str::to_owned).collect(),
        None => vec!["common_mp".to_owned(), "common_patch_mp".to_owned()],
    }
}

/// The left-hand weapon's name, empty when the def names none.
fn left_hand(w: &WeaponRef) -> &str {
    w.strings.iter().find(|(o, _)| *o == WeaponDef::szDualWieldWeaponName).map_or("", |(_, n)| n.as_str())
}

/// Whether a weapon is listed: by name when names were given, else every dual-wield one.
fn is_hit(names: &[String], w: &WeaponRef) -> bool {
    let dual = w.def_u8(WeaponDef::bDualWield) != 0 || !left_hand(w).is_empty();
    names.first().map_or(dual, |_| names.iter().any(|n| n == &w.name))
}

/// The animation slots to print: all of them, or the dual-wield set.
fn slot_list(all: bool, slot_total: usize) -> Vec<usize> {
    if all { (0..slot_total).collect() } else { SLOTS.to_vec() }
}

fn find_anim<'a>(anims: &'a [XAnimRef], name: &str) -> Option<&'a XAnimRef> {
    anims.iter().find(|a| a.name.eq_ignore_ascii_case(name))
}

fn anim_ms(frames: u16, fps: f32) -> f64 {
    f64::from(frames) / f64::from(fps) * 1000.0
}

/// `name@time` for each notetrack, the 0..1 time scaled to the animation's length.
fn note_list(notifies: &[(String, f32)], ms: f64) -> String {
    let notes: Vec<String> = notifies.iter().map(|(k, t)| format!("{k}@{:.0}ms", f64::from(*t) * ms)).collect();
    notes.join(",")
}

/// What follows an animation's name: its length and notes, or that the zone lacks it.
fn anim_note(anims: &[XAnimRef], wanted: &str) -> String {
    find_anim(anims, wanted).map_or_else(
        || " [anim not in this zone]".to_owned(),
        |a| {
            let ms = anim_ms(a.frame_total, a.frame_rate_hz);
            format!(" [{ms:.0} ms; notes {}]", note_list(&a.notifies, ms))
        },
    )
}

fn print_weapon(zone: &str, capture: &ZoneCapture, w: &WeaponRef) {
    let left = left_hand(w);
    println!(
        "{zone} {}: dualWield {} left='{}' ads {} fireType {} clip {}",
        w.name,
        w.def_u8(WeaponDef::bDualWield),
        left,
        w.def_u8(WeaponDef::aimDownSight),
        w.def_i32(WeaponDef::fireType),
        w.var_i32(WeaponVariantDef::iClipSize),
    );
    println!(
        "    fire {} lastFire {} fireDelay {} introFire {}/{} rechamber {} melee {} meleeDelay {}",
        w.def_i32(WeaponDef::iFireTime),
        w.def_i32(WeaponDef::iLastFireTime),
        w.def_i32(WeaponDef::iFireDelay),
        w.def_i32(WeaponDef::iIntroFireTime),
        w.def_i32(WeaponDef::iIntroFireLength),
        w.def_i32(WeaponDef::iRechamberTime),
        w.def_i32(WeaponDef::iMeleeTime),
        w.def_i32(WeaponDef::iMeleeDelay),
    );
    println!(
        "    boltAction {} shotsBeforeRechamber {} rechamberBolt {} segmentedReload {}",
        w.def_u8(WeaponDef::bBoltAction),
        w.def_i32(WeaponDef::iShotsBeforeRechamber),
        w.def_i32(WeaponDef::iRechamberBoltTime),
        w.def_u8(WeaponDef::bSegmentedReload),
    );
    println!(
        "    ammo='{}' clipName='{}' startAmmo {} maxAmmo {} sharedAmmo {} unlimitedAmmo {}",
        w.ammo_name,
        w.collision_label,
        w.def_i32(WeaponDef::iStartAmmo),
        w.def_i32(WeaponDef::iMaxAmmo),
        w.def_u8(WeaponDef::sharedAmmo),
        w.def_u8(WeaponDef::unlimitedAmmo),
    );
    println!(
        "    reload {} reloadEmpty {} add {} emptyAdd {} | def reloadRight {} reloadLeft {} reloadEmptyLeft {}",
        w.var_i32(WeaponVariantDef::iReloadTime),
        w.var_i32(WeaponVariantDef::iReloadEmptyTime),
        w.def_i32(WeaponDef::iReloadAddTime),
        w.def_i32(WeaponDef::iReloadEmptyAddTime),
        w.def_i32(WeaponDef::iReloadTimeRight),
        w.def_i32(WeaponDef::iReloadTimeLeft),
        w.def_i32(WeaponDef::iReloadEmptyTimeLeft),
    );
    println!(
        "    drop {} raise {} quickDrop {} quickRaise {} firstRaise {} emptyRaise {} emptyDrop {}",
        w.def_i32(WeaponDef::iDropTime),
        w.def_i32(WeaponDef::iRaiseTime),
        w.def_i32(WeaponDef::quickDropTime),
        w.def_i32(WeaponDef::quickRaiseTime),
        w.def_i32(WeaponDef::iFirstRaiseTime),
        w.def_i32(WeaponDef::iEmptyRaiseTime),
        w.def_i32(WeaponDef::iEmptyDropTime),
    );
    let v3 = |o: usize| [w.def_f32(o), w.def_f32(o + 4), w.def_f32(o + 8)];
    println!(
        "    sprintIn/Loop/Out {}/{}/{} moveSpeedScale {}",
        w.def_i32(WeaponDef::sprintInTime),
        w.def_i32(WeaponDef::sprintLoopTime),
        w.def_i32(WeaponDef::sprintOutTime),
        w.def_f32(WeaponDef::moveSpeedScale),
    );
    println!(
        "    sprintOfs {:?} sprintRot {:?} sprintBob {:?} cycle {} | standMove {:?} standRot {:?}",
        v3(WeaponDef::vSprintOfs),
        v3(WeaponDef::vSprintRot),
        [w.def_f32(WeaponDef::vSprintBob), w.def_f32(WeaponDef::vSprintBob + 4)],
        w.def_f32(WeaponDef::fSprintCycleScale),
        v3(WeaponDef::vStandMove),
        v3(WeaponDef::vStandRot),
    );
    // T6DW_SOUNDS=1: the sounds the fire, rechamber and reload events play.
    if std::env::var_os("T6DW_SOUNDS").is_some() {
        println!(
            "    sounds fire '{}' firePlayer '{}' rechamber '{}' rechamberPlayer '{}' reload '{}' reloadPlayer '{}'",
            w.def_string(WeaponDef::fireSound),
            w.def_string(WeaponDef::fireSoundPlayer),
            w.def_string(WeaponDef::rechamberSound),
            w.def_string(WeaponDef::rechamberSoundPlayer),
            w.def_string(WeaponDef::reloadSound),
            w.def_string(WeaponDef::reloadSoundPlayer),
        );
    }
    // T6DW_ALL=1: every slot, not just the dual-wield set.
    for slot in slot_list(std::env::var_os("T6DW_ALL").is_some(), w.animation_refs.len()) {
        let Some(n) = w.animation_refs.get(slot).filter(|n| !n.is_empty()) else { continue };
        println!("    xanim 0x{slot:02X} = {n}{}", anim_note(&capture.animation_refs, n));
        // T6DW_TRACKS=1: each animation's part-type counts and track names.
        if std::env::var_os("T6DW_TRACKS").is_some()
            && let Some(a) = find_anim(&capture.animation_refs, n)
        {
            println!("        boneCount {:?} delta {} type {}", a.joint_total, a.delta, a.asset_type);
            println!("        tracks {}", a.names.join(","));
        }
    }
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
        // T6DW_LIST=1: just the weapon names in each zone.
        if std::env::var_os("T6DW_LIST").is_some() {
            let listed: Vec<&str> = capture.weapons.iter().map(|w| w.name.as_str()).collect();
            println!("{zone}: {list}", list = listed.join(" "));
            continue;
        }
        capture
            .weapons
            .iter()
            .filter(|w| is_hit(&names, w))
            .for_each(|w| print_weapon(&zone, &capture, w));
    }
}

#[cfg(test)]
mod tests {
    use super::{zone_list, slot_list, SLOTS, anim_ms, note_list};

    #[test]
    fn zones_come_from_the_setting_or_the_two_common_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp"]);
        assert_eq!(zone_list(Some("x".into())), ["x"]);
    }

    #[test]
    fn slots_are_the_dual_wield_set_unless_all_are_asked_for() {
        assert_eq!(slot_list(false, 100), SLOTS.to_vec());
        assert_eq!(slot_list(true, 3), [0, 1, 2]);
    }

    #[test]
    fn notes_sit_at_their_fraction_of_the_length() {
        let ms = anim_ms(15, 30.0);
        assert_eq!(ms, 500.0);
        assert_eq!(note_list(&[("fire".into(), 0.5)], ms), "fire@250ms");
    }
}
