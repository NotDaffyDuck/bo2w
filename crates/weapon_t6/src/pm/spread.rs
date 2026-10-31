//! Window 12: Recoil, spread and sway.
//!
//! This file holds the per-weapon numbers recoil, spread, sway, bob and idle read
//! ([`SpreadFacts`]) and the two hooks PM_Weapon calls: the sway idle clock every tick and the
//! hip spread bump after every shot. The math itself lives in [`crate::recoil`].
//! Spec: docs/gun/RECOIL.md.

use super::Pm;
use fastfile_t6::layout::{WeaponAttachment, WeaponDef, WeaponVariantDef};

/// BO2 `eAttachment`: the rangefinder (landing dip shrinks when aimed with it).
pub const ATTACHMENT_RANGEFINDER: u8 = 19;

pub(crate) fn rd_f32(b: &[u8], at: usize) -> f32 {
    b.get(at..at + 4)
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0.0)
}

pub(crate) fn rd_i32(b: &[u8], at: usize) -> i32 {
    b.get(at..at + 4)
        .map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

fn rd_u64(b: &[u8], at: usize) -> u64 {
    let lo = rd_i32(b, at) as u32 as u64;
    let hi = rd_i32(b, at + 4) as u32 as u64;
    lo | (hi << 32)
}

fn rd_bool(b: &[u8], off: usize) -> bool {
    b.get(off).copied().unwrap_or(0) != 0
}

fn rd_vec2(b: &[u8], at: usize) -> [f32; 2] {
    [rd_f32(b, at), rd_f32(b, at + 4)]
}

/// One kick range: pitch and yaw, each drawn uniformly between min and max.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KickRange {
    pub pitch_lower_bound: f32,
    pub pitch_upper_bound: f32,
    pub turn_floor: f32,
    pub turn_ceiling: f32,
}

impl KickRange {
    fn read(b: &[u8], pitch_lower_bound: usize, pitch_upper_bound: usize, turn_floor: usize, turn_ceiling: usize) -> Self {
        Self {
            pitch_lower_bound: rd_f32(b, pitch_lower_bound),
            pitch_upper_bound: rd_f32(b, pitch_upper_bound),
            turn_floor: rd_f32(b, turn_floor),
            turn_ceiling: rd_f32(b, turn_ceiling),
        }
    }

    /// Two ranges added (both dual-wield guns firing together).
    pub fn plus(self, o: Self) -> Self {
        Self {
            pitch_lower_bound: self.pitch_lower_bound + o.pitch_lower_bound,
            pitch_upper_bound: self.pitch_upper_bound + o.pitch_upper_bound,
            turn_floor: self.turn_floor + o.turn_floor,
            turn_ceiling: self.turn_ceiling + o.turn_ceiling,
        }
    }

    pub fn scaled(self, s: f32) -> Self {
        Self {
            pitch_lower_bound: self.pitch_lower_bound * s,
            pitch_upper_bound: self.pitch_upper_bound * s,
            turn_floor: self.turn_floor * s,
            turn_ceiling: self.turn_ceiling * s,
        }
    }
}

/// The gun's recoil spring (hip and ADS values, lerped by the ADS fraction).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunSpring {
    pub speed_up_rate: f32,
    pub velocity_cap: f32,
    pub velocity_falloff: f32,
    pub resting_falloff: f32,
}

impl GunSpring {
    pub fn lerp(self, ads: Self, f: f32) -> Self {
        let l = |a: f32, b: f32| (b - a) * f + a;
        Self {
            speed_up_rate: l(self.speed_up_rate, ads.speed_up_rate),
            velocity_cap: l(self.velocity_cap, ads.velocity_cap),
            velocity_falloff: l(self.velocity_falloff, ads.velocity_falloff),
            resting_falloff: l(self.resting_falloff, ads.resting_falloff),
        }
    }
}

/// One set of sway settings (hip or ADS).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwaySettings {
    pub widest_cone: f32,
    pub ease_rate: f32,
    pub tilt_multiplier: f32,
    pub turn_factor: f32,
    pub sideways_factor: f32,
    pub upward_factor: f32,
}

impl SwaySettings {
    pub fn lerp(self, ads: Self, f: f32) -> Self {
        let l = |a: f32, b: f32| (b - a) * f + a;
        Self {
            widest_cone: l(self.widest_cone, ads.widest_cone),
            ease_rate: l(self.ease_rate, ads.ease_rate),
            tilt_multiplier: l(self.tilt_multiplier, ads.tilt_multiplier),
            turn_factor: l(self.turn_factor, ads.turn_factor),
            sideways_factor: l(self.sideways_factor, ads.sideways_factor),
            upward_factor: l(self.upward_factor, ads.upward_factor),
        }
    }
}

/// Window 12's per-weapon facts. `read` stores the weapon file's own values (no attachments);
/// a loader that knows the variant's attachments calls [`SpreadFacts::fold_in_addons`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpreadFacts {
    // Hip spread (degrees) by stance, and how the 0..255 scale moves.
    pub upright_narrowest: f32,
    pub crouch_narrowest: f32,
    pub lying_narrowest: f32,
    pub upright_widest: f32,
    pub crouch_widest: f32,
    pub lying_widest: f32,
    pub settle_rate: f32,
    pub per_shot_bloom: f32,
    pub turn_gain: f32,
    pub motion_bloom: f32,
    pub crouch_settle: f32,
    pub lying_settle: f32,
    /// Attachment products on the hip min / max (1 without attachments).
    pub hip_min_scale: f32,
    pub hip_max_scale: f32,
    pub aimed_cone: f32,
    pub reticle_side_pos: f32,
    pub reticle_min_ofs: i32,
    pub stack_fire_spread: f32,
    pub stack_fire_accuracy_decay: f32,
    /// Turret/vehicle gun spread (degrees) when no client shoots (`aiSpread`), and when a
    /// seated client does (`playerSpread`); `Fire_Lead` / `VEH_FireGunnerWeapon` pick one.
    pub ai_spread: f32,
    pub player_spread: f32,

    // Kick per shot.
    pub ads_view_kick: KickRange,
    pub ads_view_kick_min_magnitude: f32,
    pub hip_view_kick: KickRange,
    pub hip_view_kick_min_magnitude: f32,
    pub ads_gun_kick: KickRange,
    pub hip_gun_kick: KickRange,
    pub ads_reduced_kick_percent: f32,
    pub hip_reduced_kick_percent: f32,
    pub ads_recoil_reduction_rate: f32,
    pub ads_recoil_reduction_limit: f32,
    pub ads_recoil_return_rate: f32,

    // View kick settle.
    /// Variant `fAdsViewKickCenterSpeed` (the raw value; aimed-in settle uses the scaled one).
    pub ads_center_speed_raw: f32,
    /// `BG_GetADSViewKickCenterSpeed`: the raw value times every attachment's scale.
    pub ads_center_speed: f32,
    pub hip_center_speed: f32,
    pub ads_center_ducked_scale: f32,
    pub ads_center_prone_scale: f32,

    // Gun spring.
    pub ads_spring: GunSpring,
    pub hip_spring: GunSpring,
    pub weapon_pitch_ceiling: f32,
    pub weapon_yaw_ceiling: f32,

    // Sway.
    pub sway: SwaySettings,
    pub ads_sway: SwaySettings,
    pub sway_shellshock_scale: f32,

    // Idle drift.
    pub hip_idle_amount: f32,
    /// `BG_GetADSIdleAmount`: the def value times every attachment's scale.
    pub ads_idle_amount: f32,
    pub hip_idle_speed: f32,
    pub ads_idle_speed: f32,
    pub idle_crouch_factor: f32,
    pub idle_prone_factor: f32,

    // Bob.
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub sprint_bob: [f32; 2],
    pub dtp_bob: [f32; 2],
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,

    // Anti-quickscope.
    pub anti_quick_scope: bool,
    pub aqs_time: f32,
    pub aqs_scale: f32,
    pub aqs_spread_multiplier: f32,
    pub aqs_spread_max: f32,
    pub aqs_sway_factor: f32,

    // What the weapon is.
    pub scoped_aiming: bool,
    /// The weapon has a scope overlay (`overlayReticle != NONE`).
    pub overlay: bool,
    pub laser_sight: bool,
    pub rangefinder: bool,
    /// Variant perk bits (plus the attachments' after `fold_in_addons`).
    pub perks: u64,

    /// BO2's own gun pose: stance, walk, sprint, dive, slide and climb offsets.
    pub pose: crate::pose::PoseFacts,
}

impl SpreadFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let f = |at| rd_f32(def, at);
        Self {
            upright_narrowest: f(WeaponDef::fHipSpreadStandMin),
            crouch_narrowest: f(WeaponDef::fHipSpreadDuckedMin),
            lying_narrowest: f(WeaponDef::fHipSpreadProneMin),
            upright_widest: f(WeaponDef::hipSpreadStandMax),
            crouch_widest: f(WeaponDef::hipSpreadDuckedMax),
            lying_widest: f(WeaponDef::hipSpreadProneMax),
            settle_rate: f(WeaponDef::fHipSpreadDecayRate),
            per_shot_bloom: f(WeaponDef::fHipSpreadFireAdd),
            turn_gain: f(WeaponDef::fHipSpreadTurnAdd),
            motion_bloom: f(WeaponDef::fHipSpreadMoveAdd),
            crouch_settle: f(WeaponDef::fHipSpreadDuckedDecay),
            lying_settle: f(WeaponDef::fHipSpreadProneDecay),
            hip_min_scale: 1.0,
            hip_max_scale: 1.0,
            aimed_cone: f(WeaponDef::fAdsSpread),
            reticle_side_pos: f(WeaponDef::fHipReticleSidePos),
            reticle_min_ofs: rd_i32(def, WeaponDef::iReticleMinOfs),
            stack_fire_spread: f(WeaponDef::stackFireSpread),
            stack_fire_accuracy_decay: f(WeaponDef::stackFireAccuracyDecay),
            ai_spread: f(WeaponDef::aiSpread),
            player_spread: f(WeaponDef::playerSpread),

            ads_view_kick: KickRange::read(
                def,
                WeaponDef::fAdsViewKickPitchMin,
                WeaponDef::fAdsViewKickPitchMax,
                WeaponDef::fAdsViewKickYawMin,
                WeaponDef::fAdsViewKickYawMax,
            ),
            ads_view_kick_min_magnitude: f(WeaponDef::fAdsViewKickMinMagnitude),
            hip_view_kick: KickRange::read(
                def,
                WeaponDef::fHipViewKickPitchMin,
                WeaponDef::fHipViewKickPitchMax,
                WeaponDef::fHipViewKickYawMin,
                WeaponDef::fHipViewKickYawMax,
            ),
            hip_view_kick_min_magnitude: f(WeaponDef::fHipViewKickMinMagnitude),
            ads_gun_kick: KickRange::read(
                def,
                WeaponDef::fAdsGunKickPitchMin,
                WeaponDef::fAdsGunKickPitchMax,
                WeaponDef::fAdsGunKickYawMin,
                WeaponDef::fAdsGunKickYawMax,
            ),
            hip_gun_kick: KickRange::read(
                def,
                WeaponDef::fHipGunKickPitchMin,
                WeaponDef::fHipGunKickPitchMax,
                WeaponDef::fHipGunKickYawMin,
                WeaponDef::fHipGunKickYawMax,
            ),
            ads_reduced_kick_percent: f(WeaponDef::adsGunKickReducedKickPercent),
            hip_reduced_kick_percent: f(WeaponDef::hipGunKickReducedKickPercent),
            ads_recoil_reduction_rate: f(WeaponDef::fAdsRecoilReductionRate),
            ads_recoil_reduction_limit: f(WeaponDef::fAdsRecoilReductionLimit),
            ads_recoil_return_rate: f(WeaponDef::fAdsRecoilReturnRate),

            ads_center_speed_raw: rd_f32(variant, WeaponVariantDef::fAdsViewKickCenterSpeed),
            ads_center_speed: rd_f32(variant, WeaponVariantDef::fAdsViewKickCenterSpeed),
            hip_center_speed: rd_f32(variant, WeaponVariantDef::fHipViewKickCenterSpeed),
            ads_center_ducked_scale: f(WeaponDef::fAdsViewKickCenterDuckedScale),
            ads_center_prone_scale: f(WeaponDef::fAdsViewKickCenterProneScale),

            ads_spring: GunSpring {
                speed_up_rate: f(WeaponDef::fAdsGunKickAccel),
                velocity_cap: f(WeaponDef::fAdsGunKickSpeedMax),
                velocity_falloff: f(WeaponDef::fAdsGunKickSpeedDecay),
                resting_falloff: f(WeaponDef::fAdsGunKickStaticDecay),
            },
            hip_spring: GunSpring {
                speed_up_rate: f(WeaponDef::fHipGunKickAccel),
                velocity_cap: f(WeaponDef::fHipGunKickSpeedMax),
                velocity_falloff: f(WeaponDef::fHipGunKickSpeedDecay),
                resting_falloff: f(WeaponDef::fHipGunKickStaticDecay),
            },
            weapon_pitch_ceiling: f(WeaponDef::fGunMaxPitch),
            weapon_yaw_ceiling: f(WeaponDef::fGunMaxYaw),

            sway: SwaySettings {
                widest_cone: f(WeaponDef::swayMaxAngle),
                ease_rate: f(WeaponDef::swayLerpSpeed),
                tilt_multiplier: f(WeaponDef::swayPitchScale),
                turn_factor: f(WeaponDef::swayYawScale),
                sideways_factor: f(WeaponDef::swayHorizScale),
                upward_factor: f(WeaponDef::swayVertScale),
            },
            ads_sway: SwaySettings {
                widest_cone: f(WeaponDef::adsSwayMaxAngle),
                ease_rate: f(WeaponDef::adsSwayLerpSpeed),
                tilt_multiplier: f(WeaponDef::adsSwayPitchScale),
                turn_factor: f(WeaponDef::adsSwayYawScale),
                sideways_factor: rd_f32(variant, WeaponVariantDef::fAdsSwayHorizScale),
                upward_factor: rd_f32(variant, WeaponVariantDef::fAdsSwayVertScale),
            },
            sway_shellshock_scale: f(WeaponDef::swayShellShockScale),

            hip_idle_amount: f(WeaponDef::fHipIdleAmount),
            ads_idle_amount: f(WeaponDef::fAdsIdleAmount),
            hip_idle_speed: f(WeaponDef::hipIdleSpeed),
            ads_idle_speed: f(WeaponDef::adsIdleSpeed),
            idle_crouch_factor: f(WeaponDef::fIdleCrouchFactor),
            idle_prone_factor: f(WeaponDef::fIdleProneFactor),

            ads_bob_factor: f(WeaponDef::fAdsBobFactor),
            ads_view_bob_mult: f(WeaponDef::fAdsViewBobMult),
            sprint_bob: rd_vec2(def, WeaponDef::vSprintBob),
            dtp_bob: rd_vec2(def, WeaponDef::vDtpBob),
            sprint_cycle_scale: f(WeaponDef::fSprintCycleScale),
            ducked_sprint_cycle_scale: f(WeaponDef::fDuckedSprintCycleScale),
            dtp_cycle_scale: f(WeaponDef::fDtpCycleScale),

            anti_quick_scope: rd_bool(variant, WeaponVariantDef::bAntiQuickScope),
            aqs_time: f(WeaponDef::fAntiQuickScopeTime),
            aqs_scale: f(WeaponDef::fAntiQuickScopeScale),
            aqs_spread_multiplier: f(WeaponDef::fAntiQuickScopeSpreadMultiplier),
            aqs_spread_max: f(WeaponDef::fAntiQuickScopeSpreadMax),
            aqs_sway_factor: f(WeaponDef::fAntiQuickScopeSwayFactor),

            scoped_aiming: rd_bool(def, WeaponDef::aimDownSight),
            overlay: rd_i32(def, WeaponDef::overlayReticle) != 0,
            laser_sight: rd_bool(def, WeaponDef::laserSight),
            rangefinder: false,
            perks: rd_u64(variant, WeaponVariantDef::perks),
            pose: crate::pose::PoseFacts::read(def),
        }
    }

    /// Fold in the variant's attachments (up to 3, in order) the way BO2's getters read them.
    /// `default` is `bg_weaponAttachmentDefault`: "overridden" means differing from it.
    pub fn fold_in_addons(&mut self, atts: &[&[u8]], default: &[u8]) {
        let atts = &atts[..atts.len().min(3)];
        let differs = |a: &[u8], at: usize| rd_f32(a, at) != rd_f32(default, at);

        // Recoil reduction: every overriding attachment writes, so the last one wins.
        for a in atts {
            if differs(a, WeaponAttachment::fAdsRecoilReductionRate) {
                self.ads_recoil_reduction_rate = rd_f32(a, WeaponAttachment::fAdsRecoilReductionRate);
            }
            if differs(a, WeaponAttachment::fAdsRecoilReductionLimit) {
                self.ads_recoil_reduction_limit = rd_f32(a, WeaponAttachment::fAdsRecoilReductionLimit);
            }
        }
        // Hip idle amount: the first overriding attachment.
        if let Some(a) = atts.iter().find(|a| differs(a, WeaponAttachment::fHipIdleAmount)) {
            self.hip_idle_amount = rd_f32(a, WeaponAttachment::fHipIdleAmount);
        }
        // Stack-fire spread and accuracy decay: each the first attachment with a positive value.
        if let Some(a) = atts.iter().find(|a| rd_f32(a, WeaponAttachment::stackFireSpread) > 0.0) {
            self.stack_fire_spread = rd_f32(a, WeaponAttachment::stackFireSpread);
        }
        if let Some(a) = atts
            .iter()
            .find(|a| rd_f32(a, WeaponAttachment::stackFireAccuracyDecay) > 0.0)
        {
            self.stack_fire_accuracy_decay = rd_f32(a, WeaponAttachment::stackFireAccuracyDecay);
        }
        // Sway: the first attachment with an override flag.
        if let Some(a) = atts.iter().find(|a| rd_bool(a, WeaponAttachment::swayOverride)) {
            self.sway = SwaySettings {
                widest_cone: rd_f32(a, WeaponAttachment::swayMaxAngle),
                ease_rate: rd_f32(a, WeaponAttachment::swayLerpSpeed),
                tilt_multiplier: rd_f32(a, WeaponAttachment::swayPitchScale),
                turn_factor: rd_f32(a, WeaponAttachment::swayYawScale),
                sideways_factor: rd_f32(a, WeaponAttachment::swayHorizScale),
                upward_factor: rd_f32(a, WeaponAttachment::swayVertScale),
            };
        }
        if let Some(a) = atts.iter().find(|a| rd_bool(a, WeaponAttachment::adsSwayOverride)) {
            self.ads_sway = SwaySettings {
                widest_cone: rd_f32(a, WeaponAttachment::adsSwayMaxAngle),
                ease_rate: rd_f32(a, WeaponAttachment::adsSwayLerpSpeed),
                tilt_multiplier: rd_f32(a, WeaponAttachment::adsSwayPitchScale),
                turn_factor: rd_f32(a, WeaponAttachment::adsSwayYawScale),
                sideways_factor: rd_f32(a, WeaponAttachment::fAdsSwayHorizScale),
                upward_factor: rd_f32(a, WeaponAttachment::fAdsSwayVertScale),
            };
        }
        // Walk pose (BG_GetStandMoveAndRotation, BG_GetStrafeMoveAndRotation): every
        // overriding attachment writes, so the last one wins.
        for a in atts {
            if differs(a, WeaponAttachment::standMoveF) {
                self.pose.stand_move[0] = rd_f32(a, WeaponAttachment::standMoveF);
            }
            if differs(a, WeaponAttachment::vStandRot) {
                self.pose.stand_rot[0] = rd_f32(a, WeaponAttachment::vStandRot);
            }
            if differs(a, WeaponAttachment::vStandRot + 8) {
                self.pose.stand_rot[2] = rd_f32(a, WeaponAttachment::vStandRot + 8);
            }
            if differs(a, WeaponAttachment::strafeRotR) {
                self.pose.strafe_rot[2] = rd_f32(a, WeaponAttachment::strafeRotR);
            }
        }
        // Products: every attachment scales (BO2 asserts each scale is positive).
        for a in atts {
            self.ads_center_speed *= rd_f32(a, WeaponAttachment::fAdsViewKickCenterSpeedScale);
            self.ads_idle_amount *= rd_f32(a, WeaponAttachment::fAdsIdleAmountScale);
            self.hip_min_scale *= rd_f32(a, WeaponAttachment::fHipSpreadMinScale);
            self.hip_max_scale *= rd_f32(a, WeaponAttachment::fHipSpreadMaxScale);
            self.perks |= rd_u64(a, WeaponAttachment::perks);
            self.laser_sight |= rd_bool(a, WeaponAttachment::laserSight);
            self.rangefinder |= a.get(WeaponAttachment::attachmentType).copied() == Some(ATTACHMENT_RANGEFINDER);
        }
    }

    pub fn has_perk(&self, perk: u32) -> bool {
        perk < 64 && self.perks & (1u64 << perk) != 0
    }
}

/// PM_UpdateSwayPrediction: while aimed at all, the idle-drift clock runs. Scoped weapons run
/// it twice (the camera's idle and the gun's both step `weaponIdleTime`).
pub fn update_sway_prediction(pm: &mut Pm) {
    if pm.ps.weapon_raise_fraction == 0.0 || pm.ps.weapon == 0 {
        return;
    }
    let s = pm.weapon_facts().spread;
    let dt = pm.cmd.frame_ms as f32 * 0.001;
    let ps = &mut *pm.ps;
    ps.weapon_idle_time = crate::recoil::step_idle_time(
        ps.weapon_idle_time,
        &s,
        ps.weapon_raise_fraction,
        ps.steady_aim_factor,
        dt,
    );
}

/// PM_Weapon_AddFiringAimSpreadScale: every shot from the hip widens the spread by the
/// weapon's fire add (fully aimed shots add nothing).
pub fn widen_cone_on_shot(pm: &mut Pm) {
    let s = pm.weapon_facts().spread;
    pm.ps.sight_spread_factor =
        crate::recoil::widen_cone_on_shot(pm.ps.sight_spread_factor, &s, pm.ps.weapon_raise_fraction);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turret_guns_spread_comes_from_its_weapon_file() {
        // aiSpread and playerSpread: the turret's (Fire_Lead) and a seated
        // player's (VEH_FireGunnerWeapon) cone, in degrees.
        let mut def = vec![0u8; 4096];
        def[WeaponDef::aiSpread..WeaponDef::aiSpread + 4].copy_from_slice(&3.5f32.to_le_bytes());
        def[WeaponDef::playerSpread..WeaponDef::playerSpread + 4].copy_from_slice(&1.25f32.to_le_bytes());
        let s = SpreadFacts::read(&[0u8; 1024], &def);
        assert_eq!(s.ai_spread, 3.5);
        assert_eq!(s.player_spread, 1.25);
    }

    #[test]
    fn attachments_override_the_walk_pose_and_the_last_one_wins() {
        let put = |b: &mut [u8], at: usize, v: f32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let mut def = vec![0u8; 4096];
        for (k, v) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            put(&mut def, WeaponDef::vStandMove + 4 * k, v);
            put(&mut def, WeaponDef::vStandRot + 4 * k, v + 3.0);
            put(&mut def, WeaponDef::vStrafeRot + 4 * k, v + 6.0);
        }
        let mut s = SpreadFacts::read(&[0u8; 1024], &def);
        assert_eq!(s.pose.stand_move, [1.0, 2.0, 3.0]);
        let default = vec![0u8; 1024];
        let mut first = vec![0u8; 1024];
        put(&mut first, WeaponAttachment::standMoveF, 10.0);
        let mut second = vec![0u8; 1024];
        put(&mut second, WeaponAttachment::standMoveF, 20.0);
        put(&mut second, WeaponAttachment::vStandRot + 8, 30.0);
        put(&mut second, WeaponAttachment::strafeRotR, 40.0);
        s.fold_in_addons(&[&first, &second], &default);
        assert_eq!(s.pose.stand_move, [20.0, 2.0, 3.0]);
        assert_eq!(s.pose.stand_rot, [4.0, 5.0, 30.0]);
        assert_eq!(s.pose.strafe_rot, [7.0, 8.0, 40.0]);
    }
}
