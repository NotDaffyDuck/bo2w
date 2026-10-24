//! Window 12: Recoil, spread and sway.
//!
//! This file holds the per-weapon numbers recoil, spread, sway, bob and idle read
//! ([`SpreadFacts`]) and the two hooks PM_Weapon calls: the sway idle clock every tick and the
//! hip spread bump after every shot. The math itself lives in [`crate::recoil`].
//! Spec: docs/gun/RECOIL.md.

use super::Pm;
use fastfile_t6::layout::{WeaponAttachment as A, WeaponDef as D, WeaponVariantDef as V};

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

fn rd_bool(b: &[u8], at: usize) -> bool {
    b.get(at).copied().unwrap_or(0) != 0
}

fn rd_vec2(b: &[u8], at: usize) -> [f32; 2] {
    [rd_f32(b, at), rd_f32(b, at + 4)]
}

/// One kick range: pitch and yaw, each drawn uniformly between min and max.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KickRange {
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub yaw_min: f32,
    pub yaw_max: f32,
}

impl KickRange {
    fn read(b: &[u8], pitch_min: usize, pitch_max: usize, yaw_min: usize, yaw_max: usize) -> Self {
        Self {
            pitch_min: rd_f32(b, pitch_min),
            pitch_max: rd_f32(b, pitch_max),
            yaw_min: rd_f32(b, yaw_min),
            yaw_max: rd_f32(b, yaw_max),
        }
    }

    /// Two ranges added (both dual-wield guns firing together).
    pub fn plus(self, o: Self) -> Self {
        Self {
            pitch_min: self.pitch_min + o.pitch_min,
            pitch_max: self.pitch_max + o.pitch_max,
            yaw_min: self.yaw_min + o.yaw_min,
            yaw_max: self.yaw_max + o.yaw_max,
        }
    }

    pub fn scaled(self, s: f32) -> Self {
        Self {
            pitch_min: self.pitch_min * s,
            pitch_max: self.pitch_max * s,
            yaw_min: self.yaw_min * s,
            yaw_max: self.yaw_max * s,
        }
    }
}

/// The gun's recoil spring (hip and ADS values, lerped by the ADS fraction).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunSpring {
    pub accel: f32,
    pub speed_max: f32,
    pub speed_decay: f32,
    pub static_decay: f32,
}

impl GunSpring {
    pub fn lerp(self, ads: Self, f: f32) -> Self {
        let l = |a: f32, b: f32| (b - a) * f + a;
        Self {
            accel: l(self.accel, ads.accel),
            speed_max: l(self.speed_max, ads.speed_max),
            speed_decay: l(self.speed_decay, ads.speed_decay),
            static_decay: l(self.static_decay, ads.static_decay),
        }
    }
}

/// One set of sway settings (hip or ADS).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwaySettings {
    pub max_angle: f32,
    pub lerp_speed: f32,
    pub pitch_scale: f32,
    pub yaw_scale: f32,
    pub horiz_scale: f32,
    pub vert_scale: f32,
}

impl SwaySettings {
    pub fn lerp(self, ads: Self, f: f32) -> Self {
        let l = |a: f32, b: f32| (b - a) * f + a;
        Self {
            max_angle: l(self.max_angle, ads.max_angle),
            lerp_speed: l(self.lerp_speed, ads.lerp_speed),
            pitch_scale: l(self.pitch_scale, ads.pitch_scale),
            yaw_scale: l(self.yaw_scale, ads.yaw_scale),
            horiz_scale: l(self.horiz_scale, ads.horiz_scale),
            vert_scale: l(self.vert_scale, ads.vert_scale),
        }
    }
}

/// Window 12's per-weapon facts. `read` stores the weapon file's own values (no attachments);
/// a loader that knows the variant's attachments calls [`SpreadFacts::apply_attachments`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpreadFacts {
    // Hip spread (degrees) by stance, and how the 0..255 scale moves.
    pub stand_min: f32,
    pub ducked_min: f32,
    pub prone_min: f32,
    pub stand_max: f32,
    pub ducked_max: f32,
    pub prone_max: f32,
    pub decay_rate: f32,
    pub fire_add: f32,
    pub turn_add: f32,
    pub move_add: f32,
    pub ducked_decay: f32,
    pub prone_decay: f32,
    /// Attachment products on the hip min / max (1 without attachments).
    pub hip_min_scale: f32,
    pub hip_max_scale: f32,
    pub ads_spread: f32,
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
    pub gun_max_pitch: f32,
    pub gun_max_yaw: f32,

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
    pub aim_down_sight: bool,
    /// The weapon has a scope overlay (`overlayReticle != NONE`).
    pub overlay: bool,
    pub laser_sight: bool,
    pub rangefinder: bool,
    /// Variant perk bits (plus the attachments' after `apply_attachments`).
    pub perks: u64,

    /// BO2's own gun pose: stance, walk, sprint, dive, slide and climb offsets.
    pub pose: crate::pose::PoseFacts,
}

impl SpreadFacts {
    pub fn read(variant: &[u8], def: &[u8]) -> Self {
        let f = |at| rd_f32(def, at);
        Self {
            stand_min: f(D::fHipSpreadStandMin),
            ducked_min: f(D::fHipSpreadDuckedMin),
            prone_min: f(D::fHipSpreadProneMin),
            stand_max: f(D::hipSpreadStandMax),
            ducked_max: f(D::hipSpreadDuckedMax),
            prone_max: f(D::hipSpreadProneMax),
            decay_rate: f(D::fHipSpreadDecayRate),
            fire_add: f(D::fHipSpreadFireAdd),
            turn_add: f(D::fHipSpreadTurnAdd),
            move_add: f(D::fHipSpreadMoveAdd),
            ducked_decay: f(D::fHipSpreadDuckedDecay),
            prone_decay: f(D::fHipSpreadProneDecay),
            hip_min_scale: 1.0,
            hip_max_scale: 1.0,
            ads_spread: f(D::fAdsSpread),
            reticle_side_pos: f(D::fHipReticleSidePos),
            reticle_min_ofs: rd_i32(def, D::iReticleMinOfs),
            stack_fire_spread: f(D::stackFireSpread),
            stack_fire_accuracy_decay: f(D::stackFireAccuracyDecay),
            ai_spread: f(D::aiSpread),
            player_spread: f(D::playerSpread),

            ads_view_kick: KickRange::read(
                def,
                D::fAdsViewKickPitchMin,
                D::fAdsViewKickPitchMax,
                D::fAdsViewKickYawMin,
                D::fAdsViewKickYawMax,
            ),
            ads_view_kick_min_magnitude: f(D::fAdsViewKickMinMagnitude),
            hip_view_kick: KickRange::read(
                def,
                D::fHipViewKickPitchMin,
                D::fHipViewKickPitchMax,
                D::fHipViewKickYawMin,
                D::fHipViewKickYawMax,
            ),
            hip_view_kick_min_magnitude: f(D::fHipViewKickMinMagnitude),
            ads_gun_kick: KickRange::read(
                def,
                D::fAdsGunKickPitchMin,
                D::fAdsGunKickPitchMax,
                D::fAdsGunKickYawMin,
                D::fAdsGunKickYawMax,
            ),
            hip_gun_kick: KickRange::read(
                def,
                D::fHipGunKickPitchMin,
                D::fHipGunKickPitchMax,
                D::fHipGunKickYawMin,
                D::fHipGunKickYawMax,
            ),
            ads_reduced_kick_percent: f(D::adsGunKickReducedKickPercent),
            hip_reduced_kick_percent: f(D::hipGunKickReducedKickPercent),
            ads_recoil_reduction_rate: f(D::fAdsRecoilReductionRate),
            ads_recoil_reduction_limit: f(D::fAdsRecoilReductionLimit),
            ads_recoil_return_rate: f(D::fAdsRecoilReturnRate),

            ads_center_speed_raw: rd_f32(variant, V::fAdsViewKickCenterSpeed),
            ads_center_speed: rd_f32(variant, V::fAdsViewKickCenterSpeed),
            hip_center_speed: rd_f32(variant, V::fHipViewKickCenterSpeed),
            ads_center_ducked_scale: f(D::fAdsViewKickCenterDuckedScale),
            ads_center_prone_scale: f(D::fAdsViewKickCenterProneScale),

            ads_spring: GunSpring {
                accel: f(D::fAdsGunKickAccel),
                speed_max: f(D::fAdsGunKickSpeedMax),
                speed_decay: f(D::fAdsGunKickSpeedDecay),
                static_decay: f(D::fAdsGunKickStaticDecay),
            },
            hip_spring: GunSpring {
                accel: f(D::fHipGunKickAccel),
                speed_max: f(D::fHipGunKickSpeedMax),
                speed_decay: f(D::fHipGunKickSpeedDecay),
                static_decay: f(D::fHipGunKickStaticDecay),
            },
            gun_max_pitch: f(D::fGunMaxPitch),
            gun_max_yaw: f(D::fGunMaxYaw),

            sway: SwaySettings {
                max_angle: f(D::swayMaxAngle),
                lerp_speed: f(D::swayLerpSpeed),
                pitch_scale: f(D::swayPitchScale),
                yaw_scale: f(D::swayYawScale),
                horiz_scale: f(D::swayHorizScale),
                vert_scale: f(D::swayVertScale),
            },
            ads_sway: SwaySettings {
                max_angle: f(D::adsSwayMaxAngle),
                lerp_speed: f(D::adsSwayLerpSpeed),
                pitch_scale: f(D::adsSwayPitchScale),
                yaw_scale: f(D::adsSwayYawScale),
                horiz_scale: rd_f32(variant, V::fAdsSwayHorizScale),
                vert_scale: rd_f32(variant, V::fAdsSwayVertScale),
            },
            sway_shellshock_scale: f(D::swayShellShockScale),

            hip_idle_amount: f(D::fHipIdleAmount),
            ads_idle_amount: f(D::fAdsIdleAmount),
            hip_idle_speed: f(D::hipIdleSpeed),
            ads_idle_speed: f(D::adsIdleSpeed),
            idle_crouch_factor: f(D::fIdleCrouchFactor),
            idle_prone_factor: f(D::fIdleProneFactor),

            ads_bob_factor: f(D::fAdsBobFactor),
            ads_view_bob_mult: f(D::fAdsViewBobMult),
            sprint_bob: rd_vec2(def, D::vSprintBob),
            dtp_bob: rd_vec2(def, D::vDtpBob),
            sprint_cycle_scale: f(D::fSprintCycleScale),
            ducked_sprint_cycle_scale: f(D::fDuckedSprintCycleScale),
            dtp_cycle_scale: f(D::fDtpCycleScale),

            anti_quick_scope: rd_bool(variant, V::bAntiQuickScope),
            aqs_time: f(D::fAntiQuickScopeTime),
            aqs_scale: f(D::fAntiQuickScopeScale),
            aqs_spread_multiplier: f(D::fAntiQuickScopeSpreadMultiplier),
            aqs_spread_max: f(D::fAntiQuickScopeSpreadMax),
            aqs_sway_factor: f(D::fAntiQuickScopeSwayFactor),

            aim_down_sight: rd_bool(def, D::aimDownSight),
            overlay: rd_i32(def, D::overlayReticle) != 0,
            laser_sight: rd_bool(def, D::laserSight),
            rangefinder: false,
            perks: rd_u64(variant, V::perks),
            pose: crate::pose::PoseFacts::read(def),
        }
    }

    /// Fold in the variant's attachments (up to 3, in order) the way BO2's getters read them.
    /// `default` is `bg_weaponAttachmentDefault`: "overridden" means differing from it.
    pub fn apply_attachments(&mut self, atts: &[&[u8]], default: &[u8]) {
        let atts = &atts[..atts.len().min(3)];
        let differs = |a: &[u8], at: usize| rd_f32(a, at) != rd_f32(default, at);

        // Recoil reduction: every overriding attachment writes, so the last one wins.
        for a in atts {
            if differs(a, A::fAdsRecoilReductionRate) {
                self.ads_recoil_reduction_rate = rd_f32(a, A::fAdsRecoilReductionRate);
            }
            if differs(a, A::fAdsRecoilReductionLimit) {
                self.ads_recoil_reduction_limit = rd_f32(a, A::fAdsRecoilReductionLimit);
            }
        }
        // Hip idle amount: the first overriding attachment.
        if let Some(a) = atts.iter().find(|a| differs(a, A::fHipIdleAmount)) {
            self.hip_idle_amount = rd_f32(a, A::fHipIdleAmount);
        }
        // Stack-fire spread and accuracy decay: each the first attachment with a positive value.
        if let Some(a) = atts.iter().find(|a| rd_f32(a, A::stackFireSpread) > 0.0) {
            self.stack_fire_spread = rd_f32(a, A::stackFireSpread);
        }
        if let Some(a) = atts
            .iter()
            .find(|a| rd_f32(a, A::stackFireAccuracyDecay) > 0.0)
        {
            self.stack_fire_accuracy_decay = rd_f32(a, A::stackFireAccuracyDecay);
        }
        // Sway: the first attachment with an override flag.
        if let Some(a) = atts.iter().find(|a| rd_bool(a, A::swayOverride)) {
            self.sway = SwaySettings {
                max_angle: rd_f32(a, A::swayMaxAngle),
                lerp_speed: rd_f32(a, A::swayLerpSpeed),
                pitch_scale: rd_f32(a, A::swayPitchScale),
                yaw_scale: rd_f32(a, A::swayYawScale),
                horiz_scale: rd_f32(a, A::swayHorizScale),
                vert_scale: rd_f32(a, A::swayVertScale),
            };
        }
        if let Some(a) = atts.iter().find(|a| rd_bool(a, A::adsSwayOverride)) {
            self.ads_sway = SwaySettings {
                max_angle: rd_f32(a, A::adsSwayMaxAngle),
                lerp_speed: rd_f32(a, A::adsSwayLerpSpeed),
                pitch_scale: rd_f32(a, A::adsSwayPitchScale),
                yaw_scale: rd_f32(a, A::adsSwayYawScale),
                horiz_scale: rd_f32(a, A::fAdsSwayHorizScale),
                vert_scale: rd_f32(a, A::fAdsSwayVertScale),
            };
        }
        // Walk pose (BG_GetStandMoveAndRotation, BG_GetStrafeMoveAndRotation): every
        // overriding attachment writes, so the last one wins.
        for a in atts {
            if differs(a, A::standMoveF) {
                self.pose.stand_move[0] = rd_f32(a, A::standMoveF);
            }
            if differs(a, A::vStandRot) {
                self.pose.stand_rot[0] = rd_f32(a, A::vStandRot);
            }
            if differs(a, A::vStandRot + 8) {
                self.pose.stand_rot[2] = rd_f32(a, A::vStandRot + 8);
            }
            if differs(a, A::strafeRotR) {
                self.pose.strafe_rot[2] = rd_f32(a, A::strafeRotR);
            }
        }
        // Products: every attachment scales (BO2 asserts each scale is positive).
        for a in atts {
            self.ads_center_speed *= rd_f32(a, A::fAdsViewKickCenterSpeedScale);
            self.ads_idle_amount *= rd_f32(a, A::fAdsIdleAmountScale);
            self.hip_min_scale *= rd_f32(a, A::fHipSpreadMinScale);
            self.hip_max_scale *= rd_f32(a, A::fHipSpreadMaxScale);
            self.perks |= rd_u64(a, A::perks);
            self.laser_sight |= rd_bool(a, A::laserSight);
            self.rangefinder |= a.get(A::attachmentType).copied() == Some(ATTACHMENT_RANGEFINDER);
        }
    }

    pub fn has_perk(&self, perk: u32) -> bool {
        perk < 64 && self.perks & (1u64 << perk) != 0
    }
}

/// PM_UpdateSwayPrediction: while aimed at all, the idle-drift clock runs. Scoped weapons run
/// it twice (the camera's idle and the gun's both step `weaponIdleTime`).
pub fn update_sway_prediction(pm: &mut Pm) {
    if pm.ps.f_weapon_pos_frac == 0.0 || pm.ps.weapon == 0 {
        return;
    }
    let s = pm.weapon_facts().spread;
    let dt = pm.cmd.msec as f32 * 0.001;
    let ps = &mut *pm.ps;
    ps.weapon_idle_time = crate::recoil::step_idle_time(
        ps.weapon_idle_time,
        &s,
        ps.f_weapon_pos_frac,
        ps.hold_breath_scale,
        dt,
    );
}

/// PM_Weapon_AddFiringAimSpreadScale: every shot from the hip widens the spread by the
/// weapon's fire add (fully aimed shots add nothing).
pub fn add_firing_aim_spread(pm: &mut Pm) {
    let s = pm.weapon_facts().spread;
    pm.ps.aim_spread_scale =
        crate::recoil::add_firing_aim_spread(pm.ps.aim_spread_scale, &s, pm.ps.f_weapon_pos_frac);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turret_guns_spread_comes_from_its_weapon_file() {
        // aiSpread and playerSpread: the turret's (Fire_Lead) and a seated
        // player's (VEH_FireGunnerWeapon) cone, in degrees.
        let mut def = vec![0u8; 4096];
        def[D::aiSpread..D::aiSpread + 4].copy_from_slice(&3.5f32.to_le_bytes());
        def[D::playerSpread..D::playerSpread + 4].copy_from_slice(&1.25f32.to_le_bytes());
        let s = SpreadFacts::read(&[0u8; 1024], &def);
        assert_eq!(s.ai_spread, 3.5);
        assert_eq!(s.player_spread, 1.25);
    }

    #[test]
    fn attachments_override_the_walk_pose_and_the_last_one_wins() {
        let put = |b: &mut [u8], at: usize, v: f32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let mut def = vec![0u8; 4096];
        for (k, v) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            put(&mut def, D::vStandMove + 4 * k, v);
            put(&mut def, D::vStandRot + 4 * k, v + 3.0);
            put(&mut def, D::vStrafeRot + 4 * k, v + 6.0);
        }
        let mut s = SpreadFacts::read(&[0u8; 1024], &def);
        assert_eq!(s.pose.stand_move, [1.0, 2.0, 3.0]);
        let default = vec![0u8; 1024];
        let mut first = vec![0u8; 1024];
        put(&mut first, A::standMoveF, 10.0);
        let mut second = vec![0u8; 1024];
        put(&mut second, A::standMoveF, 20.0);
        put(&mut second, A::vStandRot + 8, 30.0);
        put(&mut second, A::strafeRotR, 40.0);
        s.apply_attachments(&[&first, &second], &default);
        assert_eq!(s.pose.stand_move, [20.0, 2.0, 3.0]);
        assert_eq!(s.pose.stand_rot, [4.0, 5.0, 30.0]);
        assert_eq!(s.pose.strafe_rot, [7.0, 8.0, 40.0]);
    }
}
