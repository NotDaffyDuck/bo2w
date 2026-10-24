//! bo2mp gun brain window 10: what a Black Ops II weapon's definition says
//! about aiming down the sights, read-only. `t6ads [grep]` prints, for every
//! weapon in the multiplayer zones (`T6ADS_ZONES` to change them) whose name
//! holds `grep`: aim in / out time, zoom and crosshair fractions, overlay,
//! anti-quickscope numbers, hold breath, reload / rechamber while aiming,
//! the alt-optic (Hybrid) fields and idle sway. `T6ADS_ATT=1` also prints
//! every attachment's ADS scales and overrides. `T6ADS_HEAT=1` prints only
//! the weapons that overheat, with their heat and cooldown rates.
//! `T6ADS_PERKS=1` prints only the weapons and attachments that grant perks
//! (BG_GetPerks' sources). `T6ADS_AMMO=1` prints the alt-mode, charge-shot
//! and variant-linked weapons (the ones BG_GetWeaponForAmmoClip / Pool remap).
//! `T6ADS_RANGE=1` prints every weapon's class, fire type, shot count, six damage points
//! (Bullet_Fire's pellet range and Bullet_GetDamage's falloff) and whether its bullets blow
//! up (Bullet_Process's G_RadiusDamage; Bullet_Fire's choice of Bullet_FireExtended).

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{PackSet, capture_zone};
use fastfile_t6::layout::{WeaponAttachment as a, WeaponDef as d, WeaponVariantDef as v};

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let grep = std::env::args()
        .nth(1)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dir = Path::new(ZONES);
    if let Err(e) = PackSet::open_dir(dir) {
        eprintln!("packs: {e}");
        return ExitCode::FAILURE;
    }
    let zones: Vec<String> = std::env::var("T6ADS_ZONES").map_or_else(
        |_| {
            ["common_mp", "common_patch_mp", "patch_mp"]
                .map(str::to_owned)
                .to_vec()
        },
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let show_att = std::env::var_os("T6ADS_ATT").is_some();
    let heat_only = std::env::var_os("T6ADS_HEAT").is_some();
    let show_perks = std::env::var_os("T6ADS_PERKS").is_some();
    let show_ammo = std::env::var_os("T6ADS_AMMO").is_some();
    let show_range = std::env::var_os("T6ADS_RANGE").is_some();
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        if show_ammo {
            for w in capture
                .weapons
                .iter()
                .filter(|w| w.name.to_ascii_lowercase().contains(&grep))
            {
                if w.def.is_empty() {
                    continue;
                }
                let inv = w.def_i32(d::inventoryType);
                let fire = w.def_i32(d::fireType);
                let count = w.var_i32(v::iVariantCount);
                if inv == 3 || fire == 8 || count < 0 {
                    println!(
                        "{zone} {}: inventory {inv} fireType {fire} variantCount {count} alt {} ({}) ignoreAtt {} dualWield {} clip {} ammo {}",
                        w.name,
                        w.var_i32(v::altWeaponIndex),
                        w.alt_weapon_name,
                        w.var_u8(v::bIgnoreAttachments),
                        w.def_u8(d::bDualWield),
                        w.var_i32(v::iClipIndex),
                        w.var_i32(v::iAmmoIndex),
                    );
                }
            }
            continue;
        }
        if show_perks {
            for w in capture
                .weapons
                .iter()
                .filter(|w| w.name.to_ascii_lowercase().contains(&grep))
            {
                let perks = perk_bits(w.var_i32(v::perks), w.var_i32(v::perks + 4));
                if perks != 0 {
                    println!("{zone} weapon {}: perks {perks:#x} {}", w.name, bits(perks));
                }
            }
            for at in &capture.attachments {
                let perks = perk_bits(at.i32_at(a::perks), at.i32_at(a::perks + 4));
                if perks != 0 {
                    println!("{zone} att {}: perks {perks:#x} {}", at.name, bits(perks));
                }
            }
            continue;
        }
        for w in capture
            .weapons
            .iter()
            .filter(|w| w.name.to_ascii_lowercase().contains(&grep))
        {
            if w.def.is_empty() {
                continue;
            }
            if show_range {
                let pts: Vec<String> = (0..6)
                    .map(|i| {
                        format!(
                            "{}@{}",
                            w.def_i32(d::damage + i * 4),
                            w.def_f32(d::damageRange + i * 4)
                        )
                    })
                    .collect();
                println!(
                    "{zone} {}: class {} fireType {} shots {} damage {} impactExplode {} inner {}",
                    w.name,
                    w.def_i32(d::weapClass),
                    w.def_i32(d::fireType),
                    w.def_i32(d::shotCount),
                    pts.join(" "),
                    w.def_u8(d::bBulletImpactExplode),
                    w.def_i32(d::iExplosionInnerDamage),
                );
                continue;
            }
            if heat_only {
                if w.def_i32(d::overheatWeapon) != 0 {
                    println!(
                        "{zone} {}: overheat rate {} cooldown {} endVal {} coolWhileFiring {} sound {:?} player sound {:?}",
                        w.name,
                        w.def_f32(d::overheatRate),
                        w.def_f32(d::cooldownRate),
                        w.def_f32(d::overheatEndVal),
                        w.def_u8(d::coolWhileFiring),
                        w.def_string(d::overheatSound),
                        w.def_string(d::overheatSoundPlayer),
                    );
                }
                continue;
            }
            println!(
                "{zone} {}: type {} class {} animType {} ads {} in {} out {} zoomIn {} zoomOut {} xhairIn {} xhairOut {} overlay {} breath {} adsFireOnly {} noAdsEmpty {} reloadWhileAds {} rechamberWhileAds {} posReloadTrans {}",
                w.name,
                w.def_i32(d::weapType),
                w.def_i32(d::weapClass),
                w.def_i32(d::playerAnimType),
                w.def_u8(d::aimDownSight),
                w.var_i32(v::iAdsTransInTime),
                w.var_i32(v::iAdsTransOutTime),
                w.var_f32(v::fAdsZoomInFrac),
                w.var_f32(v::fAdsZoomOutFrac),
                w.def_f32(d::fAdsCrosshairInFrac),
                w.def_f32(d::fAdsCrosshairOutFrac),
                w.def_i32(d::overlayReticle),
                w.def_u8(d::bHoldBreathToSteady),
                w.def_u8(d::adsFireOnly),
                w.def_u8(d::noAdsWhenMagEmpty),
                w.def_u8(d::bReloadWhileAds),
                w.def_u8(d::bRechamberWhileAds),
                w.def_i32(d::iPositionReloadTransTime),
            );
            println!(
                "  aqs {} time {} scale {} spreadMul {} spreadMax {} sway {} | altAdsOnly {} altIn {} altOut {} | idle hip {} ads {} | fov {} {} {}",
                w.var_u8(v::bAntiQuickScope),
                w.def_f32(d::fAntiQuickScopeTime),
                w.def_f32(d::fAntiQuickScopeScale),
                w.def_f32(d::fAntiQuickScopeSpreadMultiplier),
                w.def_f32(d::fAntiQuickScopeSpreadMax),
                w.def_f32(d::fAntiQuickScopeSwayFactor),
                w.def_u8(d::bAltWeaponAdsOnly),
                w.def_f32(d::altScopeADSTransInTime),
                w.def_f32(d::altScopeADSTransOutTime),
                w.def_f32(d::fHipIdleAmount),
                w.def_f32(d::fAdsIdleAmount),
                w.var_f32(v::fAdsZoomFov1),
                w.var_f32(v::fAdsZoomFov2),
                w.var_f32(v::fAdsZoomFov3),
            );
        }
        if show_att {
            for at in &capture.attachments {
                println!(
                    "{zone} att {}: inScale {} outScale {} zoomIn {} zoomOut {} altAdsOnly {} altIn {} altOut {} idleScale {} moveScale {}",
                    at.name,
                    at.f32_at(a::fAdsTransInTimeScale),
                    at.f32_at(a::fAdsTransOutTimeScale),
                    at.f32_at(a::fAdsZoomInFrac),
                    at.f32_at(a::fAdsZoomOutFrac),
                    at.u8_at(a::bAltWeaponAdsOnly),
                    at.f32_at(a::altScopeADSTransInTime),
                    at.f32_at(a::altScopeADSTransOutTime),
                    at.f32_at(a::fAdsIdleAmountScale),
                    at.f32_at(a::adsMoveSpeedScale),
                );
            }
        }
    }
    ExitCode::SUCCESS
}

fn perk_bits(lo: i32, hi: i32) -> u64 {
    u64::from(lo as u32) | (u64::from(hi as u32) << 32)
}

fn bits(perks: u64) -> String {
    (0..64)
        .filter(|b| perks & (1u64 << b) != 0)
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",")
}
