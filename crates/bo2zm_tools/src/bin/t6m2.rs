//! bo2zm: check what the M2 capture reads from Nuketown's zones: weapons,
//! their models and animations, effects, sound banks and the bank files on
//! disk. Read-only; prints a report.
//!
//! `t6m2 [BO2 folder] [--weapons] [--fx] [--sound] [--decode]`

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const NUKETOWN_ZONES: [&str; 9] = [
    "code_pre_gfx_zm",
    "code_post_gfx_zm",
    "common_zm",
    "patch_zm",
    "ui_zm",
    "patch_ui_zm",
    "dlczm0_load_zm",
    "zm_nuked",
    "zm_nuked_patch",
];

/// Decode one sound entry; returns the tally slot (0 ok, 1 md5 mismatch, 2 failed) and its frames.
fn judge(path: &Path, e: &sab_t6::BankEntry, data: &[u8]) -> (usize, u64) {
    match e.format {
        sab_t6::SoundFormat::Flac => match sab_t6::decode_flac(data) {
            Ok(stream) => {
                let frames = stream.pcm.frames() as u64;
                if stream.md5_ok == Some(false) {
                    println!("  md5 mismatch: {:#x} in {}", e.id, path.to_string_lossy());
                    (1, frames)
                } else {
                    (0, frames)
                }
            }
            Err(err) => {
                println!("  flac fail {:#x} in {}: {err}", e.id, path.to_string_lossy());
                (2, 0)
            }
        },
        sab_t6::SoundFormat::Pcm16 => (0, u64::from(e.frame_count)),
        other => {
            println!("  format {other:?} {:#x}", e.id);
            (2, 0)
        }
    }
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort();
    items
}

/// A run that already said why on its own.
struct Failed;

/// A little-endian f32 at byte `at`, NaN when the bytes stop short.
fn f32_in(bytes: &[u8], at: usize) -> f32 {
    match bytes.get(at..at + 4) {
        Some(b) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        None => f32::NAN,
    }
}

/// `N` consecutive f32s starting at byte `at`.
fn f32_run<const N: usize>(bytes: &[u8], at: usize) -> [f32; N] {
    let mut out = [0.0; N];
    for (slot, offset) in out.iter_mut().zip((at..).step_by(4)) {
        *slot = f32_in(bytes, offset);
    }
    out
}

/// Whether a sound bank file is one the Nuketown zones draw from.
fn is_wanted_bank(name: &str) -> bool {
    const FAMILIES: [&str; 5] = ["zmb_nuked", "zmb_common", "zmb_patch", "zmb_code_post_gfx", "cmn_root"];
    let language_ok = [".all.", ".english."].iter().any(|l| name.contains(l));
    language_ok && FAMILIES.iter().any(|p| name.starts_with(p))
}

/// The outcome of decoding every indexed sound entry.
struct DecodeSummary {
    /// Entries ok, with an md5 mismatch, and failed.
    tally: [u32; 3],
    audio_frames: u64,
    secs: f32,
}

impl DecodeSummary {
    fn line(&self) -> String {
        let [ok, md5_bad, failed] = self.tally;
        format!(
            "decode: {ok} ok, {md5_bad} md5 mismatch, {failed} failed, {:.1} h of audio in {:.1}s",
            self.audio_frames as f64 / 48000.0 / 3600.0,
            self.secs
        )
    }
}

fn decode_all(index: &HashMap<u32, (PathBuf, sab_t6::BankEntry)>) -> DecodeSummary {
    let clock = std::time::Instant::now();
    let mut summary = DecodeSummary { tally: [0; 3], audio_frames: 0, secs: 0.0 };
    for (path, e) in index.values() {
        let Ok(bank) = sab_t6::BankFile::open(path) else { continue };
        let (slot, got) = bank.read(e).map_or((2, 0), |data| judge(path, e, &data));
        summary.tally[slot] += 1;
        summary.audio_frames += got;
    }
    summary.secs = clock.elapsed().as_secs_f32();
    summary
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let (flags, plain): (Vec<String>, Vec<String>) = std::env::args().skip(1).partition(|a| a.starts_with("--"));
    let root = plain.into_iter().next().map_or_else(bo2zm_tools::game_dir, PathBuf::from);
    let want = |flag: &str| flags.is_empty() || flags.iter().any(|a| a == flag);
    let mut captures = Vec::new();
    // T6M2_ZONES=a,b: other zones (e.g. the campaign's `common`).
    let zones: Vec<String> = std::env::var("T6M2_ZONES")
        .map(|z| z.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|_| NUKETOWN_ZONES.iter().map(|z| (*z).to_owned()).collect());
    for zone in &zones {
        let path = root.join("zone").join("all").join(format!("{zone}.ff"));
        match asset_t6::capture_zone(&path) {
            Ok(c) => captures.push(c),
            Err(e) => {
                eprintln!("{zone}: {e}");
                return Err(Failed);
            }
        }
    }
    for c in &captures {
        println!(
            "zone {}: weapons={} fx={} tracers={} impact_tables={} footstep_tables={} footstep_fx={} banks={} xmodels={} xanims={} strings={} unresolved={}",
            c.zone,
            c.weapons.len(),
            c.fx.len(),
            c.tracers.len(),
            c.impact_tables.len(),
            c.footstep_tables.len(),
            c.footstep_fx_tables.len(),
            c.sound_banks.len(),
            c.xmodels.len(),
            c.animation_refs.len(),
            c.strings.len(),
            c.unresolved_refs
        );
    }
    // T6M2_LOC=<substring>: every English language zone's text whose key
    // holds it (which zone a gun's name lives in).
    if let Ok(want) = std::env::var("T6M2_LOC") {
        let dir = root.join("zone").join("english");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.sort();
        for path in files.iter().filter(|p| p.extension().is_some_and(|x| x == "ff")) {
            let Ok(c) = asset_t6::capture_zone(path) else { continue };
            for (key, value) in c.localize.iter().filter(|(k, _)| k.contains(&want)) {
                println!("loc {}: {key} = {value}", path.file_name().unwrap_or_default().to_string_lossy());
            }
        }
    }
    // Models and anims by name across zones (later zones win).
    let mut models: HashMap<String, (&str, &asset_t6::XModelRef)> = HashMap::new();
    let mut anims: HashMap<String, &asset_t6::XAnimRef> = HashMap::new();
    let mut fx: HashMap<String, &asset_t6::FxEffectRef> = HashMap::new();
    for c in &captures {
        for m in &c.xmodels {
            if !m.name.starts_with(',') {
                models.insert(m.name.clone(), (c.zone.as_str(), m));
            }
        }
        for a in &c.animation_refs {
            if !a.name.starts_with(',') {
                anims.insert(a.name.clone(), a);
            }
        }
        for e in &c.fx {
            if !e.name.starts_with(',') {
                fx.insert(e.name.clone(), e);
            }
        }
    }
    // T6M2_RAW=prefix: every raw file whose name starts with the prefix, as text.
    if let Ok(prefix) = std::env::var("T6M2_RAW") {
        for c in &captures {
            for (name, bytes) in &c.raw_files {
                if name.starts_with(&prefix) {
                    println!("raw {name} ({}):
{}", c.zone, String::from_utf8_lossy(bytes));
                }
            }
        }
    }
    // T6M2_NOTES=a,b: the notetracks of every animation whose name starts
    // with one of the prefixes.
    if let Ok(prefixes) = std::env::var("T6M2_NOTES") {
        let prefixes: Vec<&str> = prefixes.split(',').collect();
        let names = sorted(anims.keys().filter(|n| prefixes.iter().any(|p| n.starts_with(p))).collect::<Vec<_>>());
        for n in names {
            let a = anims[n];
            println!("notes {n}: {:?}", a.notifies);
            println!(
                "   frames {} rate {} loop {} delta {} assetType {} boneCount {:?} bones {:?}",
                a.frame_total, a.frame_rate_hz, a.looping, a.delta, a.asset_type, a.joint_total, a.names
            );
        }
    }
    if want("--weapons") {
        weapons_report(&captures, &models, &anims, &fx);
    }
    if want("--fx") {
        fx_report(&captures, &fx);
    }
    if want("--sound") {
        sound_report(&root, &captures, want("--decode"));
    }
    Ok(())
}

fn weapons_report(
    captures: &[asset_t6::ZoneCapture],
    models: &HashMap<String, (&str, &asset_t6::XModelRef)>,
    anims: &HashMap<String, &asset_t6::XAnimRef>,
    fx: &HashMap<String, &asset_t6::FxEffectRef>,
) {
    use fastfile_t6::layout::WeaponDef;
    let mut weapons: BTreeMap<String, &asset_t6::WeaponRef> = BTreeMap::new();
    for c in captures {
        for w in &c.weapons {
            if !w.name.starts_with(',') {
                weapons.insert(w.name.clone(), w);
            }
        }
    }
    println!("\n== weapons: {}", weapons.len());
    let mut missing_models = 0;
    let mut missing_anims = 0;
    let mut missing_fx = 0;
    for (name, w) in &weapons {
        let gun = w.gun_models.first().cloned().unwrap_or_default();
        let hand = w.def_model(WeaponDef::handXModel).to_owned();
        // T6M2_SOUNDS=a,b: the named weapons' sound fields.
        if std::env::var("T6M2_SOUNDS").is_ok_and(|v| v.split(',').any(|x| x == name)) {
            for (field, off) in [
                ("fireSound", WeaponDef::fireSound), ("fireSoundPlayer", WeaponDef::fireSoundPlayer), ("fireLastSound", WeaponDef::fireLastSound),
                ("fireKillcamSound", WeaponDef::fireKillcamSound), ("crackSound", WeaponDef::crackSound), ("whizbySound", WeaponDef::whizbySound),
                ("reloadSound", WeaponDef::reloadSound), ("reloadEmptySound", WeaponDef::reloadEmptySound), ("reloadStartSound", WeaponDef::reloadStartSound),
                ("reloadEndSound", WeaponDef::reloadEndSound), ("rechamberSound", WeaponDef::rechamberSound), ("raiseSound", WeaponDef::raiseSound),
                ("emptyFireSound", WeaponDef::emptyFireSound), ("pickupSound", WeaponDef::pickupSound), ("meleeHitSound", WeaponDef::meleeHitSound),
            ] {
                println!("sound {name}.{field} = '{}'", w.def_string(off));
            }
        }
        // T6M2_FIRE=1: each weapon's fire timing, with its first-shots (intro) fire; the spin fields are
        // engine defaults on every gun.
        if std::env::var_os("T6M2_FIRE").is_some() {
            println!(
                "fire {name}: introFire {} x{} fireType {} fireTime {} lastFireTime {} fireDelay {} spinUp {} spinDown {} spinRate {} holdFire {} burstDelay {} rechamber {}",
                w.def_i32(WeaponDef::iIntroFireTime),
                w.def_i32(WeaponDef::iIntroFireLength),
                w.def_i32(WeaponDef::fireType),
                w.def_i32(WeaponDef::iFireTime),
                w.def_i32(WeaponDef::iLastFireTime),
                w.def_i32(WeaponDef::iFireDelay),
                w.def_i32(WeaponDef::iSpinUpTime),
                w.def_i32(WeaponDef::iSpinDownTime),
                w.def_f32(WeaponDef::spinRate),
                w.def_i32(WeaponDef::iHoldFireTime),
                w.def_i32(WeaponDef::iBurstDelayTime),
                w.def_i32(WeaponDef::iRechamberTime),
            );
        }
        // T6M2_TRACERS=1: each weapon's tracer.
        if std::env::var_os("T6M2_TRACERS").is_some() && name.ends_with("_zm") {
            println!(
                "tracer {name}: {:?} view flash {:?} world flash {:?} reticle {:?}/{:?} sizes {}/{}",
                w.def_tracer(WeaponDef::tracerType),
                w.def_fx(WeaponDef::viewFlashEffect),
                w.def_fx(WeaponDef::worldFlashEffect),
                w.def_material(WeaponDef::reticleCenter),
                w.def_material(WeaponDef::reticleSide),
                w.def_i32(WeaponDef::iReticleCenterSize),
                w.def_i32(WeaponDef::iReticleSideSize)
            );
        }
        // T6M2_OFFHAND=1: each grenade/mine's offhand class and slot.
        if std::env::var_os("T6M2_OFFHAND").is_some()
            && (w.def_i32(WeaponDef::offhandClass) != 0 || w.def_i32(WeaponDef::offhandSlot) != 0)
        {
            println!(
                "offhand {name}: class {} slot {} inventory {}",
                w.def_i32(WeaponDef::offhandClass),
                w.def_i32(WeaponDef::offhandSlot),
                w.def_i32(WeaponDef::inventoryType)
            );
        }
        if std::env::var("T6M2_WEAPON").is_ok_and(|want| want == *name) {
            println!(
                "weapon {name} models {:?} tracer {:?} hide tags {:?} attach view models {:?} camo {:?} camo ptr {:#x}",
                w.models,
                w.def_tracer(WeaponDef::tracerType),
                w.concealed_bones,
                w.attach_view_models,
                w.camo,
                w.def_i32(WeaponDef::weaponCamo)
            );
            println!(
                "   gun models {:?} world models {:?} anims idle {:?} strings {:?}",
                w.gun_models,
                w.world_models,
                w.animation_refs.get(1),
                w.strings
            );
            let tiers: Vec<(i32, f32)> = (0..6)
                .map(|i| {
                    (
                        w.def_i32(WeaponDef::damage + i * 4),
                        w.def_f32(WeaponDef::damageRange + i * 4),
                    )
                })
                .collect();
            println!(
                "   damage tiers (damage, range) {tiers:?} min player {} melee {} charge time {} delay {}",
                w.def_i32(WeaponDef::minPlayerDamage),
                w.def_i32(WeaponDef::iMeleeDamage),
                w.def_i32(WeaponDef::meleeChargeTime),
                w.def_i32(WeaponDef::meleeChargeDelay)
            );
            println!("   location multipliers {:?}", w.location_damage);
            println!(
                "   heat: overheat {} rate {} cooldown {} end {} coolWhileFiring {} fuelTank {} tankLife {} | spin up {} down {} rate {} | fireType {} weapType {} fireTime {} clip {}",
                w.def_u8(WeaponDef::overheatWeapon),
                w.def_f32(WeaponDef::overheatRate),
                w.def_f32(WeaponDef::cooldownRate),
                w.def_f32(WeaponDef::overheatEndVal),
                w.def_u8(WeaponDef::coolWhileFiring),
                w.def_u8(WeaponDef::fuelTankWeapon),
                w.def_i32(WeaponDef::iTankLifeTime),
                w.def_i32(WeaponDef::iSpinUpTime),
                w.def_i32(WeaponDef::iSpinDownTime),
                w.def_f32(WeaponDef::spinRate),
                w.def_i32(WeaponDef::fireType),
                w.def_i32(WeaponDef::weapType),
                w.def_i32(WeaponDef::iFireTime),
                w.var_i32(fastfile_t6::layout::WeaponVariantDef::iClipSize)
            );
            for (i, a) in w.animation_refs.iter().enumerate().filter(|(_, a)| !a.is_empty()) {
                println!("   anim {i:#04x} {a}");
            }
            println!(
                "   fuseTime {} aiFuseTime {} cookOffHold {} timedDetonation {} explosionRadius {} inner {} projSpeed {} activateDist {} projExplosion {} stickiness {} holdToThrow {} fx {:?}",
                w.def_i32(WeaponDef::fuseTime),
                w.def_i32(WeaponDef::aiFuseTime),
                w.def_u8(WeaponDef::bCookOffHold),
                w.def_u8(WeaponDef::timedDetonation),
                w.def_i32(WeaponDef::iExplosionRadius),
                w.def_i32(WeaponDef::iExplosionInnerDamage),
                w.def_i32(WeaponDef::iProjectileSpeed),
                w.def_i32(WeaponDef::iProjectileActivateDist),
                w.def_i32(WeaponDef::projExplosion),
                w.def_i32(WeaponDef::stickiness),
                w.def_u8(WeaponDef::holdButtonToThrow),
                w.def_fx(WeaponDef::projExplosionEffect),
            );
            let proj = w.def_model(WeaponDef::projectileModel);
            println!(
                "   rolling {} rotateType {} rotate {} keepRolling {} forceBounce {} noCrumple {} projModel '{proj}' bounds {:?} clipModel '{}' parallel {:?} perpendicular {:?}",
                w.def_i32(WeaponDef::isRollingGrenade),
                w.def_i32(WeaponDef::rotateType),
                w.def_u8(WeaponDef::rotate),
                w.def_u8(WeaponDef::bKeepRolling),
                w.def_u8(WeaponDef::bForceBounce),
                w.def_u8(WeaponDef::bNoCrumpleMissile),
                models
                    .get(proj)
                    .map(|(_, m)| (m.lo_corner, m.hi_corner, m.radius, m.contents)),
                w.def_model(WeaponDef::worldClipModel),
                w.along_rebound,
                w.across_rebound,
            );
            for (slot, a) in w.animation_refs.iter().enumerate() {
                if !a.is_empty() {
                    let notes = anims
                        .get(a.as_str())
                        .or_else(|| anims.get(a.to_ascii_lowercase().as_str()))
                        .map(|x| x.notifies.clone())
                        .unwrap_or_default();
                    println!("   anim slot {slot:#04x}: {a} notes {notes:?}");
                }
            }
            println!("   notetrack sound map {:?}", w.notetrack_sounds);
            println!(
                "   strings {:?}",
                w.strings
                    .iter()
                    .filter(|(_, s)| !s.is_empty())
                    .collect::<Vec<_>>()
            );
        }
        // T6M2_AMMO=1: one line per weapon of its ammo and fire rules.
        if std::env::var_os("T6M2_AMMO").is_some() {
            use fastfile_t6::layout::WeaponVariantDef;
            println!(
                "ammo {name}: type {} class {} fire {} clip {} start {} max {} unlimited {} clipOnly {} ammoName '{}' fireTime {} damage {} meleeDamage {} alt '{}' inv {} bolt {} rechamber {}",
                w.def_i32(WeaponDef::weapType),
                w.def_i32(WeaponDef::weapClass),
                w.def_i32(WeaponDef::fireType),
                w.var_i32(WeaponVariantDef::iClipSize),
                w.def_i32(WeaponDef::iStartAmmo),
                w.def_i32(WeaponDef::iMaxAmmo),
                w.def_u8(WeaponDef::unlimitedAmmo),
                w.def_u8(WeaponDef::bClipOnly),
                w.ammo_name,
                w.def_i32(WeaponDef::iFireTime),
                w.def_i32(WeaponDef::damage),
                w.def_i32(WeaponDef::iMeleeDamage),
                w.alt_weapon_name,
                w.def_i32(WeaponDef::inventoryType),
                w.def_u8(WeaponDef::bBoltAction),
                w.def_i32(WeaponDef::iShotsBeforeRechamber),
            );
        }
        // T6M2_AIM=1: one line per weapon of what aims, throws and sounds it.
        if std::env::var_os("T6M2_AIM").is_some() {
            let f = |o: usize| w.def_f32(o);
            println!(
                "aim {name}: impactType {} hip min {}/{}/{} max {}/{}/{} decay {} fire_add {} turn_add {} move_add {} | ads {} | hip view kick p[{}, {}] y[{}, {}] min {} | ads view kick p[{}, {}] y[{}, {}] min {} | hip gun kick p[{}, {}] | ads gun kick p[{}, {}] | ads scatter [{}, {}] | hip scatter [{}, {}] playerSpread {} aiSpread {} stackFireSpread {} reticleSidePos {} fireTime {} | flash ofs view {:?} world {:?} altTag {} | proj speed {} up {} rel up {} fwd {} parent {} | melee snd swipe '{}' player '{}' hit '{}' miss '{}' | tracer {:?}",
                w.def_i32(WeaponDef::impactType),
                f(WeaponDef::fHipSpreadStandMin),
                f(WeaponDef::fHipSpreadDuckedMin),
                f(WeaponDef::fHipSpreadProneMin),
                f(WeaponDef::hipSpreadStandMax),
                f(WeaponDef::hipSpreadDuckedMax),
                f(WeaponDef::hipSpreadProneMax),
                f(WeaponDef::fHipSpreadDecayRate),
                f(WeaponDef::fHipSpreadFireAdd),
                f(WeaponDef::fHipSpreadTurnAdd),
                f(WeaponDef::fHipSpreadMoveAdd),
                f(WeaponDef::fAdsSpread),
                f(WeaponDef::fHipViewKickPitchMin),
                f(WeaponDef::fHipViewKickPitchMax),
                f(WeaponDef::fHipViewKickYawMin),
                f(WeaponDef::fHipViewKickYawMax),
                f(WeaponDef::fHipViewKickMinMagnitude),
                f(WeaponDef::fAdsViewKickPitchMin),
                f(WeaponDef::fAdsViewKickPitchMax),
                f(WeaponDef::fAdsViewKickYawMin),
                f(WeaponDef::fAdsViewKickYawMax),
                f(WeaponDef::fAdsViewKickMinMagnitude),
                f(WeaponDef::fHipGunKickPitchMin),
                f(WeaponDef::fHipGunKickPitchMax),
                f(WeaponDef::fAdsGunKickPitchMin),
                f(WeaponDef::fAdsGunKickPitchMax),
                f(WeaponDef::fAdsViewScatterMin),
                f(WeaponDef::fAdsViewScatterMax),
                f(WeaponDef::fHipViewScatterMin),
                f(WeaponDef::fHipViewScatterMax),
                f(WeaponDef::playerSpread),
                f(WeaponDef::aiSpread),
                f(WeaponDef::stackFireSpread),
                f(WeaponDef::fHipReticleSidePos),
                w.def_i32(WeaponDef::iFireTime),
                w.def_vec3(WeaponDef::vViewFlashOffset),
                w.def_vec3(WeaponDef::vWorldFlashOffset),
                w.def_u8(WeaponDef::bUseAltTagFlash),
                w.def_i32(WeaponDef::iProjectileSpeed),
                w.def_i32(WeaponDef::iProjectileSpeedUp),
                w.def_i32(WeaponDef::iProjectileSpeedRelativeUp),
                w.def_i32(WeaponDef::iProjectileSpeedForward),
                f(WeaponDef::fProjectileTakeParentVelocity),
                w.def_string(WeaponDef::meleeSwipeSound),
                w.def_string(WeaponDef::meleeSwipeSoundPlayer),
                w.def_string(WeaponDef::meleeHitSound),
                w.def_string(WeaponDef::meleeMissSound),
                w.def_tracer(WeaponDef::tracerType),
            );
        }
        let world = w.world_models.first().cloned().unwrap_or_default();
        let anim_named = w.animation_refs.iter().filter(|a| !a.is_empty()).count();
        let anim_found = w
            .animation_refs
            .iter()
            .filter(|a| !a.is_empty() && anims.contains_key(a.as_str()))
            .count();
        let gun_found = gun.is_empty() || models.contains_key(&gun);
        let hand_found = hand.is_empty() || models.contains_key(&hand);
        if !gun_found || !hand_found {
            missing_models += 1;
        }
        missing_anims += anim_named - anim_found;
        for (_, f) in &w.fx {
            if !fx.contains_key(f) {
                missing_fx += 1;
            }
        }
        let gun_bones = models.get(&gun).map_or(0, |m| m.1.joint_labels.len());
        let hand_bones = models.get(&hand).map_or(0, |m| m.1.joint_labels.len());
        println!(
            "{name}: type={} class={} fire={}ms clip={} dmg={} gun='{gun}'{} ({gun_bones} bones) hands='{hand}'{} ({hand_bones} bones) world='{world}' anims {anim_found}/{anim_named} fire_snd='{}' flash='{}' wflash='{}' eject='{}' notetrack_snds={}",
            w.def_i32(WeaponDef::weapType),
            w.def_i32(WeaponDef::weapClass),
            w.def_i32(WeaponDef::iFireTime),
            w.var_i32(fastfile_t6::layout::WeaponVariantDef::iClipSize),
            w.def_i32(WeaponDef::damage),
            if gun_found { "" } else { " MISSING" },
            if hand_found { "" } else { " MISSING" },
            w.def_string(WeaponDef::fireSoundPlayer),
            w.def_fx(WeaponDef::viewFlashEffect),
            w.def_fx(WeaponDef::worldFlashEffect),
            w.def_fx(WeaponDef::viewShellEjectEffect),
            w.notetrack_sounds.len(),
        );
    }
    println!(
        "weapons: {} with a missing gun/hand model, {missing_anims} named anims not found, {missing_fx} fx names not found",
        missing_models
    );
    // Skinning sanity for every model with bones.
    let mut checked = 0;
    let mut bad = Vec::new();
    for (name, (_, m)) in models {
        if m.joint_labels.len() < 2 {
            continue;
        }
        checked += 1;
        for (i, srf) in m.lod0.iter().enumerate() {
            let rigid: usize = srf.vert_lists.iter().map(|v| usize::from(v.1)).sum();
            let blended: usize = srf.blend_counts.iter().map(|&c| c.max(0) as usize).sum();
            if rigid + blended != usize::from(srf.vertex_tally) {
                bad.push(format!(
                    "{name} surf {i}: rigid {rigid} + blended {blended} != verts {}",
                    srf.vertex_tally
                ));
            }
        }
        if m.foundation_surface.len() != m.joint_labels.len() {
            bad.push(format!("{name}: base_mat {} != bones {}", m.foundation_surface.len(), m.joint_labels.len()));
        }
    }
    println!("skinned models checked: {checked}, problems: {}", bad.len());
    for b in bad.iter().take(20) {
        println!("  {b}");
    }
    // Animation data sanity: track names vs counts, data presence.
    let mut empty = 0;
    let mut with_data = 0;
    for a in anims.values() {
        if a.names.len() != usize::from(a.joint_total[9]) {
            empty += 1;
        } else if !a.payload_i16.is_empty() || !a.payload_u8.is_empty() || !a.noise_word16.is_empty() {
            with_data += 1;
        }
    }
    println!(
        "anims: {} total, {with_data} with data, {empty} with name/count mismatch",
        anims.len()
    );
}

fn fx_report(captures: &[asset_t6::ZoneCapture], fx: &HashMap<String, &asset_t6::FxEffectRef>) {
    let mut types: BTreeMap<u8, usize> = BTreeMap::new();
    let mut visual_kinds: BTreeMap<&'static str, usize> = BTreeMap::default();
    let mut missing_children = BTreeMap::new();
    for e in fx.values() {
        for el in &e.elems {
            *types.entry(el.elem_type()).or_default() += 1;
            for v in &el.visuals {
                let k = match v {
                    asset_t6::FxVisualRef::Material(_) => "material",
                    asset_t6::FxVisualRef::Mark(_) => "mark",
                    asset_t6::FxVisualRef::Model(_) => "model",
                    asset_t6::FxVisualRef::Effect(name) => {
                        if !fx.contains_key(name) {
                            *missing_children.entry(name.clone()).or_insert(0) += 1;
                        }
                        "effect"
                    }
                    asset_t6::FxVisualRef::Sound(_) => "sound",
                    asset_t6::FxVisualRef::Light(_) => "light",
                    asset_t6::FxVisualRef::None => "none",
                };
                *visual_kinds.entry(k).or_default() += 1;
            }
            for child in [
                &el.effect_upon_strike,
                &el.effect_upon_demise,
                &el.effect_spawned,
            ] {
                if !child.is_empty() && !fx.contains_key(child) {
                    *missing_children.entry(child.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    println!(
        "\n== fx: {} effects; elem types {:?}; visuals {:?}",
        fx.len(),
        types,
        visual_kinds
    );
    // T6M2_FLAGS=1: per element flag bit, how many elements carry it (by
    // element type) and a few of their effects.
    if std::env::var_os("T6M2_FLAGS").is_some() {
        use fastfile_t6::layout::FxElemDef;
        let names = sorted(fx.keys().collect::<Vec<_>>());
        for bit in 0..32 {
            let mask = 1i32 << bit;
            let mut by_type: BTreeMap<u8, usize> = BTreeMap::default();
            let mut some: Vec<String> = Vec::new();
            for name in &names {
                for (i, el) in fx[*name].elems.iter().enumerate() {
                    if el.int_from_bytes(FxElemDef::flags) & mask != 0 {
                        *by_type.entry(el.elem_type()).or_default() += 1;
                        if some.len() < 400 {
                            some.push(format!("{name}#{i}"));
                        }
                    }
                }
            }
            if !by_type.is_empty() {
                println!("flag {mask:#010x}: by type {by_type:?} e.g. {some:?}");
            }
        }
    }
    // T6M2_FONT=<substring>: each matching font: size, material, glyphs.
    if let Ok(want) = std::env::var("T6M2_FONT") {
        for c in captures {
            for f in c.fonts.iter().filter(|f| f.name.contains(&want)) {
                println!(
                    "font {} ({}): pixel height {}, material {}, {} glyphs, {} kerning pairs",
                    f.name,
                    c.zone,
                    f.raster_rows,
                    f.material,
                    f.glyphs.len(),
                    f.kerning.len()
                );
                for g in &f.glyphs {
                    println!(
                        "   {:?}: x0 {} y0 {} dx {} size {}x{} st {:?}",
                        char::from_u32(u32::from(g.letter)),
                        g.x0,
                        g.y0,
                        g.step_along_x,
                        g.texel_span,
                        g.raster_rows,
                        g.st
                    );
                }
            }
        }
    }
    // T6M2_MAT=<name>: that material in every zone: technique set, textures
    // (sampler hash, image, size), constants, draw state.
    if let Ok(want) = std::env::var("T6M2_MAT") {
        for cc in captures {
            for mat in cc.materials.iter().filter(|x| x.name == want) {
                let ts = mat
                    .method_set
                    .map_or("-".to_owned(), |k| cc.technique_sets[k.index].name.clone());
                let texs: Vec<String> = mat
                    .textures
                    .iter()
                    .map(|x| {
                        format!(
                            "{:#010x}:{}",
                            x.label_digest,
                            x.image.map_or("-".to_owned(), |k| {
                                let i = &cc.images[k.index];
                                format!(
                                    "{} {}x{} hash {:#010x} parts {} embedded {:?}",
                                    i.name,
                                    i.width,
                                    i.height,
                                    i.label_digest,
                                    i.streamed_parts,
                                    i.embedded.as_ref().map(|e| (e.dxgi_format, e.data.len()))
                                )
                            })
                        )
                    })
                    .collect();
                let state = mat
                    .state_words
                    .first()
                    .map(|b| asset_t6::DrawState::decode(*b));
                println!(
                    "material {} ({}) techset {ts} textures {texs:?} consts {:?} state0 {:?}",
                    mat.name,
                    cc.zone,
                    mat.constants
                        .iter()
                        .map(|k| (&k.1, k.2))
                        .collect::<Vec<_>>(),
                    state
                );
            }
        }
    }
    // T6M2_TABLE=<substring>: each matching string table, row by row.
    if let Ok(want) = std::env::var("T6M2_TABLE") {
        for c in captures {
            for t in c.string_tables.iter().filter(|t| t.name.contains(&want)) {
                println!("table {} ({}): {} x {}", t.name, c.zone, t.rows, t.columns);
                for r in 0..t.rows {
                    let row: Vec<&str> = (0..t.columns)
                        .map(|k| t.cells.get(r * t.columns + k).map_or("", String::as_str))
                        .collect();
                    println!("   {r}: {}", row.join(" | "));
                }
            }
        }
    }
    // T6M2_CAMO=<substring>: each matching weapon camo: its sets (images
    // per camo index) and material swaps; and the weapons that use it.
    if let Ok(want) = std::env::var("T6M2_CAMO") {
        for c in captures {
            println!(
                "zone {}: {} camos loaded, {} kept",
                c.zone,
                c.loads
                    .get(&fastfile_t6::XAssetKind::WeaponCamo)
                    .copied()
                    .unwrap_or(0),
                c.weapon_camos.len()
            );
            for camo in c.weapon_camos.iter().filter(|x| x.name.contains(&want)) {
                println!(
                    "camo {} ({}): base images '{}' '{}', {} sets, {} material sets",
                    camo.name,
                    c.zone,
                    camo.solid_base_image,
                    camo.pattern_base_image,
                    camo.sets.len(),
                    camo.material_sets.len()
                );
                for (i, set) in camo.sets.iter().enumerate() {
                    println!(
                        "   set {i}: solid '{}' pattern '{}' offset {:?} scale {}",
                        set.solid, set.pattern, set.offset, set.scale
                    );
                }
                for (i, list) in camo.material_sets.iter().enumerate() {
                    for m in list {
                        println!(
                            "   materials {i}: flags {:#x} {:?} -> {:?} consts {:?}",
                            m.replace_flags, m.base, m.camo, m.consts
                        );
                    }
                }
                let users: Vec<&str> = captures
                    .iter()
                    .flat_map(|cc| cc.weapons.iter())
                    .filter(|w| w.camo == camo.name)
                    .map(|w| w.name.as_str())
                    .collect();
                println!("   used by {users:?}");
            }
        }
    }
    // T6M2_FX=<substring>: each matching effect's elements: type, flags,
    // counts, atlas, visuals and each material's technique set and textures.
    if let Ok(want) = std::env::var("T6M2_FX") {
        use fastfile_t6::layout::FxElemDef;
        let names = sorted(fx.keys().filter(|n| n.contains(&want)).collect::<Vec<_>>());
        for (name, e) in names.into_iter().map(|n| (n, fx[n])) {
            println!("effect {name}: flags {:#x} elems {} (looping {} oneshot {} emission {}) loop life {}",
                e.flags, e.elems.len(), e.repeat_total, e.single_play_total, e.spawn_total, e.loop_lifetime_millis);
            for (i, el) in e.elems.iter().enumerate() {
                let b = |o: usize| el.raw.get(o).copied().unwrap_or(0);
                println!("  elem {i}: type {} flags {:#010x} spawn [{}, {}] life [{}, {}] vis {} lighting {} sort {} atlas {:?} visuals {:?}",
                    el.elem_type(), el.int_from_bytes(FxElemDef::flags), el.int_from_bytes(FxElemDef::spawn), el.int_from_bytes(FxElemDef::spawn + 4),
                    el.int_from_bytes(FxElemDef::lifeSpanMsec), el.int_from_bytes(FxElemDef::lifeSpanMsec + 4), b(FxElemDef::visualCount), b(FxElemDef::lightingFrac), b(FxElemDef::sortOrder),
                    el.raw.get(FxElemDef::atlas..FxElemDef::atlas + 8), el.visuals);
                let vel_n = b(FxElemDef::velIntervalCount);
                println!("     spawn range [{}, {}] fade in range [{}, {}] fade out range [{}, {}] spawn frustum cull radius {}",
                    el.float_from_bytes(FxElemDef::spawnRange), el.float_from_bytes(FxElemDef::spawnRange + 4),
                    el.float_from_bytes(FxElemDef::fadeInRange), el.float_from_bytes(FxElemDef::fadeInRange + 4), el.float_from_bytes(FxElemDef::fadeOutRange), el.float_from_bytes(FxElemDef::fadeOutRange + 4), el.float_from_bytes(FxElemDef::spawnFrustumCullRadius));
                println!(
                    "     rotation axis {} initial rotation [{}, {}] spawn angles {:?} angular vel {:?} billboard pivot [{}, {}] spawn delay [{}, {}] alpha fade {} offset height [{}, {}]",
                    el.int_from_bytes(FxElemDef::rotationAxis),
                    el.float_from_bytes(FxElemDef::initialRotation),
                    el.float_from_bytes(FxElemDef::initialRotation + 4),
                    (0..6)
                        .map(|k| el.float_from_bytes(FxElemDef::spawnAngles + k * 4))
                        .collect::<Vec<_>>(),
                    (0..6)
                        .map(|k| el.float_from_bytes(FxElemDef::angularVelocity + k * 4))
                        .collect::<Vec<_>>(),
                    el.float_from_bytes(FxElemDef::billboardPivot),
                    el.float_from_bytes(FxElemDef::billboardPivot + 4),
                    el.int_from_bytes(FxElemDef::spawnDelayMsec),
                    el.int_from_bytes(FxElemDef::spawnDelayMsec + 4),
                    el.raw
                        .get(FxElemDef::alphaFadeTimeMsec..FxElemDef::alphaFadeTimeMsec + 2)
                        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]])),
                    el.float_from_bytes(FxElemDef::spawnOffsetHeight),
                    el.float_from_bytes(FxElemDef::spawnOffsetHeight + 4)
                );
                println!("     gravity [{}, {}] wind {} vel intervals {} vis intervals {} u (cloud density) [{}, {}] / [{:#x}, {:#x}] spawn origin {:?} offset radius [{}, {}]",
                    el.float_from_bytes(FxElemDef::gravity), el.float_from_bytes(FxElemDef::gravity + 4), el.float_from_bytes(FxElemDef::windInfluence), vel_n, b(FxElemDef::visStateIntervalCount),
                    el.float_from_bytes(FxElemDef::u), el.float_from_bytes(FxElemDef::u + 4), el.int_from_bytes(FxElemDef::u), el.int_from_bytes(FxElemDef::u + 4),
                    (0..6).map(|k| el.float_from_bytes(FxElemDef::spawnOrigin + k * 4)).collect::<Vec<_>>(),
                    el.float_from_bytes(FxElemDef::spawnOffsetRadius), el.float_from_bytes(FxElemDef::spawnOffsetRadius + 4));
                let vf = |k: usize, at: usize| f32_run::<6>(&el.speed_readings, k * 96 + at);
                let vis = |k: usize, at: usize| f32_in(&el.visibility_readings, k * 48 + at);
                let vis_n = usize::from(b(FxElemDef::visStateIntervalCount));
                let rgba = |k: usize, at: usize| -> Vec<u8> {
                    el.visibility_readings.get(k * 48 + at..k * 48 + at + 4).map_or(Vec::new(), |b| b.to_vec())
                };
                println!("     vis colour first {:?} (+{:?}) last {:?} (+{:?})", rgba(0, 0), rgba(0, 24), rgba(vis_n, 0), rgba(vis_n, 24));
                if std::env::var_os("T6M2_VIS_RAW").is_some() {
                    for k in 0..=vis_n {
                        let raw: Vec<String> = (0..12).map(|i| {
                            let o = k * 48 + i * 4;
                            el.visibility_readings.get(o..o + 4).map_or("-".to_owned(), |b| format!("{:02x}{:02x}{:02x}{:02x}/{:.3}", b[0], b[1], b[2], b[3], f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
                        }).collect();
                        println!("       vis[{k}] {}", raw.join(" "));
                    }
                }
                println!("     vis first: size {} (+{}) x {} (+{}) scale {} (+{}); last: size {} (+{}) x {} scale {}",
                    vis(0, 12), vis(0, 36), vis(0, 16), vis(0, 40), vis(0, 20), vis(0, 44),
                    vis(vis_n, 12), vis(vis_n, 36), vis(vis_n, 16), vis(vis_n, 20));
                let last = usize::from(vel_n);
                println!("     vel first sample: local vel {:?} delta {:?} world vel {:?} delta {:?}",
                    vf(0, 0), vf(0, 24), vf(0, 48), vf(0, 72));
                println!("     vel last sample: local vel {:?} delta {:?} world vel {:?} delta {:?}",
                    vf(last, 0), vf(last, 24), vf(last, 48), vf(last, 72));
                for v in &el.visuals {
                    let names: Vec<&String> = match v {
                        asset_t6::FxVisualRef::Material(m) => vec![m],
                        asset_t6::FxVisualRef::Mark(pair) => pair.iter().collect(),
                        _ => Vec::new(),
                    };
                    for m in names {
                        for c in captures {
                            if let Some(mat) = c.materials.iter().find(|x| x.name == *m) {
                                let ts = mat.method_set.map_or("-".to_owned(), |t| {
                                    c.technique_sets[t.index].name.clone()
                                });
                                let texs: Vec<String> = mat
                                    .textures
                                    .iter()
                                    .map(|t| {
                                        let img = t.image.map(|k| &c.images[k.index]);
                                        let home = img
                                            .and_then(|i| i.name.strip_prefix(','))
                                            .map(|real| {
                                                captures
                                                    .iter()
                                                    .find_map(|cc| {
                                                        cc.images
                                                            .iter()
                                                            .find(|x| x.name == real)
                                                            .map(|x| {
                                                                format!(
                                                                    " [home {} {}x{}]",
                                                                    cc.zone, x.width, x.height
                                                                )
                                                            })
                                                    })
                                                    .unwrap_or_else(|| " [NO HOME]".to_owned())
                                            })
                                            .unwrap_or_default();
                                        format!(
                                            "{:#010x}:{}{home}",
                                            t.label_digest,
                                            img.map_or("-".to_owned(), |i| format!(
                                                "{} {}x{}",
                                                i.name, i.width, i.height
                                            ))
                                        )
                                    })
                                    .collect();
                                let state = mat
                                    .state_words
                                    .first()
                                    .map(|b| asset_t6::DrawState::decode(*b));
                                println!(
                                    "     material {m} ({}) techset {ts} textures {texs:?} consts {:?} state0 {:?}",
                                    c.zone,
                                    mat.constants
                                        .iter()
                                        .map(|k| (&k.1, k.2))
                                        .collect::<Vec<_>>(),
                                    state
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "fx children not found: {} {:?}",
        missing_children.len(),
        missing_children.keys().take(10).collect::<Vec<_>>()
    );
    for c in captures {
        for t in &c.impact_tables {
            let titled = t
                .rows
                .iter()
                .map(|r| {
                    r.0.iter()
                        .chain(r.1.iter())
                        .filter(|n| !n.is_empty())
                        .count()
                })
                .sum::<usize>();
            let found = t
                .rows
                .iter()
                .map(|r| {
                    r.0.iter()
                        .chain(r.1.iter())
                        .filter(|n| !n.is_empty() && fx.contains_key(n.as_str()))
                        .count()
                })
                .sum::<usize>();
            println!(
                "impact table {} ({}): {} rows, {found}/{titled} effects found",
                t.name,
                c.zone,
                t.rows.len()
            );
            for (i, r) in t.rows.iter().enumerate().take(3) {
                println!(
                    "  row {i}: concrete='{}' metal='{}' flesh0='{}'",
                    r.0[5], r.0[13], r.1[0]
                );
            }
            // T6M2_IMPACT=1: every row, every surface.
            if std::env::var_os("T6M2_IMPACT").is_some() {
                for (i, r) in t.rows.iter().enumerate() {
                    println!("  row {i}:");
                    for (s, n) in r.0.iter().enumerate() {
                        println!("     surf {s:>2}: {n}");
                    }
                    println!("     flesh: {:?}", r.1);
                }
            }
        }
        for t in &c.tracers {
            println!("tracer {} ({}): mat='{}' type={} every={} speed={} len={} width={} screw={}/{} fade={}/{} repeat={} colours={:?}", t.name, c.zone, t.material, t.ty, t.render_spacing, t.speed, t.ray_extent, t.ray_thickness, t.spiral_radius, t.spiral_pitch_distance, t.fade_secs, t.fade_scale, t.tex_repeat_rate, t.colors);
            for cc in captures {
                if let Some(mat) = cc.materials.iter().find(|x| x.name == t.material) {
                    let ts = mat.method_set.map_or("-".to_owned(), |k| cc.technique_sets[k.index].name.clone());
                    let texs: Vec<String> = mat.textures.iter().map(|x| format!("{:#010x}:{}", x.label_digest, x.image.map_or("-".to_owned(), |k| { let i = &cc.images[k.index]; format!("{} {}x{} hash {:#010x} parts {} part_hash {:#010x} embedded {:?}", i.name, i.width, i.height, i.label_digest, i.streamed_parts, i.part_hash, i.embedded.as_ref().map(|e| (e.dxgi_format, e.data.len()))) }))).collect();
                    let state = mat.state_words.first().map(|b| asset_t6::DrawState::decode(*b));
                    println!("   material {} ({}) techset {ts} textures {texs:?} consts {:?} state0 {:?}", t.material, cc.zone, mat.constants.iter().map(|k| (&k.1, k.2)).collect::<Vec<_>>(), state);
                    break;
                }
            }
        }
        for t in &c.footstep_fx_tables {
            println!(
                "footstep fx table {} ({}): dirt='{}' concrete='{}'",
                t.name, c.zone, t.fx[6], t.fx[5]
            );
        }
    }
}

fn sound_report(root: &Path, captures: &[asset_t6::ZoneCapture], decode: bool) {
    // Every bank file in the sound folder, indexed by entry id.
    let dir = root.join("sound");
    let mut index: HashMap<u32, (PathBuf, sab_t6::BankEntry)> = HashMap::new();
    let mut files = 0;
    if let Ok(read) = std::fs::read_dir(&dir) {
        let names = sorted(read.flatten().map(|e| e.path()).collect::<Vec<_>>());
        for path in names {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
            if !is_wanted_bank(&name) {
                continue;
            }
            match sab_t6::BankFile::open(&path) {
                Ok(bank) => {
                    files += 1;
                    for e in &bank.entries {
                        index.entry(e.id).or_insert_with(|| (path.clone(), *e));
                    }
                }
                Err(e) => println!("bank {name}: {e}"),
            }
        }
    }
    println!("\n== sound: {files} bank files, {} entries indexed", index.len());
    let total: usize = captures.iter().flat_map(|c| &c.sound_banks).map(|b| b.aliases.len()).sum();
    let mut entries = 0;
    let mut resolved = 0;
    let mut unresolved_names = Vec::new();
    for c in captures {
        for b in &c.sound_banks {
            for a in &b.aliases {
                for e in &a.entries {
                    entries += 1;
                    if e.asset_id == 0 || index.contains_key(&e.asset_id) {
                        resolved += 1;
                    } else if unresolved_names.len() < 12 {
                        unresolved_names.push(format!(
                            "{} -> {} ({:#x})",
                            e.name, e.asset_file, e.asset_id
                        ));
                    }
                }
            }
            println!("bank {} ({}): {} aliases, {} ducks", b.name, c.zone, b.aliases.len(), b.ducks.len());
            if std::env::var_os("T6M2_DUCKS").is_some() {
                for d in &b.ducks {
                    let users: Vec<&str> = captures
                        .iter()
                        .flat_map(|c| c.sound_banks.iter())
                        .flat_map(|b| b.aliases.iter())
                        .filter(|a| a.entries.iter().any(|e| e.duck == d.id))
                        .map(|a| a.name.as_str())
                        .collect();
                    println!("   duck {} id {:#x} in {} out {} delay {} distance {} length {} paused {} users {} {:?}",
                        d.name, d.id, d.ease_on, d.ease_off, d.start_delay, d.distance, d.length,
                        d.update_while_paused, users.len(), &users[..users.len().min(6)]);
                    let att: Vec<String> = d.attenuation.iter().enumerate().filter(|(_, v)| **v < 1.0)
                        .map(|(k, v)| format!("{k}:{v:.2}")).collect();
                    let filt: Vec<String> = d.filter.iter().enumerate().filter(|(_, v)| **v < 1.0)
                        .map(|(k, v)| format!("{k}:{v:.2}")).collect();
                    println!("      attenuation {} filter {}", att.join(" "), filt.join(" "));
                }
            }
        }
        if let Some(g) = &c.snd_globals {
            println!("driver globals {} ({}): {} curves, {} groups; curve 0 = {} {:?}", g.name, c.zone, g.curves.len(), g.groups.len(),
                g.curves.first().map_or("", |c| c.name.as_str()), g.curves.first().map(|c| c.points));
            if std::env::var_os("T6M2_DUCKS").is_some() {
                for (k, (name, id)) in g.duck_groups.iter().enumerate() {
                    println!("   duck group {k:>2} {name} id {id:#x}");
                }
            }
            // T6M2_GROUPS=1: each volume group as the mixer scales it, and how many
            // of this zone's alias variants sit in it.
            if std::env::var_os("T6M2_GROUPS").is_some() {
                let mut uses = vec![0usize; g.groups.len() + 1];
                for b in captures.iter().flat_map(|c| c.sound_banks.iter()) {
                    for e in b.aliases.iter().flat_map(|a| a.entries.iter()) {
                        let at = (e.volume_group() as usize).min(g.groups.len());
                        uses[at] += 1;
                    }
                }
                for (i, ((name, parent, sp, mp), mix)) in g.groups.iter().zip(&g.group_mix).enumerate() {
                    let mut scalar = 1.0f32;
                    let mut at = i as u32;
                    for _ in 0..100 {
                        let Some(m) = g.group_mix.get(at as usize) else { break };
                        scalar *= f32::from(m.attenuation_mp) / 65535.0;
                        if m.parent == u32::MAX { break; }
                        at = m.parent;
                    }
                    println!("   group {i:>2} {name} parent '{parent}' ({}) category {} sp {sp} mp {mp} scalar {scalar:.3} variants {}",
                        mix.parent as i64, mix.category, uses[i]);
                }
                println!("   no group: {} variants", uses[g.groups.len()]);
                let mut pan_uses: std::collections::BTreeMap<(u8, bool, u32), usize> = Default::default();
                for b in captures.iter().flat_map(|c| c.sound_banks.iter()) {
                    for e in b.aliases.iter().flat_map(|a| a.entries.iter()) {
                        *pan_uses.entry((e.pan, e.flags[0] & 2 != 0, e.volume_group())).or_default() += 1;
                    }
                }
                for (i, p) in g.pans.iter().enumerate() {
                    println!("   pan {i:>2} {} front {} back {} center {} lfe {} left {} right {}",
                        p.name, p.front, p.back, p.center, p.lfe, p.left, p.right);
                }
                for ((pan, spatial, group), n) in &pan_uses {
                    println!("   uses pan {pan} spatialized {spatial} group {group}: {n}");
                }
            }
        }
        for t in &c.footstep_tables {
            println!(
                "footstep table {} ({}): dirt ids {:?}",
                t.name, c.zone, t.aliases[6]
            );
            if std::env::var_os("T6M2_FOOTSTEPS").is_some() && t.name == "default_1st_person" {
                let names: std::collections::HashMap<u32, &str> = captures
                    .iter()
                    .flat_map(|c| c.sound_banks.iter())
                    .flat_map(|b| b.aliases.iter())
                    .map(|a| (sab_t6::sound_hash(&a.name), a.name.as_str()))
                    .collect();
                for (surf, row) in t.aliases.iter().enumerate() {
                    let row: Vec<String> = row
                        .iter()
                        .map(|id| {
                            names
                                .get(id)
                                .map_or(format!("{id:#x}"), |n| (*n).to_owned())
                        })
                        .collect();
                    println!("   surf {surf:>2}: {}", row.join(" | "));
                }
            }
        }
    }
    // T6M2_BUS=1: how many aliases each sound bus holds (and how many are
    // music), with a few names each.
    if std::env::var_os("T6M2_BUS").is_some() {
        let mut buses: std::collections::BTreeMap<u32, (usize, usize, Vec<String>)> =
            Default::default();
        for c in captures {
            for b in &c.sound_banks {
                for a in &b.aliases {
                    let Some(e) = a.entries.first() else { continue };
                    let row = buses.entry(e.bus()).or_default();
                    row.0 += 1;
                    row.1 += usize::from(e.is_music());
                    if row.2.len() < 8
                        && !row
                            .2
                            .iter()
                            .any(|n| n[..n.len().min(6)] == a.name[..a.name.len().min(6)])
                    {
                        row.2.push(a.name.clone());
                    }
                }
            }
        }
        for (bus, (n, music, names)) in buses {
            let list = names.join(", ");
            println!("bus {bus}: {n} aliases ({music} music): {list}");
        }
    }
    if let Ok(want) = std::env::var("T6M2_ALIAS") {
        for c in captures {
            for b in &c.sound_banks {
                for a in b.aliases.iter().filter(|a| a.name.contains(&want)) {
                    let e = a.entries.first();
                    println!(
                        "alias {} ({}): {} variants, flags {:?}, file {}, secondary '{}', ids {:?}",
                        a.name,
                        c.zone,
                        a.entries.len(),
                        e.map(|e| e.flags),
                        e.map_or("", |e| e.asset_file.as_str()),
                        e.map_or("", |e| e.secondary.as_str()),
                        a.entries
                            .iter()
                            .map(|e| format!("{:#x}", e.asset_id))
                            .collect::<Vec<_>>()
                    );
                    if std::env::var_os("T6M2_FIND_BANK").is_some() {
                        for e in &a.entries {
                            let mut homes = Vec::new();
                            if let Ok(dir) = std::fs::read_dir(root.join("sound")) {
                                for f in dir.flatten() {
                                    if let Ok(bank) = sab_t6::BankFile::open(&f.path())
                                        && bank.entries.iter().any(|x| x.id == e.asset_id)
                                    {
                                        homes.push(f.file_name().to_string_lossy().into_owned());
                                    }
                                }
                            }
                            println!(
                                "   variant {:#010x} {} -> {:?}",
                                e.asset_id, e.asset_file, homes
                            );
                        }
                    }
                }
            }
        }
    }
    println!(
        "aliases {total}, variants {entries}, file resolved {resolved}, unresolved {}",
        entries - resolved
    );
    for n in &unresolved_names {
        println!("  unresolved: {n}");
    }
    if decode {
        println!("{shown}", shown = decode_all(&index).line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_read_little_endian_and_pad_with_nan() {
        let bytes: Vec<u8> = [1.5f32, -2.0, 3.25].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(f32_in(&bytes, 4), -2.0);
        assert!(f32_in(&bytes, 10).is_nan());
        let run = f32_run::<4>(&bytes, 4);
        assert_eq!(run[..2], [-2.0, 3.25]);
        assert!(run[2].is_nan() && run[3].is_nan());
    }

    #[test]
    fn banks_must_match_a_family_and_a_language() {
        assert!(is_wanted_bank("zmb_nuked.all.sabs"));
        assert!(is_wanted_bank("cmn_root.english.sabl"));
        assert!(!is_wanted_bank("zmb_nuked.french.sabl"));
        assert!(!is_wanted_bank("mp_nuked.all.sabs"));
    }

    #[test]
    fn decode_line_reports_hours_of_audio() {
        let s = DecodeSummary { tally: [3, 1, 2], audio_frames: 48000 * 3600 * 2, secs: 4.25 };
        assert_eq!(
            s.line(),
            "decode: 3 ok, 1 md5 mismatch, 2 failed, 2.0 h of audio in 4.2s"
        );
    }
}
