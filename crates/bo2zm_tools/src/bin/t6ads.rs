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

use asset_t6::m2::{AttachmentRef, WeaponRef};
use asset_t6::{PackSet, ZoneCapture, capture_zone};
use fastfile_t6::layout::{WeaponAttachment, WeaponDef, WeaponVariantDef};

/// Zones read when `T6ADS_ZONES` is not set.
const DEFAULT_ZONES: [&str; 3] = ["common_mp", "common_patch_mp", "patch_mp"];

/// A run that already said why on its own.
struct Failed;

/// What to print for each weapon; the first switch set wins, in this order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Ammo,
    Perks,
    Range,
    Heat,
    Ads,
}

/// The environment switches that choose a [`View`] and the attachment extras.
#[derive(Default)]
struct Switches {
    attachments: bool,
    heat: bool,
    perks: bool,
    ammo: bool,
    range: bool,
}

impl Switches {
    fn from_env() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some();
        Switches {
            attachments: set("T6ADS_ATT"),
            heat: set("T6ADS_HEAT"),
            perks: set("T6ADS_PERKS"),
            ammo: set("T6ADS_AMMO"),
            range: set("T6ADS_RANGE"),
        }
    }

    fn view(&self) -> View {
        let ranked = [
            (self.ammo, View::Ammo),
            (self.perks, View::Perks),
            (self.range, View::Range),
            (self.heat, View::Heat),
        ];
        ranked.iter().find(|(on, _)| *on).map_or(View::Ads, |&(_, view)| view)
    }

    /// Only the plain weapon views go on to list the attachments.
    fn lists_attachments(&self) -> bool {
        self.attachments && !matches!(self.view(), View::Ammo | View::Perks)
    }
}

/// The zones to read: the comma-separated override, or the multiplayer defaults.
fn zone_list(override_text: Option<&str>) -> Vec<String> {
    match override_text {
        Some(text) => text.split(',').map(str::to_owned).collect(),
        None => DEFAULT_ZONES.iter().map(|z| (*z).to_owned()).collect(),
    }
}

/// Whether a weapon name holds the (already lower-case) filter text.
fn name_matches(name: &str, grep: &str) -> bool {
    name.to_ascii_lowercase().contains(grep)
}

fn matching<'z>(capture: &'z ZoneCapture, grep: &'z str) -> impl Iterator<Item = &'z WeaponRef> {
    capture.weapons.iter().filter(move |w| name_matches(&w.name, grep))
}

fn print_ammo(zone: &str, w: &WeaponRef) {
    let stow_kind = w.def_i32(WeaponDef::inventoryType);
    let fire = w.def_i32(WeaponDef::fireType);
    let count = w.var_i32(WeaponVariantDef::iVariantCount);
    if stow_kind != 3 && fire != 8 && count >= 0 {
        return;
    }
    println!(
        "{zone} {}: inventory {stow_kind} fireType {fire} variantCount {count} alt {} ({}) ignoreAtt {} dualWield {} clip {} ammo {}",
        w.name,
        w.var_i32(WeaponVariantDef::altWeaponIndex),
        w.alt_weapon_name,
        w.var_u8(WeaponVariantDef::bIgnoreAttachments),
        w.def_u8(WeaponDef::bDualWield),
        w.var_i32(WeaponVariantDef::iClipIndex),
        w.var_i32(WeaponVariantDef::iAmmoIndex),
    );
}

/// The report line for something granting perk bits, or `None` when it grants none.
fn perk_line(zone: &str, kind: &str, name: &str, mask: u64) -> Option<String> {
    (mask != 0).then(|| format!("{zone} {kind} {name}: perks {mask:#x} {}", bits(mask)))
}

fn print_perks(zone: &str, capture: &ZoneCapture, grep: &str) {
    let from_weapons = matching(capture, grep).map(|w| {
        let mask = perk_bits(w.var_i32(WeaponVariantDef::perks), w.var_i32(WeaponVariantDef::perks + 4));
        perk_line(zone, "weapon", &w.name, mask)
    });
    let from_attachments = capture.attachments.iter().map(|at| {
        let mask = perk_bits(at.int_from_bytes(WeaponAttachment::perks), at.int_from_bytes(WeaponAttachment::perks + 4));
        perk_line(zone, "att", &at.name, mask)
    });
    from_weapons.chain(from_attachments).flatten().for_each(|text| println!("{text}"));
}

/// The six "damage@range" points of a weapon, space-separated.
fn damage_points(w: &WeaponRef) -> String {
    let point = |i: usize| format!("{}@{}", w.def_i32(WeaponDef::damage + i * 4), w.def_f32(WeaponDef::damageRange + i * 4));
    (0..6).map(point).collect::<Vec<_>>().join(" ")
}

fn print_range(zone: &str, w: &WeaponRef) {
    println!(
        "{zone} {}: class {} fireType {} shots {} damage {} impactExplode {} inner {}",
        w.name,
        w.def_i32(WeaponDef::weapClass),
        w.def_i32(WeaponDef::fireType),
        w.def_i32(WeaponDef::shotCount),
        damage_points(w),
        w.def_u8(WeaponDef::bBulletImpactExplode),
        w.def_i32(WeaponDef::iExplosionInnerDamage),
    );
}

fn print_heat(zone: &str, w: &WeaponRef) {
    if w.def_i32(WeaponDef::overheatWeapon) == 0 {
        return;
    }
    println!(
        "{zone} {}: overheat rate {} cooldown {} endVal {} coolWhileFiring {} sound {:?} player sound {:?}",
        w.name,
        w.def_f32(WeaponDef::overheatRate),
        w.def_f32(WeaponDef::cooldownRate),
        w.def_f32(WeaponDef::overheatEndVal),
        w.def_u8(WeaponDef::coolWhileFiring),
        w.def_string(WeaponDef::overheatSound),
        w.def_string(WeaponDef::overheatSoundPlayer),
    );
}

fn print_ads(zone: &str, w: &WeaponRef) {
    println!(
        "{zone} {}: type {} class {} animType {} ads {} in {} out {} zoomIn {} zoomOut {} xhairIn {} xhairOut {} overlay {} breath {} adsFireOnly {} noAdsEmpty {} reloadWhileAds {} rechamberWhileAds {} posReloadTrans {}",
        w.name,
        w.def_i32(WeaponDef::weapType),
        w.def_i32(WeaponDef::weapClass),
        w.def_i32(WeaponDef::playerAnimType),
        w.def_u8(WeaponDef::aimDownSight),
        w.var_i32(WeaponVariantDef::iAdsTransInTime),
        w.var_i32(WeaponVariantDef::iAdsTransOutTime),
        w.var_f32(WeaponVariantDef::fAdsZoomInFrac),
        w.var_f32(WeaponVariantDef::fAdsZoomOutFrac),
        w.def_f32(WeaponDef::fAdsCrosshairInFrac),
        w.def_f32(WeaponDef::fAdsCrosshairOutFrac),
        w.def_i32(WeaponDef::overlayReticle),
        w.def_u8(WeaponDef::bHoldBreathToSteady),
        w.def_u8(WeaponDef::adsFireOnly),
        w.def_u8(WeaponDef::noAdsWhenMagEmpty),
        w.def_u8(WeaponDef::bReloadWhileAds),
        w.def_u8(WeaponDef::bRechamberWhileAds),
        w.def_i32(WeaponDef::iPositionReloadTransTime),
    );
    println!(
        "  aqs {} time {} scale {} spreadMul {} spreadMax {} sway {} | altAdsOnly {} altIn {} altOut {} | idle hip {} ads {} | fov {} {} {}",
        w.var_u8(WeaponVariantDef::bAntiQuickScope),
        w.def_f32(WeaponDef::fAntiQuickScopeTime),
        w.def_f32(WeaponDef::fAntiQuickScopeScale),
        w.def_f32(WeaponDef::fAntiQuickScopeSpreadMultiplier),
        w.def_f32(WeaponDef::fAntiQuickScopeSpreadMax),
        w.def_f32(WeaponDef::fAntiQuickScopeSwayFactor),
        w.def_u8(WeaponDef::bAltWeaponAdsOnly),
        w.def_f32(WeaponDef::altScopeADSTransInTime),
        w.def_f32(WeaponDef::altScopeADSTransOutTime),
        w.def_f32(WeaponDef::fHipIdleAmount),
        w.def_f32(WeaponDef::fAdsIdleAmount),
        w.var_f32(WeaponVariantDef::fAdsZoomFov1),
        w.var_f32(WeaponVariantDef::fAdsZoomFov2),
        w.var_f32(WeaponVariantDef::fAdsZoomFov3),
    );
}

fn print_attachment(zone: &str, at: &AttachmentRef) {
    println!(
        "{zone} att {}: inScale {} outScale {} zoomIn {} zoomOut {} altAdsOnly {} altIn {} altOut {} idleScale {} moveScale {}",
        at.name,
        at.float_from_bytes(WeaponAttachment::fAdsTransInTimeScale),
        at.float_from_bytes(WeaponAttachment::fAdsTransOutTimeScale),
        at.float_from_bytes(WeaponAttachment::fAdsZoomInFrac),
        at.float_from_bytes(WeaponAttachment::fAdsZoomOutFrac),
        at.byte_field(WeaponAttachment::bAltWeaponAdsOnly),
        at.float_from_bytes(WeaponAttachment::altScopeADSTransInTime),
        at.float_from_bytes(WeaponAttachment::altScopeADSTransOutTime),
        at.float_from_bytes(WeaponAttachment::fAdsIdleAmountScale),
        at.float_from_bytes(WeaponAttachment::adsMoveSpeedScale),
    );
}

/// Prints everything one zone's capture has for the chosen view.
fn print_zone(zone: &str, capture: &ZoneCapture, grep: &str, switches: &Switches) {
    let view = switches.view();
    match view {
        View::Perks => print_perks(zone, capture, grep),
        _ => {
            for w in matching(capture, grep).filter(|w| !w.def.is_empty()) {
                match view {
                    View::Ammo => print_ammo(zone, w),
                    View::Range => print_range(zone, w),
                    View::Heat => print_heat(zone, w),
                    _ => print_ads(zone, w),
                }
            }
        }
    }
    if switches.lists_attachments() {
        capture.attachments.iter().for_each(|at| print_attachment(zone, at));
    }
}

fn main() {
    std::process::exit(match drive() {
        Ok(()) => 0,
        Err(Failed) => 1,
    });
}

fn drive() -> Result<(), Failed> {
    let grep = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let dir = &bo2zm_tools::zones_dir();
    PackSet::open_dir(dir).map_err(|e| {
        eprintln!("packs: {e}");
        Failed
    })?;
    let zones = zone_list(std::env::var("T6ADS_ZONES").ok().as_deref());
    let switches = Switches::from_env();
    for zone in &zones {
        match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(capture) => print_zone(zone, &capture, &grep, &switches),
            Err(e) => eprintln!("{zone}: {e}"),
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_follows_the_switch_priority() {
        assert_eq!(Switches::default().view(), View::Ads);
        let heat = Switches { heat: true, ..Switches::default() };
        assert_eq!(heat.view(), View::Heat);
        let heat_and_range = Switches { heat: true, range: true, ..Switches::default() };
        assert_eq!(heat_and_range.view(), View::Range);
        let all = Switches { heat: true, range: true, perks: true, ammo: true, attachments: true };
        assert_eq!(all.view(), View::Ammo);
        assert!(!all.lists_attachments());
    }

    #[test]
    fn attachments_follow_plain_views_only() {
        let heat = Switches { attachments: true, heat: true, ..Switches::default() };
        assert!(heat.lists_attachments());
        assert!(!Switches::default().lists_attachments());
    }

    #[test]
    fn zone_list_prefers_the_override() {
        assert_eq!(zone_list(Some("a,b")), ["a", "b"]);
        assert_eq!(zone_list(None), ["common_mp", "common_patch_mp", "patch_mp"]);
    }

    #[test]
    fn names_match_ignoring_case() {
        assert!(name_matches("AK47_mp", "ak"));
        assert!(!name_matches("ak47_mp", "m16"));
        assert!(name_matches("anything", ""));
    }

    #[test]
    fn perk_lines_exist_only_for_granted_perks() {
        assert_eq!(perk_line("z", "att", "grip", 0), None);
        assert_eq!(perk_line("z", "weapon", "ak", 0b101).as_deref(), Some("z weapon ak: perks 0x5 0,2"));
    }

    #[test]
    fn perk_words_join_into_set_bits() {
        assert_eq!(perk_bits(1, 1), 1 | (1 << 32));
        assert_eq!(perk_bits(-1, 0), 0xFFFF_FFFF);
        assert_eq!(bits(0b1010), "1,3");
        assert_eq!(bits(0), "");
    }
}
