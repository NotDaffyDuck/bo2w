//! bo2mp: print a T6 weapon def's fire, loop and spin sound aliases.
//! `t6wsnd <zone.ff> <weapon name>...`
use std::fmt::Debug;
use std::path::PathBuf;

use fastfile_t6::layout::WeaponDef;

/// The sound-alias fields printed first, by name and def offset.
const SOUND_FIELDS: [(&str, usize); 14] = [
    ("fireSound", WeaponDef::fireSound),
    ("fireSoundPlayer", WeaponDef::fireSoundPlayer),
    ("fireLoopSound", WeaponDef::fireLoopSound),
    ("fireLoopSoundPlayer", WeaponDef::fireLoopSoundPlayer),
    ("fireStopSound", WeaponDef::fireStopSound),
    ("fireStopSoundPlayer", WeaponDef::fireStopSoundPlayer),
    ("fireLastSound", WeaponDef::fireLastSound),
    ("fireLastSoundPlayer", WeaponDef::fireLastSoundPlayer),
    ("spinLoopSound", WeaponDef::spinLoopSound),
    ("spinLoopSoundPlayer", WeaponDef::spinLoopSoundPlayer),
    ("startSpinSound", WeaponDef::startSpinSound),
    ("startSpinSoundPlayer", WeaponDef::startSpinSoundPlayer),
    ("stopSpinSound", WeaponDef::stopSpinSound),
    ("stopSpinSoundPlayer", WeaponDef::stopSpinSoundPlayer),
];

/// The zone (first word) and the weapon names asked for (the rest; none means all).
fn split_command(mut words: impl Iterator<Item = String>) -> Option<(PathBuf, Vec<String>)> {
    let zone = PathBuf::from(words.next()?);
    Some((zone, words.collect()))
}

fn is_wanted(asked: &[String], wanted: &str) -> bool {
    asked.is_empty() || asked.iter().any(|a| a == wanted)
}

/// One `  label value` row, the value quoted the way `{:?}` shows it.
fn field_line(label: &str, value: impl Debug) -> String {
    format!("  {label} {value:?}")
}

fn main() {
    let Some((zone, asked)) = split_command(std::env::args().skip(1)) else {
        eprintln!("usage: t6wsnd <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(&zone).expect("capture");
    for w in c.weapons.iter().filter(|w| is_wanted(&asked, &w.name)) {
        println!("{}", w.name);
        SOUND_FIELDS.iter().for_each(|&(label, off)| println!("{}", field_line(label, w.def_string(off))));
        println!("  iSpinUpTime {} iSpinDownTime {} spinRate {}", w.def_i32(WeaponDef::iSpinUpTime), w.def_i32(WeaponDef::iSpinDownTime), w.def_f32(WeaponDef::spinRate));
        println!("  fireType {} fireTime {}", w.def_i32(WeaponDef::fireType), w.def_i32(WeaponDef::iFireTime));
        println!("  reticle center/side sizes {} {} minofs {} activeType {} hipSidePos {}", w.def_i32(WeaponDef::iReticleCenterSize), w.def_i32(WeaponDef::iReticleSideSize), w.def_i32(WeaponDef::iReticleMinOfs), w.def_i32(WeaponDef::activeReticleType), w.def_f32(WeaponDef::fHipReticleSidePos));
        for (off, m) in &w.materials {
            println!("  material @{off} {m}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{field_line, SOUND_FIELDS, split_command, is_wanted};

    #[test]
    fn a_sound_row_quotes_its_alias() {
        assert_eq!(field_line("fireSound", "wpn_fire"), "  fireSound \"wpn_fire\"");
        assert_eq!(field_line("fireSound", ""), "  fireSound \"\"");
    }

    #[test]
    fn the_table_keeps_the_player_alias_after_each_world_alias() {
        assert_eq!(SOUND_FIELDS.len(), 14);
        assert!(SOUND_FIELDS.chunks(2).all(|pair| pair[1].0 == format!("{}Player", pair[0].0)));
    }

    #[test]
    fn the_zone_comes_first_and_empty_names_want_every_weapon() {
        let (zone, names) = split_command(["z.ff"].map(str::to_owned).into_iter()).unwrap();
        assert_eq!(zone.to_str(), Some("z.ff"));
        assert!(names.is_empty() && is_wanted(&names, "any"));
        assert!(!is_wanted(&["a".to_owned()], "b"));
    }
}
