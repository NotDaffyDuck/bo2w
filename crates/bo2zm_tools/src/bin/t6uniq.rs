//! bo2mp: each Black Ops II multiplayer gun's attachment-unique overrides, read-only.
//! `t6uniq [weapon...]` prints, per gun, the def/variant first raise, alt raise and drop,
//! reload ammo add, start add and segmented flag, then every `au_<gun>_<attachment>` whose
//! value for one of them is not 0. BO2's getters (BG_GetFirstRaiseTime etc.) take the first
//! attachment unique that differs from the gun's base one, else the def's. Zones as t6scope
//! (`T6UNIQ_ZONES` to change them). Also lists the uniques not named `au_...`.
//! `T6UNIQ_LOC=1` instead prints each `au_` whose hit-location table differs from its gun's
//! base unique `au_<gun>_none` (BG_GetWeaponHitLocationMultiplier: only those change a hit).

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{PackSet, capture_zone};
use fastfile_t6::layout::{WeaponAttachmentUnique as u, WeaponDef as d, WeaponVariantDef as v};

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let dir = Path::new(ZONES);
    if let Err(e) = PackSet::open_dir(dir) {
        eprintln!("packs: {e}");
        return ExitCode::FAILURE;
    }
    let zones: Vec<String> = std::env::var("T6UNIQ_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let loc_only = std::env::var_os("T6UNIQ_LOC").is_some();
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        let odd: Vec<&str> = capture
            .attachment_uniques
            .iter()
            .map(|a| a.name.as_str())
            .filter(|n| !n.starts_with("au_"))
            .collect();
        println!("{zone}: {} uniques, not au_: {:?}", capture.attachment_uniques.len(), odd);
        for w in capture
            .weapons
            .iter()
            .filter(|w| w.name.ends_with("_mp") && (names.is_empty() || names.contains(&w.name)))
        {
            let base = w.name.trim_end_matches("_mp");
            let prefix = format!("au_{base}_");
            if loc_only {
                for au in capture
                    .attachment_uniques
                    .iter()
                    .filter(|a| a.name.starts_with(&prefix))
                {
                    let Some(t) = &au.location_damage else {
                        continue;
                    };
                    let none = format!("{prefix}none");
                    let gun = capture
                        .attachment_uniques
                        .iter()
                        .find(|a| a.name == none)
                        .and_then(|a| a.location_damage.as_deref())
                        .unwrap_or(&[]);
                    let diff: Vec<String> = t
                        .iter()
                        .enumerate()
                        .filter(|&(i, x)| gun.get(i) != Some(x))
                        .map(|(i, x)| {
                            format!("{i}:{}->{x}", gun.get(i).copied().unwrap_or(f32::NAN))
                        })
                        .collect();
                    if !diff.is_empty() {
                        println!("{zone} {} {}: {}", w.name, au.name, diff.join(" "));
                    }
                }
                continue;
            }
            let mut lines = Vec::new();
            for au in capture.attachment_uniques.iter().filter(|a| a.name.starts_with(&prefix)) {
                let vals = [
                    au.i32_at(u::iFirstRaiseTime),
                    au.i32_at(u::iAltRaiseTime),
                    au.i32_at(u::iAltDropTime),
                    au.i32_at(u::iReloadAmmoAdd),
                    au.i32_at(u::iReloadStartAdd),
                    au.bytes.get(u::bSegmentedReload).copied().unwrap_or(0) as i32,
                ];
                if vals.iter().any(|&x| x != 0) {
                    lines.push(format!("    {} {:?}", au.name, vals));
                }
            }
            println!(
                "{zone} {} first {} altRaise {} altDrop {} ammoAdd {} startAdd {} segmented {}",
                w.name,
                w.def_i32(d::iFirstRaiseTime),
                w.var_i32(v::iAltRaiseTime),
                w.def_i32(d::iAltDropTime),
                w.def_i32(d::iReloadAmmoAdd),
                w.def_i32(d::iReloadStartAdd),
                w.def_u8(d::bSegmentedReload),
            );
            for l in lines {
                println!("{l}");
            }
        }
    }
    ExitCode::SUCCESS
}
