//! bo2zm: Black Ops II weapons into the weapon catalog.
//!
//! `asset_t6::WeaponRef` carries a weapon's variant and shared definition as
//! bytes plus every name they point at; this reads the fields the engine's
//! weapon rules, viewmodel and sounds use, by their T6 offsets. Field
//! meanings follow the legacy and Black Ops ones of the same name; where T6
//! numbers an enum differently (class, offhand class, type), it is mapped
//! to the legacy value the engine expects.

use fastfile_t6::layout::{WeaponDef, WeaponVariantDef};
use weapon_t6::asset_slots::weap_anim_extra;
use weapon_t6::placement::{GunRestDriftFeed, GunMotionShiftFeed};

use super::*;

/// T6 `weapAnimFiles_t` slot -> the catalog's slot (legacy numbering plus the
/// quick-reload extras). Slots legacy has no use for are left out.
const T6_ANIM_SLOTS: [(usize, usize); 59] = [
    (0x01, weap_anim_slot::IDLE),
    (0x02, weap_anim_slot::DRY_REST_CLIP),
    (0x03, weap_anim_extra::FIRE_INTRO),
    (0x04, weap_anim_slot::TRIGGER_SLOT),
    (0x05, weap_anim_slot::HOLD_FIRE),
    (0x06, weap_anim_slot::FINAL_SHOT_CLIP),
    (0x08, weap_anim_slot::RECHAMBER),
    (0x09, weap_anim_slot::CLOSE_COMBAT),
    (0x0D, weap_anim_extra::MELEE_EMPTY),
    (0x0E, weap_anim_slot::MELEE_CHARGE),
    (0x0F, weap_anim_extra::MELEE_CHARGE_EMPTY),
    (0x10, weap_anim_slot::RELOAD),
    (0x12, weap_anim_slot::RELOAD_EMPTY),
    (0x13, weap_anim_slot::RELOAD_START),
    (0x14, weap_anim_slot::RELOAD_END),
    (0x15, weap_anim_extra::RELOAD_QUICK),
    (0x16, weap_anim_extra::RELOAD_QUICK_EMPTY),
    (0x17, weap_anim_slot::RAISE),
    (0x18, weap_anim_slot::FIRST_RAISE),
    (0x19, weap_anim_slot::DROP),
    (0x1A, weap_anim_slot::ATTACHMENT_LIFT_CLIP),
    (0x1B, weap_anim_slot::ATTACHMENT_LOWER_CLIP),
    (0x1C, weap_anim_slot::QUICK_RAISE),
    (0x1D, weap_anim_slot::QUICK_DROP),
    (0x1E, weap_anim_slot::EMPTY_RAISE),
    (0x1F, weap_anim_slot::EMPTY_DROP),
    (0x20, weap_anim_slot::SPRINT_IN),
    (0x21, weap_anim_slot::SPRINT_LOOP),
    (0x22, weap_anim_slot::SPRINT_OUT),
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
    (0x3A, weap_anim_slot::DETONATE),
    (0x3B, weap_anim_slot::NIGHTVISION_WEAR),
    (0x3C, weap_anim_slot::NIGHTVISION_REMOVE),
    (0x3D, weap_anim_slot::SCOPE_SHOT_CLIP),
    (0x3E, weap_anim_slot::SCOPE_FINAL_SHOT_CLIP),
    (0x3F, weap_anim_extra::ADS_FIRE_INTRO),
    (0x40, weap_anim_slot::ADS_RECHAMBER),
    (0x41, weap_anim_extra::DIVE_IN),
    (0x42, weap_anim_extra::DIVE_LOOP),
    (0x43, weap_anim_extra::DIVE_OUT),
    (0x44, weap_anim_extra::DIVE_IN_EMPTY),
    (0x45, weap_anim_extra::DIVE_LOOP_EMPTY),
    (0x46, weap_anim_extra::DIVE_OUT_EMPTY),
    (0x4D, weap_anim_extra::CAMERA_MANTLE),
    (0x55, weap_anim_slot::SCOPE_RAISE_CLIP),
];

/// T6 asset names are case-insensitive (the knife names
/// `viewmodel_M4m203_knife_melee_1`, the zone holds `viewmodel_m4m203_...`);
/// animations are keyed lower case.
fn remap_t6_sz_xanims(t6: &[String]) -> [Option<String>; WEAPON_ANIM_SLOTS] {
    let mut slot_names = [const { None }; WEAPON_ANIM_SLOTS];
    for (src, dst) in T6_ANIM_SLOTS {
        if let Some(name) = t6.get(src).filter(|n| !n.is_empty()) {
            slot_names[dst] = Some(name.to_ascii_lowercase());
        }
    }
    // ADS down (0x56) has no legacy slot of its own beside ADS up's pair.
    if let Some(name) = t6.get(0x56).filter(|n| !n.is_empty()) {
        slot_names[weap_anim_slot::SCOPE_LOWER_CLIP] = Some(name.to_ascii_lowercase());
    }
    // A gun without empty-magazine dive, sprint, crawl or melee animations
    // (the M14 has no empty dive) plays its full ones; one without a
    // first-shots fire plays its fire.
    for (empty, full) in [
        (weap_anim_extra::DIVE_IN_EMPTY, weap_anim_extra::DIVE_IN),
        (weap_anim_extra::DIVE_LOOP_EMPTY, weap_anim_extra::DIVE_LOOP),
        (weap_anim_extra::DIVE_OUT_EMPTY, weap_anim_extra::DIVE_OUT),
        (weap_anim_extra::SPRINT_IN_EMPTY, weap_anim_slot::SPRINT_IN),
        (weap_anim_extra::SPRINT_LOOP_EMPTY, weap_anim_slot::SPRINT_LOOP),
        (weap_anim_extra::SPRINT_OUT_EMPTY, weap_anim_slot::SPRINT_OUT),
        (weap_anim_extra::CRAWL_IN_EMPTY, weap_anim_extra::CRAWL_IN),
        (weap_anim_extra::CRAWL_FORWARD_EMPTY, weap_anim_extra::CRAWL_FORWARD),
        (weap_anim_extra::CRAWL_BACK_EMPTY, weap_anim_extra::CRAWL_BACK),
        (weap_anim_extra::CRAWL_RIGHT_EMPTY, weap_anim_extra::CRAWL_RIGHT),
        (weap_anim_extra::CRAWL_LEFT_EMPTY, weap_anim_extra::CRAWL_LEFT),
        (weap_anim_extra::CRAWL_OUT_EMPTY, weap_anim_extra::CRAWL_OUT),
        (weap_anim_extra::MELEE_EMPTY, weap_anim_slot::CLOSE_COMBAT),
        (weap_anim_extra::MELEE_CHARGE_EMPTY, weap_anim_slot::MELEE_CHARGE),
        (weap_anim_extra::FIRE_INTRO, weap_anim_slot::TRIGGER_SLOT),
        (weap_anim_extra::ADS_FIRE_INTRO, weap_anim_slot::SCOPE_SHOT_CLIP),
    ] {
        if slot_names[empty].is_none() {
            slot_names[empty] = slot_names[full].clone();
        }
    }
    slot_names
}

/// bo2zm: a dual-wield pair's left-hand animations (the left weapon's
/// `dw_left_*` slots) on the catalog's slots: its other slots (raise, drop,
/// sprint) as they are, then fire 0x4E, last shot 0x4F, idle 0x51, empty
/// idle 0x52, empty reload 0x53, reload 0x54 (measured on fivesevenlh_zm and
/// m1911lh_upgraded_zm).
fn remap_t6_left_hand_xanims(t6: &[String]) -> [Option<String>; WEAPON_ANIM_SLOTS] {
    let mut out = remap_t6_sz_xanims(t6);
    for (src, dst) in [
        (0x4E, weap_anim_slot::TRIGGER_SLOT),
        (0x4F, weap_anim_slot::FINAL_SHOT_CLIP),
        (0x51, weap_anim_slot::IDLE),
        (0x52, weap_anim_slot::DRY_REST_CLIP),
        (0x53, weap_anim_slot::RELOAD_EMPTY),
        (0x54, weap_anim_slot::RELOAD),
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
        code => code,
    }
}

/// T6 `weapType_t` -> legacy kind: bullet, grenade, launched round or riot shield.
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
        kept => kept,
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

/// A view-model attachment: its model, and where it sits on the gun or the arms.
fn view_mount(
    xmodel: &str,
    tag_name: &str,
    (offset, rotation): ([f32; 3], [f32; 3]),
    on_arms: bool,
) -> (String, asset_model::FpvMountOffset) {
    let placed = asset_model::FpvMountOffset { tag: tag_name.into(), offset, rotation, on_arms };
    (String::from(xmodel), placed)
}

fn opt(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Bullet penetration (`perksEnum` 6, PERK_EXTRABP) in the low word of a perk mask.
const T6_PERK_BULLET_PENETRATION: i32 = 1 << 6;
/// BO2's PERK_ACCURACY (bit 3 of a variant's perks).
const T6_PERK_ACCURACY: i32 = 1 << 3;

fn t6_body_facts(w: &asset_t6::WeaponRef) -> WeaponChassisSummary {
    let mut f = WeaponChassisSummary {
        payload_linked: !w.def.is_empty(),
        magazine_capacity: w.var_i32(WeaponVariantDef::iClipSize),
        reload_duration_millis: w.var_i32(WeaponVariantDef::iReloadTime),
        refill_dry_millis: w.var_i32(WeaponVariantDef::iReloadEmptyTime),
        alternate_raise_time_ms: w.var_i32(WeaponVariantDef::iAltRaiseTime),
        aim_assist_range_ads: w.var_f32(WeaponVariantDef::fAimAssistRangeAds),
        aim_in_share: w.var_f32(WeaponVariantDef::fAdsZoomInFrac),
        aim_out_share: w.var_f32(WeaponVariantDef::fAdsZoomOutFrac),
        overlay_alpha_scale: w.var_f32(WeaponVariantDef::fOverlayAlphaScale),
        select_requires_ammo: Some(false),
        carries_over_talents: true,
        ..WeaponChassisSummary::default()
    };
    let ads_in = w.var_i32(WeaponVariantDef::iAdsTransInTime);
    let ads_out = w.var_i32(WeaponVariantDef::iAdsTransOutTime);
    f.scope_raise_speed = if ads_in > 0 { 1.0 / ads_in as f32 } else { 0.0 };
    f.scope_lower_speed = if ads_out > 0 { 1.0 / ads_out as f32 } else { 0.0 };
    f.aim_magnified_view = [WeaponVariantDef::fAdsZoomFov1, WeaponVariantDef::fAdsZoomFov2, WeaponVariantDef::fAdsZoomFov3]
        .into_iter()
        .map(|o| w.var_f32(o))
        .find(|&fov| fov > 0.0)
        .unwrap_or(0.0);
    if w.var_u8(WeaponVariantDef::bDualMag) != 0 {
        f.dual_mag = Some(weapon_t6::dual_mag::DualMagTimes {
            reload_ms: w.var_i32(WeaponVariantDef::iReloadQuickTime),
            reload_empty_ms: w.var_i32(WeaponVariantDef::iReloadQuickEmptyTime),
            add_ms: w.def_i32(WeaponDef::iReloadQuickAddTime),
            empty_add_ms: w.def_i32(WeaponDef::iReloadQuickEmptyAddTime),
        });
    }
    if w.def.is_empty() {
        return f;
    }
    f.shot_gap_ms = w.def_i32(WeaponDef::iFireTime);
    f.intro_fire_time_ms = w.def_i32(WeaponDef::iIntroFireTime);
    f.intro_fire_length = w.def_i32(WeaponDef::iIntroFireLength);
    f.charge_shot = w.def_i32(WeaponDef::fireType) == 8;
    f.impact_kind_id = w.def_i32(WeaponDef::impactType);
    f.draw_ms = w.def_i32(WeaponDef::iRaiseTime);
    f.holster_ms = w.def_i32(WeaponDef::iDropTime);
    f.alternate_drop_time_ms = w.def_i32(WeaponDef::iAltDropTime);
    f.shot_lead_ms = w.def_i32(WeaponDef::iFireDelay);
    f.trigger_hold_millis = w.def_i32(WeaponDef::iHoldFireTime);
    f.gun_family_code = remap_t6_weap_type(w.def_i32(WeaponDef::weapType));
    f.player_motion_kind = w.def_i32(WeaponDef::playerAnimType);
    f.gun_kind = remap_t6_weap_class(w.def_i32(WeaponDef::weapClass));
    f.secondary_hand_category = remap_t6_offhand_class(w.def_i32(WeaponDef::offhandClass));
    f.offhand_slot = w.def_i32(WeaponDef::offhandSlot);
    // BG_IsQuickSwitch: offhand slot not NONE, or bCanUseInVehicle; target
    // class PISTOL (4), CLOSE_COMBAT (11) or PISTOL_SPREAD (13).
    f.quick_switch = w.def_i32(WeaponDef::offhandSlot) != 0 || w.def_u8(WeaponDef::bCanUseInVehicle) != 0;
    f.quick_switch_target = matches!(w.def_i32(WeaponDef::weapClass), 4 | 11 | 13);
    f.rounds_each_trigger = w.def_i32(WeaponDef::shotCount);
    f.scarce_rounds_alert_level = w.def_f32(WeaponDef::lowAmmoWarningThreshold);
    f.hip_cone_up_lo = w.def_f32(WeaponDef::fHipSpreadStandMin);
    f.hip_cone_crouch_lo = w.def_f32(WeaponDef::fHipSpreadDuckedMin);
    f.hip_cone_lie_lo = w.def_f32(WeaponDef::fHipSpreadProneMin);
    f.hip_cone_up_hi = w.def_f32(WeaponDef::hipSpreadStandMax);
    f.hip_cone_crouch_hi = w.def_f32(WeaponDef::hipSpreadDuckedMax);
    f.hip_cone_lie_hi = w.def_f32(WeaponDef::hipSpreadProneMax);
    f.hip_cone_settle = w.def_f32(WeaponDef::fHipSpreadDecayRate);
    f.hip_cone_per_shot = w.def_f32(WeaponDef::fHipSpreadFireAdd);
    f.hip_cone_per_turn = w.def_f32(WeaponDef::fHipSpreadTurnAdd);
    f.hip_cone_per_step = w.def_f32(WeaponDef::fHipSpreadMoveAdd);
    f.hip_cone_crouch_settle = w.def_f32(WeaponDef::fHipSpreadDuckedDecay);
    f.hip_cone_lie_settle = w.def_f32(WeaponDef::fHipSpreadProneDecay);
    f.crosshair_arm_length = w.def_i32(WeaponDef::iReticleSideSize);
    f.crosshair_inner_gap = w.def_i32(WeaponDef::iReticleMinOfs);
    f.hipfire_reticle_flank_offset = w.def_f32(WeaponDef::fHipReticleSidePos);
    f.aimed_pitch = w.def_f32(WeaponDef::fAdsAimPitch);
    f.scope_reticle_fade_in = w.def_f32(WeaponDef::fAdsCrosshairInFrac);
    f.scope_reticle_fade_out = w.def_f32(WeaponDef::fAdsCrosshairOutFrac);
    f.aimed_cone = w.def_f32(WeaponDef::fAdsSpread);
    f.scoped_aiming = w.def_u8(WeaponDef::aimDownSight) != 0;
    f.hold_breath_to_steady = w.def_u8(WeaponDef::bHoldBreathToSteady) != 0;
    let dof = [w.def_f32(WeaponDef::adsDofStart), w.def_f32(WeaponDef::adsDofEnd)];
    f.aim_blur_depth = (dof[1] > 0.0).then_some(dof);
    f.block_aim_if_clip_dry = w.def_u8(WeaponDef::noAdsWhenMagEmpty) != 0;
    f.rack_bolt_when_scoped = w.def_u8(WeaponDef::bRechamberWhileAds) != 0;
    f.reload_lowers_ads = w.def_u8(WeaponDef::bReloadWhileAds) == 0;
    f.position_reload_trans_ms = w.def_i32(WeaponDef::iPositionReloadTransTime);
    f.fire_only_when_scoped = w.def_u8(WeaponDef::adsFireOnly) != 0;
    f.bash_hurt = w.def_i32(WeaponDef::iMeleeDamage);
    f.hud_crosshair_layer = w.def_i32(WeaponDef::overlayReticle);
    f.overlay_interface = w.def_i32(WeaponDef::overlayInterface);
    f.infrared = w.var_u8(WeaponVariantDef::bInfraRed) != 0;
    f.scope_pic_w = w.def_f32(WeaponDef::overlayWidth);
    f.scope_pic_h = w.def_f32(WeaponDef::overlayHeight);
    f.bash_ms = w.def_i32(WeaponDef::iMeleeTime);
    f.strike_wait_millis = w.def_i32(WeaponDef::iMeleeDelay);
    f.bash_windup_ms = w.def_i32(WeaponDef::meleeChargeTime);
    f.lunge_windup_wait_millis = w.def_i32(WeaponDef::meleeChargeDelay);
    f.use_as_melee = w.def_u8(WeaponDef::bUseAsMelee) != 0;
    f.fast_draw_ms = w.def_i32(WeaponDef::quickRaiseTime);
    f.fast_holster_ms = w.def_i32(WeaponDef::quickDropTime);
    f.offhand_hold_is_cancelable = Some(w.def_u8(WeaponDef::offhandHoldIsCancelable) != 0);
    f.walk_factor = w.def_f32(WeaponDef::moveSpeedScale);
    // BO2 starts the aimed walk scale at 1 and takes the gun's own only when it is set.
    f.aimed_walk_factor = Some(w.def_f32(WeaponDef::adsMoveSpeedScale)).filter(|s| *s > 0.0).unwrap_or(1.0);
    f.run_time_multiplier = w.def_f32(WeaponDef::sprintDurationScale);
    f.ducked_ofs = w.def_vec3(WeaponDef::vDuckedOfs);
    f.prone_ofs = w.def_vec3(WeaponDef::vProneOfs);
    f.goggles_equip_duration = w.def_i32(WeaponDef::nightVisionWearTime);
    f.ads_bob_factor = w.def_f32(WeaponDef::fAdsBobFactor);
    f.ads_view_bob_mult = w.def_f32(WeaponDef::fAdsViewBobMult);
    f.movement = GunMotionShiftFeed {
        stand_move: w.def_vec3(WeaponDef::vStandMove),
        stand_rot: w.def_vec3(WeaponDef::vStandRot),
        strafe_move: w.def_vec3(WeaponDef::vStrafeMove),
        strafe_rot: w.def_vec3(WeaponDef::vStrafeRot),
        ducked_move: w.def_vec3(WeaponDef::vDuckedMove),
        ducked_rot: w.def_vec3(WeaponDef::vDuckedRot),
        prone_move: w.def_vec3(WeaponDef::vProneMove),
        prone_rot: w.def_vec3(WeaponDef::vProneRot),
        pos_move_rate: w.def_f32(WeaponDef::fPosMoveRate),
        pos_prone_move_rate: w.def_f32(WeaponDef::fPosProneMoveRate),
        stand_move_min_speed: w.def_f32(WeaponDef::fStandMoveMinSpeed),
        ducked_move_min_speed: w.def_f32(WeaponDef::fDuckedMoveMinSpeed),
        prone_move_min_speed: w.def_f32(WeaponDef::fProneMoveMinSpeed),
        pos_rot_rate: w.def_f32(WeaponDef::fPosRotRate),
        pos_prone_rot_rate: w.def_f32(WeaponDef::fPosProneRotRate),
        mantle_ofs: w.def_vec3(WeaponDef::vMantleOfs),
        mantle_rot: w.def_vec3(WeaponDef::vMantleRot),
        ..GunMotionShiftFeed::default()
    };
    f.idle = GunRestDriftFeed {
        ads_idle_amount: w.def_f32(WeaponDef::fAdsIdleAmount),
        hip_idle_amount: w.def_f32(WeaponDef::fHipIdleAmount),
        ads_idle_speed: w.def_f32(WeaponDef::adsIdleSpeed),
        hip_idle_speed: w.def_f32(WeaponDef::hipIdleSpeed),
        idle_crouch_factor: w.def_f32(WeaponDef::fIdleCrouchFactor),
        idle_prone_factor: w.def_f32(WeaponDef::fIdleProneFactor),
    };
    f.wall_pierce_class = w.def_i32(WeaponDef::penetrateTWeaponAttachmentype);
    // bo2mp: Bullet_FirePenetrate scales the depth by
    // perk_bulletPenetrationMultiplier when the firing weapon has bullet
    // penetration (perk 6): the variant's own perks here (Zombies' upgraded
    // Five-seven), an attachment's in `apply_t6_attachment_stats` (FMJ).
    f.wall_pierce_scale = if w.var_i32(WeaponVariantDef::perks) & T6_PERK_BULLET_PENETRATION != 0 {
        weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT
    } else {
        1.0
    };
    f.long_gun_round = w.def_u8(WeaponDef::bRifleBullet) != 0;
    let var_vec3 = |off: usize| [w.var_f32(off), w.var_f32(off + 4), w.var_f32(off + 8)];
    f.stowed_offset = var_vec3(WeaponVariantDef::stowedModelOffsets);
    f.stowed_rotation = var_vec3(WeaponVariantDef::stowedModelRotations);
    f.carry_slot_kind = match w.def_i32(WeaponDef::inventoryType) {
        t @ 0..=3 => t,
        _ => 2,
    };
    f.trigger_mode = remap_t6_fire_type(w.def_i32(WeaponDef::fireType));
    f.t6_inventory_type = w.def_i32(WeaponDef::inventoryType);
    f.reserve_cap = w.def_i32(WeaponDef::iMaxAmmo);
    f.spawn_rounds = w.def_i32(WeaponDef::iStartAmmo);
    f.rounds_in_magazine_relative = w.def_u8(WeaponDef::ammoCountClipRelative) != 0;
    // T6 damage falls off over up to six ranges; the engine knows a near
    // and a far value.
    let damage: Vec<i32> = (0..6).map(|i| w.def_i32(WeaponDef::damage + i * 4)).collect();
    let ranges: Vec<f32> = (0..6).map(|i| w.def_f32(WeaponDef::damageRange + i * 4)).collect();
    f.damage = damage[0];
    f.full_damage_reach = ranges[0];
    let last = (1..6).rev().find(|&i| ranges[i] > 0.0 && damage[i] > 0);
    let far = last.unwrap_or(0);
    f.hurt_floor = damage[far];
    f.minimum_damage_reach = ranges[far];
    let step_ranges: [f32; 6] = std::array::from_fn(|i| ranges[i]);
    let backwards: Vec<usize> = weapon_t6::damage::DamageSteps::backwards_ranges(&step_ranges)
        .iter()
        .enumerate()
        .filter_map(|(at, &b)| b.then_some(at))
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
    f.smallest_hurt_amount = w.def_i32(WeaponDef::minPlayerDamage);
    f.cycle_ms = w.def_i32(WeaponDef::iRechamberTime);
    f.bolt_rack_duration_millis = w.def_i32(WeaponDef::iRechamberBoltTime);
    f.rocket_reappear_millis = w.def_i32(WeaponDef::reloadShowRocketTime);
    f.reload_refill_millis = w.def_i32(WeaponDef::iReloadAddTime);
    f.empty_reload_refill_millis = w.def_i32(WeaponDef::iReloadEmptyAddTime);
    f.refill_begin_millis = w.def_i32(WeaponDef::iReloadStartTime);
    f.reload_begin_refill_millis = w.def_i32(WeaponDef::iReloadStartAddTime);
    f.refill_finish_millis = w.def_i32(WeaponDef::iReloadEndTime);
    f.reload_rounds_gained = w.def_i32(WeaponDef::iReloadAmmoAdd);
    f.reload_begin_bonus = w.def_i32(WeaponDef::iReloadStartAdd);
    f.forbid_topup_reload = w.def_u8(WeaponDef::bNoPartialReload) != 0;
    // An overheating gun (Buried's Paralyzer, clip 0) heats up instead of
    // spending ammo, so it fires with an empty clip.
    f.unlimited_ammo = w.def_u8(WeaponDef::unlimitedAmmo) != 0 || w.def_u8(WeaponDef::overheatWeapon) != 0;
    f.manual_cycle = w.def_u8(WeaponDef::bBoltAction) != 0;
    f.retrievable = w.def_u8(WeaponDef::bRetrievable) != 0;
    f.staged_reload = w.def_u8(WeaponDef::bSegmentedReload) != 0;
    f.run_lift_millis = w.def_i32(WeaponDef::sprintInTime);
    f.run_cycle_millis = w.def_i32(WeaponDef::sprintLoopTime);
    f.run_lower_millis = w.def_i32(WeaponDef::sprintOutTime);
    f.dive_times_ms = [WeaponDef::dtpInTime, WeaponDef::dtpLoopTime, WeaponDef::dtpOutTime].map(|o| w.def_i32(o));
    f.crawl_times_ms = [
        WeaponDef::crawlInTime,
        WeaponDef::crawlForwardTime,
        WeaponDef::crawlBackTime,
        WeaponDef::crawlRightTime,
        WeaponDef::crawlLeftTime,
        WeaponDef::crawlOutFireTime,
        WeaponDef::crawlOutTime,
    ]
    .map(|o| w.def_i32(o));
    f.detonation_delay_millis = w.def_i32(WeaponDef::fuseTime);
    f.auto_aim_range = w.def_f32(WeaponDef::autoAimRange);
    f.aim_assist_range = w.def_f32(WeaponDef::aimAssistRange);
    f.grenade_cookoff_wait = w.def_u8(WeaponDef::bCookOffHold) != 0;
    f.motion_clip_exclusive = w.def_u8(WeaponDef::bClipOnly) != 0;
    f.has_detonator = w.def_u8(WeaponDef::hasDetonator) != 0;
    f.detonate_delay_ms = w.def_i32(WeaponDef::iDetonateDelay);
    f.detonate_time_ms = w.def_i32(WeaponDef::iDetonateTime);
    f.projectile_rotates = w.def_u8(WeaponDef::rotate) != 0;
    f.projectile_rotate_type = w.def_i32(WeaponDef::rotateType);
    // bo2zm fix list 2: BO2's own minimums only matter this way (see
    // `WeaponChassisSummary::spread_before_fire_add`).
    f.spread_before_fire_add = true;
    // Bullet_Endpos: in Zombies, BG_WeaponHasPerk(weapon, PERK_ACCURACY)
    // (bit 3 of the variant's perks) puts the pellets in a flat line.
    f.flat_spread = w.var_i32(WeaponVariantDef::perks) & T6_PERK_ACCURACY != 0;
    f.timed_detonation = w.def_u8(WeaponDef::timedDetonation) != 0;
    f.explode_on_ground = w.def_u8(WeaponDef::bExplodeOnGround) != 0;
    f.shell_detonate_on_hit = w.def_u8(WeaponDef::bProjImpactExplode) != 0;
    f.bullet_impact_explode = w.def_u8(WeaponDef::bBulletImpactExplode) != 0;
    f.force_bounce = w.def_u8(WeaponDef::bForceBounce) != 0;
    // bo2mp: what its projectile is when it goes off (BO2's projExplosion: 2 a
    // flash, 6 a heavy explosive, 9 a bolt).
    f.shell_blast_kind = w.def_i32(WeaponDef::projExplosion);
    f.laser_sensor = w.def_u8(WeaponDef::isLaserSensor) != 0;
    f.blast_outer_reach = w.def_i32(WeaponDef::iExplosionRadius);
    f.blast_reach_floor = w.def_i32(WeaponDef::iExplosionRadiusMin);
    f.blast_core_damage = w.def_i32(WeaponDef::iExplosionInnerDamage);
    f.blast_rim_damage = w.def_i32(WeaponDef::iExplosionOuterDamage);
    f.damage_cone_angle = w.def_f32(WeaponDef::damageConeAngle);
    f.stickiness = w.def_i32(WeaponDef::stickiness);
    // bo2mp: the grenade warning's own facts (BO2's raw weapType, iIndicatorRadius).
    f.t6_weap_type = w.def_i32(WeaponDef::weapType);
    f.t6_weap_class = w.def_i32(WeaponDef::weapClass);
    f.t6_fire_type = w.def_i32(WeaponDef::fireType);
    f.indicator_radius = w.def_i32(WeaponDef::iIndicatorRadius);
    // Sticks to everything, or to flesh: it sticks to the player it hits.
    f.adheres_to_bodies = matches!(f.stickiness, 1 | 5);
    f.missile_velocity = w.def_i32(WeaponDef::iProjectileSpeed);
    f.missile_upward_velocity = w.def_i32(WeaponDef::iProjectileSpeedUp);
    f.projectile_speed_forward = w.def_i32(WeaponDef::iProjectileSpeedForward);
    f.projectile_speed_relative_up = w.def_i32(WeaponDef::iProjectileSpeedRelativeUp);
    f.projectile_parent_velocity = Some(w.def_f32(WeaponDef::fProjectileTakeParentVelocity));
    f.shell_arming_range = w.def_i32(WeaponDef::iProjectileActivateDist);
    // How it steers (BO2's guidedMissileType: the Hunter Killer's drone is 7,
    // the SMAW and FHJ-18 are 1) and its soft launch.
    f.missile_guidance = w.def_i32(WeaponDef::guidedMissileType);
    f.max_steering_accel = w.def_f32(WeaponDef::maxSteeringAccel);
    f.ignition_delay_ms = w.def_i32(WeaponDef::projIgnitionDelay);
    let array31 = |v: &Option<Vec<f32>>| -> Option<[f32; 31]> {
        let v = v.as_ref()?;
        let mut vals = [0.0; 31];
        for (o, x) in vals.iter_mut().zip(v.iter()) {
            *o = *x;
        }
        Some(vals)
    };
    f.along_rebound = array31(&w.along_rebound);
    f.across_rebound = array31(&w.across_rebound);
    f.location_damage_mult = w.location_damage.as_ref().map(|v| {
        let mut out = [1.0f32; weapon_t6::hitloc::HITLOC_COUNT];
        for (o, x) in out.iter_mut().zip(v.iter()) {
            *o = *x;
        }
        out
    });
    f.forbid_twin_grip = w.def_u8(WeaponDef::bDualWield) == 0;
    f.kick = WeaponRecoilSummary {
        scoped_view_kick_recenter_speed: w.var_f32(WeaponVariantDef::fAdsViewKickCenterSpeed),
        hipfire_view_kick_recenter_speed: w.var_f32(WeaponVariantDef::fHipViewKickCenterSpeed),
        weapon_pitch_ceiling: w.def_f32(WeaponDef::fGunMaxPitch),
        weapon_yaw_ceiling: w.def_f32(WeaponDef::fGunMaxYaw),
        aimed_soft_kick_rounds: w.def_i32(WeaponDef::adsGunKickReducedKickBullets),
        scoped_weapon_kick_damped_percent: w.def_f32(WeaponDef::adsGunKickReducedKickPercent),
        scoped_weapon_kick_pitch_bottom: w.def_f32(WeaponDef::fAdsGunKickPitchMin),
        scoped_weapon_kick_pitch_top: w.def_f32(WeaponDef::fAdsGunKickPitchMax),
        scoped_weapon_kick_yaw_bottom: w.def_f32(WeaponDef::fAdsGunKickYawMin),
        scoped_weapon_kick_yaw_top: w.def_f32(WeaponDef::fAdsGunKickYawMax),
        scoped_weapon_kick_accel: w.def_f32(WeaponDef::fAdsGunKickAccel),
        scoped_weapon_kick_velocity_cap: w.def_f32(WeaponDef::fAdsGunKickSpeedMax),
        scoped_weapon_kick_velocity_falloff: w.def_f32(WeaponDef::fAdsGunKickSpeedDecay),
        scoped_weapon_kick_resting_falloff: w.def_f32(WeaponDef::fAdsGunKickStaticDecay),
        scoped_camera_kick_pitch_bottom: w.def_f32(WeaponDef::fAdsViewKickPitchMin),
        scoped_camera_kick_pitch_top: w.def_f32(WeaponDef::fAdsViewKickPitchMax),
        scoped_camera_kick_yaw_bottom: w.def_f32(WeaponDef::fAdsViewKickYawMin),
        scoped_camera_kick_yaw_top: w.def_f32(WeaponDef::fAdsViewKickYawMax),
        hip_soft_kick_rounds: w.def_i32(WeaponDef::hipGunKickReducedKickBullets),
        hipfire_weapon_kick_damped_percent: w.def_f32(WeaponDef::hipGunKickReducedKickPercent),
        hipfire_weapon_kick_pitch_bottom: w.def_f32(WeaponDef::fHipGunKickPitchMin),
        hipfire_weapon_kick_pitch_top: w.def_f32(WeaponDef::fHipGunKickPitchMax),
        hipfire_weapon_kick_yaw_bottom: w.def_f32(WeaponDef::fHipGunKickYawMin),
        hipfire_weapon_kick_yaw_top: w.def_f32(WeaponDef::fHipGunKickYawMax),
        hipfire_weapon_kick_accel: w.def_f32(WeaponDef::fHipGunKickAccel),
        hipfire_weapon_kick_velocity_cap: w.def_f32(WeaponDef::fHipGunKickSpeedMax),
        hipfire_weapon_kick_velocity_falloff: w.def_f32(WeaponDef::fHipGunKickSpeedDecay),
        hipfire_weapon_kick_resting_falloff: w.def_f32(WeaponDef::fHipGunKickStaticDecay),
        hipfire_camera_kick_pitch_bottom: w.def_f32(WeaponDef::fHipViewKickPitchMin),
        hipfire_camera_kick_pitch_top: w.def_f32(WeaponDef::fHipViewKickPitchMax),
        hipfire_camera_kick_yaw_bottom: w.def_f32(WeaponDef::fHipViewKickYawMin),
        hipfire_camera_kick_yaw_top: w.def_f32(WeaponDef::fHipViewKickYawMax),
        hip_view_kick_min_magnitude: w.def_f32(WeaponDef::fHipViewKickMinMagnitude),
        ads_view_kick_min_magnitude: w.def_f32(WeaponDef::fAdsViewKickMinMagnitude),
    };
    f.sway = WeaponDriftSummary {
        drift_cap: w.def_f32(WeaponDef::swayMaxAngle),
        drift_ease: w.def_f32(WeaponDef::swayLerpSpeed),
        drift_pitch_k: w.def_f32(WeaponDef::swayPitchScale),
        drift_yaw_k: w.def_f32(WeaponDef::swayYawScale),
        drift_side_k: w.def_f32(WeaponDef::swayHorizScale),
        drift_up_k: w.def_f32(WeaponDef::swayVertScale),
        drift_stun_k: w.def_f32(WeaponDef::swayShellShockScale),
        aim_drift_angle_cap: w.def_f32(WeaponDef::adsSwayMaxAngle),
        aim_drift_blend_rate: w.def_f32(WeaponDef::adsSwayLerpSpeed),
        aim_drift_tilt_gain: w.def_f32(WeaponDef::adsSwayPitchScale),
        aim_drift_turn_gain: w.def_f32(WeaponDef::adsSwayYawScale),
        aim_drift_sideways_gain: w.var_f32(WeaponVariantDef::fAdsSwayHorizScale),
        aim_drift_vertical_gain: w.var_f32(WeaponVariantDef::fAdsSwayVertScale),
    };
    if f.payload_linked {
        let pair = |off: usize| [w.def_f32(off), w.def_f32(off + 4)];
        f.bob = WeaponBobFacts {
            sprint_cycle_scale: w.def_f32(WeaponDef::fSprintCycleScale),
            ducked_sprint_cycle_scale: w.def_f32(WeaponDef::fDuckedSprintCycleScale),
            dtp_cycle_scale: w.def_f32(WeaponDef::fDtpCycleScale),
            sprint_bob: pair(WeaponDef::vSprintBob),
            dtp_bob: pair(WeaponDef::vDtpBob),
        };
    }
    f
}

fn t6_sounds(w: &asset_t6::WeaponRef) -> WeaponAudioNicknames {
    let s = |off: usize| opt(w.def_string(off));
    WeaponAudioNicknames {
        fire: s(WeaponDef::fireSound),
        trigger_sound_name: s(WeaponDef::fireSoundPlayer),
        empty_fire: s(WeaponDef::emptyFireSound),
        empty_fire_player: s(WeaponDef::emptyFireSoundPlayer),
        melee_swipe: s(WeaponDef::meleeSwipeSound),
        melee_swipe_player: s(WeaponDef::meleeSwipeSoundPlayer),
        melee_hit: s(WeaponDef::meleeHitSound),
        melee_miss: s(WeaponDef::meleeMissSound),
        pickup: s(WeaponDef::pickupSound),
        pickup_sound_name: s(WeaponDef::pickupSoundPlayer),
        ammo_pickup: s(WeaponDef::ammoPickupSound),
        ammo_pickup_player: s(WeaponDef::ammoPickupSoundPlayer),
        pullback: s(WeaponDef::pullbackSound),
        pullback_player: s(WeaponDef::pullbackSoundPlayer),
        reload: s(WeaponDef::reloadSound),
        reload_player: s(WeaponDef::reloadSoundPlayer),
        reload_empty: s(WeaponDef::reloadEmptySound),
        reload_empty_player: s(WeaponDef::reloadEmptySoundPlayer),
        reload_start: s(WeaponDef::reloadStartSound),
        reload_start_player: s(WeaponDef::reloadStartSoundPlayer),
        reload_end: s(WeaponDef::reloadEndSound),
        reload_end_player: s(WeaponDef::reloadEndSoundPlayer),
        rechamber: s(WeaponDef::rechamberSound),
        rechamber_player: s(WeaponDef::rechamberSoundPlayer),
        alt_switch: s(WeaponDef::altSwitchSound),
        alt_switch_player: s(WeaponDef::altSwitchSoundPlayer),
        raise: s(WeaponDef::raiseSound),
        raise_player: s(WeaponDef::raiseSoundPlayer),
        first_raise: s(WeaponDef::firstRaiseSound),
        first_raise_player: s(WeaponDef::firstRaiseSoundPlayer),
        putaway: s(WeaponDef::putawaySound),
        putaway_player: s(WeaponDef::putawaySoundPlayer),
        projectile_blast_fx: s(WeaponDef::projExplosionSound),
        projectile: s(WeaponDef::projectileSound),
        proj_ignition_sound: s(WeaponDef::projIgnitionSound),
        fire_loop: s(WeaponDef::fireLoopSound),
        fire_loop_player: s(WeaponDef::fireLoopSoundPlayer),
        fire_stop: s(WeaponDef::fireStopSound),
        fire_stop_player: s(WeaponDef::fireStopSoundPlayer),
        final_round_fired: s(WeaponDef::fireLastSound),
        fire_last_player: s(WeaponDef::fireLastSoundPlayer),
        ads_raise_player: s(WeaponDef::adsRaiseSoundPlayer),
        ads_lower_player: s(WeaponDef::adsLowerSoundPlayer),
        ads_zoom: s(WeaponDef::adsZoomSound),
        spin_loop: s(WeaponDef::spinLoopSound),
        spin_loop_player: s(WeaponDef::spinLoopSoundPlayer),
        start_spin: s(WeaponDef::startSpinSound),
        start_spin_player: s(WeaponDef::startSpinSoundPlayer),
        stop_spin: s(WeaponDef::stopSpinSound),
        stop_spin_player: s(WeaponDef::stopSpinSoundPlayer),
        cue_to_audio_lookup: w.notetrack_sounds.clone(),
        ricochet: std::array::from_fn(|surf| w.bounce_sounds.get(surf).and_then(|n| opt(n))),
        ..WeaponAudioNicknames::default()
    }
}

impl ArmoryRoster {
    /// bo2zm: one Black Ops II weapon (a variant with its definition).
    /// Returns false for a reference stub (a comma-named asset another zone
    /// defines) or a weapon without a name.
    /// bo2zm fix list 2: a rolling grenade's rest radius (measured on its
    /// projectile model by the lane, which holds the models).
    pub fn set_t6_rolling_radius(&mut self, name: &str, roll_radius: f32) {
        for e in self.captured.iter_mut().filter(|e| e.name == name) {
            e.facts.rolling_radius = Some(roll_radius);
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
        // A reference stub (comma-named) or an unnamed asset is another zone's to define.
        if matches!(w.name.chars().next(), None | Some(',')) {
            return false;
        }
        let camo_or_first = |name: &str, models: &[String]| {
            let first = || models.first().and_then(|n| opt(n));
            camo_models.get(name).cloned().or_else(first)
        };
        // bo2zm: BO2's Turned zombie holds the left half of a pair
        // (`zombiemelee_dw`: its own moves are the left claw's alone); the
        // right claw moves with its partner's (`zombiemelee_zm`). Both arms
        // show and move, as with a pair of pistols.
        let has = |x: &[String], i: usize| x.get(i).is_some_and(|n| !n.is_empty());
        let held_left_half =
            left.filter(|p| !has(&w.animation_refs, 0x01) && has(&w.animation_refs, 0x51) && has(&p.animation_refs, 0x01));
        let left = left.filter(|_| w.def_u8(WeaponDef::bDualWield) != 0 && held_left_half.is_none());
        // The right hand's moves: the partner's, else the weapon's own.
        let right_t6: Vec<String> = match held_left_half {
            Some(p) => (0..w.animation_refs.len().max(p.animation_refs.len()))
                .map(|i| {
                    let partner = p.animation_refs.get(i).filter(|n| !n.is_empty());
                    partner.or_else(|| w.animation_refs.get(i)).cloned().unwrap_or_default()
                })
                .collect(),
            None => w.animation_refs.clone(),
        };
        let left_gun = left.and_then(|l| camo_or_first(&l.name, &l.gun_models));
        let hint = |text: &str| opt(text);
        let reticle = WeaponCrosshairResources {
            center_authored: !w.def_material(WeaponDef::reticleCenter).is_empty(),
            side_authored: !w.def_material(WeaponDef::reticleSide).is_empty(),
            middle_surface: hint(w.def_material(WeaponDef::reticleCenter)),
            flank_surface: hint(w.def_material(WeaponDef::reticleSide)),
            middle_extent: w.def_i32(WeaponDef::iReticleCenterSize),
            flank_extent: w.def_i32(WeaponDef::iReticleSideSize),
            indicator_icon: hint(w.def_material(WeaponDef::indicatorIcon)),
            ..WeaponCrosshairResources::default()
        };
        let combat_fx = WeaponBattleEffects {
            own_muzzle_name: hint(w.def_fx(WeaponDef::viewFlashEffect)),
            far_muzzle_name: hint(w.def_fx(WeaponDef::worldFlashEffect)),
            own_casing_name: hint(w.def_fx(WeaponDef::viewShellEjectEffect)),
            far_casing_name: hint(w.def_fx(WeaponDef::worldShellEjectEffect)),
            blast_clue: hint(w.def_fx(WeaponDef::projExplosionEffect)),
            streak_name: hint(w.def_tracer(WeaponDef::tracerType)),
            ..WeaponBattleEffects::default()
        };
        let carried = w
            .attach_view_models
            .iter()
            .map(|(_, model, tag, offset, rotation)| view_mount(model, tag, (*offset, *rotation), false));
        let left_mount = left_gun.map(|model| view_mount(&model, "tag_weapon1", ([0.0; 3], [0.0; 3]), true));
        let own_anims = || remap_t6_sz_xanims(&right_t6);
        let none_anims = || [const { None }; WEAPON_ANIM_SLOTS];
        let world_parts = w
            .attach_world_models
            .iter()
            .map(|(_, model, mount_tag, offset, rotation)| {
                let at = asset_model::FpvMountOffset {
                    tag: mount_tag.clone(),
                    offset: *offset,
                    rotation: *rotation,
                    on_arms: false,
                };
                (model.clone(), at)
            })
            .collect();
        let mut facts = t6_body_facts(w);
        facts.forbid_twin_grip &= held_left_half.is_none();
        let weapon = RosterWeapon {
            namespace: self.into_ns,
            name: w.name.clone(),
            alternate_weapon: hint(&w.alt_weapon_name),
            shown_title_token: hint(&w.shown_label),
            reticle,
            hud_layer_surface: hint(&w.hud_layer_surface).or_else(|| hint(&w.overlay_material_low)),
            badge_pic: hint(w.def_material(WeaponDef::hudIcon)),
            elimination_emblem: hint(w.def_material(WeaponDef::killIcon)),
            shell_wake: hint(w.def_fx(WeaponDef::projTrailEffect)),
            shell_ignite: hint(w.def_fx(WeaponDef::projIgnitionEffect)),
            weapon_mesh: camo_or_first(&w.name, &w.gun_models),
            third_person_mesh: w.world_models.first().and_then(|n| hint(n)),
            shell_mesh: hint(w.def_model(WeaponDef::projectileModel)),
            missile_mesh: hint(w.def_model(WeaponDef::rocketModel)),
            clip_names: own_anims(),
            right_clip_names: match (left, held_left_half) {
                (None, None) => none_anims(),
                _ => own_anims(),
            },
            left_clip_names: match (left, held_left_half) {
                (Some(l), _) => remap_t6_left_hand_xanims(&l.animation_refs),
                (None, Some(_)) => remap_t6_left_hand_xanims(&w.animation_refs),
                (None, None) => none_anims(),
            },
            // The held left half names its partner, as BO2's own
            // `szDualWieldWeaponName` does, so it is given as both hands.
            dual_wield_weapon: left.or(held_left_half).map(|l| l.name.clone()),
            spawned_grenade_weapon: hint(w.def_string(WeaponDef::szSpawnedGrenadeWeaponName)),
            concealed_bones: w.concealed_bones.clone(),
            ads_view_attachments: vec![],
            world_attachments: vec![],
            world_parts,
            view_attachments: carried.chain(left_mount).collect(),
            sounds: t6_sounds(w),
            combat_fx,
            facts,
        };
        self.captured.push(weapon);
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
    f: &mut WeaponChassisSummary,
    w: &asset_t6::WeaponRef,
    attachment: &asset_t6::AttachmentRef,
) {
    use fastfile_t6::layout::WeaponAttachment;
    let scale_ms = |ms: &mut i32, scale: f32| {
        if scale > 0.0 && *ms > 0 {
            *ms = (*ms as f32 * scale) as i32;
        }
    };
    // Clip size (BO2: only stack fire sets it outright; every other attachment scales it, so
    // the grenade launcher's own clip number never reaches its rifle).
    const ATTACHMENT_STACKFIRE: i32 = 24;
    let clip = attachment.int_from_bytes(WeaponAttachment::iClipSize);
    let clip_scale = attachment.float_from_bytes(WeaponAttachment::clipSizeScale);
    if attachment.int_from_bytes(WeaponAttachment::attachmentType) == ATTACHMENT_STACKFIRE {
        f.magazine_capacity = clip;
    } else if clip_scale > 0.0 && f.magazine_capacity > 0 {
        f.magazine_capacity = ((f.magazine_capacity as f32 * clip_scale).round() as i32).max(1);
    }
    // Fire time.
    scale_ms(&mut f.shot_gap_ms, attachment.float_from_bytes(WeaponAttachment::fFireTimeScale));
    // Reloads.
    scale_ms(&mut f.reload_duration_millis, attachment.float_from_bytes(WeaponAttachment::fReloadTimeScale));
    scale_ms(&mut f.refill_dry_millis, attachment.float_from_bytes(WeaponAttachment::fReloadEmptyTimeScale));
    scale_ms(&mut f.reload_refill_millis, attachment.float_from_bytes(WeaponAttachment::fReloadAddTimeScale));
    if attachment.byte_field(WeaponAttachment::bDualMag) != 0 && f.dual_mag.is_none() {
        f.dual_mag = Some(weapon_t6::dual_mag::DualMagTimes {
            reload_ms: w.var_i32(WeaponVariantDef::iReloadQuickTime),
            reload_empty_ms: w.var_i32(WeaponVariantDef::iReloadQuickEmptyTime),
            add_ms: w.def_i32(WeaponDef::iReloadQuickAddTime),
            empty_add_ms: w.def_i32(WeaponDef::iReloadQuickEmptyAddTime),
        });
    }
    if let Some(quick) = f.dual_mag.as_mut() {
        scale_ms(&mut quick.reload_ms, attachment.float_from_bytes(WeaponAttachment::fReloadQuickTimeScale));
        scale_ms(&mut quick.reload_empty_ms, attachment.float_from_bytes(WeaponAttachment::fReloadQuickEmptyTimeScale));
        scale_ms(&mut quick.add_ms, attachment.float_from_bytes(WeaponAttachment::fReloadQuickAddTimeScale));
    }
    // ADS in and out (a rate: a shorter time is a faster rate).
    for (rate, scale) in [
        (&mut f.scope_raise_speed, attachment.float_from_bytes(WeaponAttachment::fAdsTransInTimeScale)),
        (&mut f.scope_lower_speed, attachment.float_from_bytes(WeaponAttachment::fAdsTransOutTimeScale)),
    ] {
        if scale > 0.0 {
            *rate /= scale;
        }
    }
    // Aimed walk speed: an attachment that sets its own (Stalker) replaces the gun's, it does
    // not scale it. "Sets its own" means it differs from the empty attachment's, which is 1
    // in the zones (`defaultattachment`); every other attachment there holds 1 too.
    const EMPTY_ATTACHMENT_ADS_MOVE: f32 = 1.0;
    let ads_move = attachment.float_from_bytes(WeaponAttachment::adsMoveSpeedScale);
    if ads_move > 0.0 && ads_move != EMPTY_ATTACHMENT_ADS_MOVE {
        f.aimed_walk_factor = ads_move;
    }
    // Hip spread.
    let (min, max) = (attachment.float_from_bytes(WeaponAttachment::fHipSpreadMinScale), attachment.float_from_bytes(WeaponAttachment::fHipSpreadMaxScale));
    if min > 0.0 {
        for s in [&mut f.hip_cone_up_lo, &mut f.hip_cone_crouch_lo, &mut f.hip_cone_lie_lo] {
            *s *= min;
        }
    }
    if max > 0.0 {
        for s in [&mut f.hip_cone_up_hi, &mut f.hip_cone_crouch_hi, &mut f.hip_cone_lie_hi] {
            *s *= max;
        }
    }
    // Damage ranges.
    let range = attachment.float_from_bytes(WeaponAttachment::fDamageRangeScale);
    if range > 0.0 {
        f.full_damage_reach *= range;
        f.minimum_damage_reach *= range;
        if let Some(steps) = &mut f.damage_steps {
            steps.range_scale *= range;
        }
    }
    // Aimed kick and idle sway.
    let centre = attachment.float_from_bytes(WeaponAttachment::fAdsViewKickCenterSpeedScale);
    if centre > 0.0 {
        f.kick.scoped_view_kick_recenter_speed *= centre;
    }
    let idle = attachment.float_from_bytes(WeaponAttachment::fAdsIdleAmountScale);
    if idle > 0.0 {
        f.idle.ads_idle_amount *= idle;
    }
    let hip_idle = attachment.float_from_bytes(WeaponAttachment::fHipIdleAmount);
    if hip_idle > 0.0 {
        f.idle.hip_idle_amount = hip_idle;
    }
    // Penetration: BG_GetPenetrateType takes the first attachment that has a
    // type, else the gun's own. FMJ is Black Ops II's bullet penetration
    // (MW2's perk, an attachment here): walls and bodies are
    // perk_bulletPenetrationMultiplier times as deep.
    let pen = attachment.int_from_bytes(WeaponAttachment::penetrateType);
    if pen != 0 && !f.penetrate_type_from_attachment {
        f.wall_pierce_class = pen;
        f.penetrate_type_from_attachment = true;
    }
    if attachment.int_from_bytes(WeaponAttachment::perks) & T6_PERK_BULLET_PENETRATION != 0 {
        f.wall_pierce_scale = weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT;
    }
    // Suppressor: no firing ping on the enemy's minimap.
    if attachment.byte_field(WeaponAttachment::bSilenced) != 0 {
        f.silenced = true;
    }
    // Sway it overrides, hip and aimed.
    if attachment.byte_field(WeaponAttachment::swayOverride) != 0 {
        f.sway.drift_cap = attachment.float_from_bytes(WeaponAttachment::swayMaxAngle);
        f.sway.drift_ease = attachment.float_from_bytes(WeaponAttachment::swayLerpSpeed);
        f.sway.drift_pitch_k = attachment.float_from_bytes(WeaponAttachment::swayPitchScale);
        f.sway.drift_yaw_k = attachment.float_from_bytes(WeaponAttachment::swayYawScale);
        f.sway.drift_side_k = attachment.float_from_bytes(WeaponAttachment::swayHorizScale);
        f.sway.drift_up_k = attachment.float_from_bytes(WeaponAttachment::swayVertScale);
    }
    if attachment.byte_field(WeaponAttachment::adsSwayOverride) != 0 {
        f.sway.aim_drift_angle_cap = attachment.float_from_bytes(WeaponAttachment::adsSwayMaxAngle);
        f.sway.aim_drift_blend_rate = attachment.float_from_bytes(WeaponAttachment::adsSwayLerpSpeed);
        f.sway.aim_drift_tilt_gain = attachment.float_from_bytes(WeaponAttachment::adsSwayPitchScale);
        f.sway.aim_drift_turn_gain = attachment.float_from_bytes(WeaponAttachment::adsSwayYawScale);
        f.sway.aim_drift_sideways_gain = attachment.float_from_bytes(WeaponAttachment::fAdsSwayHorizScale);
        f.sway.aim_drift_vertical_gain = attachment.float_from_bytes(WeaponAttachment::fAdsSwayVertScale);
    }
}

impl ArmoryRoster {
    /// bo2mp: the alternate mode `name` (already captured) shares its gun's
    /// magazine: Black Ops II's defs name the same ammo and clip (Select
    /// Fire). The rows of the pair key one magazine and one reserve.
    pub fn mark_t6_shared_magazine(&mut self, mag_name: &str) {
        let ns = self.into_ns;
        if let Some(e) = self.captured.iter_mut().rev().find(|e| e.name == mag_name && e.namespace == ns) {
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
        use fastfile_t6::layout::WeaponAttachment;
        use fastfile_t6::layout::WeaponAttachmentUnique;
        let ns = self.into_ns;
        let Some(mut e) = self
            .captured
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
            e.hud_layer_surface =
                opt(&unique.hud_layer_surface).or_else(|| opt(&unique.overlay_material_low));
            e.facts.hud_crosshair_layer = unique.int_from_bytes(WeaponAttachmentUnique::overlayReticle);
        }
        // BO2 `BG_IsInfraRed`: any infrared attachment makes the gun infrared.
        // BO2 ATTACHMENT_RANGEFINDER (Target Finder): full ADS draws its sight screen.
        if attachment.name == "rangefinder" {
            e.facts.rangefinder = true;
        }
        if attachment.byte_field(WeaponAttachment::bInfraRed) != 0 {
            e.facts.infrared = true;
        }
        // Models: the gun's own attachment models, less the one in the
        // attachment's point when the unique replaces it (a sniper's scope
        // under an ACOG), moved when it says so; then the unique's.
        let point = attachment.int_from_bytes(WeaponAttachment::attachmentPoint);
        let replace = unique.bytes.get(WeaponAttachmentUnique::disableBaseWeaponAttachment).is_some_and(|&b| b != 0);
        let moved = unique
            .bytes
            .get(WeaponAttachmentUnique::overrideBaseWeaponAttachmentOffsets)
            .is_some_and(|&b| b != 0);
        let mut views: Vec<(String, asset_model::FpvMountOffset)> = Vec::new();
        for (slot, model, tag, offset, rotation) in &w.attach_view_models {
            // Attachment points count from 1 (1: the sight on top); the
            // gun's own attachment models from 0 (0: a sniper's scope).
            let here = usize::try_from(point - 1).is_ok_and(|p| p == *slot);
            if here && replace {
                continue;
            }
            let offset = match here && moved {
                true => unique.vec3_at(WeaponAttachmentUnique::viewModelOffsetBaseAttachment),
                false => *offset,
            };
            views.push(view_mount(model, tag, (offset, *rotation), false));
        }
        for (model, offset, rotation) in [
            (&unique.view_model, WeaponAttachmentUnique::viewModelOffsets, WeaponAttachmentUnique::viewModelRotations),
            (
                &unique.view_model_additional,
                WeaponAttachmentUnique::viewModelAddOffsets,
                WeaponAttachmentUnique::viewModelAddRotations,
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
                .map(|(mesh, at)| {
                    if *mesh == main {
                        (ads.clone(), at.clone())
                    } else {
                        (mesh.clone(), at.clone())
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
            &mut e.sounds.trigger_sound_name,
            &mut e.sounds.fire_loop,
            &mut e.sounds.fire_loop_player,
            &mut e.sounds.fire_stop,
            &mut e.sounds.fire_stop_player,
            &mut e.sounds.final_round_fired,
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
            e.combat_fx.own_muzzle_name = Some(name);
        }
        if let Some(name) = opt(&unique.world_flash) {
            e.combat_fx.far_muzzle_name = Some(name);
        }
        // The unique's hidden tags (the iron sights under a reflex).
        if !unique.concealed_bones.is_empty() {
            e.concealed_bones = unique.concealed_bones.clone();
        }
        // Its animations (ADS up and down to the sight's height).
        // (The alternate keeps its own: they are the other mode's.)
        let animated = remap_t6_sz_xanims(&unique.animation_refs).into_iter().enumerate();
        for (slot, name) in animated.filter(|(_, name)| alt_of.is_none() && name.is_some()) {
            e.clip_names[slot] = name;
        }
        // Dual Wield (`dw`): the unique's left-hand gun in the other hand,
        // on the arms, its `dw_left_*` animations driving the left arm and
        // the gun's (with the unique's) the right.
        if let Some(l) = dual_wield_left {
            e.right_clip_names = e.clip_names.clone();
            e.left_clip_names = remap_t6_left_hand_xanims(&l.animation_refs);
            e.dual_wield_weapon = Some(l.name.clone());
            e.facts.forbid_twin_grip = false;
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
            && let Some(up) = w.animation_refs.get(0x57).filter(|n| !n.is_empty())
        {
            e.clip_names[weap_anim_slot::SCOPE_RAISE_CLIP] = Some(up.to_ascii_lowercase());
        }
        // Zoom: the attachment's field of view (1: the gun's own) and its
        // zoom-in and -out windows when it names them.
        // (The Hybrid has two: the attachment's first (30) is the tube's, its
        // second (50, the gun's own) the reflex window's. The gun's first mode
        // aims through the window (its ADS slots 0x55/0x56 are the `top` ones,
        // its zoom-in/out windows and idle scale are a reflex's, not an
        // ACOG's); the alternate looks through the tube.)
        let fov = if sight { attachment.float_from_bytes(WeaponAttachment::fAdsZoomFov1) } else { 0.0 };
        if hybrid {
            let mode_fov = if alt_of.is_some() { fov } else { attachment.float_from_bytes(WeaponAttachment::fAdsZoomFov2) };
            if mode_fov > 1.0 {
                e.facts.aim_magnified_view = mode_fov;
            }
        } else if fov > 1.0 {
            e.facts.aim_magnified_view = fov;
        }
        // Variable Zoom names no first zoom (1) and two others: it opens at
        // the wider and zooms in to the narrower.
        let (fov2, fov3) = (attachment.float_from_bytes(WeaponAttachment::fAdsZoomFov2), attachment.float_from_bytes(WeaponAttachment::fAdsZoomFov3));
        if sight && fov <= 1.0 && fov2 > 1.0 && fov3 > 1.0 {
            let (wide, narrow) = (fov2.max(fov3), fov2.min(fov3));
            e.facts.aim_magnified_view = wide;
            e.facts.variable_zoom_fovs = [wide, narrow];
        }
        let zin = attachment.float_from_bytes(WeaponAttachment::fAdsZoomInFrac);
        if sight && zin > 0.0 {
            e.facts.aim_in_share = zin;
        }
        let zout = attachment.float_from_bytes(WeaponAttachment::fAdsZoomOutFrac);
        if sight && zout > 0.0 {
            e.facts.aim_out_share = zout;
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
            && let Some(ms) = e.clip_names[weap_anim_slot::ATTACHMENT_LIFT_CLIP].as_deref().and_then(anim_ms)
            && ms > 0
        {
            e.facts.alternate_raise_time_ms = ms;
        }
        self.captured.push(e);
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
        let ns = self.into_ns;
        let find = |entries: &[RosterWeapon], name: &str| {
            entries.iter().rev().find(|e| e.name == name && e.namespace == ns).cloned()
        };
        let base_name = format!("{}+{}", w.name, first.0.name);
        let Some(mut e) = find(&self.captured, &base_name) else { return false };
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
            && let Some(mut alt) = find(&self.captured, &alt_name)
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
            && let Some(mut alt) = find(&self.captured, &format!("{}+{}", launcher.name, second.0.name))
        {
            // The second's launcher (`gl`, `mk`) with the first's models.
            lay_t6_attachment(&mut alt, w, second.1, first, None, T6Layer::Launcher);
            alt.name = format!("{}+{atts}", launcher.name);
            alt.alternate_weapon = Some(name.clone());
            e.alternate_weapon = Some(alt.name.clone());
            alt_pair = Some(alt);
        }
        self.captured.push(e);
        self.captured.extend(alt_pair);
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
        let ns = self.into_ns;
        let mut patched = 0;
        for e in self.captured.iter_mut().filter(|e| e.namespace == ns) {
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
                patched += 1;
            }
        }
        patched
    }
}

/// BG_GetWeaponHitLocationMultiplier's pick at each hit location: the first slot's unique
/// whose multiplier differs from the base unique's, else the gun's own (`own`).
fn t6_unique_hit_table(
    own: [f32; weapon_t6::hitloc::HITLOC_COUNT],
    base: &[f32],
    slots: &[&[f32]],
) -> [f32; weapon_t6::hitloc::HITLOC_COUNT] {
    std::array::from_fn(|hit| {
        let differs = |x: &f32| base.get(hit) != Some(x);
        let first = slots.iter().filter_map(|s| s.get(hit)).find(|x| differs(x));
        first.map_or(own[hit], |&x| x)
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
    e: &mut RosterWeapon,
    w: &asset_t6::WeaponRef,
    under: &asset_t6::AttachmentUniqueRef,
    (attachment, unique): (&asset_t6::AttachmentRef, &asset_t6::AttachmentUniqueRef),
    dual_wield_left: Option<&asset_t6::WeaponRef>,
    layer: T6Layer,
) {
    use fastfile_t6::layout::WeaponAttachment;
    use fastfile_t6::layout::WeaponAttachmentUnique;
    let point = attachment.int_from_bytes(WeaponAttachment::attachmentPoint);
    let replace = unique.bytes.get(WeaponAttachmentUnique::disableBaseWeaponAttachment).is_some_and(|&b| b != 0);
    let moved = unique.bytes.get(WeaponAttachmentUnique::overrideBaseWeaponAttachmentOffsets).is_some_and(|&b| b != 0);
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
                        at.offset = unique.vec3_at(WeaponAttachmentUnique::viewModelOffsetBaseAttachment);
                    }
                }
            }
        }
        for (model, offset, rotation) in [
            (&unique.view_model, WeaponAttachmentUnique::viewModelOffsets, WeaponAttachmentUnique::viewModelRotations),
            (&unique.view_model_additional, WeaponAttachmentUnique::viewModelAddOffsets, WeaponAttachmentUnique::viewModelAddRotations),
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
    for tag in &unique.concealed_bones {
        if !e.concealed_bones.contains(tag) {
            e.concealed_bones.push(tag.clone());
        }
    }
    // A launcher shows the attachment and no more.
    if layer == T6Layer::Launcher {
        return;
    }
    for (sound, name) in [
        &mut e.sounds.fire,
        &mut e.sounds.trigger_sound_name,
        &mut e.sounds.fire_loop,
        &mut e.sounds.fire_loop_player,
        &mut e.sounds.fire_stop,
        &mut e.sounds.fire_stop_player,
        &mut e.sounds.final_round_fired,
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
        e.combat_fx.own_muzzle_name = Some(name);
    }
    if let Some(name) = opt(&unique.world_flash) {
        e.combat_fx.far_muzzle_name = Some(name);
    }
    apply_t6_attachment_stats(&mut e.facts, w, attachment);
    // The other mode keeps its own animations.
    if layer == T6Layer::OtherMode {
        return;
    }
    let taken = remap_t6_sz_xanims(&under.animation_refs);
    for (slot, name) in remap_t6_sz_xanims(&unique.animation_refs).into_iter().enumerate() {
        if name.is_some() && taken[slot].is_none() {
            e.clip_names[slot] = name;
        }
    }
    if let Some(l) = dual_wield_left
        && e.dual_wield_weapon.is_none()
    {
        e.right_clip_names = e.clip_names.clone();
        e.left_clip_names = remap_t6_left_hand_xanims(&l.animation_refs);
        e.dual_wield_weapon = Some(l.name.clone());
        e.facts.forbid_twin_grip = false;
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
fn push_t6_world_attachments(e: &mut RosterWeapon, unique: &asset_t6::AttachmentUniqueRef) {
    if let Some(model) = opt(&unique.third_person_mesh)
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
        let mut blob = vec![0u8; fastfile_t6::layout::WeaponAttachment::SIZE];
        for &(off, x) in fields {
            blob[off..off + 4].copy_from_slice(&x.to_le_bytes());
        }
        for &(off, x) in ints {
            blob[off..off + 4].copy_from_slice(&x.to_le_bytes());
        }
        asset_t6::AttachmentRef { name: "test".into(), bytes: blob }
    }

    fn gun_facts() -> WeaponChassisSummary {
        let mut f = WeaponChassisSummary {
            magazine_capacity: 30,
            shot_gap_ms: 100,
            reload_duration_millis: 2000,
            refill_dry_millis: 2500,
            scope_raise_speed: 1.0 / 250.0,
            scope_lower_speed: 0.005,
            aimed_walk_factor: 0.5,
            hip_cone_up_lo: 2.0,
            hip_cone_up_hi: 8.0,
            full_damage_reach: 1000.0,
            minimum_damage_reach: 2000.0,
            wall_pierce_class: 2,
            ..WeaponChassisSummary::default()
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
        use fastfile_t6::layout::WeaponAttachment;
        let mut f = gun_facts();
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &attachment(&[(WeaponAttachment::adsMoveSpeedScale, 1.0)], &[]));
        assert_eq!(f.aimed_walk_factor, 0.5);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &attachment(&[(WeaponAttachment::adsMoveSpeedScale, 2.3)], &[]));
        assert_eq!(f.aimed_walk_factor, 2.3);
    }

    #[test]
    fn t6_attachment_scales_the_guns_stats() {
        use fastfile_t6::layout::WeaponAttachment;
        let mut f = gun_facts();
        let att = attachment(
            &[
                (WeaponAttachment::clipSizeScale, 1.5),
                (WeaponAttachment::fFireTimeScale, 0.8),
                (WeaponAttachment::fReloadTimeScale, 0.5),
                (WeaponAttachment::fAdsTransInTimeScale, 0.5),
                (WeaponAttachment::adsMoveSpeedScale, 2.0),
                (WeaponAttachment::fHipSpreadMinScale, 0.5),
                (WeaponAttachment::fHipSpreadMaxScale, 0.25),
                (WeaponAttachment::fDamageRangeScale, 1.5),
                (WeaponAttachment::fAdsIdleAmountScale, 0.5),
            ],
            &[(WeaponAttachment::penetrateType, 3)],
        );
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &att);
        assert_eq!(f.magazine_capacity, 45);
        assert_eq!(f.shot_gap_ms, 80);
        assert_eq!(f.reload_duration_millis, 1000);
        assert_eq!(f.refill_dry_millis, 2500);
        assert!((f.scope_raise_speed - 1.0 / 125.0).abs() < 1e-9);
        assert!((f.scope_lower_speed - 1.0 / 200.0).abs() < 1e-9);
        assert_eq!(f.aimed_walk_factor, 2.0);
        assert_eq!((f.hip_cone_up_lo, f.hip_cone_up_hi), (1.0, 2.0));
        assert_eq!((f.full_damage_reach, f.minimum_damage_reach), (1500.0, 3000.0));
        assert_eq!(f.idle.ads_idle_amount, 5.0);
        assert_eq!(f.wall_pierce_class, 3);
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
        f.wall_pierce_scale = 1.0;
        // FMJ's perk mask in the zones: armor piercing (1) and bullet penetration (6).
        let mut fmj = attachment(&[], &[(fastfile_t6::layout::WeaponAttachment::perks, 0x42)]);
        fmj.name = "fmj".into();
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.wall_pierce_scale, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        // Once, however many grant it (BG_WeaponHasPerk is a yes or no).
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.wall_pierce_scale, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        let mut other = gun_facts();
        other.wall_pierce_scale = 1.0;
        apply_t6_attachment_stats(&mut other, &asset_t6::WeaponRef::default(), &attachment(&[], &[]));
        assert_eq!(other.wall_pierce_scale, 1.0);
        // A gun whose own variant grants it (fiveseven_upgraded_zm) goes deeper with no attachment.
        let mut variant = vec![0u8; fastfile_t6::layout::WeaponVariantDef::perks + 8];
        variant[fastfile_t6::layout::WeaponVariantDef::perks] = 0x42;
        let def = vec![0u8; fastfile_t6::layout::WeaponDef::SIZE];
        let pap = asset_t6::WeaponRef { variant, def: def.clone(), ..asset_t6::WeaponRef::default() };
        assert_eq!(t6_body_facts(&pap).wall_pierce_scale, weapon_t6::penetration::PERK_BULLET_PENETRATION_MULTIPLIER_DEFAULT);
        let plain = asset_t6::WeaponRef { def, ..asset_t6::WeaponRef::default() };
        assert_eq!(t6_body_facts(&plain).wall_pierce_scale, 1.0);
    }

    #[test]
    fn t6_attachment_clip_size_replaces_and_the_first_penetration_type_wins() {
        use fastfile_t6::layout::WeaponAttachment;
        let mut f = gun_facts();
        let att = attachment(
            &[(WeaponAttachment::clipSizeScale, 1.5)],
            &[(WeaponAttachment::attachmentType, 24), (WeaponAttachment::iClipSize, 40), (WeaponAttachment::penetrateType, 1)],
        );
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &att);
        assert_eq!(f.magazine_capacity, 40);
        assert_eq!(f.wall_pierce_class, 1);
        // A second attachment with a type of its own changes nothing.
        let fmj = attachment(&[], &[(WeaponAttachment::penetrateType, 3)]);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &fmj);
        assert_eq!(f.wall_pierce_class, 1);
    }

    #[test]
    fn t6_only_stack_fire_replaces_the_clip() {
        // The grenade launcher carries its own clip number; on the rifle only its scale counts.
        use fastfile_t6::layout::WeaponAttachment;
        let mut f = gun_facts();
        let base = f.magazine_capacity;
        let gl = attachment(&[(WeaponAttachment::clipSizeScale, 1.0)], &[(WeaponAttachment::attachmentType, 11), (WeaponAttachment::iClipSize, 10)]);
        apply_t6_attachment_stats(&mut f, &asset_t6::WeaponRef::default(), &gl);
        assert_eq!(f.magazine_capacity, base);
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
        assert_eq!(out[weap_anim_slot::IDLE].as_deref(), Some("idle"));
        assert_eq!(out[weap_anim_slot::TRIGGER_SLOT].as_deref(), Some("fire"));
        assert_eq!(out[weap_anim_slot::RELOAD].as_deref(), Some("reload"));
        assert_eq!(out[weap_anim_slot::SCOPE_RAISE_CLIP].as_deref(), Some("ads_up"));
        assert_eq!(out[weap_anim_slot::SCOPE_LOWER_CLIP].as_deref(), Some("ads_down"));
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

    fn captured_gun() -> asset_t6::WeaponRef {
        let mut def = vec![0u8; 2448];
        def[WeaponDef::bDualWield] = 1;
        asset_t6::WeaponRef {
            name: "dw_mp".into(),
            def,
            shown_label: "WEAPON_DW".into(),
            alt_weapon_name: "dw_alt_mp".into(),
            animation_refs: (0..0x60).map(|i| format!("Anim_{i}")).collect(),
            concealed_bones: vec!["tag_a".into(), "tag_b".into()],
            attach_view_models: vec![(0, "mag".into(), "tag_mag".into(), [1.0, 2.0, 3.0], [4.0, 5.0, 6.0])],
            gun_models: vec!["viewgun".into(), "second".into()],
            world_models: vec!["worldgun".into()],
            materials: vec![(WeaponDef::hudIcon, "hud_icon".into()), (WeaponDef::reticleCenter, "ret_c".into())],
            fx: vec![(WeaponDef::projTrailEffect, "trail".into())],
            overlay_material_low: "scope_low".into(),
            notetrack_sounds: vec![("a".into(), "b".into())],
            bounce_sounds: vec!["bn".into(), String::new()],
            ..asset_t6::WeaponRef::default()
        }
    }

    #[test]
    fn capture_t6_is_pinned() {
        let right = captured_gun();
        let left = asset_t6::WeaponRef {
            name: "dw_left_mp".into(),
            gun_models: vec!["leftgun".into()],
            animation_refs: right.animation_refs.iter().rev().cloned().collect(),
            ..asset_t6::WeaponRef::default()
        };
        let camo = std::collections::HashMap::from([("dw_left_mp".to_string(), "camo_left".to_string())]);
        let mut catalog = ArmoryRoster::default();
        let none = std::collections::HashMap::new();
        assert!(!catalog.capture_t6(&asset_t6::WeaponRef::default(), None, &none));
        assert!(!catalog.capture_t6(&asset_t6::WeaponRef { name: ",stub".into(), ..right.clone() }, None, &none));
        assert!(catalog.capture_t6(&right, Some(&left), &camo));
        let solo = asset_t6::WeaponRef { def: Vec::new(), name: "solo_mp".into(), ..right.clone() };
        assert!(catalog.capture_t6(&solo, Some(&left), &none));
        assert!(catalog.capture_t6(&right, None, &std::collections::HashMap::from([("dw_mp".to_string(), "camo_right".to_string())])));
        assert_eq!(catalog.captured.len(), 3);
        let shown = catalog.captured.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join("
");
        let digest = shown.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
        assert_eq!(digest, PINNED_CAPTURE);
    }

    const PINNED_CAPTURE: u64 = 0x224144fb0406a649;
}
