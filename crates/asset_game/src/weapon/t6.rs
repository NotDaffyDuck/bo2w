//! bo2zm: Black Ops II weapons into the weapon catalog.
//!
//! `asset_t6::WeaponRef` carries a weapon's variant and shared definition as
//! bytes plus every name they point at; this reads the fields the engine's
//! weapon rules, viewmodel and sounds use, by their T6 offsets. Field
//! meanings follow the legacy and Black Ops ones of the same name; where T6
//! numbers an enum differently (class, offhand class, type), it is mapped
//! to the legacy value the engine expects.

use fastfile_t6::layout::{WeaponDef as d, WeaponVariantDef as v};
use weapon_t6::asset_slots::weap_anim_extra;
use weapon_t6::placement::{WeaponIdleInputs, WeaponMovementOfsInputs};

use super::*;

/// T6 `weapAnimFiles_t` slot -> the catalog's slot (legacy numbering plus the
/// quick-reload extras). Slots legacy has no use for are left out.
const T6_ANIM_SLOTS: [(usize, usize); 59] = [
    (0x01, weap_anim::IDLE),
    (0x02, weap_anim::EMPTY_IDLE),
    (0x03, weap_anim_extra::FIRE_INTRO),
    (0x04, weap_anim::FIRE),
    (0x05, weap_anim::HOLD_FIRE),
    (0x06, weap_anim::LASTSHOT),
    (0x08, weap_anim::RECHAMBER),
    (0x09, weap_anim::MELEE),
    (0x0D, weap_anim_extra::MELEE_EMPTY),
    (0x0E, weap_anim::MELEE_CHARGE),
    (0x0F, weap_anim_extra::MELEE_CHARGE_EMPTY),
    (0x10, weap_anim::RELOAD),
    (0x12, weap_anim::RELOAD_EMPTY),
    (0x13, weap_anim::RELOAD_START),
    (0x14, weap_anim::RELOAD_END),
    (0x15, weap_anim_extra::RELOAD_QUICK),
    (0x16, weap_anim_extra::RELOAD_QUICK_EMPTY),
    (0x17, weap_anim::RAISE),
    (0x18, weap_anim::FIRST_RAISE),
    (0x19, weap_anim::DROP),
    (0x1A, weap_anim::ALT_RAISE),
    (0x1B, weap_anim::ALT_DROP),
    (0x1C, weap_anim::QUICK_RAISE),
    (0x1D, weap_anim::QUICK_DROP),
    (0x1E, weap_anim::EMPTY_RAISE),
    (0x1F, weap_anim::EMPTY_DROP),
    (0x20, weap_anim::SPRINT_IN),
    (0x21, weap_anim::SPRINT_LOOP),
    (0x22, weap_anim::SPRINT_OUT),
    (0x23, weap_anim_extra::SPRINT_IN_EMPTY),
    (0x24, weap_anim_extra::SPRINT_LOOP_EMPTY),
    (0x25, weap_anim_extra::SPRINT_OUT_EMPTY),
    (0x2C, weap_anim_extra::CRAWL_IN),
    (0x2D, weap_anim_extra::CRAWL_FORWARD),
    (0x2E, weap_anim_extra::CRAWL_BACK),
    (0x2F, weap_anim_extra::CRAWL_RIGHT),
    (0x30, weap_anim_extra::CRAWL_LEFT),
    (0x31, weap_anim_extra::CRAWL_OUT),
    (0x32, weap_anim_extra::CRAWL_IN_EMPTY),
    (0x33, weap_anim_extra::CRAWL_FORWARD_EMPTY),
    (0x34, weap_anim_extra::CRAWL_BACK_EMPTY),
    (0x35, weap_anim_extra::CRAWL_RIGHT_EMPTY),
    (0x36, weap_anim_extra::CRAWL_LEFT_EMPTY),
    (0x37, weap_anim_extra::CRAWL_OUT_EMPTY),
    (0x3A, weap_anim::DETONATE),
    (0x3B, weap_anim::NIGHTVISION_WEAR),
    (0x3C, weap_anim::NIGHTVISION_REMOVE),
    (0x3D, weap_anim::ADS_FIRE),
    (0x3E, weap_anim::ADS_LASTSHOT),
    (0x3F, weap_anim_extra::ADS_FIRE_INTRO),
    (0x40, weap_anim::ADS_RECHAMBER),
    (0x41, weap_anim_extra::DIVE_IN),
    (0x42, weap_anim_extra::DIVE_LOOP),
    (0x43, weap_anim_extra::DIVE_OUT),
    (0x44, weap_anim_extra::DIVE_IN_EMPTY),
    (0x45, weap_anim_extra::DIVE_LOOP_EMPTY),
    (0x46, weap_anim_extra::DIVE_OUT_EMPTY),
    (0x4D, weap_anim_extra::CAMERA_MANTLE),
    (0x55, weap_anim::ADS_UP),
];

/// T6 asset names are case-insensitive (the knife names
/// `viewmodel_M4m203_knife_melee_1`, the zone holds `viewmodel_m4m203_...`);
/// animations are keyed lower case.
fn remap_t6_sz_xanims(t6: &[String]) -> [Option<String>; WEAPON_ANIM_SLOTS] {
    let mut out = [const { None }; WEAPON_ANIM_SLOTS];
    for (src, dst) in T6_ANIM_SLOTS {
        if let Some(name) = t6.get(src).filter(|n| !n.is_empty()) {
            out[dst] = Some(name.to_ascii_lowercase());
        }
    }
    // ADS down (0x56) has no legacy slot of its own beside ADS up's pair.
    if let Some(name) = t6.get(0x56).filter(|n| !n.is_empty()) {
        out[weap_anim::ADS_DOWN] = Some(name.to_ascii_lowercase());
    }
    // A gun without empty-magazine dive, sprint, crawl or melee animations
    // (the M14 has no empty dive) plays its full ones; one without a
    // first-shots fire plays its fire.
    for (empty, full) in [
        (weap_anim_extra::DIVE_IN_EMPTY, weap_anim_extra::DIVE_IN),
        (weap_anim_extra::DIVE_LOOP_EMPTY, weap_anim_extra::DIVE_LOOP),
        (weap_anim_extra::DIVE_OUT_EMPTY, weap_anim_extra::DIVE_OUT),
        (weap_anim_extra::SPRINT_IN_EMPTY, weap_anim::SPRINT_IN),
        (weap_anim_extra::SPRINT_LOOP_EMPTY, weap_anim::SPRINT_LOOP),
        (weap_anim_extra::SPRINT_OUT_EMPTY, weap_anim::SPRINT_OUT),
        (weap_anim_extra::CRAWL_IN_EMPTY, weap_anim_extra::CRAWL_IN),
        (weap_anim_extra::CRAWL_FORWARD_EMPTY, weap_anim_extra::CRAWL_FORWARD),
        (weap_anim_extra::CRAWL_BACK_EMPTY, weap_anim_extra::CRAWL_BACK),
        (weap_anim_extra::CRAWL_RIGHT_EMPTY, weap_anim_extra::CRAWL_RIGHT),
        (weap_anim_extra::CRAWL_LEFT_EMPTY, weap_anim_extra::CRAWL_LEFT),
        (weap_anim_extra::CRAWL_OUT_EMPTY, weap_anim_extra::CRAWL_OUT),
        (weap_anim_extra::MELEE_EMPTY, weap_anim::MELEE),
        (weap_anim_extra::MELEE_CHARGE_EMPTY, weap_anim::MELEE_CHARGE),
        (weap_anim_extra::FIRE_INTRO, weap_anim::FIRE),
        (weap_anim_extra::ADS_FIRE_INTRO, weap_anim::ADS_FIRE),
    ] {
        if out[empty].is_none() {
            out[empty] = out[full].clone();
        }
    }
    out
}

/// bo2zm: a dual-wield pair's left-hand animations (the left weapon's
/// `dw_left_*` slots) on the catalog's slots: its other slots (raise, drop,
/// sprint) as they are, then fire 0x4E, last shot 0x4F, idle 0x51, empty
/// idle 0x52, empty reload 0x53, reload 0x54 (measured on fivesevenlh_zm and
/// m1911lh_upgraded_zm).
fn remap_t6_left_hand_xanims(t6: &[String]) -> [Option<String>; WEAPON_ANIM_SLOTS] {
    let mut out = remap_t6_sz_xanims(t6);
    for (src, dst) in [
        (0x4E, weap_anim::FIRE),
        (0x4F, weap_anim::LASTSHOT),
        (0x51, weap_anim::IDLE),
        (0x52, weap_anim::EMPTY_IDLE),
        (0x53, weap_anim::RELOAD_EMPTY),
        (0x54, weap_anim::RELOAD),
    ] {
        if let Some(name) = t6.get(src).filter(|n| !n.is_empty()) {
            out[dst] = Some(name.to_ascii_lowercase());
        }
    }
    out
}

/// T6 `weapClass_t` -> legacy. T6: rifle, mg, smg, spread, pistol, grenade,
/// rocket launcher, turret, non-player, gas, item, melee, killstreak alt
/// stored weapon, pistol spread. legacy: rifle, sniper, mg, smg, spread,
/// pistol, grenade, rocket launcher, turret, throwing knife, non-player,
/// item.
fn remap_t6_weap_class(raw: i32) -> i32 {
    match raw {
        0 => 0,
        1 => 2,
        2 => 3,
        3 => 4,
        4 => 5,
        5 => 6,
        6 => 7,
        7 => 8,
        8 => 10,
        9 => 10,
        10 => 11,
        11 => 11,
        13 => 4,
        other => other,
    }
}

/// T6 `weapType_t` -> legacy (bullet, grenade, projectile, riot shield).
/// Melee, gas, bomb and mine weapons are not fired like guns.
fn remap_t6_weap_type(raw: i32) -> i32 {
    match raw {
        0 => 0,
        1 => 1,
        2 => 2,
        8 => 3,
        4 | 5 | 6 => 1,
        _ => 0,
    }
}

/// T6 `OffhandClass`: none, frag, smoke, flash, gear, supply drop marker.
fn remap_t6_offhand_class(raw: i32) -> i32 {
    match raw {
        4 | 5 => 5,
        other => other,
    }
}

/// T6 `weapFireType_t`: full auto, single, burst 2..5, stacked, minigun,
/// charge shot, jet gun. legacy knows full auto, single and bursts 2..4.
fn remap_t6_fire_type(raw: i32) -> i32 {
    match raw {
        0..=4 => raw,
        5 => 4,
        7 | 9 => 0,
        _ => 1,
    }
}

fn opt(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Bullet penetration (`perksEnum` 6, PERK_EXTRABP) in the low word of a perk mask.
const T6_PERK_BULLET_PENETRATION: i32 = 1 << 6;

fn t6_body_facts(w: &asset_t6::WeaponRef) -> WeaponBodyFacts {
    let mut f = WeaponBodyFacts {
        body_resolved: !w.def.is_empty(),
        clip_size: w.var_i32(v::iClipSize),
        reload_time_ms: w.var_i32(v::iReloadTime),
        reload_empty_time_ms: w.var_i32(v::iReloadEmptyTime),
        alternate_raise_time_ms: w.var_i32(v::iAltRaiseTime),
        aim_assist_range_ads: w.var_f32(v::fAimAssistRangeAds),
        ads_zoom_in_frac: w.var_f32(v::fAdsZoomInFrac),
        ads_zoom_out_frac: w.var_f32(v::fAdsZoomOutFrac),
        overlay_alpha_scale: w.var_f32(v::fOverlayAlphaScale),
        select_requires_ammo: Some(false),
        inherits_perks: true,
        ..WeaponBodyFacts::default()
    };
    let ads_in = w.var_i32(v::iAdsTransInTime);
    let ads_out = w.var_i32(v::iAdsTransOutTime);
    f.ads_in_rate = if ads_in > 0 { 1.0 / ads_in as f32 } else { 0.0 };
    f.ads_out_rate = if ads_out > 0 { 1.0 / ads_out as f32 } else { 0.0 };
    f.ads_zoom_fov = [v::fAdsZoomFov1, v::fAdsZoomFov2, v::fAdsZoomFov3]
        .into_iter()
        .map(|o| w.var_f32(o))
        .find(|&fov| fov > 0.0)
        .unwrap_or(0.0);
    if w.var_u8(v::bDualMag) != 0 {
        f.dual_mag = Some(weapon_t6::dual_mag::DualMagTimes {
            reload_ms: w.var_i32(v::iReloadQuickTime),
            reload_empty_ms: w.var_i32(v::iReloadQuickEmptyTime),
            add_ms: w.def_i32(d::iReloadQuickAddTime),
            empty_add_ms: w.def_i32(d::iReloadQuickEmptyAddTime),
        });
    }
    if w.def.is_empty() {
        return f;
    }
    f.fire_time_ms = w.def_i32(d::iFireTime);
    f.intro_fire_time_ms = w.def_i32(d::iIntroFireTime);
    f.intro_fire_length = w.def_i32(d::iIntroFireLength);
    f.impact_type = w.def_i32(d::impactType);
    f.raise_time_ms = w.def_i32(d::iRaiseTime);
    f.drop_time_ms = w.def_i32(d::iDropTime);
    f.alternate_drop_time_ms = w.def_i32(d::iAltDropTime);
    f.fire_delay_ms = w.def_i32(d::iFireDelay);
    f.hold_fire_time_ms = w.def_i32(d::iHoldFireTime);
    f.weap_type = remap_t6_weap_type(w.def_i32(d::weapType));
    f.player_anim_type = w.def_i32(d::playerAnimType);
    f.weap_class = remap_t6_weap_class(w.def_i32(d::weapClass));
    f.offhand_class = remap_t6_offhand_class(w.def_i32(d::offhandClass));
    // BG_IsQuickSwitch: offhand slot not NONE, or bCanUseInVehicle; target
    // class PISTOL (4), MELEE (11) or PISTOL_SPREAD (13).
    f.quick_switch = w.def_i32(d::offhandSlot) != 0 || w.def_u8(d::bCanUseInVehicle) != 0;
    f.quick_switch_target = matches!(w.def_i32(d::weapClass), 4 | 11 | 13);
    f.shots_per_fire = w.def_i32(d::shotCount);
    f.low_ammo_warning_threshold = w.def_f32(d::lowAmmoWarningThreshold);
    f.hip_spread_stand_min = w.def_f32(d::fHipSpreadStandMin);
    f.hip_spread_ducked_min = w.def_f32(d::fHipSpreadDuckedMin);
    f.hip_spread_prone_min = w.def_f32(d::fHipSpreadProneMin);
    f.hip_spread_stand_max = w.def_f32(d::hipSpreadStandMax);
    f.hip_spread_ducked_max = w.def_f32(d::hipSpreadDuckedMax);
    f.hip_spread_prone_max = w.def_f32(d::hipSpreadProneMax);
    f.hip_spread_decay_rate = w.def_f32(d::fHipSpreadDecayRate);
    f.hip_spread_fire_add = w.def_f32(d::fHipSpreadFireAdd);
    f.hip_spread_turn_add = w.def_f32(d::fHipSpreadTurnAdd);
    f.hip_spread_move_add = w.def_f32(d::fHipSpreadMoveAdd);
    f.hip_spread_ducked_decay = w.def_f32(d::fHipSpreadDuckedDecay);
    f.hip_spread_prone_decay = w.def_f32(d::fHipSpreadProneDecay);
    f.i_reticle_side_size = w.def_i32(d::iReticleSideSize);
    f.i_reticle_min_ofs = w.def_i32(d::iReticleMinOfs);
    f.hip_reticle_side_pos = w.def_f32(d::fHipReticleSidePos);
    f.ads_aim_pitch = w.def_f32(d::fAdsAimPitch);
    f.ads_crosshair_in_frac = w.def_f32(d::fAdsCrosshairInFrac);
    f.ads_crosshair_out_frac = w.def_f32(d::fAdsCrosshairOutFrac);
    f.ads_spread = w.def_f32(d::fAdsSpread);
    f.aim_down_sight = w.def_u8(d::aimDownSight) != 0;
    f.hold_breath_to_steady = w.def_u8(d::bHoldBreathToSteady) != 0;
    let dof = [w.def_f32(d::adsDofStart), w.def_f32(d::adsDofEnd)];
    f.ads_dof = (dof[1] > 0.0).then_some(dof);
    f.no_ads_when_mag_empty = w.def_u8(d::noAdsWhenMagEmpty) != 0;
    f.rechamber_while_ads = w.def_u8(d::bRechamberWhileAds) != 0;
    f.reload_lowers_ads = w.def_u8(d::bReloadWhileAds) == 0;
    f.position_reload_trans_ms = w.def_i32(d::iPositionReloadTransTime);
    f.ads_fire_only = w.def_u8(d::adsFireOnly) != 0;
    f.melee_damage = w.def_i32(d::iMeleeDamage);
    f.overlay_reticle = w.def_i32(d::overlayReticle);
    f.overlay_interface = w.def_i32(d::overlayInterface);
    f.infrared = w.var_u8(v::bInfraRed) != 0;
    f.ads_overlay_width = w.def_f32(d::overlayWidth);
    f.ads_overlay_height = w.def_f32(d::overlayHeight);
    f.melee_time_ms = w.def_i32(d::iMeleeTime);
    f.melee_delay_ms = w.def_i32(d::iMeleeDelay);
    f.melee_charge_time_ms = w.def_i32(d::meleeChargeTime);
    f.melee_charge_delay_ms = w.def_i32(d::meleeChargeDelay);
    f.use_as_melee = w.def_u8(d::bUseAsMelee) != 0;
    f.quick_raise_time_ms = w.def_i32(d::quickRaiseTime);
    f.quick_drop_time_ms = w.def_i32(d::quickDropTime);
    f.offhand_hold_is_cancelable = Some(w.def_u8(d::offhandHoldIsCancelable) != 0);
    f.move_speed_scale = w.def_f32(d::moveSpeedScale);
    // BO2 starts the aimed walk scale at 1 and takes the gun's own only when it is set.
    f.ads_move_speed_scale = Some(w.def_f32(d::adsMoveSpeedScale)).filter(|s| *s > 0.0).unwrap_or(1.0);
    f.sprint_duration_scale = w.def_f32(d::sprintDurationScale);
    f.ducked_ofs = w.def_vec3(d::vDuckedOfs);
    f.prone_ofs = w.def_vec3(d::vProneOfs);
    f.night_vision_wear_time = w.def_i32(d::nightVisionWearTime);
    f.ads_bob_factor = w.def_f32(d::fAdsBobFactor);
    f.ads_view_bob_mult = w.def_f32(d::fAdsViewBobMult);
    f.movement = WeaponMovementOfsInputs {
        stand_move: w.def_vec3(d::vStandMove),
        stand_rot: w.def_vec3(d::vStandRot),
        strafe_move: w.def_vec3(d::vStrafeMove),
        strafe_rot: w.def_vec3(d::vStrafeRot),
        ducked_move: w.def_vec3(d::vDuckedMove),
        ducked_rot: w.def_vec3(d::vDuckedRot),
        prone_move: w.def_vec3(d::vProneMove),
        prone_rot: w.def_vec3(d::vProneRot),
        pos_move_rate: w.def_f32(d::fPosMoveRate),
        pos_prone_move_rate: w.def_f32(d::fPosProneMoveRate),
        stand_move_min_speed: w.def_f32(d::fStandMoveMinSpeed),
        ducked_move_min_speed: w.def_f32(d::fDuckedMoveMinSpeed),
        prone_move_min_speed: w.def_f32(d::fProneMoveMinSpeed),
        pos_rot_rate: w.def_f32(d::fPosRotRate),
        pos_prone_rot_rate: w.def_f32(d::fPosProneRotRate),
        mantle_ofs: w.def_vec3(d::vMantleOfs),
        mantle_rot: w.def_vec3(d::vMantleRot),
        ..WeaponMovementOfsInputs::default()
    };
    f.idle = WeaponIdleInputs {
        ads_idle_amount: w.def_f32(d::fAdsIdleAmount),
        hip_idle_amount: w.def_f32(d::fHipIdleAmount),
        ads_idle_speed: w.def_f32(d::adsIdleSpeed),
        hip_idle_speed: w.def_f32(d::hipIdleSpeed),
        idle_crouch_factor: w.def_f32(d::fIdleCrouchFactor),
        idle_prone_factor: w.def_f32(d::fIdleProneFactor),
    };
    f.penetrate_type = w.def_i32(d::penetrateTWeaponAttachmentype);
    // bo2mp: Bullet_FirePenetrate scales the depth by
    // perk_bulletPenetrationMultiplier when the firing weapon has bullet
    // penetration (perk 6): the variant's own perks here (Zombies' upgraded
    // Five-seven), an attachment's in `apply_t6_attachment_stats` (FMJ).
    f.penetrate_multiplier = if w.var_i32(v::perks) & T6_PERK_BULLET_PENETRATION != 0 {
        weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT
    } else {
        1.0
    };
    f.rifle_bullet = w.def_u8(d::bRifleBullet) != 0;
    let var_vec3 = |off: usize| [w.var_f32(off), w.var_f32(off + 4), w.var_f32(off + 8)];
    f.stowed_offset = var_vec3(v::stowedModelOffsets);
    f.stowed_rotation = var_vec3(v::stowedModelRotations);
    f.inventory_type = match w.def_i32(d::inventoryType) {
        t @ 0..=3 => t,
        _ => 2,
    };
    f.fire_type = remap_t6_fire_type(w.def_i32(d::fireType));
    f.max_ammo = w.def_i32(d::iMaxAmmo);
    f.start_ammo = w.def_i32(d::iStartAmmo);
    f.ammo_count_clip_relative = w.def_u8(d::ammoCountClipRelative) != 0;
    // T6 damage falls off over up to six ranges; the engine knows a near
    // and a far value.
    let damage: Vec<i32> = (0..6).map(|i| w.def_i32(d::damage + i * 4)).collect();
    let ranges: Vec<f32> = (0..6).map(|i| w.def_f32(d::damageRange + i * 4)).collect();
    f.damage = damage[0];
    f.max_damage_range = ranges[0];
    let last = (1..6).rev().find(|&i| ranges[i] > 0.0 && damage[i] > 0);
    match last {
        Some(i) => {
            f.min_damage = damage[i];
            f.min_damage_range = ranges[i];
        }
        None => {
            f.min_damage = damage[0];
            f.min_damage_range = ranges[0];
        }
    }
    let step_ranges: [f32; 6] = std::array::from_fn(|i| ranges[i]);
    let backwards: Vec<usize> = weapon_t6::damage::DamageSteps::backwards_ranges(&step_ranges)
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| b.then_some(i))
        .collect();
    if !backwards.is_empty() {
        diag::warn!(
            Zone,
            "weapon {}: damage range distances went backwards at {backwards:?} ({step_ranges:?}); dropped",
            w.name
        );
    }
    f.damage_steps = Some(weapon_t6::damage::DamageSteps::from_file(
        std::array::from_fn(|i| damage[i]),
        step_ranges,
    ));
    f.min_player_damage = w.def_i32(d::minPlayerDamage);
    f.rechamber_time_ms = w.def_i32(d::iRechamberTime);
    f.rechamber_bolt_time_ms = w.def_i32(d::iRechamberBoltTime);
    f.reload_show_rocket_time_ms = w.def_i32(d::reloadShowRocketTime);
    f.reload_add_time_ms = w.def_i32(d::iReloadAddTime);
    f.reload_empty_add_time_ms = w.def_i32(d::iReloadEmptyAddTime);
    f.reload_start_time_ms = w.def_i32(d::iReloadStartTime);
    f.reload_start_add_time_ms = w.def_i32(d::iReloadStartAddTime);
    f.reload_end_time_ms = w.def_i32(d::iReloadEndTime);
    f.reload_ammo_add = w.def_i32(d::iReloadAmmoAdd);
    f.reload_start_add = w.def_i32(d::iReloadStartAdd);
    f.no_partial_reload = w.def_u8(d::bNoPartialReload) != 0;
    f.bolt_action = w.def_u8(d::bBoltAction) != 0;
    f.segmented_reload = w.def_u8(d::bSegmentedReload) != 0;
    f.sprint_raise_time_ms = w.def_i32(d::sprintInTime);
    f.sprint_loop_time_ms = w.def_i32(d::sprintLoopTime);
    f.sprint_drop_time_ms = w.def_i32(d::sprintOutTime);
    f.dive_times_ms = [d::dtpInTime, d::dtpLoopTime, d::dtpOutTime].map(|o| w.def_i32(o));
    f.crawl_times_ms = [
        d::crawlInTime,
        d::crawlForwardTime,
        d::crawlBackTime,
        d::crawlRightTime,
        d::crawlLeftTime,
        d::crawlOutFireTime,
        d::crawlOutTime,
    ]
    .map(|o| w.def_i32(o));
    f.fuse_time_ms = w.def_i32(d::fuseTime);
    f.auto_aim_range = w.def_f32(d::autoAimRange);
    f.aim_assist_range = w.def_f32(d::aimAssistRange);
    f.cook_off_hold = w.def_u8(d::bCookOffHold) != 0;
    f.clip_only = w.def_u8(d::bClipOnly) != 0;
    f.has_detonator = w.def_u8(d::hasDetonator) != 0;
    f.detonate_delay_ms = w.def_i32(d::iDetonateDelay);
    f.detonate_time_ms = w.def_i32(d::iDetonateTime);
    f.projectile_rotates = w.def_u8(d::rotate) != 0;
    f.projectile_rotate_type = w.def_i32(d::rotateType);
    // bo2zm fix list 2: BO2's own minimums only matter this way (see
    // `WeaponBodyFacts::spread_before_fire_add`).
    f.spread_before_fire_add = true;
    f.timed_detonation = w.def_u8(d::timedDetonation) != 0;
    f.explode_on_ground = w.def_u8(d::bExplodeOnGround) != 0;
    f.proj_impact_explode = w.def_u8(d::bProjImpactExplode) != 0;
    // bo2mp: what its projectile is when it goes off (BO2's projExplosion: 2 a
    // flash, 6 a heavy explosive, 9 a bolt).
    f.projectile_explosion_type = w.def_i32(d::projExplosion);
    f.laser_sensor = w.def_u8(d::isLaserSensor) != 0;
    f.explosion_radius = w.def_i32(d::iExplosionRadius);
    f.explosion_radius_min = w.def_i32(d::iExplosionRadiusMin);
    f.explosion_inner_damage = w.def_i32(d::iExplosionInnerDamage);
    f.explosion_outer_damage = w.def_i32(d::iExplosionOuterDamage);
    f.damage_cone_angle = w.def_f32(d::damageConeAngle);
    f.stickiness = w.def_i32(d::stickiness);
    // bo2mp: the grenade warning's own facts (BO2's raw weapType, iIndicatorRadius).
    f.t6_weap_type = w.def_i32(d::weapType);
    f.indicator_radius = w.def_i32(d::iIndicatorRadius);
    // Sticks to everything, or to flesh: it sticks to the player it hits.
    f.stick_to_players = matches!(f.stickiness, 1 | 5);
    f.projectile_speed = w.def_i32(d::iProjectileSpeed);
    f.projectile_speed_up = w.def_i32(d::iProjectileSpeedUp);
    f.projectile_speed_forward = w.def_i32(d::iProjectileSpeedForward);
    f.projectile_speed_relative_up = w.def_i32(d::iProjectileSpeedRelativeUp);
    f.projectile_parent_velocity = Some(w.def_f32(d::fProjectileTakeParentVelocity));
    f.projectile_activate_dist = w.def_i32(d::iProjectileActivateDist);
    // How it steers (BO2's guidedMissileType: the Hunter Killer's drone is 7,
    // the SMAW and FHJ-18 are 1) and its soft launch.
    f.missile_guidance = w.def_i32(d::guidedMissileType);
    f.max_steering_accel = w.def_f32(d::maxSteeringAccel);
    f.ignition_delay_ms = w.def_i32(d::projIgnitionDelay);
    let array31 = |v: &Option<Vec<f32>>| -> Option<[f32; 31]> {
        let v = v.as_ref()?;
        let mut out = [0.0; 31];
        for (o, x) in out.iter_mut().zip(v.iter()) {
            *o = *x;
        }
        Some(out)
    };
    f.parallel_bounce = array31(&w.parallel_bounce);
    f.perpendicular_bounce = array31(&w.perpendicular_bounce);
    f.location_damage_mult = w.location_damage.as_ref().map(|v| {
        let mut out = [1.0f32; weapon_t6::hitloc::HITLOC_COUNT];
        for (o, x) in out.iter_mut().zip(v.iter()) {
            *o = *x;
        }
        out
    });
    f.no_dual_wield = w.def_u8(d::bDualWield) == 0;
    f.kick = WeaponKickFacts {
        f_ads_view_kick_center_speed: w.var_f32(v::fAdsViewKickCenterSpeed),
        f_hip_view_kick_center_speed: w.var_f32(v::fHipViewKickCenterSpeed),
        gun_max_pitch: w.def_f32(d::fGunMaxPitch),
        gun_max_yaw: w.def_f32(d::fGunMaxYaw),
        ads_gun_kick_reduced_kick_bullets: w.def_i32(d::adsGunKickReducedKickBullets),
        ads_gun_kick_reduced_kick_percent: w.def_f32(d::adsGunKickReducedKickPercent),
        ads_gun_kick_pitch_min: w.def_f32(d::fAdsGunKickPitchMin),
        ads_gun_kick_pitch_max: w.def_f32(d::fAdsGunKickPitchMax),
        ads_gun_kick_yaw_min: w.def_f32(d::fAdsGunKickYawMin),
        ads_gun_kick_yaw_max: w.def_f32(d::fAdsGunKickYawMax),
        ads_gun_kick_accel: w.def_f32(d::fAdsGunKickAccel),
        ads_gun_kick_speed_max: w.def_f32(d::fAdsGunKickSpeedMax),
        ads_gun_kick_speed_decay: w.def_f32(d::fAdsGunKickSpeedDecay),
        ads_gun_kick_static_decay: w.def_f32(d::fAdsGunKickStaticDecay),
        ads_view_kick_pitch_min: w.def_f32(d::fAdsViewKickPitchMin),
        ads_view_kick_pitch_max: w.def_f32(d::fAdsViewKickPitchMax),
        ads_view_kick_yaw_min: w.def_f32(d::fAdsViewKickYawMin),
        ads_view_kick_yaw_max: w.def_f32(d::fAdsViewKickYawMax),
        hip_gun_kick_reduced_kick_bullets: w.def_i32(d::hipGunKickReducedKickBullets),
        hip_gun_kick_reduced_kick_percent: w.def_f32(d::hipGunKickReducedKickPercent),
        hip_gun_kick_pitch_min: w.def_f32(d::fHipGunKickPitchMin),
        hip_gun_kick_pitch_max: w.def_f32(d::fHipGunKickPitchMax),
        hip_gun_kick_yaw_min: w.def_f32(d::fHipGunKickYawMin),
        hip_gun_kick_yaw_max: w.def_f32(d::fHipGunKickYawMax),
        hip_gun_kick_accel: w.def_f32(d::fHipGunKickAccel),
        hip_gun_kick_speed_max: w.def_f32(d::fHipGunKickSpeedMax),
        hip_gun_kick_speed_decay: w.def_f32(d::fHipGunKickSpeedDecay),
        hip_gun_kick_static_decay: w.def_f32(d::fHipGunKickStaticDecay),
        hip_view_kick_pitch_min: w.def_f32(d::fHipViewKickPitchMin),
        hip_view_kick_pitch_max: w.def_f32(d::fHipViewKickPitchMax),
        hip_view_kick_yaw_min: w.def_f32(d::fHipViewKickYawMin),
        hip_view_kick_yaw_max: w.def_f32(d::fHipViewKickYawMax),
        hip_view_kick_min_magnitude: w.def_f32(d::fHipViewKickMinMagnitude),
        ads_view_kick_min_magnitude: w.def_f32(d::fAdsViewKickMinMagnitude),
    };
    f.sway = WeaponSwayFacts {
        sway_max_angle: w.def_f32(d::swayMaxAngle),
        sway_lerp_speed: w.def_f32(d::swayLerpSpeed),
        sway_pitch_scale: w.def_f32(d::swayPitchScale),
        sway_yaw_scale: w.def_f32(d::swayYawScale),
        sway_horiz_scale: w.def_f32(d::swayHorizScale),
        sway_vert_scale: w.def_f32(d::swayVertScale),
        sway_shell_shock_scale: w.def_f32(d::swayShellShockScale),
        ads_sway_max_angle: w.def_f32(d::adsSwayMaxAngle),
        ads_sway_lerp_speed: w.def_f32(d::adsSwayLerpSpeed),
        ads_sway_pitch_scale: w.def_f32(d::adsSwayPitchScale),
        ads_sway_yaw_scale: w.def_f32(d::adsSwayYawScale),
        ads_sway_horiz_scale: w.var_f32(v::fAdsSwayHorizScale),
        ads_sway_vert_scale: w.var_f32(v::fAdsSwayVertScale),
    };
    if f.body_resolved {
        let pair = |off: usize| [w.def_f32(off), w.def_f32(off + 4)];
        f.bob = WeaponBobFacts {
            sprint_cycle_scale: w.def_f32(d::fSprintCycleScale),
            ducked_sprint_cycle_scale: w.def_f32(d::fDuckedSprintCycleScale),
            dtp_cycle_scale: w.def_f32(d::fDtpCycleScale),
            sprint_bob: pair(d::vSprintBob),
            dtp_bob: pair(d::vDtpBob),
        };
    }
    f
}

fn t6_sounds(w: &asset_t6::WeaponRef) -> WeaponSoundAliases {
    let s = |off: usize| opt(w.def_string(off));
    WeaponSoundAliases {
        fire: s(d::fireSound),
        fire_player: s(d::fireSoundPlayer),
        empty_fire: s(d::emptyFireSound),
        empty_fire_player: s(d::emptyFireSoundPlayer),
        melee_swipe: s(d::meleeSwipeSound),
        melee_swipe_player: s(d::meleeSwipeSoundPlayer),
        melee_hit: s(d::meleeHitSound),
        melee_miss: s(d::meleeMissSound),
        pickup: s(d::pickupSound),
        pickup_player: s(d::pickupSoundPlayer),
        ammo_pickup: s(d::ammoPickupSound),
        ammo_pickup_player: s(d::ammoPickupSoundPlayer),
        pullback: s(d::pullbackSound),
        pullback_player: s(d::pullbackSoundPlayer),
        reload: s(d::reloadSound),
        reload_player: s(d::reloadSoundPlayer),
        reload_empty: s(d::reloadEmptySound),
        reload_empty_player: s(d::reloadEmptySoundPlayer),
        reload_start: s(d::reloadStartSound),
        reload_start_player: s(d::reloadStartSoundPlayer),
        reload_end: s(d::reloadEndSound),
        reload_end_player: s(d::reloadEndSoundPlayer),
        rechamber: s(d::rechamberSound),
        rechamber_player: s(d::rechamberSoundPlayer),
        alt_switch: s(d::altSwitchSound),
        alt_switch_player: s(d::altSwitchSoundPlayer),
        raise: s(d::raiseSound),
        raise_player: s(d::raiseSoundPlayer),
        first_raise: s(d::firstRaiseSound),
        first_raise_player: s(d::firstRaiseSoundPlayer),
        putaway: s(d::putawaySound),
        putaway_player: s(d::putawaySoundPlayer),
        proj_explosion: s(d::projExplosionSound),
        projectile: s(d::projectileSound),
        proj_ignition_sound: s(d::projIgnitionSound),
        fire_loop: s(d::fireLoopSound),
        fire_loop_player: s(d::fireLoopSoundPlayer),
        fire_stop: s(d::fireStopSound),
        fire_stop_player: s(d::fireStopSoundPlayer),
        fire_last: s(d::fireLastSound),
        fire_last_player: s(d::fireLastSoundPlayer),
        ads_raise_player: s(d::adsRaiseSoundPlayer),
        ads_lower_player: s(d::adsLowerSoundPlayer),
        ads_zoom: s(d::adsZoomSound),
        spin_loop: s(d::spinLoopSound),
        spin_loop_player: s(d::spinLoopSoundPlayer),
        start_spin: s(d::startSpinSound),
        start_spin_player: s(d::startSpinSoundPlayer),
        stop_spin: s(d::stopSpinSound),
        stop_spin_player: s(d::stopSpinSoundPlayer),
        notetrack_sound_map: w.notetrack_sounds.clone(),
        bounce: std::array::from_fn(|surf| w.bounce_sounds.get(surf).and_then(|n| opt(n))),
        ..WeaponSoundAliases::default()
    }
}

impl WeaponCatalog {
    /// bo2zm: one Black Ops II weapon (a variant with its definition).
    /// Returns false for a reference stub (a comma-named asset another zone
    /// defines) or a weapon without a name.
    /// bo2zm fix list 2: a rolling grenade's rest radius (measured on its
    /// projectile model by the lane, which holds the models).
    pub fn set_t6_rolling_radius(&mut self, name: &str, radius: f32) {
        for e in self.entries.iter_mut().filter(|e| e.name == name) {
            e.facts.rolling_radius = Some(radius);
        }
    }

    /// `left`: a dual-wield weapon's left-hand weapon (its
    /// `szDualWieldWeaponName`), whose gun hangs on the arms' `tag_weapon1`
    /// and whose `dw_left_*` animations drive the left arm.
    ///
    /// `camo_models`: an upgraded weapon's gun model with Pack-a-Punch's
    /// camo swapped in (by weapon name), for it and its left-hand gun.
    pub fn capture_t6(
        &mut self,
        w: &asset_t6::WeaponRef,
        left: Option<&asset_t6::WeaponRef>,
        camo_models: &std::collections::HashMap<String, String>,
    ) -> bool {
        if w.name.is_empty() || w.name.starts_with(',') {
            return false;
        }
        let left = left.filter(|_| w.def_u8(d::bDualWield) != 0);
        let left_gun = left.and_then(|l| {
            camo_models
                .get(&l.name)
                .cloned()
                .or_else(|| l.gun_models.first().and_then(|n| opt(n)))
        });
        let gun_xmodel = camo_models
            .get(&w.name)
            .cloned()
            .or_else(|| w.gun_models.first().and_then(|n| opt(n)));
        let combat_fx = WeaponCombatFx {
            view_flash_hint: opt(w.def_fx(d::viewFlashEffect)),
            world_flash_hint: opt(w.def_fx(d::worldFlashEffect)),
            view_shell_eject_hint: opt(w.def_fx(d::viewShellEjectEffect)),
            world_shell_eject_hint: opt(w.def_fx(d::worldShellEjectEffect)),
            explosion_hint: opt(w.def_fx(d::projExplosionEffect)),
            tracer_hint: opt(w.def_tracer(d::tracerType)),
            ..WeaponCombatFx::default()
        };
        let reticle = WeaponReticleAssets {
            center_material: opt(w.def_material(d::reticleCenter)),
            side_material: opt(w.def_material(d::reticleSide)),
            center_authored: !w.def_material(d::reticleCenter).is_empty(),
            side_authored: !w.def_material(d::reticleSide).is_empty(),
            center_size: w.def_i32(d::iReticleCenterSize),
            side_size: w.def_i32(d::iReticleSideSize),
            indicator_icon: opt(w.def_material(d::indicatorIcon)),
            ..WeaponReticleAssets::default()
        };
        self.entries.push(CatalogWeapon {
            namespace: self.capture_ns,
            name: w.name.clone(),
            alternate_weapon: opt(&w.alt_weapon_name),
            display_name_key: opt(&w.display_name),
            reticle,
            overlay_material: opt(&w.overlay_material).or_else(|| opt(&w.overlay_material_low)),
            hud_icon: opt(w.def_material(d::hudIcon)),
            kill_icon: opt(w.def_material(d::killIcon)),
            proj_trail: opt(w.def_fx(d::projTrailEffect)),
            proj_ignition: opt(w.def_fx(d::projIgnitionEffect)),
            gun_xmodel,
            world_model: w.world_models.first().and_then(|n| opt(n)),
            projectile_model: opt(w.def_model(d::projectileModel)),
            rocket_model: opt(w.def_model(d::rocketModel)),
            sz_xanims: remap_t6_sz_xanims(&w.xanims),
            sz_xanims_right: match left {
                Some(_) => remap_t6_sz_xanims(&w.xanims),
                None => [const { None }; WEAPON_ANIM_SLOTS],
            },
            sz_xanims_left: match left {
                Some(l) => remap_t6_left_hand_xanims(&l.xanims),
                None => [const { None }; WEAPON_ANIM_SLOTS],
            },
            dual_wield_weapon: left.map(|l| l.name.clone()),
            spawned_grenade_weapon: opt(w.def_string(d::szSpawnedGrenadeWeaponName)),
            hide_tags: w.hide_tags.clone(),
            ads_view_attachments: Vec::new(),
            world_attachments: Vec::new(),
            view_attachments: w
                .attach_view_models
                .iter()
                .map(|(_, model, tag, offset, rotation)| {
                    (
                        model.clone(),
                        asset_model::FpvMountOffset {
                            tag: tag.clone(),
                            offset: *offset,
                            rotation: *rotation,
                            on_arms: false,
                        },
                    )
                })
                .chain(left_gun.map(|model| {
                    (
                        model,
                        asset_model::FpvMountOffset {
                            tag: "tag_weapon1".to_owned(),
                            offset: [0.0; 3],
                            rotation: [0.0; 3],
                            on_arms: true,
                        },
                    )
                }))
                .collect(),
            sounds: t6_sounds(w),
            combat_fx,
            facts: t6_body_facts(w),
        });
        true
    }
}

/// bo2mp: Black Ops II's sight attachments, by attachment name: the ones a
/// gun is given as its own catalog weapon (`mp7_mp+reflex`), so its sight
/// model, hidden iron sights, ADS animations, zoom and scope overlay are the
/// attachment's. Reflex, EOTech (`holo`), ACOG, Hybrid (`dualoptic`), Dual
/// Band (`ir`), Target Finder (`rangefinder`), Millimeter Scanner (`mms`),
/// Variable Zoom (`vzoom`), Ballistics CPU (`swayreduc`) and the Ballista's
/// iron sights (`is`).
pub const T6_SIGHT_ATTACHMENTS: [&str; 10] = [
    "reflex",
    "holo",
    "acog",
    "dualoptic",
    "ir",
    "rangefinder",
    "mms",
    "vzoom",
    "swayreduc",
    "is",
];

/// bo2mp: what a Black Ops II attachment (`WeaponAttachment`) changes on
/// any gun it goes on, read from the attachment at run time (no number of
/// our own): clip size (stack fire's `iClipSize` replaces, else
/// `clipSizeScale` scales: Extended Clip), fire time (`fFireTimeScale`: Rapid Fire),
/// reload times (`fReload*TimeScale`; `bDualMag` gives the gun's quick
/// reloads: Fast Mag), ADS in/out time (`fAdsTrans*TimeScale`:
/// Quickdraw), aimed move speed (`adsMoveSpeedScale`: Stalker), hip
/// spread (`fHipSpreadMin/MaxScale`: Laser Sight), damage ranges
/// (`fDamageRangeScale`: Long Barrel), aimed view kick centring
/// (`fAdsViewKickCenterSpeedScale`), idle sway (`fAdsIdleAmountScale`,
/// `fHipIdleAmount`), penetration (`penetrateType`: FMJ), a suppressed
/// gun (`bSilenced`) and the hip and ADS sway it overrides
/// (`swayOverride`, `adsSwayOverride`).
/// A scale of 0 is read as "not set" (left as the gun's).
/// Code fix, needs local proof (that 0 means unset and `iClipSize`
/// replaces rather than adds is this engine's reading of the fields).
pub fn apply_t6_attachment_stats(
    f: &mut WeaponBodyFacts,
    w: &asset_t6::WeaponRef,
    attachment: &asset_t6::AttachmentRef,
) {
    use fastfile_t6::layout::WeaponAttachment as a;
    let scale_ms = |ms: &mut i32, scale: f32| {
        if scale > 0.0 && *ms > 0 {
            *ms = (*ms as f32 * scale) as i32;
        }
    };
    // Clip size (BO2: only stack fire sets it outright; every other attachment scales it, so
    // the grenade launcher's own clip number never reaches its rifle).
    const ATTACHMENT_STACKFIRE: i32 = 24;
    let clip = attachment.i32_at(a::iClipSize);
    let clip_scale = attachment.f32_at(a::clipSizeScale);
    if attachment.i32_at(a::attachmentType) == ATTACHMENT_STACKFIRE {
        f.clip_size = clip;
    } else if clip_scale > 0.0 && f.clip_size > 0 {
        f.clip_size = ((f.clip_size as f32 * clip_scale).round() as i32).max(1);
    }
    // Fire time.
    scale_ms(&mut f.fire_time_ms, attachment.f32_at(a::fFireTimeScale));
    // Reloads.
    scale_ms(&mut f.reload_time_ms, attachment.f32_at(a::fReloadTimeScale));
    scale_ms(&mut f.reload_empty_time_ms, attachment.f32_at(a::fReloadEmptyTimeScale));
    scale_ms(&mut f.reload_add_time_ms, attachment.f32_at(a::fReloadAddTimeScale));
    if attachment.u8_at(a::bDualMag) != 0 && f.dual_mag.is_none() {
        f.dual_mag = Some(weapon_t6::dual_mag::DualMagTimes {
            reload_ms: w.var_i32(v::iReloadQuickTime),
            reload_empty_ms: w.var_i32(v::iReloadQuickEmptyTime),
            add_ms: w.def_i32(d::iReloadQuickAddTime),
            empty_add_ms: w.def_i32(d::iReloadQuickEmptyAddTime),
        });
    }
    if let Some(quick) = f.dual_mag.as_mut() {
        scale_ms(&mut quick.reload_ms, attachment.f32_at(a::fReloadQuickTimeScale));
        scale_ms(&mut quick.reload_empty_ms, attachment.f32_at(a::fReloadQuickEmptyTimeScale));
        scale_ms(&mut quick.add_ms, attachment.f32_at(a::fReloadQuickAddTimeScale));
    }
    // ADS in and out (a rate: a shorter time is a faster rate).
    for (rate, scale) in [
        (&mut f.ads_in_rate, attachment.f32_at(a::fAdsTransInTimeScale)),
        (&mut f.ads_out_rate, attachment.f32_at(a::fAdsTransOutTimeScale)),
    ] {
        if scale > 0.0 {
            *rate /= scale;
        }
    }
    // Aimed walk speed: an attachment that sets its own (Stalker) replaces the gun's, it does
    // not scale it. "Sets its own" means it differs from the empty attachment's, which is 1
    // in the zones (`defaultattachment`); every other attachment there holds 1 too.
    const EMPTY_ATTACHMENT_ADS_MOVE: f32 = 1.0;
    let ads_move = attachment.f32_at(a::adsMoveSpeedScale);
    if ads_move > 0.0 && ads_move != EMPTY_ATTACHMENT_ADS_MOVE {
        f.ads_move_speed_scale = ads_move;
    }
    // Hip spread.
    let (min, max) = (attachment.f32_at(a::fHipSpreadMinScale), attachment.f32_at(a::fHipSpreadMaxScale));
    if min > 0.0 {
        for s in [&mut f.hip_spread_stand_min, &mut f.hip_spread_ducked_min, &mut f.hip_spread_prone_min] {
            *s *= min;
        }
    }
    if max > 0.0 {
        for s in [&mut f.hip_spread_stand_max, &mut f.hip_spread_ducked_max, &mut f.hip_spread_prone_max] {
            *s *= max;
        }
    }
    // Damage ranges.
    let range = attachment.f32_at(a::fDamageRangeScale);
    if range > 0.0 {
        f.max_damage_range *= range;
        f.min_damage_range *= range;
        if let Some(steps) = &mut f.damage_steps {
            steps.range_scale *= range;
        }
    }
    // Aimed kick and idle sway.
    let centre = attachment.f32_at(a::fAdsViewKickCenterSpeedScale);
    if centre > 0.0 {
        f.kick.f_ads_view_kick_center_speed *= centre;
    }
    let idle = attachment.f32_at(a::fAdsIdleAmountScale);
    if idle > 0.0 {
        f.idle.ads_idle_amount *= idle;
    }
    let hip_idle = attachment.f32_at(a::fHipIdleAmount);
    if hip_idle > 0.0 {
        f.idle.hip_idle_amount = hip_idle;
    }
    // Penetration: BG_GetPenetrateType takes the first attachment that has a
    // type, else the gun's own. FMJ is Black Ops II's bullet penetration
    // (MW2's perk, an attachment here): walls and bodies are
    // perk_bulletPenetrationMultiplier times as deep.
    let pen = attachment.i32_at(a::penetrateType);
    if pen != 0 && !f.penetrate_type_from_attachment {
        f.penetrate_type = pen;
        f.penetrate_type_from_attachment = true;
    }
    if attachment.i32_at(a::perks) & T6_PERK_BULLET_PENETRATION != 0 {
        f.penetrate_multiplier = weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT;
    }
    // Suppressor: no firing ping on the enemy's minimap.
    if attachment.u8_at(a::bSilenced) != 0 {
        f.silenced = true;
    }
    // Sway it overrides, hip and aimed.
    if attachment.u8_at(a::swayOverride) != 0 {
        f.sway.sway_max_angle = attachment.f32_at(a::swayMaxAngle);
        f.sway.sway_lerp_speed = attachment.f32_at(a::swayLerpSpeed);
        f.sway.sway_pitch_scale = attachment.f32_at(a::swayPitchScale);
        f.sway.sway_yaw_scale = attachment.f32_at(a::swayYawScale);
        f.sway.sway_horiz_scale = attachment.f32_at(a::swayHorizScale);
        f.sway.sway_vert_scale = attachment.f32_at(a::swayVertScale);
    }
    if attachment.u8_at(a::adsSwayOverride) != 0 {
        f.sway.ads_sway_max_angle = attachment.f32_at(a::adsSwayMaxAngle);
        f.sway.ads_sway_lerp_speed = attachment.f32_at(a::adsSwayLerpSpeed);
        f.sway.ads_sway_pitch_scale = attachment.f32_at(a::adsSwayPitchScale);
        f.sway.ads_sway_yaw_scale = attachment.f32_at(a::adsSwayYawScale);
        f.sway.ads_sway_horiz_scale = attachment.f32_at(a::fAdsSwayHorizScale);
        f.sway.ads_sway_vert_scale = attachment.f32_at(a::fAdsSwayVertScale);
    }
}

impl WeaponCatalog {
    /// bo2mp: the alternate mode `name` (already captured) shares its gun's
    /// magazine: Black Ops II's defs name the same ammo and clip (Select
    /// Fire). The rows of the pair key one magazine and one reserve.
    pub fn mark_t6_shared_magazine(&mut self, name: &str) {
        let ns = self.capture_ns;
        if let Some(e) = self.entries.iter_mut().rev().find(|e| e.name == name && e.namespace == ns) {
            e.facts.shares_alt_magazine = true;
        }
    }

    /// bo2mp: `<gun>+<attachment>` for one attachment: the gun's own entry
    /// (already captured) with the attachment unique's models, hidden tags
    /// and animations, a sight's overlay and zoom, and what the attachment
    /// changes on the gun (`apply_t6_attachment_stats`).
    pub fn capture_t6_attachment(
        &mut self,
        w: &asset_t6::WeaponRef,
        attachment: &asset_t6::AttachmentRef,
        unique: &asset_t6::AttachmentUniqueRef,
        reticle_scale: f32,
        alt_of: Option<&str>,
        dual_wield_left: Option<&asset_t6::WeaponRef>,
        anim_ms: &dyn Fn(&str) -> Option<i32>,
    ) -> bool {
        use fastfile_t6::layout::WeaponAttachment as a;
        use fastfile_t6::layout::WeaponAttachmentUnique as u;
        let ns = self.capture_ns;
        let Some(mut e) = self
            .entries
            .iter()
            .rev()
            .find(|e| e.name == w.name && e.namespace == ns)
            .cloned()
        else {
            return false;
        };
        e.name = format!("{}+{}", w.name, attachment.name);
        // Hybrid Optic: the unique names the gun's other mode (an alternate
        // weapon with its own ADS animations); each is the other's alternate.
        // An underbarrel (Grenade Launcher `gl`, Masterkey `mk`): the unique
        // names the launcher, a weapon of its own the gun switches to and
        // back from (`<launcher>+<attachment>`, its alternate the gun's).
        let sight = T6_SIGHT_ATTACHMENTS.contains(&attachment.name.as_str());
        let hybrid = attachment.name == "dualoptic" && !unique.alt_weapon_name.is_empty();
        let underbarrel = !sight && !unique.alt_weapon_name.is_empty();
        if let Some(primary) = alt_of {
            e.alternate_weapon = Some(primary.to_owned());
            e.facts.optic_switch = !underbarrel;
        } else if hybrid || underbarrel {
            e.alternate_weapon = Some(format!("{}+{}", unique.alt_weapon_name, attachment.name));
            e.facts.optic_switch = hybrid;
        }
        // The scope overlay is a sight's unique's (none: an ACOG on a
        // sniper); any other attachment keeps the gun's own (a sniper's
        // scope under FMJ).
        if sight {
            e.overlay_material =
                opt(&unique.overlay_material).or_else(|| opt(&unique.overlay_material_low));
            e.facts.overlay_reticle = unique.i32_at(u::overlayReticle);
        }
        // BO2 `BG_IsInfraRed`: any infrared attachment makes the gun infrared.
        // BO2 ATTACHMENT_RANGEFINDER (Target Finder): full ADS draws its sight screen.
        if attachment.name == "rangefinder" {
            e.facts.rangefinder = true;
        }
        if attachment.u8_at(a::bInfraRed) != 0 {
            e.facts.infrared = true;
        }
        // Models: the gun's own attachment models, less the one in the
        // attachment's point when the unique replaces it (a sniper's scope
        // under an ACOG), moved when it says so; then the unique's.
        let point = attachment.i32_at(a::attachmentPoint);
        let replace = unique.bytes.get(u::disableBaseWeaponAttachment).is_some_and(|&b| b != 0);
        let moved = unique
            .bytes
            .get(u::overrideBaseWeaponAttachmentOffsets)
            .is_some_and(|&b| b != 0);
        let mut views: Vec<(String, asset_model::FpvMountOffset)> = Vec::new();
        for (slot, model, tag, offset, rotation) in &w.attach_view_models {
            // Attachment points count from 1 (1: the sight on top); the
            // gun's own attachment models from 0 (0: a sniper's scope).
            let here = usize::try_from(point - 1).is_ok_and(|p| p == *slot);
            if here && replace {
                continue;
            }
            views.push((
                model.clone(),
                asset_model::FpvMountOffset {
                    tag: tag.clone(),
                    offset: if here && moved {
                        unique.vec3_at(u::viewModelOffsetBaseAttachment)
                    } else {
                        *offset
                    },
                    rotation: *rotation,
                    on_arms: false,
                },
            ));
        }
        for (model, offset, rotation) in [
            (&unique.view_model, u::viewModelOffsets, u::viewModelRotations),
            (
                &unique.view_model_additional,
                u::viewModelAddOffsets,
                u::viewModelAddRotations,
            ),
        ] {
            if let Some(model) = opt(model) {
                views.push((
                    model,
                    asset_model::FpvMountOffset {
                        tag: unique.view_model_tag.clone(),
                        offset: unique.vec3_at(offset),
                        rotation: unique.vec3_at(rotation),
                        on_arms: false,
                    },
                ));
            }
        }
        // A dual-wield left gun stays on the arms.
        views.extend(e.view_attachments.iter().filter(|(_, m)| m.on_arms).cloned());
        // A sight that names a stripped model for aiming (a Rangefinder's
        // `viewModelADS`: no housing or lens, so the eyeline is clear)
        // swaps it for its main model while aimed, at the same placement.
        e.ads_view_attachments = match (opt(&unique.view_model_ads), opt(&unique.view_model)) {
            (Some(ads), Some(main)) => views
                .iter()
                .map(|(model, at)| {
                    if *model == main {
                        (ads.clone(), at.clone())
                    } else {
                        (model.clone(), at.clone())
                    }
                })
                .collect(),
            _ => Vec::new(),
        };
        e.view_attachments = views;
        // Third person and on the floor: its world models on their tag.
        push_t6_world_attachments(&mut e, unique);
        // Its fire sounds (a Suppressor's), where it names them.
        for (sound, name) in [
            &mut e.sounds.fire,
            &mut e.sounds.fire_player,
            &mut e.sounds.fire_loop,
            &mut e.sounds.fire_loop_player,
            &mut e.sounds.fire_stop,
            &mut e.sounds.fire_stop_player,
            &mut e.sounds.fire_last,
            &mut e.sounds.fire_last_player,
        ]
        .into_iter()
        .zip(&unique.fire_sounds)
        {
            if let Some(name) = opt(name) {
                *sound = Some(name);
            }
        }
        // Its muzzle flashes (a Suppressor's), where it names them.
        if let Some(name) = opt(&unique.view_flash) {
            e.combat_fx.view_flash_hint = Some(name);
        }
        if let Some(name) = opt(&unique.world_flash) {
            e.combat_fx.world_flash_hint = Some(name);
        }
        // The unique's hidden tags (the iron sights under a reflex).
        if !unique.hide_tags.is_empty() {
            e.hide_tags = unique.hide_tags.clone();
        }
        // Its animations (ADS up and down to the sight's height).
        // (The alternate keeps its own: they are the other mode's.)
        for (slot, name) in remap_t6_sz_xanims(&unique.xanims).into_iter().enumerate() {
            if alt_of.is_some() {
                break;
            }
            if name.is_some() {
                e.sz_xanims[slot] = name;
            }
        }
        // Dual Wield (`dw`): the unique's left-hand gun in the other hand,
        // on the arms, its `dw_left_*` animations driving the left arm and
        // the gun's (with the unique's) the right.
        if let Some(l) = dual_wield_left {
            e.sz_xanims_right = e.sz_xanims.clone();
            e.sz_xanims_left = remap_t6_left_hand_xanims(&l.xanims);
            e.dual_wield_weapon = Some(l.name.clone());
            e.facts.no_dual_wield = false;
            if let Some(model) = l.gun_models.first().and_then(|n| opt(n)) {
                e.view_attachments.push((
                    model,
                    asset_model::FpvMountOffset {
                        tag: "tag_weapon1".to_owned(),
                        offset: [0.0; 3],
                        rotation: [0.0; 3],
                        on_arms: true,
                    },
                ));
            }
        }
        // The Hybrid's other mode lines the gun up on the other window: its
        // own slot 0x57 (`rmr_bottom_ads_up`) is the way up to it; its slot
        // 0x55 (`rmr_top_ads_up`) is the way back to the first window, and
        // each mode's slot 0x56 is the way down from its own.
        if alt_of.is_some()
            && let Some(up) = w.xanims.get(0x57).filter(|n| !n.is_empty())
        {
            e.sz_xanims[weap_anim::ADS_UP] = Some(up.to_ascii_lowercase());
        }
        // Zoom: the attachment's field of view (1: the gun's own) and its
        // zoom-in and -out windows when it names them.
        // (The Hybrid has two: the attachment's first (30) is the tube's, its
        // second (50, the gun's own) the reflex window's. The gun's first mode
        // aims through the window (its ADS slots 0x55/0x56 are the `top` ones,
        // its zoom-in/out windows and idle scale are a reflex's, not an
        // ACOG's); the alternate looks through the tube.)
        let fov = if sight { attachment.f32_at(a::fAdsZoomFov1) } else { 0.0 };
        if hybrid {
            let mode_fov = if alt_of.is_some() { fov } else { attachment.f32_at(a::fAdsZoomFov2) };
            if mode_fov > 1.0 {
                e.facts.ads_zoom_fov = mode_fov;
            }
        } else if fov > 1.0 {
            e.facts.ads_zoom_fov = fov;
        }
        // Variable Zoom names no first zoom (1) and two others: it opens at
        // the wider and zooms in to the narrower.
        let (fov2, fov3) = (attachment.f32_at(a::fAdsZoomFov2), attachment.f32_at(a::fAdsZoomFov3));
        if sight && fov <= 1.0 && fov2 > 1.0 && fov3 > 1.0 {
            let (wide, narrow) = (fov2.max(fov3), fov2.min(fov3));
            e.facts.ads_zoom_fov = wide;
            e.facts.variable_zoom_fovs = [wide, narrow];
        }
        let zin = attachment.f32_at(a::fAdsZoomInFrac);
        if sight && zin > 0.0 {
            e.facts.ads_zoom_in_frac = zin;
        }
        let zout = attachment.f32_at(a::fAdsZoomOutFrac);
        if sight && zout > 0.0 {
            e.facts.ads_zoom_out_frac = zout;
        }
        if sight {
            e.facts.sight_reticle_scale = reticle_scale;
        }
        // What it changes on the gun (ADS times, clip, reloads, spread...);
        // an underbarrel's launcher keeps its own.
        if !(underbarrel && alt_of.is_some()) {
            apply_t6_attachment_stats(&mut e.facts, w, attachment);
        }
        // Hybrid Optic: the swap takes as long as the gun's own swap
        // animation (`rmr_alt_raise_down`, slot 0x1A: 10 frames at 30 fps =
        // 333 ms on every Hybrid gun), not the stat's 700 ms (16 on the
        // HK416), which no swap in the game honours.
        if e.facts.optic_switch
            && let Some(ms) = e.sz_xanims[weap_anim::ALT_RAISE].as_deref().and_then(anim_ms)
            && ms > 0
        {
            e.facts.alternate_raise_time_ms = ms;
        }
        self.entries.push(e);
        true
    }

    /// bo2mp: a gun with two attachments, `<gun>+<a>+<b>` (the two in name
    /// order, as `t6_attachment_combo_name` spells it): the gun with its
    /// first attachment (`<gun>+<first>`, already captured: a sight's model,
    /// overlay, zoom and ADS animations, an alternate) and the second laid
    /// over it (`lay_t6_attachment`): its models, hidden tags, fire sounds,
    /// flashes, the animation slots the first's unique leaves to the gun,
    /// and what it changes on the gun. The first's alternate (the Hybrid's
    /// other mode, an underbarrel's launcher) becomes a pair of its own,
    /// the two each other's alternate; so does the second's launcher.
    /// Pass a sight first when there is one. Code fix, needs local proof.
    pub fn capture_t6_attachment_pair(
        &mut self,
        w: &asset_t6::WeaponRef,
        first: (&asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef),
        second: (&asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef),
        second_alt: Option<&asset_t6::WeaponRef>,
        dual_wield_left: Option<&asset_t6::WeaponRef>,
    ) -> bool {
        let ns = self.capture_ns;
        let find = |entries: &[CatalogWeapon], name: &str| {
            entries.iter().rev().find(|e| e.name == name && e.namespace == ns).cloned()
        };
        let base_name = format!("{}+{}", w.name, first.0.name);
        let Some(mut e) = find(&self.entries, &base_name) else { return false };
        let name = t6_attachment_combo_name(&w.name, &[first.0.name.as_str(), second.0.name.as_str()]);
        let atts = name.split_once('+').map_or("", |(_, a)| a).to_owned();
        let second_launcher = !second.1.alt_weapon_name.is_empty() && second_alt.is_some();
        // Two alternates (a Hybrid and a launcher) do not go together here.
        if e.alternate_weapon.is_some() && second_launcher {
            return false;
        }
        lay_t6_attachment(&mut e, w, first.1, second, dual_wield_left, T6Layer::Gun);
        e.name = name.clone();
        // The first's alternate: its own pair, back to this one. A launcher
        // keeps its own stats (it shows the second's models: the sight stays
        // on the gun); the Hybrid's other mode takes both.
        let mut alt_pair = None;
        if let Some(alt_name) = e.alternate_weapon.clone()
            && let Some(mut alt) = find(&self.entries, &alt_name)
        {
            let launcher = !T6_SIGHT_ATTACHMENTS.contains(&first.0.name.as_str());
            let alt_base = alt_name.split('+').next().unwrap_or(&alt_name).to_owned();
            let layer = if launcher { T6Layer::Launcher } else { T6Layer::OtherMode };
            lay_t6_attachment(&mut alt, w, first.1, second, None, layer);
            alt.name = format!("{alt_base}+{atts}");
            alt.alternate_weapon = Some(name.clone());
            e.alternate_weapon = Some(alt.name.clone());
            alt_pair = Some(alt);
        } else if second_launcher
            && let Some(launcher) = second_alt
            && let Some(mut alt) = find(&self.entries, &format!("{}+{}", launcher.name, second.0.name))
        {
            // The second's launcher (`gl`, `mk`) with the first's models.
            lay_t6_attachment(&mut alt, w, second.1, first, None, T6Layer::Launcher);
            alt.name = format!("{}+{atts}", launcher.name);
            alt.alternate_weapon = Some(name.clone());
            e.alternate_weapon = Some(alt.name.clone());
            alt_pair = Some(alt);
        }
        self.entries.push(e);
        self.entries.extend(alt_pair);
        true
    }

    /// BG_GetWeaponHitLocationMultiplier: a gun with attachments (`<gun>+<a>[+<b>]`) takes,
    /// at each hit location, the first of its attachments' uniques (`au_<gun>_<att>`; the
    /// first two fall back to the gun's base unique `au_<gun>_none`, the third stops the
    /// search) whose multiplier differs from the base unique's; else the gun's own. In
    /// multiplayer only the sniper Suppressors change one (the SVU's loses its head bonus).
    /// `unique` gives a unique's 21 multipliers by name. Returns how many entries changed.
    pub fn apply_t6_unique_hit_tables<'a>(
        &mut self,
        unique: impl Fn(&str) -> Option<&'a [f32]>,
    ) -> usize {
        let ns = self.capture_ns;
        let mut changed = 0;
        for e in self.entries.iter_mut().filter(|e| e.namespace == ns) {
            let Some((gun, atts)) = e.name.split_once('+') else {
                continue;
            };
            let stem = gun.strip_suffix("_mp").unwrap_or(gun);
            let Some(base) = unique(&format!("au_{stem}_none")) else {
                continue;
            };
            let slots: Vec<&[f32]> = atts
                .split('+')
                .take(3)
                .enumerate()
                .map_while(|(i, a)| {
                    let own = unique(&format!("au_{stem}_{a}"));
                    if i < 2 {
                        Some(own.unwrap_or(base))
                    } else {
                        own
                    }
                })
                .collect();
            let own = e.facts.location_damage_mult.unwrap_or([1.0; weapon_t6::hitloc::HITLOC_COUNT]);
            let table = t6_unique_hit_table(own, base, &slots);
            if table != own {
                e.facts.location_damage_mult = Some(table);
                changed += 1;
            }
        }
        changed
    }
}

/// BG_GetWeaponHitLocationMultiplier's pick at each hit location: the first slot's unique
/// whose multiplier differs from the base unique's, else the gun's own (`own`).
fn t6_unique_hit_table(
    own: [f32; weapon_t6::hitloc::HITLOC_COUNT],
    base: &[f32],
    slots: &[&[f32]],
) -> [f32; weapon_t6::hitloc::HITLOC_COUNT] {
    std::array::from_fn(|i| {
        slots
            .iter()
            .filter_map(|s| s.get(i).copied())
            .find(|&x| base.get(i) != Some(&x))
            .unwrap_or(own[i])
    })
}

/// bo2mp: the catalog's name for a gun with several attachments: the gun,
/// then its attachments in name order (`an94_mp+fmj+reflex`), so a class's
/// `an94_mp+reflex+fmj` finds the same weapon.
pub fn t6_attachment_combo_name(gun: &str, attachments: &[&str]) -> String {
    let mut atts: Vec<&str> = attachments.to_vec();
    atts.sort_unstable();
    atts.dedup();
    let mut name = gun.to_owned();
    for att in atts {
        name.push('+');
        name.push_str(att);
    }
    name
}

/// bo2mp: one more attachment laid over a gun's entry that already has one
/// (`under`, its unique): the gun's own attachment model at the new one's
/// point dropped when its unique replaces it (moved when it says so); its
/// unique's view models on their tag with their offsets; its hidden tags
/// added; its fire sounds and flashes where it names them; its animations
/// in the slots `under`'s unique leaves to the gun (a sight's ADS
/// animations win); its Dual Wield left gun; and what it changes on the gun
/// (`apply_t6_attachment_stats`), as far as `layer` takes it.
fn lay_t6_attachment(
    e: &mut CatalogWeapon,
    w: &asset_t6::WeaponRef,
    under: &asset_t6::AttachmentUniqueRef,
    (attachment, unique): (&asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef),
    dual_wield_left: Option<&asset_t6::WeaponRef>,
    layer: T6Layer,
) {
    use fastfile_t6::layout::WeaponAttachment as a;
    use fastfile_t6::layout::WeaponAttachmentUnique as u;
    let point = attachment.i32_at(a::attachmentPoint);
    let replace = unique.bytes.get(u::disableBaseWeaponAttachment).is_some_and(|&b| b != 0);
    let moved = unique.bytes.get(u::overrideBaseWeaponAttachmentOffsets).is_some_and(|&b| b != 0);
    // The gun's own model at its point (attachment points count from 1, the
    // gun's own models from 0).
    let own = w
        .attach_view_models
        .iter()
        .find(|(slot, ..)| usize::try_from(point - 1).is_ok_and(|p| p == *slot))
        .map(|(_, model, tag, ..)| (model.clone(), tag.clone()));
    let ads_swap = !e.ads_view_attachments.is_empty();
    for (ads, views) in [(false, &mut e.view_attachments), (true, &mut e.ads_view_attachments)] {
        // (An empty ADS list means "no swap": it stays empty.)
        if ads && !ads_swap {
            continue;
        }
        if let Some((model, tag)) = &own {
            if replace {
                views.retain(|(m, at)| !(m == model && at.tag == *tag && !at.on_arms));
            } else if moved {
                for (m, at) in views.iter_mut() {
                    if m == model && at.tag == *tag && !at.on_arms {
                        at.offset = unique.vec3_at(u::viewModelOffsetBaseAttachment);
                    }
                }
            }
        }
        for (model, offset, rotation) in [
            (&unique.view_model, u::viewModelOffsets, u::viewModelRotations),
            (&unique.view_model_additional, u::viewModelAddOffsets, u::viewModelAddRotations),
        ] {
            if let Some(model) = opt(model) {
                views.push((
                    model,
                    asset_model::FpvMountOffset {
                        tag: unique.view_model_tag.clone(),
                        offset: unique.vec3_at(offset),
                        rotation: unique.vec3_at(rotation),
                        on_arms: false,
                    },
                ));
            }
        }
    }
    push_t6_world_attachments(e, unique);
    for tag in &unique.hide_tags {
        if !e.hide_tags.contains(tag) {
            e.hide_tags.push(tag.clone());
        }
    }
    // A launcher shows the attachment and no more.
    if layer == T6Layer::Launcher {
        return;
    }
    for (sound, name) in [
        &mut e.sounds.fire,
        &mut e.sounds.fire_player,
        &mut e.sounds.fire_loop,
        &mut e.sounds.fire_loop_player,
        &mut e.sounds.fire_stop,
        &mut e.sounds.fire_stop_player,
        &mut e.sounds.fire_last,
        &mut e.sounds.fire_last_player,
    ]
    .into_iter()
    .zip(&unique.fire_sounds)
    {
        if let Some(name) = opt(name) {
            *sound = Some(name);
        }
    }
    if let Some(name) = opt(&unique.view_flash) {
        e.combat_fx.view_flash_hint = Some(name);
    }
    if let Some(name) = opt(&unique.world_flash) {
        e.combat_fx.world_flash_hint = Some(name);
    }
    apply_t6_attachment_stats(&mut e.facts, w, attachment);
    // The other mode keeps its own animations.
    if layer == T6Layer::OtherMode {
        return;
    }
    let taken = remap_t6_sz_xanims(&under.xanims);
    for (slot, name) in remap_t6_sz_xanims(&unique.xanims).into_iter().enumerate() {
        if name.is_some() && taken[slot].is_none() {
            e.sz_xanims[slot] = name;
        }
    }
    if let Some(l) = dual_wield_left
        && e.dual_wield_weapon.is_none()
    {
        e.sz_xanims_right = e.sz_xanims.clone();
        e.sz_xanims_left = remap_t6_left_hand_xanims(&l.xanims);
        e.dual_wield_weapon = Some(l.name.clone());
        e.facts.no_dual_wield = false;
        if let Some(model) = l.gun_models.first().and_then(|n| opt(n)) {
            e.view_attachments.push((
                model,
                asset_model::FpvMountOffset {
                    tag: "tag_weapon1".to_owned(),
                    offset: [0.0; 3],
                    rotation: [0.0; 3],
                    on_arms: true,
                },
            ));
        }
    }
}

/// bo2mp: an attachment unique's world model (`worldModel`) on the gun tag
/// it names (`worldModelTag`), for other players' hands and a gun on the
/// floor. (Its `worldModelOffsets` / `Rotations` are not applied: the
/// third-person attach has no offset; the models sit on the tag.)
fn push_t6_world_attachments(e: &mut CatalogWeapon, unique: &asset_t6::AttachmentUniqueRef) {
    if let Some(model) = opt(&unique.world_model)
        && !e.world_attachments.iter().any(|(m, _)| *m == model)
    {
        e.world_attachments.push((model, unique.world_model_tag.clone()));
    }
}

/// bo2mp: what an attachment laid over an entry changes on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum T6Layer {
    /// The gun: everything.
    Gun,
    /// The Hybrid's other mode: all but the animations (its own).
    OtherMode,
    /// An underbarrel's launcher: the attachment's models and hidden tags
    /// only (it keeps its own stats, sounds and animations).
    Launcher,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An attachment whose fields are all 0 but the ones given.
    fn attachment(fields: &[(usize, f32)], ints: &[(usize, i32)]) -> asset_t6::AttachmentRef {
        let mut bytes = vec![0u8; fastfile_t6::layout::WeaponAttachment::SIZE];
        for &(off, x) in fields {
            bytes[off..off + 4].copy_from_slice(&x.to_le_bytes());
        }
        for &(off, x) in ints {
            bytes[off..off + 4].copy_from_slice(&x.to_le_bytes());
        }
        asset_t6::AttachmentRef { name: "test".into(), bytes }
    }

    fn gun_facts() -> WeaponBodyFacts {
        let mut f = WeaponBodyFacts {
            clip_size: 30,
            fire_time_ms: 100,
            reload_time_ms: 2000,
            reload_empty_time_ms: 2500,
            ads_in_rate: 1.0 / 250.0,
            ads_out_rate: 1.0 / 200.0,
            ads_move_speed_scale: 0.5,
            hip_spread_stand_min: 2.0,
            hip_spread_stand_max: 8.0,
            max_damage_range: 1000.0,
            min_damage_range: 2000.0,
            penetrate_type: 2,
            ..WeaponBodyFacts::default()
        };
        f.idle.ads_idle_amount = 10.0;
        f
    }

    #[test]
    fn t6_attachment_with_nothing_set_changes_nothing() {
        let mut f = gun_facts();
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &attachment(&[], &[]));
        assert_eq!(f, gun_facts());
    }

    #[test]
    fn t6_stalker_replaces_the_aimed_walk_speed_and_a_one_leaves_it() {
        use fastfile_t6::layout::WeaponAttachment as a;
        let mut f = gun_facts();
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &attachment(&[(a::adsMoveSpeedScale, 1.0)], &[]));
        assert_eq!(f.ads_move_speed_scale, 0.5);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &attachment(&[(a::adsMoveSpeedScale, 2.3)], &[]));
        assert_eq!(f.ads_move_speed_scale, 2.3);
    }

    #[test]
    fn t6_attachment_scales_the_guns_stats() {
        use fastfile_t6::layout::WeaponAttachment as a;
        let mut f = gun_facts();
        let att = attachment(
            &[
                (a::clipSizeScale, 1.5),
                (a::fFireTimeScale, 0.8),
                (a::fReloadTimeScale, 0.5),
                (a::fAdsTransInTimeScale, 0.5),
                (a::adsMoveSpeedScale, 2.0),
                (a::fHipSpreadMinScale, 0.5),
                (a::fHipSpreadMaxScale, 0.25),
                (a::fDamageRangeScale, 1.5),
                (a::fAdsIdleAmountScale, 0.5),
            ],
            &[(a::penetrateType, 3)],
        );
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &att);
        assert_eq!(f.clip_size, 45);
        assert_eq!(f.fire_time_ms, 80);
        assert_eq!(f.reload_time_ms, 1000);
        assert_eq!(f.reload_empty_time_ms, 2500);
        assert!((f.ads_in_rate - 1.0 / 125.0).abs() < 1e-9);
        assert!((f.ads_out_rate - 1.0 / 200.0).abs() < 1e-9);
        assert_eq!(f.ads_move_speed_scale, 2.0);
        assert_eq!((f.hip_spread_stand_min, f.hip_spread_stand_max), (1.0, 2.0));
        assert_eq!((f.max_damage_range, f.min_damage_range), (1500.0, 3000.0));
        assert_eq!(f.idle.ads_idle_amount, 5.0);
        assert_eq!(f.penetrate_type, 3);
    }

    #[test]
    fn t6_suppressor_marks_the_gun_silenced() {
        let mut f = gun_facts();
        let mut bytes = attachment(&[], &[]).bytes;
        bytes[fastfile_t6::layout::WeaponAttachment::bSilenced] = 1;
        apply_t6_attachment_stats(
            &mut f,
            &asset_t6::WeaponRef::default(),
            &asset_t6::AttachmentRef { name: "silencer".into(), bytes },
        );
        assert!(f.silenced);
    }

    #[test]
    fn t6_fmj_goes_perk_bullet_penetration_deeper() {
        let mut f = gun_facts();
        f.penetrate_multiplier = 1.0;
        // FMJ's perk mask in the zones: armor piercing (1) and bullet penetration (6).
        let mut fmj = attachment(&[], &[(fastfile_t6::layout::WeaponAttachment::perks, 0x42)]);
        fmj.name = "fmj".into();
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.penetrate_multiplier, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        // Once, however many grant it (BG_WeaponHasPerk is a yes or no).
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.penetrate_multiplier, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        let mut other = gun_facts();
        other.penetrate_multiplier = 1.0;
        apply_t6_attachment_stats(&mut other, &asset_t6::WeaponRef::default(), &attachment(&[], &[]));
        assert_eq!(other.penetrate_multiplier, 1.0);
        // A gun whose own variant grants it (fiveseven_upgraded_zm) goes deeper with no attachment.
        let mut variant = vec![0u8; fastfile_t6::layout::WeaponVariantDef::perks + 8];
        variant[fastfile_t6::layout::WeaponVariantDef::perks] = 0x42;
        let def = vec![0u8; fastfile_t6::layout::WeaponDef::SIZE];
        let pap = asset_t6::WeaponRef { variant, def: def.clone(), ..asset_t6::WeaponRef::default() };
        assert_eq!(t6_body_facts(&pap).penetrate_multiplier, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        let plain = asset_t6::WeaponRef { def, ..asset_t6::WeaponRef::default() };
        assert_eq!(t6_body_facts(&plain).penetrate_multiplier, 1.0);
    }

    #[test]
    fn t6_attachment_clip_size_replaces_and_the_first_penetration_type_wins() {
        use fastfile_t6::layout::WeaponAttachment as a;
        let mut f = gun_facts();
        let att = attachment(
            &[(a::clipSizeScale, 1.5)],
            &[(a::attachmentType, 24), (a::iClipSize, 40), (a::penetrateType, 1)],
        );
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &att);
        assert_eq!(f.clip_size, 40);
        assert_eq!(f.penetrate_type, 1);
        // A second attachment with a type of its own changes nothing.
        let fmj = attachment(&[], &[(a::penetrateType, 3)]);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.penetrate_type, 1);
    }

    #[test]
    fn t6_only_stack_fire_replaces_the_clip() {
        // The grenade launcher carries its own clip number; on the rifle only its scale counts.
        use fastfile_t6::layout::WeaponAttachment as a;
        let mut f = gun_facts();
        let base = f.clip_size;
        let gl = attachment(&[(a::clipSizeScale, 1.0)], &[(a::attachmentType, 11), (a::iClipSize, 10)]);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &gl);
        assert_eq!(f.clip_size, base);
    }

    #[test]
    fn t6_attachment_pairs_are_named_in_attachment_order() {
        assert_eq!(t6_attachment_combo_name("an94_mp", &["reflex", "fmj"]), "an94_mp+fmj+reflex");
        assert_eq!(t6_attachment_combo_name("an94_mp", &["fmj", "reflex"]), "an94_mp+fmj+reflex");
        assert_eq!(t6_attachment_combo_name("an94_mp", &["fmj", "fmj"]), "an94_mp+fmj");
    }

    #[test]
    fn t6_anim_slots_land_on_their_engine_names() {
        let mut t6 = vec![String::new(); 88];
        t6[0x01] = "idle".into();
        t6[0x04] = "fire".into();
        t6[0x10] = "reload".into();
        t6[0x55] = "ads_up".into();
        t6[0x56] = "ads_down".into();
        let out = remap_t6_sz_xanims(&t6);
        assert_eq!(out[weap_anim::IDLE].as_deref(), Some("idle"));
        assert_eq!(out[weap_anim::FIRE].as_deref(), Some("fire"));
        assert_eq!(out[weap_anim::RELOAD].as_deref(), Some("reload"));
        assert_eq!(out[weap_anim::ADS_UP].as_deref(), Some("ads_up"));
        assert_eq!(out[weap_anim::ADS_DOWN].as_deref(), Some("ads_down"));
    }

    #[test]
    fn t6_empty_melee_lands_and_falls_back_to_the_full_one() {
        let mut t6 = vec![String::new(); 88];
        t6[0x09] = "viewmodel_fn57_tactical_melee".into();
        t6[0x0D] = "viewmodel_fn57_tactical_melee_empty".into();
        t6[0x0E] = "viewmodel_fn57_tactical_melee".into();
        let out = remap_t6_sz_xanims(&t6);
        assert_eq!(
            out[weap_anim_extra::MELEE_EMPTY].as_deref(),
            Some("viewmodel_fn57_tactical_melee_empty")
        );
        assert_eq!(
            out[weap_anim_extra::MELEE_CHARGE_EMPTY].as_deref(),
            Some("viewmodel_fn57_tactical_melee")
        );
    }

    #[test]
    fn t6_first_shots_fire_lands_and_falls_back_to_the_fire() {
        let mut t6 = vec![String::new(); 88];
        t6[0x03] = "viewmodel_hamr_fire".into();
        t6[0x04] = "viewmodel_hamr_secondary_fire".into();
        t6[0x3D] = "viewmodel_hamr_ads_secondary_fire".into();
        let out = remap_t6_sz_xanims(&t6);
        assert_eq!(
            out[weap_anim_extra::FIRE_INTRO].as_deref(),
            Some("viewmodel_hamr_fire")
        );
        assert_eq!(
            out[weap_anim_extra::ADS_FIRE_INTRO].as_deref(),
            Some("viewmodel_hamr_ads_secondary_fire")
        );
    }

    #[test]
    fn t6_unique_hit_table_takes_the_first_attachment_that_differs_from_the_base() {
        let mut own = [1.0f32; weapon_t6::hitloc::HITLOC_COUNT];
        own[2] = 1.1;
        // The base unique's table need not be the gun's: only a difference from it counts.
        let mut base = vec![4.0f32; 21];
        base[9] = 0.6;
        let mut silencer = base.clone();
        silencer[2] = 1.0;
        silencer[9] = 1.0;
        let mut other = base.clone();
        other[9] = 2.0;
        other[5] = 3.0;
        assert_eq!(t6_unique_hit_table(own, &base, &[&base, &base]), own);
        let t = t6_unique_hit_table(own, &base, &[&base, &silencer]);
        assert_eq!((t[2], t[9], t[1]), (1.0, 1.0, 1.0));
        let t = t6_unique_hit_table(own, &base, &[&other, &silencer]);
        assert_eq!((t[2], t[9], t[5]), (1.0, 2.0, 3.0));
    }

    #[test]
    fn t6_classes_map_to_engine_classes() {
        assert_eq!(remap_t6_weap_class(4), 5); // pistol
        assert_eq!(remap_t6_weap_class(2), 3); // smg
        assert_eq!(remap_t6_weap_class(13), 4); // pistol spread -> spread
    }
}
