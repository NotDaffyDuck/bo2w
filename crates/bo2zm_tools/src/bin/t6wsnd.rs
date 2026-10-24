//! bo2mp: print a T6 weapon def's fire, loop and spin sound aliases.
//! `t6wsnd <zone.ff> <weapon name>...`
use fastfile_t6::layout::WeaponDef as d;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((zone, names)) = args.split_first() else {
        eprintln!("usage: t6wsnd <zone.ff> <name>...");
        return;
    };
    let c = asset_t6::capture_zone(std::path::Path::new(zone)).expect("capture");
    let fields = [
        ("fireSound", d::fireSound),
        ("fireSoundPlayer", d::fireSoundPlayer),
        ("fireLoopSound", d::fireLoopSound),
        ("fireLoopSoundPlayer", d::fireLoopSoundPlayer),
        ("fireStopSound", d::fireStopSound),
        ("fireStopSoundPlayer", d::fireStopSoundPlayer),
        ("fireLastSound", d::fireLastSound),
        ("fireLastSoundPlayer", d::fireLastSoundPlayer),
        ("spinLoopSound", d::spinLoopSound),
        ("spinLoopSoundPlayer", d::spinLoopSoundPlayer),
        ("startSpinSound", d::startSpinSound),
        ("startSpinSoundPlayer", d::startSpinSoundPlayer),
        ("stopSpinSound", d::stopSpinSound),
        ("stopSpinSoundPlayer", d::stopSpinSoundPlayer),
    ];
    for w in c.weapons.iter().filter(|w| names.is_empty() || names.contains(&w.name)) {
        println!("{}", w.name);
        for (label, off) in fields {
            println!("  {label} {:?}", w.def_string(off));
        }
        println!("  iSpinUpTime {} iSpinDownTime {} spinRate {}", w.def_i32(d::iSpinUpTime), w.def_i32(d::iSpinDownTime), w.def_f32(d::spinRate));
        println!("  fireType {} fireTime {}", w.def_i32(d::fireType), w.def_i32(d::iFireTime));
        println!("  reticle center/side sizes {} {} minofs {} activeType {} hipSidePos {}", w.def_i32(d::iReticleCenterSize), w.def_i32(d::iReticleSideSize), w.def_i32(d::iReticleMinOfs), w.def_i32(d::activeReticleType), w.def_f32(d::fHipReticleSidePos));
        for (off, m) in &w.materials {
            println!("  material @{off} {m}");
        }
    }
}
