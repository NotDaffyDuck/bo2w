//! bo2mp: print a T6 weapon def's projectile and guidance values.
//! `t6wdef <zone.ff> <weapon name>...`
use fastfile_t6::layout::WeaponDef as d;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6wdef <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    for w in c.weapons.iter().filter(|w| names.is_empty() || names.contains(&w.name)) {
        println!("{}", w.name);
        println!("  guidedMissileType {}", w.def_i32(d::guidedMissileType));
        println!("  weapClass {} inventoryType {} offhandSlot {} carriedKillstreak {}", w.def_i32(d::weapClass), w.def_i32(d::inventoryType), w.def_i32(d::offhandSlot), w.def_u8(d::bIsCarriedKillstreakWeapon));
        println!("  maxSteeringAccel {}", w.def_f32(d::maxSteeringAccel));
        println!("  projIgnitionDelay {}", w.def_i32(d::projIgnitionDelay));
        println!("  iProjectileSpeed {}", w.def_i32(d::iProjectileSpeed));
        println!("  iProjectileSpeedUp {}", w.def_i32(d::iProjectileSpeedUp));
        println!("  iProjectileSpeedForward {}", w.def_i32(d::iProjectileSpeedForward));
        println!("  projLifetime {}", w.def_f32(d::projLifetime));
        println!("  timeToAccelerate {}", w.def_f32(d::timeToAccelerate));
        // G_FireRocket's other launch rules.
        println!(
            "  weapType {} impactType {} projectileCurvature {} airburst {} destabilizeDistance {} destabilizationRateTime {} destabilizationCurvatureMax {}",
            w.def_i32(d::weapType),
            w.def_i32(d::impactType),
            w.def_f32(d::projectileCurvature),
            w.def_u8(d::bAirburstWeapon),
            w.def_i32(d::destabilizeDistance),
            w.def_f32(d::destabilizationRateTime),
            w.def_f32(d::destabilizationCurvatureMax),
        );
        // A turret's turn limits and fire time (G_SpawnTurret's arcs), and
        // its idle sweep.
        println!(
            "  leftArc {} rightArc {} topArc {} bottomArc {} iFireTime {} scanSpeed {} scanAccel {} scanPauseTime {}",
            w.def_f32(d::leftArc),
            w.def_f32(d::rightArc),
            w.def_f32(d::topArc),
            w.def_f32(d::bottomArc),
            w.def_i32(d::iFireTime),
            w.def_f32(d::scanSpeed),
            w.def_f32(d::scanAccel),
            w.def_i32(d::scanPauseTime),
        );
        // What it does when it lands: sticks, goes off, and its bounce sounds.
        println!(
            "  stickiness {} projExplosion {} projImpactExplode {} bounce sounds {:?}",
            w.def_i32(d::stickiness),
            w.def_i32(d::projExplosion),
            w.def_u8(d::bProjImpactExplode),
            w.bounce_sounds.iter().filter(|s| !s.is_empty()).take(4).collect::<Vec<_>>()
        );
        for (off, s) in &w.strings {
            println!("  string @{off} {s}");
        }
        for (off, m) in &w.models {
            println!("  model @{off} {m}");
        }
        for (off, f) in &w.fx {
            println!("  fx @{off} {f}");
        }
    }
}
