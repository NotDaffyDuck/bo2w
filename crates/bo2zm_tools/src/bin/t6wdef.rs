//! bo2mp: print a T6 weapon def's projectile and guidance values.
//! `t6wdef <zone.ff> <weapon name>...`
use std::fmt::Display;
use std::path::PathBuf;

use fastfile_t6::layout::WeaponDef;

/// The zone (first word) and the weapon names asked for (the rest; none means all).
fn split_command(mut words: impl Iterator<Item = String>) -> Option<(PathBuf, Vec<String>)> {
    let zone = PathBuf::from(words.next()?);
    Some((zone, words.collect()))
}

fn is_wanted(asked: &[String], wanted: &str) -> bool {
    asked.is_empty() || asked.iter().any(|a| a == wanted)
}

/// `  <kind> @<offset> <value>` for each (offset, value) pair the def points at.
fn pointer_rows(kind: &str, pairs: &[(impl Display, impl Display)]) -> Vec<String> {
    pairs.iter().map(|(off, v)| format!("  {kind} @{off} {v}")).collect()
}

/// The first four bounce sounds that are set.
fn bounces(sounds: &[String]) -> Vec<&String> {
    sounds.iter().filter(|s| !s.is_empty()).take(4).collect()
}

fn main() {
    let Some((zone, asked)) = split_command(std::env::args().skip(1)) else {
        eprintln!("usage: t6wdef <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(&zone).expect("capture");
    for w in c.weapons.iter().filter(|w| is_wanted(&asked, &w.name)) {
        println!("{}", w.name);
        println!("  guidedMissileType {}", w.def_i32(WeaponDef::guidedMissileType));
        println!("  weapClass {} inventoryType {} offhandSlot {} carriedKillstreak {}", w.def_i32(WeaponDef::weapClass), w.def_i32(WeaponDef::inventoryType), w.def_i32(WeaponDef::offhandSlot), w.def_u8(WeaponDef::bIsCarriedKillstreakWeapon));
        println!("  maxSteeringAccel {}", w.def_f32(WeaponDef::maxSteeringAccel));
        println!("  projIgnitionDelay {}", w.def_i32(WeaponDef::projIgnitionDelay));
        println!("  iProjectileSpeed {}", w.def_i32(WeaponDef::iProjectileSpeed));
        println!("  iProjectileSpeedUp {}", w.def_i32(WeaponDef::iProjectileSpeedUp));
        println!("  iProjectileSpeedForward {}", w.def_i32(WeaponDef::iProjectileSpeedForward));
        println!("  projLifetime {}", w.def_f32(WeaponDef::projLifetime));
        println!("  timeToAccelerate {}", w.def_f32(WeaponDef::timeToAccelerate));
        // G_FireRocket's other launch rules.
        println!(
            "  weapType {} impactType {} projectileCurvature {} airburst {} destabilizeDistance {} destabilizationRateTime {} destabilizationCurvatureMax {}",
            w.def_i32(WeaponDef::weapType),
            w.def_i32(WeaponDef::impactType),
            w.def_f32(WeaponDef::projectileCurvature),
            w.def_u8(WeaponDef::bAirburstWeapon),
            w.def_i32(WeaponDef::destabilizeDistance),
            w.def_f32(WeaponDef::destabilizationRateTime),
            w.def_f32(WeaponDef::destabilizationCurvatureMax),
        );
        // A turret's turn limits and fire time (G_SpawnTurret's arcs), and
        // its idle sweep.
        println!(
            "  leftArc {} rightArc {} topArc {} bottomArc {} iFireTime {} scanSpeed {} scanAccel {} scanPauseTime {}",
            w.def_f32(WeaponDef::leftArc),
            w.def_f32(WeaponDef::rightArc),
            w.def_f32(WeaponDef::topArc),
            w.def_f32(WeaponDef::bottomArc),
            w.def_i32(WeaponDef::iFireTime),
            w.def_f32(WeaponDef::scanSpeed),
            w.def_f32(WeaponDef::scanAccel),
            w.def_i32(WeaponDef::scanPauseTime),
        );
        // What it does when it lands: sticks, goes off, and its bounce sounds.
        println!(
            "  stickiness {} projExplosion {} projImpactExplode {} bounce sounds {:?}",
            w.def_i32(WeaponDef::stickiness),
            w.def_i32(WeaponDef::projExplosion),
            w.def_u8(WeaponDef::bProjImpactExplode),
            bounces(&w.bounce_sounds)
        );
        let pointers = [("string", &w.strings), ("model", &w.models), ("fx", &w.fx)];
        pointers.iter().flat_map(|(kind, pairs)| pointer_rows(kind, pairs)).for_each(|row| println!("{row}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{split_command, is_wanted, pointer_rows, bounces};

    #[test]
    fn the_zone_comes_first_and_names_follow() {
        let (zone, names) = split_command(["z.ff", "w1"].map(str::to_owned).into_iter()).unwrap();
        assert_eq!(zone.to_str(), Some("z.ff"));
        assert_eq!(names, ["w1"]);
        assert!(split_command(std::iter::empty()).is_none());
        assert!(is_wanted(&[], "x") && !is_wanted(&names, "x"));
    }

    #[test]
    fn pointer_rows_print_offset_then_value() {
        let pairs = [(940usize, "a".to_owned()), (956, "b".to_owned())];
        assert_eq!(pointer_rows("model", &pairs), ["  model @940 a", "  model @956 b"]);
        assert!(pointer_rows("fx", &Vec::<(usize, String)>::new()).is_empty());
    }

    #[test]
    fn only_four_set_bounce_sounds_are_shown() {
        let sounds: Vec<String> = ["", "a", "", "b", "c", "d", "e"].map(str::to_owned).to_vec();
        assert_eq!(bounces(&sounds), ["a", "b", "c", "d"]);
    }
}
