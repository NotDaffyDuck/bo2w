//! bo2mp: a weapon's movement-state gun times, read-only (spec aid for
//! docs/gun/SPRINT.md). `t6sprint <weapon>...` prints sprint, dive, slide,
//! crawl and low-ready times plus the movement animation lengths from the
//! zones in `T6ICON_ZONES` (default common_mp,common_patch_mp).
//! With `T6SPRINT_DUMP=<file>` it also writes the weapons as a gun_trace facts
//! dump (`name<TAB>variant hex<TAB>def hex`, later zones win), in argument order.

use std::path::Path;

use asset_t6::{WeaponRef, XAnimRef, ZoneCapture, capture_zone};
use fastfile_t6::layout::WeaponDef;

/// Animation file slots: sprint in/loop/out (+empty), crawl in..out, dive
/// in/loop/out (+empty), slide in, mantle, camera sprint/dive/mantle.
const SLOTS: [(usize, &str); 23] = [
    (32, "sprint_in"),
    (33, "sprint_loop"),
    (34, "sprint_out"),
    (35, "sprint_empty_in"),
    (37, "sprint_empty_out"),
    (38, "lowready_in"),
    (44, "crawl_in"),
    (45, "crawl_forward"),
    (46, "crawl_back"),
    (47, "crawl_right"),
    (48, "crawl_left"),
    (49, "crawl_out"),
    (65, "dtp_in"),
    (66, "dtp_loop"),
    (67, "dtp_out"),
    (68, "dtp_empty_in"),
    (70, "dtp_empty_out"),
    (71, "slide_in"),
    (72, "mantle"),
    (73, "camera_sprint_loop"),
    (74, "camera_dtp_in"),
    (76, "camera_dtp_out"),
    (77, "camera_mantle"),
];

/// The zones to read: the comma list in `T6ICON_ZONES`, else the two common ones.
fn zone_list(setting: Option<String>) -> Vec<String> {
    setting.map_or_else(
        || vec!["common_mp".to_owned(), "common_patch_mp".to_owned()],
        |list| list.split(',').map(str::to_owned).collect(),
    )
}

fn hex(raw: &[u8]) -> String {
    raw.iter().map(|x| format!("{x:02x}")).collect()
}

/// The facts dump: one `name<TAB>variant hex<TAB>def hex` line per weapon, in
/// argument order; the first weapon never found in any zone is the error.
fn dump_text(names: &[String], blobs: &[Option<(Vec<u8>, Vec<u8>)>]) -> Result<String, String> {
    names
        .iter()
        .zip(blobs)
        .map(|(label, blob)| match blob {
            Some((v, d)) => Ok(format!("{label}\t{}\t{}\n", hex(v), hex(d))),
            None => Err(format!("{label}: not in the zones")),
        })
        .collect()
}

/// What follows an animation's name: its length, or nothing when the zone lacks it.
fn length_note(anims: &[XAnimRef], wanted: &str) -> String {
    anims
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(wanted))
        .map(|a| format!(" [{:.0} ms]", f64::from(a.frame_total) / f64::from(a.frame_rate_hz) * 1000.0))
        .unwrap_or_default()
}

fn print_weapon(zone: &str, capture: &ZoneCapture, w: &WeaponRef) {
    println!(
        "{zone} {}: sprint[in {} loop {} out {}] lowready[in {} loop {} out {}] dtp[in {} loop {} out {}] crawl[in {} fwd {} back {} right {} left {} outFire {} out {}] slideIn {} sprintDurationScale {} blocksProne {} sprintCycleScale {} moveSpeedScale {}",
        w.name,
        w.def_i32(WeaponDef::sprintInTime),
        w.def_i32(WeaponDef::sprintLoopTime),
        w.def_i32(WeaponDef::sprintOutTime),
        w.def_i32(WeaponDef::lowReadyInTime),
        w.def_i32(WeaponDef::lowReadyLoopTime),
        w.def_i32(WeaponDef::lowReadyOutTime),
        w.def_i32(WeaponDef::dtpInTime),
        w.def_i32(WeaponDef::dtpLoopTime),
        w.def_i32(WeaponDef::dtpOutTime),
        w.def_i32(WeaponDef::crawlInTime),
        w.def_i32(WeaponDef::crawlForwardTime),
        w.def_i32(WeaponDef::crawlBackTime),
        w.def_i32(WeaponDef::crawlRightTime),
        w.def_i32(WeaponDef::crawlLeftTime),
        w.def_i32(WeaponDef::crawlOutFireTime),
        w.def_i32(WeaponDef::crawlOutTime),
        w.def_i32(WeaponDef::slideInTime),
        w.def_f32(WeaponDef::sprintDurationScale),
        w.def_u8(WeaponDef::blocksProne),
        w.def_f32(WeaponDef::fSprintCycleScale),
        w.def_f32(WeaponDef::moveSpeedScale),
    );
    SLOTS
        .iter()
        .filter_map(|&(slot, label)| w.animation_refs.get(slot).filter(|n| !n.is_empty()).map(|n| (label, n)))
        .for_each(|(label, n)| println!("    {label} = {n}{}", length_note(&capture.animation_refs, n)));
}

fn run(names: &[String]) -> Result<(), String> {
    let mut blobs: Vec<Option<(Vec<u8>, Vec<u8>)>> = vec![None; names.len()];
    for zone in zone_list(std::env::var("T6ICON_ZONES").ok()) {
        let capture = match capture_zone(&bo2zm_tools::zones_dir().join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for w in &capture.weapons {
            let Some(i) = names.iter().position(|n| n == &w.name) else { continue };
            blobs[i] = Some((w.variant.clone(), w.def.clone()));
            print_weapon(&zone, &capture, w);
        }
    }
    let Ok(out_path) = std::env::var("T6SPRINT_DUMP") else { return Ok(()) };
    let body = dump_text(names, &blobs)?;
    std::fs::write(&out_path, body).map_err(|e| format!("{out_path}: {e}"))
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
    use super::{zone_list, hex, dump_text};

    #[test]
    fn zones_come_from_the_setting_or_the_two_common_ones() {
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp"]);
        assert_eq!(zone_list(Some("z".into())), ["z"]);
    }

    #[test]
    fn bytes_print_as_two_lower_case_hex_digits() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
    }

    #[test]
    fn the_dump_is_tab_separated_in_argument_order() {
        let names = ["a".to_owned(), "b".to_owned()];
        let blobs = [Some((vec![1, 2], vec![3])), Some((vec![], vec![0xab]))];
        assert_eq!(dump_text(&names, &blobs).unwrap(), "a\t0102\t03\nb\t\tab\n");
    }

    #[test]
    fn a_weapon_found_in_no_zone_fails_the_dump_by_name() {
        let names = ["a".to_owned(), "b".to_owned()];
        let blobs = [Some((vec![], vec![])), None];
        assert_eq!(dump_text(&names, &blobs), Err("b: not in the zones".to_owned()));
    }
}
