//! bo2mp gun brain window 10: what a Black Ops II weapon's definition says
//! about aiming down the sights, read-only. `t6ads [grep]` prints, for every
//! weapon in the multiplayer zones (`T6ADS_ZONES` to change them) whose name
//! holds `grep`: aim in / out time, zoom and crosshair fractions, overlay,
//! anti-quickscope numbers, hold breath, reload / rechamber while aiming,
//! the alt-optic (Hybrid) fields and idle sway. `T6ADS_ATT=1` also prints
//! every attachment's ADS scales and overrides.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{PackSet, capture_zone};
use fastfile_t6::layout::{WeaponAttachment as a, WeaponDef as d, WeaponVariantDef as v};

const ZONES: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Black Ops II\zone\all";

fn main() -> ExitCode {
    let grep = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let dir = Path::new(ZONES);
    if let Err(e) = PackSet::open_dir(dir) {
        eprintln!("packs: {e}");
        return ExitCode::FAILURE;
    }
    let zones: Vec<String> = std::env::var("T6ADS_ZONES").map_or_else(
        |_| ["common_mp", "common_patch_mp", "patch_mp"].map(str::to_owned).to_vec(),
        |z| z.split(',').map(str::to_owned).collect(),
    );
    let show_att = std::env::var_os("T6ADS_ATT").is_some();
    for zone in &zones {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for w in capture.weapons.iter().filter(|w| w.name.to_ascii_lowercase().contains(&grep)) {
            if w.def.is_empty() {
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
                    "{zone} att {}: inScale {} outScale {} zoomIn {} zoomOut {} altAdsOnly {} altIn {} altOut {} idleScale {}",
                    at.name,
                    at.f32_at(a::fAdsTransInTimeScale),
                    at.f32_at(a::fAdsTransOutTimeScale),
                    at.f32_at(a::fAdsZoomInFrac),
                    at.f32_at(a::fAdsZoomOutFrac),
                    at.u8_at(a::bAltWeaponAdsOnly),
                    at.f32_at(a::altScopeADSTransInTime),
                    at.f32_at(a::altScopeADSTransOutTime),
                    at.f32_at(a::fAdsIdleAmountScale),
                );
            }
        }
    }
    ExitCode::SUCCESS
}
