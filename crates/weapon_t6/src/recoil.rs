//! Window 12: BO2's recoil, spread, sway, bob and idle math, rebuilt in our own Rust.
//!
//! Everything here is a plain function over numbers: the weapon's [`SpreadFacts`], a small input
//! struct holding what that step reads from the player, and the state it moves. The callers
//! (PM_Weapon hooks in `pm/spread.rs`, the client view code) fill the inputs.
//! Spec and the BO2 function each piece matches: docs/gun/RECOIL.md.
//!
//! - Spread: [`adjust_aim_spread_scale`] (every move), [`add_firing_aim_spread`] (every shot),
//!   [`spread_for_weapon`], [`bullet_cone`], [`reticle_spread`].
//! - Kick: [`fire_recoil`] (every shot), [`settle_view_kick`] (every client frame),
//!   [`gun_spring`] (every weapon movement update).
//! - Gun movement: [`sway`], [`gun_bob`], [`idle_angles`], [`weapon_movement_angles`],
//!   [`landing_offset`]; the camera's [`camera_bob`].

use crate::pm::spread::{KickRange, SpreadFacts, SwaySettings};
use crate::state;
use std::f32::consts::PI;

/// The widest the hip spread scale goes.
pub const SPREAD_MAX: f32 = 255.0;
/// A command angle unit in degrees (65536 units per turn).
pub const SHORT2ANGLE: f32 = 360.0 / 65536.0;

/// The dvars this file reads, at BO2's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecoilDvars {
    /// `bg_aqs`: anti-quickscope on.
    pub aqs: bool,
    /// `bg_aqsStyle`: 1 scales the scope drift, 2 adds a wobble on top.
    pub aqs_style: i32,
    /// `adsZeroSpread`: fully aimed snaps the spread scale to 0.
    pub ads_zero_spread: bool,
    /// `zero_idle_movement`.
    pub zero_idle_movement: bool,
    /// `bg_aimSpreadMoveSpeedThreshold`.
    pub move_speed_threshold: f32,
    /// `player_standingViewHeight`.
    pub standing_view_height: i32,
    /// `perk_weapSpreadMultiplier` (Steady Aim).
    pub perk_weap_spread_multiplier: f32,
    /// `bg_chargeShotViewKickIncreasePerBullet`, `bg_chargeShotMaxViewKick`.
    pub charge_view_kick_increase: f32,
    pub charge_max_view_kick: f32,
    /// `bg_chargeShotCenterSpeedReductionPerBullet`, `bg_chargeShotMinCenterSpeed`.
    pub charge_center_reduction: f32,
    pub charge_min_center_speed: f32,
    /// `bg_weaponBobAmplitudeBase` (+Laser), `bg_weaponBobAmplitudeRoll` (+Laser), `bg_weaponBobLag`,
    /// `bg_weaponBobMax`.
    pub weapon_bob_base: f32,
    pub weapon_bob_base_laser: f32,
    pub weapon_bob_roll: f32,
    pub weapon_bob_roll_laser: f32,
    pub weapon_bob_lag: f32,
    pub weapon_bob_max: f32,
    /// `bg_weaponBobAmplitude*` (horizontal, vertical).
    pub weapon_bob_standing: [f32; 2],
    pub weapon_bob_ducked: [f32; 2],
    pub weapon_bob_prone: [f32; 2],
    pub weapon_bob_sprinting: [f32; 2],
    pub weapon_bob_dtp: [f32; 2],
    pub weapon_bob_swimming: [f32; 2],
    /// `bg_bobMax` and `bg_bobAmplitude*` (horizontal, vertical) for the camera.
    pub bob_max: f32,
    pub bob_standing: [f32; 2],
    pub bob_standing_ads: [f32; 2],
    pub bob_ducked: [f32; 2],
    pub bob_ducked_ads: [f32; 2],
    pub bob_prone: [f32; 2],
    pub bob_sprinting: [f32; 2],
    pub bob_dtp: [f32; 2],
    pub bob_swimming: [f32; 2],
}

impl Default for RecoilDvars {
    fn default() -> Self {
        Self {
            aqs: true,
            aqs_style: 2,
            ads_zero_spread: true,
            zero_idle_movement: false,
            move_speed_threshold: 11.0,
            standing_view_height: 60,
            perk_weap_spread_multiplier: 0.65,
            charge_view_kick_increase: 0.5,
            charge_max_view_kick: 85.0,
            charge_center_reduction: 0.075,
            charge_min_center_speed: 350.0,
            weapon_bob_base: 0.16,
            weapon_bob_base_laser: 0.05,
            weapon_bob_roll: 1.5,
            weapon_bob_roll_laser: 1.0,
            weapon_bob_lag: 0.25,
            weapon_bob_max: 8.0,
            weapon_bob_standing: [0.055, 0.025],
            weapon_bob_ducked: [0.045, 0.025],
            weapon_bob_prone: [0.02, 0.005],
            weapon_bob_sprinting: [0.02, 0.014],
            weapon_bob_dtp: [0.002, 0.002],
            weapon_bob_swimming: [4.0, 4.0],
            bob_max: 8.0,
            bob_standing: [0.007, 0.007],
            bob_standing_ads: [0.007, 0.007],
            bob_ducked: [0.0075, 0.0075],
            bob_ducked_ads: [0.0075, 0.0075],
            bob_prone: [0.08, 0.04],
            bob_sprinting: [0.02, 0.014],
            bob_dtp: [0.002, 0.002],
            bob_swimming: [3.0, 2.0],
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Small helpers.

/// BO2's shared random step: a 32-bit LCG; the value is the top 15 bits over 32768 (0..1).
pub fn bg_random(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    (*seed >> 17) as f32 / 32768.0
}

fn rand_range(seed: &mut u32, min: f32, max: f32) -> f32 {
    bg_random(seed) * (max - min) + min
}

/// Wrap an angle to -180..180 (rounding the turn count half up).
pub fn angle_wrap180(a: f32) -> f32 {
    let t = a * (1.0 / 360.0);
    (t - (t + 0.5).floor()) * 360.0
}

/// Wrap an angle to 0..360.
pub fn angle_wrap360(a: f32) -> f32 {
    let t = a * (1.0 / 360.0);
    (t - t.floor()) * 360.0
}

/// Ease `cur` toward `tgt` by `(tgt - cur) * rate * dt`; snap when close (0.001) or when the step
/// would overshoot.
pub fn diff_track(tgt: f32, cur: f32, rate: f32, dt: f32) -> f32 {
    let gap = tgt - cur;
    let step = gap * rate * dt;
    if gap.abs() > 0.001 && step.abs() <= gap.abs() {
        cur + step
    } else {
        tgt
    }
}

/// [`diff_track`] for angles: the target is brought within 180 of `cur` first; the result is 0..360.
pub fn diff_track_angle(mut tgt: f32, cur: f32, rate: f32, dt: f32) -> f32 {
    while tgt - cur > 180.0 {
        tgt -= 360.0;
    }
    while tgt - cur < -180.0 {
        tgt += 360.0;
    }
    angle_wrap360(diff_track(tgt, cur, rate, dt))
}

fn lerp(a: f32, b: f32, f: f32) -> f32 {
    (b - a) * f + a
}

fn is_firing(weaponstate: i32) -> bool {
    weaponstate == state::FIRING || weaponstate == state::CONT_FIRE_LOOP
}

// ---------------------------------------------------------------------------------------------
// Spread.

/// The player's spread override (script `setspreadoverride`): state 0 none, 1 easing back, 2 fixed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpreadOverride {
    pub value: i32,
    pub state: i32,
}

/// What PM_AdjustAimSpreadScale reads from the player for one move.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimSpreadInput {
    pub frametime: f32,
    /// No ground entity and not a dead/noclip move type.
    pub in_air: bool,
    pub prone: bool,
    pub ducked: bool,
    pub frac: f32,
    pub quick_scope_scale: f32,
    pub quick_scope_timer: i32,
    /// This move's and the last move's command angles (pitch, yaw) in command units.
    pub cmd_angles: [i32; 2],
    pub old_cmd_angles: [i32; 2],
    /// Forward or right move held.
    pub moving: bool,
    pub velocity_xy: [f32; 2],
    /// The player's move speed (`ps.speed`).
    pub speed: i32,
    /// Riding a vehicle that allows weapons: its velocity (xy) and its type's max speed.
    pub vehicle: Option<([f32; 2], f32)>,
}

/// PM_AddQuickScopeSpread: a just-zoomed sniper (timer still at its start) gets a spread bump.
pub fn quick_scope_spread(s: &SpreadFacts, scale: f32, qs_scale: f32, qs_timer: i32, d: &RecoilDvars) -> f32 {
    if d.aqs
        && qs_scale > 1.0
        && scale <= s.aqs_spread_max
        && qs_timer == (s.aqs_time * 1000.0) as i32
    {
        (qs_scale - 1.0) * s.aqs_spread_multiplier
    } else {
        0.0
    }
}

/// PM_AdjustAimSpreadScale: grow the spread scale from turning, moving and jumping, else decay it.
pub fn adjust_aim_spread_scale(
    scale: &mut f32,
    ov: &mut SpreadOverride,
    s: &SpreadFacts,
    i: &AimSpreadInput,
    d: &RecoilDvars,
) {
    let mut rate = s.decay_rate;
    let mut r = 1.0;
    let mut grow = 0.0;
    if rate == 0.0 {
        rate = 1.0;
    } else {
        let ovf = ov.value as f32;
        r = (ovf - s.stand_min) / (s.stand_max - s.stand_min);
        if i.in_air {
            rate *= 0.5;
        } else if i.prone {
            rate *= s.prone_decay;
            r = (ovf - s.prone_min) / (s.prone_max - s.prone_min);
        } else if i.ducked {
            rate *= s.ducked_decay;
            r = (ovf - s.ducked_min) / (s.ducked_max - s.ducked_min);
        }
        rate *= i.frametime;
        if ov.state == 1 {
            rate /= r;
        } else if i.frac != 1.0 || (d.aqs && i.quick_scope_timer != 0) {
            let mut g = quick_scope_spread(s, *scale, i.quick_scope_scale, i.quick_scope_timer, d);
            if s.turn_add != 0.0 {
                for k in 0..2 {
                    let turn = i.cmd_angles[k] as f32 * SHORT2ANGLE - i.old_cmd_angles[k] as f32 * SHORT2ANGLE;
                    g += angle_wrap180(turn).abs() * 0.01 * s.turn_add * (1.0 / i.frametime);
                }
            }
            if s.move_add != 0.0 {
                let thr = d.move_speed_threshold * d.move_speed_threshold;
                if i.moving {
                    let v2 = i.velocity_xy[0] * i.velocity_xy[0] + i.velocity_xy[1] * i.velocity_xy[1];
                    if thr < v2 {
                        g += v2.sqrt() * s.move_add / i.speed as f32;
                    }
                }
                if let Some((v, max_speed)) = i.vehicle {
                    let v2 = v[0] * v[0] + v[1] * v[1];
                    if thr < v2 && max_speed > 0.0 {
                        g += v2.sqrt() * s.move_add / max_speed;
                    }
                }
            }
            if i.in_air {
                g += 1.28 + 1.28;
            }
            grow = i.frametime * g;
        } else if d.ads_zero_spread {
            *scale = 0.0;
        }
    }
    let v = if grow > 0.0 { grow * SPREAD_MAX + *scale } else { *scale - rate * SPREAD_MAX };
    *scale = v;
    if ov.state == 1 && v * r < SPREAD_MAX {
        ov.state = 0;
        *scale = v * r;
    }
    *scale = scale.clamp(0.0, SPREAD_MAX);
}

/// PM_Weapon_AddFiringAimSpreadScale: a shot not fully aimed adds the weapon's fire add.
pub fn add_firing_aim_spread(scale: f32, s: &SpreadFacts, frac: f32) -> f32 {
    if frac == 1.0 {
        return scale;
    }
    (s.fire_add * SPREAD_MAX + scale).min(SPREAD_MAX)
}

/// What BG_GetSpreadForWeapon reads besides the weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpreadInput {
    pub view_height: f32,
    pub override_: SpreadOverride,
    /// The weapon fires stacked shots (fire type STACKED) and its clip size.
    pub stacked: bool,
    pub clip_size: i32,
    pub player_perks: u64,
    pub zombies: bool,
    pub frac: f32,
}

/// BG_GetSpreadForWeapon: the hip spread (min, max) in degrees for the current stance height.
pub fn spread_for_weapon(s: &SpreadFacts, i: &SpreadInput, d: &RecoilDvars) -> (f32, f32) {
    let (mut min, mut max);
    if i.override_.state == 2 {
        min = i.override_.value as f32;
        max = min;
    } else {
        let h = i.view_height;
        if h <= 40.0 {
            let t = (h - 11.0) * 0.034_482_76;
            min = lerp(s.prone_min, s.ducked_min, t);
            max = lerp(s.prone_max, s.ducked_max, t);
        } else {
            let t = (h - 40.0) / (d.standing_view_height - 40) as f32;
            min = lerp(s.ducked_min, s.stand_min, t);
            max = lerp(s.ducked_max, s.stand_max, t);
        }
    }
    if i.stacked && i.clip_size > 1 {
        let k = s.stack_fire_accuracy_decay.powi(i.clip_size - 1);
        min *= k;
        max *= k;
    }
    if i.override_.state == 1 {
        max = i.override_.value as f32;
    }
    min *= s.hip_min_scale;
    max *= s.hip_max_scale;
    let player_has = |bit: u32| i.player_perks & (1u64 << bit) != 0;
    let steady = if i.zombies {
        (player_has(7) || s.has_perk(7)) && i.frac < 1.0
    } else {
        player_has(3) || s.has_perk(3)
    };
    if steady {
        min *= d.perk_weap_spread_multiplier;
        max *= d.perk_weap_spread_multiplier;
    }
    (min, max)
}

/// The bullet cone in degrees at fire: fully aimed lerps from the ADS spread, else from the hip min.
pub fn bullet_cone(s: &SpreadFacts, scale: f32, min: f32, max: f32, frac: f32) -> f32 {
    let n = scale / SPREAD_MAX;
    if frac == 1.0 {
        (max - s.ads_spread) * n + s.ads_spread
    } else {
        (max - min) * n + min
    }
}

/// CG_CalcReticleSpread: each crosshair side's distance from the centre (virtual 640x480 units).
/// `cone_scale` is the drawer's multiplier, `side_shift` each axis' shift, `tan_half_fov_y` the view.
pub fn reticle_spread(
    s: &SpreadFacts,
    scale: f32,
    min: f32,
    max: f32,
    cone_scale: f32,
    side_shift: [f32; 2],
    tan_half_fov_y: f32,
) -> [f32; 2] {
    let cone = ((max - min) * scale * (1.0 / 255.0) + min) * cone_scale;
    let d = ((cone * 0.017_453_292).tan() * 240.0 / tan_half_fov_y).max(s.reticle_min_ofs as f32);
    [d - side_shift[0] * s.reticle_side_pos, d - side_shift[1] * s.reticle_side_pos]
}

// ---------------------------------------------------------------------------------------------
// Kick.

/// The kick state one player carries: the camera's kick (angles and velocity, three axes), the
/// gun's recoil spring (pitch, yaw), and the ADS recoil reduction memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KickState {
    pub view_angles: [f32; 3],
    pub view_avel: [f32; 3],
    pub gun_angles: [f32; 2],
    pub gun_speed: [f32; 2],
    pub ads_recoil_reduction: f32,
    pub last_shot_ms: i32,
}

/// What BG_WeaponFireRecoil reads besides the kick state.
#[derive(Clone, Copy, Debug)]
pub struct FireRecoilInput<'a> {
    /// The fired (viewmodel) weapon.
    pub facts: &'a SpreadFacts,
    /// The left gun's facts when the weapon is dual wield.
    pub dual: Option<&'a SpreadFacts>,
    pub frac: f32,
    pub restrict_kick_time: i32,
    /// The trigger was still held from the last shot (ADS recoil reduction builds up).
    pub held: bool,
    pub now: i32,
    /// `Some(level)` for charge-shot weapons.
    pub charge_level: Option<i32>,
    /// Gas-type weapon (flamethrower).
    pub gas: bool,
    pub weaponstate: i32,
    pub weaponstate_left: i32,
}

fn charged_pitch(p: f32, level: i32, d: &RecoilDvars) -> f32 {
    if level > 1 {
        (d.charge_view_kick_increase * level as f32 * p + p).min(d.charge_max_view_kick)
    } else {
        p
    }
}

fn min_magnitude(p: f32, y: f32, m: f32) -> (f32, f32) {
    let len = (p * p + y * y).sqrt();
    if len < m && len > 0.0 {
        let k = m / len;
        (p * k, y * k)
    } else {
        (p, y)
    }
}

/// BG_WeaponFireRecoil: one shot's kick. Sets the camera's kick velocity and pushes the gun spring.
/// Random draws, in order: view pitch, view yaw, gun pitch, gun yaw.
pub fn fire_recoil(k: &mut KickState, i: &FireRecoilInput, seed: &mut u32, d: &RecoilDvars) {
    let s = i.facts;
    let mut scale = 1.0;
    if i.restrict_kick_time > 0 {
        let pct = if i.frac == 1.0 { s.ads_reduced_kick_percent } else { s.hip_reduced_kick_percent };
        scale = pct * 0.01;
    }
    let mut reduction = 0.0;
    let (pitch, yaw);
    if i.frac == 1.0 {
        let mut range = s.ads_view_kick;
        let rate = s.ads_recoil_reduction_rate;
        if rate > 0.0 {
            reduction = if i.held {
                k.ads_recoil_reduction + rate
            } else {
                let back = (i.now - k.last_shot_ms) as f32 * 0.001 * s.ads_recoil_return_rate;
                (k.ads_recoil_reduction - back).max(0.0)
            };
            reduction = reduction.min(s.ads_recoil_reduction_limit).max(0.0);
            range = range.scaled(1.0 - reduction);
        }
        let mut p = rand_range(seed, range.pitch_min, range.pitch_max);
        if let Some(level) = i.charge_level {
            p = charged_pitch(p, level, d);
        }
        let y = rand_range(seed, range.yaw_min, range.yaw_max);
        (pitch, yaw) = min_magnitude(p, y, (1.0 - rate.max(0.0)) * s.ads_view_kick_min_magnitude);
    } else if let Some(left) = i.dual {
        let range = dual_range(s.hip_view_kick, left.hip_view_kick, i);
        pitch = rand_range(seed, range.pitch_min, range.pitch_max);
        yaw = rand_range(seed, range.yaw_min, range.yaw_max);
    } else {
        let range = s.hip_view_kick;
        let mut p = rand_range(seed, range.pitch_min, range.pitch_max);
        if let Some(level) = i.charge_level {
            p = charged_pitch(p, level, d);
        }
        let y = rand_range(seed, range.yaw_min, range.yaw_max);
        (pitch, yaw) = min_magnitude(p, y, s.hip_view_kick_min_magnitude);
    }
    k.ads_recoil_reduction = reduction;
    k.last_shot_ms = i.now;

    k.view_avel[0] = if i.gas && k.view_avel[0] == 0.0 {
        k.view_avel[0] - 0.001
    } else {
        -(pitch.abs() * scale)
    };
    k.view_avel[1] = yaw * scale;
    k.view_avel[2] = yaw * scale * -0.5;

    let gun = if i.frac > 0.0 {
        s.ads_gun_kick
    } else if let Some(left) = i.dual {
        dual_range(s.hip_gun_kick, left.hip_gun_kick, i)
    } else {
        s.hip_gun_kick
    };
    let gp = rand_range(seed, gun.pitch_min, gun.pitch_max);
    let gy = rand_range(seed, gun.yaw_min, gun.yaw_max);
    k.gun_speed[0] += gp * scale;
    k.gun_speed[1] += gy * scale;
}

/// Dual wield: both guns firing add their ranges; only the left firing uses the left gun's.
fn dual_range(right: KickRange, left: KickRange, i: &FireRecoilInput) -> KickRange {
    match (is_firing(i.weaponstate), is_firing(i.weaponstate_left)) {
        (true, true) => left.plus(right),
        (_, true) => left,
        _ => right,
    }
}

/// What the camera kick's centering speed depends on.
#[derive(Clone, Copy, Debug)]
pub struct CenterInput<'a> {
    /// `None` when the player holds no weapon.
    pub facts: Option<&'a SpreadFacts>,
    /// The left gun's facts when the weapon is dual wield.
    pub dual: Option<&'a SpreadFacts>,
    pub frac: f32,
    pub charge_level: Option<i32>,
    pub weaponstate: i32,
    pub weaponstate_left: i32,
    /// PM_GetEffectiveStance: 0 standing, 1 prone, 2 crouched.
    pub stance: i32,
}

/// BG_GetChargeShotWeaponCenterSpeed: a charged shot centers slower (never under the minimum).
fn charge_center_speed(s: &SpreadFacts, level: i32, d: &RecoilDvars) -> f32 {
    let c = s.ads_center_speed;
    if level > 1 {
        (c - d.charge_center_reduction * level as f32 * c).max(d.charge_min_center_speed)
    } else {
        c
    }
}

/// The camera kick's centering speed (degrees/s^2) in CG_KickAngles.
pub fn view_kick_center_speed(i: &CenterInput, d: &RecoilDvars) -> f32 {
    let Some(s) = i.facts else { return 2400.0 };
    if i.frac <= 0.5 {
        if let Some(left) = i.dual {
            match (is_firing(i.weaponstate), is_firing(i.weaponstate_left)) {
                (true, true) => left.hip_center_speed + s.hip_center_speed,
                (_, true) => left.hip_center_speed,
                _ => s.hip_center_speed,
            }
        } else if let Some(level) = i.charge_level {
            charge_center_speed(s, level, d)
        } else {
            // BO2 uses the ADS value at the hip, without the attachment scales.
            s.ads_center_speed_raw
        }
    } else {
        let c = match i.charge_level {
            Some(level) => charge_center_speed(s, level, d),
            None => s.ads_center_speed,
        };
        match i.stance {
            1 => c * s.ads_center_prone_scale,
            2 => c * s.ads_center_ducked_scale,
            _ => c,
        }
    }
}

/// CG_KickAngles: settle the camera kick over `frame_ms`, in steps of at most 5 ms.
pub fn settle_view_kick(k: &mut KickState, frame_ms: i32, center: f32) {
    let mut left = frame_ms;
    while left > 0 {
        let step = left.min(5) as f32;
        for ax in 0..3 {
            let (a, v) = (k.view_angles[ax], k.view_avel[ax]);
            if a == 0.0 && v == 0.0 {
                continue;
            }
            let mut v = v;
            if a != 0.0 {
                let c = if a <= 0.0 { center } else { -center };
                v += c * step * 0.001;
            }
            let mut dlt = v * step * 0.001;
            if a * dlt < 0.0 {
                dlt *= 0.06;
            }
            if (a + dlt) * a < 0.0 {
                k.view_angles[ax] = 0.0;
                k.view_avel[ax] = 0.0;
                continue;
            }
            let na = a + dlt;
            if na == 0.0 {
                k.view_angles[ax] = 0.0;
                k.view_avel[ax] = 0.0;
            } else if na.abs() > 10.0 {
                k.view_angles[ax] = if na > 0.0 { 10.0 } else { -10.0 };
                k.view_avel[ax] = 0.0;
            } else {
                k.view_angles[ax] = na;
                k.view_avel[ax] = v;
            }
        }
        left -= 5;
    }
}

/// The gun recoil spring in BG_CalculateWeaponMovement (only weapons that can aim down sights).
/// Runs `frametime` seconds in 5 ms steps; stops early once both axes rest.
pub fn gun_spring(k: &mut KickState, s: &SpreadFacts, frac: f32, frametime: f32) {
    if !s.aim_down_sight {
        return;
    }
    let sp = s.hip_spring.lerp(s.ads_spring, frac);
    let max = [s.gun_max_pitch, s.gun_max_yaw];
    let mut t = frametime;
    while t > 0.0 {
        let step = t.min(0.005);
        t = if t <= 0.005 { 0.0 } else { t - 0.005 };
        let mut rested = [false; 2];
        for ax in 0..2 {
            let (mut a, mut v) = (k.gun_angles[ax], k.gun_speed[ax]);
            if a.abs() < 0.25 && v.abs() < 1.0 {
                k.gun_angles[ax] = 0.0;
                k.gun_speed[ax] = 0.0;
                rested[ax] = true;
                continue;
            }
            a += step * v;
            if a > max[ax] {
                a = max[ax];
                if v > 0.0 {
                    v = 0.0;
                }
            } else if a < -max[ax] {
                a = -max[ax];
                if v < 0.0 {
                    v = 0.0;
                }
            }
            if a > 0.0 {
                v -= step * sp.accel;
            } else if a < 0.0 {
                v += step * sp.accel;
            }
            v -= v * sp.speed_decay * step;
            let sd = step * sp.static_decay;
            if v <= 0.0 {
                v += sd;
                if v > 0.0 {
                    v = 0.0;
                }
            } else {
                v -= sd;
                if v < 0.0 {
                    v = 0.0;
                }
            }
            v = v.clamp(-sp.speed_max, sp.speed_max);
            k.gun_angles[ax] = a;
            k.gun_speed[ax] = v;
        }
        if rested[0] && rested[1] {
            break;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Sway, idle, bob, landing.

/// The gun's sway: angles (pitch, yaw, 0..360) and its side/up offset, plus the last view angles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwayState {
    pub angles: [f32; 2],
    pub side: f32,
    pub up: f32,
    pub last_view: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwayInput {
    pub frac: f32,
    pub view_angles: [f32; 3],
    pub dt: f32,
    /// Shellshock time left (start + duration - now, ms) and its fade time (ms).
    pub shock_ms_left: i32,
    pub shock_fade_ms: i32,
}

/// BG_CalculateWeaponMovement_Sway: the gun lags behind the view's turns.
pub fn sway(st: &mut SwayState, s: &SpreadFacts, i: &SwayInput) {
    if i.dt == 0.0 {
        return;
    }
    let mut set: SwaySettings = s.sway;
    if s.aim_down_sight {
        if i.frac > 0.0 && s.overlay {
            st.last_view = i.view_angles;
            return;
        }
        set = set.lerp(s.ads_sway, i.frac);
    }
    if i.shock_ms_left > 0 {
        let mut f = 1.0;
        if i.shock_ms_left < i.shock_fade_ms {
            f = i.shock_ms_left as f32 / i.shock_fade_ms as f32;
        }
        let k = (s.sway_shellshock_scale - 1.0) * (3.0 - f * 2.0) * f * f + 1.0;
        set.pitch_scale *= k;
        set.yaw_scale *= k;
        set.horiz_scale *= k;
        set.vert_scale *= k;
    }
    let inv = 1.0 / (i.dt * 60.0);
    let m = set.max_angle;
    let dp = (angle_wrap180(i.view_angles[0] - st.last_view[0]) * inv).clamp(-m, m);
    let dy = (angle_wrap180(i.view_angles[1] - st.last_view[1]) * inv).clamp(-m, m);
    st.side = diff_track(dy * set.horiz_scale, st.side, set.lerp_speed, i.dt);
    st.up = diff_track(dp * set.vert_scale, st.up, set.lerp_speed, i.dt);
    st.angles[0] = diff_track_angle(dp * set.pitch_scale, st.angles[0], set.lerp_speed, i.dt);
    st.angles[1] = diff_track_angle(dy * set.yaw_scale, st.angles[1], set.lerp_speed, i.dt);
    st.last_view = i.view_angles;
}

/// The idle clock step (PM_UpdateSwayPrediction / BG_IncrementWeaponMovementIdleTime). The caller
/// skips it when not aimed at all. Scoped weapons step twice (the camera's idle and the gun's).
pub fn step_idle_time(time: i32, s: &SpreadFacts, frac: f32, hold_breath: f32, dt: f32) -> i32 {
    let t = step_view_idle_time(time, s, frac, hold_breath, dt);
    step_weapon_idle_time(t, s, frac, hold_breath, dt)
}

fn idle_speed(s: &SpreadFacts, frac: f32) -> f32 {
    if s.aim_down_sight { lerp(s.hip_idle_speed, s.ads_idle_speed, frac) } else { s.hip_idle_speed }
}

/// BG_IncrementViewMovementIdleTime: the camera's step, scoped weapons only.
pub fn step_view_idle_time(time: i32, s: &SpreadFacts, frac: f32, hold_breath: f32, dt: f32) -> i32 {
    if !s.overlay {
        return time;
    }
    time.wrapping_add((hold_breath * dt * 1000.0 * idle_speed(s, frac)) as i32)
}

/// BG_IncrementWeaponMovementIdleTime: the gun's step; holding the breath counts half.
pub fn step_weapon_idle_time(time: i32, s: &SpreadFacts, frac: f32, hold_breath: f32, dt: f32) -> i32 {
    let hb = if hold_breath > 1.0 { (hold_breath - 1.0) * 0.5 + 1.0 } else { hold_breath };
    time.wrapping_add((hb * dt * 1000.0 * idle_speed(s, frac)) as i32)
}

/// The client's two idle clocks. Aimed at all, the player state's clock runs (both steps, in
/// pmove); from the hip the client runs its own clock (CG_BuildWeaponState: the gun's step only)
/// and writes it into the player state. Returns the clock the drift reads this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdleClocks {
    pub ps: i32,
    pub hip: i32,
}

impl IdleClocks {
    pub fn step(&mut self, s: &SpreadFacts, frac: f32, hold_breath: f32, dt: f32) -> i32 {
        if frac != 0.0 {
            self.ps = step_idle_time(self.ps, s, frac, hold_breath, dt);
        } else {
            self.hip = step_weapon_idle_time(self.hip, s, frac, hold_breath, dt);
            self.ps = self.hip;
        }
        self.ps
    }
}

/// BG_ComputeAndApplyWeaponMovement_IdleAngles: the gun's slow drift. `factor` is the eased stance
/// factor the player keeps (starts at 1).
#[allow(clippy::too_many_arguments)]
pub fn idle_angles(
    factor: &mut f32,
    s: &SpreadFacts,
    frac: f32,
    prone: bool,
    ducked: bool,
    frametime: f32,
    idle_time: i32,
    d: &RecoilDvars,
) -> [f32; 3] {
    let mut amt = s.hip_idle_amount;
    if s.aim_down_sight {
        amt = lerp(amt, s.ads_idle_amount, frac);
        if d.zero_idle_movement {
            amt = 0.0;
        }
    }
    ease_idle_factor(factor, s, prone, ducked, frametime);
    let mut a = *factor * amt;
    if s.overlay {
        a *= 1.0 - frac;
    }
    let t = idle_time as f32;
    [(t * 0.001).sin() * a * 0.01, (t * 0.0007).sin() * a * 0.01, (t * 0.0005).sin() * a * 0.01]
}

/// The stance factor eases toward 1 / crouch / prone at half a unit per second.
fn ease_idle_factor(factor: &mut f32, s: &SpreadFacts, prone: bool, ducked: bool, frametime: f32) {
    let target = if prone {
        s.idle_prone_factor
    } else if ducked {
        s.idle_crouch_factor
    } else {
        1.0
    };
    let step = frametime * 0.5;
    if *factor < target {
        *factor = (*factor + step).min(target);
    } else if *factor > target {
        *factor = (*factor - step).max(target);
    }
}

/// What the camera's scope drift reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraIdleInput {
    pub frac: f32,
    /// Prone, and not diving (a dive keeps the standing factor).
    pub prone: bool,
    pub ducked: bool,
    pub frametime: f32,
    pub idle_time: i32,
    pub hold_breath: f32,
    pub quick_scope_scale: f32,
    /// The weapon's `fAntiQuickScopeSwayFactor`.
    pub aqs_sway_factor: f32,
}

/// BG_CalculateViewMovement_IdleAngles: a scope's drift on the camera, (pitch, yaw). Only a scoped
/// weapon at least partly aimed drifts. `factor` is the camera's own eased stance factor.
pub fn camera_idle_angles(factor: &mut f32, s: &SpreadFacts, i: &CameraIdleInput, d: &RecoilDvars) -> [f32; 2] {
    if i.frac == 0.0 || !s.overlay {
        return [0.0; 2];
    }
    let mut amt = s.hip_idle_amount;
    if s.aim_down_sight {
        amt = lerp(amt, s.ads_idle_amount, i.frac);
        if d.zero_idle_movement {
            amt = 0.0;
        }
    } else if amt == 0.0 {
        amt = 80.0;
    }
    ease_idle_factor(factor, s, i.prone, i.ducked, i.frametime);
    let mut a = *factor * amt * i.frac * i.hold_breath;
    if d.aqs && d.aqs_style == 1 {
        a *= i.quick_scope_scale;
    }
    let t = i.idle_time as f32;
    let sy = (t * 0.0007).sin();
    let mut yaw = sy * a * 0.01;
    let mut pitch = (t * 0.001).sin() * a * 0.01;
    if d.aqs && d.aqs_style == 2 {
        let q = (i.quick_scope_scale - 1.0) * i.aqs_sway_factor * 0.01;
        yaw -= q * sy;
        pitch += q;
    }
    [pitch, yaw]
}

/// The client's copy of PM_UpdateQuickScope (the server's scale is not sent): the scope's extra
/// wobble right after it comes up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuickScope {
    pub timer: i32,
    pub scale: f32,
}

impl Default for QuickScope {
    fn default() -> Self {
        Self { timer: 0, scale: 1.0 }
    }
}

impl QuickScope {
    pub fn step(&mut self, a: &crate::pm::ads::AdsFacts, prev_frac: f32, frac: f32, msec: i32) {
        let t = a.aqs_ms();
        if t < 1 || !a.anti_quick_scope {
            *self = Self::default();
            return;
        }
        self.timer = if prev_frac > frac || frac < 0.001 {
            0
        } else if frac < 1.0 {
            t
        } else {
            self.timer - msec
        }
        .max(0);
        let target = (self.timer as f32 / t as f32) * (a.aqs_scale - 1.0) * frac + 1.0;
        let s = diff_track(target, self.scale, crate::pm::ads::dvar::PLAYER_BREATH_GASP_LERP, msec as f32 * 0.001);
        self.scale = if s < 1.0 { 1.0 } else { s };
    }
}

/// What bob reads from the player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BobInput {
    pub bob_cycle: i32,
    pub xyspeed: f32,
    pub waterlevel: i32,
    /// Dive-to-prone in the air (pm_flags 0x200000).
    pub dtp: bool,
    pub view_height_target: i32,
    /// Sprinting (pm_flags 0x8000).
    pub sprinting: bool,
    pub frac: f32,
}

/// The gun bob amplitude pair (horizontal, vertical) for this state, `m` = the speed term.
fn gun_bob_amp(s: &SpreadFacts, i: &BobInput, m: f32, d: &RecoilDvars) -> [f32; 2] {
    let a = if i.waterlevel >= 4 {
        d.weapon_bob_swimming
    } else if i.dtp {
        let v = d.weapon_bob_dtp;
        [v[0] * m * s.dtp_bob[0], v[1] * m * s.dtp_bob[1]]
    } else if i.view_height_target == 11 {
        let v = d.weapon_bob_prone;
        [v[0] * m, v[1] * m]
    } else if i.view_height_target == 40 || !i.sprinting {
        let v = if i.view_height_target == 40 { d.weapon_bob_ducked } else { d.weapon_bob_standing };
        let k = (1.0 - i.frac) * m;
        [k * v[0], k * v[1]]
    } else {
        let v = d.weapon_bob_sprinting;
        [v[0] * m * s.sprint_bob[0], v[1] * m * s.sprint_bob[1]]
    };
    [a[0].min(d.weapon_bob_max), a[1].min(d.weapon_bob_max)]
}

fn bob_vertical(th: f32, amp: f32) -> f32 {
    ((th + th).sin() + (4.0 * th + 1.570_796_4).sin() * 0.2) * 0.75 * amp
}

/// BG_CalculateWeaponMovement_Bob: the gun's walk bob angles (pitch, yaw, roll).
pub fn gun_bob(s: &SpreadFacts, i: &BobInput, d: &RecoilDvars) -> [f32; 3] {
    let c = (i.bob_cycle & 0xff) as f32 * (1.0 / 255.0) * PI;
    let th = d.weapon_bob_lag * PI + c + c + 2.0 * PI + 2.0 * PI;
    let base = if s.laser_sight { d.weapon_bob_base_laser } else { d.weapon_bob_base };
    let m = i.xyspeed * base;
    let amp = gun_bob_amp(s, i, m, d);
    let roll_k = if s.laser_sight { d.weapon_bob_roll_laser } else { d.weapon_bob_roll };
    let roll_amp = gun_bob_amp(s, i, roll_k * m, d);
    let mut out = [
        -bob_vertical(th, amp[1]),
        -(th.sin() * amp[0]),
        ((th - 0.471_238_94).sin() * roll_amp[0]).min(0.0),
    ];
    if i.frac > 0.0 {
        let k = 1.0 - (1.0 - s.ads_bob_factor) * i.frac;
        for v in &mut out {
            *v *= k;
        }
    }
    out
}

/// The camera bob (BG_CalculateViewMovement_*BobFactor): (horizontal, vertical) offsets.
pub fn camera_bob(i: &BobInput, speed: f32, d: &RecoilDvars) -> [f32; 2] {
    let a = if i.waterlevel >= 4 {
        d.bob_swimming
    } else {
        let v = if i.dtp {
            d.bob_dtp
        } else if i.view_height_target == 11 {
            d.bob_prone
        } else if i.view_height_target == 40 {
            [lerp(d.bob_ducked[0], d.bob_ducked_ads[0], i.frac), lerp(d.bob_ducked[1], d.bob_ducked_ads[1], i.frac)]
        } else if i.sprinting {
            d.bob_sprinting
        } else {
            [lerp(d.bob_standing[0], d.bob_standing_ads[0], i.frac), lerp(d.bob_standing[1], d.bob_standing_ads[1], i.frac)]
        };
        [v[0] * speed, v[1] * speed]
    };
    let c = (i.bob_cycle & 0xff) as f32 * (1.0 / 255.0) * PI;
    let th = c + c + 2.0 * PI;
    [th.sin() * a[0].min(d.bob_max), bob_vertical(th, a[1].min(d.bob_max))]
}

/// The gun's angles (BG_SetWeaponMovementAngles) without the damage kick: base pose faded out
/// while aiming, minus sway, lean roll, plus the recoil spring, bob and idle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementAnglesInput {
    pub base: [f32; 3],
    pub frac: f32,
    pub sway: [f32; 2],
    pub leanf: f32,
    pub recoil: [f32; 2],
    pub bob: [f32; 3],
    pub idle: [f32; 3],
    /// The weapon has a scope overlay.
    pub overlay: bool,
}

pub fn weapon_movement_angles(i: &MovementAnglesInput) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    let f = 1.0 - i.frac * 2.0;
    if f > 0.0 {
        out = [f * i.base[0], f * i.base[1], f * i.base[2]];
    }
    out[0] = angle_wrap180(out[0] - i.sway[0]);
    out[1] = angle_wrap180(out[1] - i.sway[1]);
    if i.leanf != 0.0 {
        let lean = (2.0 - i.leanf.abs()) * i.leanf;
        out[2] -= lean + lean;
    }
    out[0] += i.recoil[0];
    out[1] += i.recoil[1];
    let bob_k = if i.overlay { 1.0 - i.frac } else { 1.0 };
    for (o, (b, idle)) in out.iter_mut().zip(i.bob.iter().zip(i.idle.iter())) {
        *o += b * bob_k + idle;
    }
    out
}

/// The landing dip offset (up), `ms` after landing: down over 150 ms, back up over the next 300.
/// The camera uses scale 1; the gun uses [`gun_landing_scale`].
pub fn landing_offset(ms: i32, dip: f32, scale: f32) -> f32 {
    if ms < 150 {
        dip * scale * ms as f32 / 150.0
    } else if ms < 450 {
        dip * scale * (450 - ms) as f32 / 300.0
    } else {
        0.0
    }
}

/// The gun's landing scale: 0.25, easing to 0.02 while aiming with the rangefinder.
pub fn gun_landing_scale(s: &SpreadFacts, frac: f32) -> f32 {
    if s.rangefinder && frac > 0.0 {
        (1.0 - frac) * 0.25 + frac * 0.02
    } else {
        0.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> SpreadFacts {
        SpreadFacts {
            stand_min: 3.0,
            stand_max: 7.0,
            ducked_min: 2.5,
            ducked_max: 6.0,
            prone_min: 2.0,
            prone_max: 5.0,
            decay_rate: 4.0,
            fire_add: 0.6,
            turn_add: 0.2,
            move_add: 0.5,
            ducked_decay: 1.05,
            prone_decay: 1.1,
            hip_min_scale: 1.0,
            hip_max_scale: 1.0,
            ads_spread: 0.5,
            reticle_min_ofs: 8,
            reticle_side_pos: 0.0,
            hip_view_kick: KickRange { pitch_min: 1.0, pitch_max: 1.0, yaw_min: -0.5, yaw_max: -0.5 },
            ads_view_kick: KickRange { pitch_min: 1.0, pitch_max: 1.0, yaw_min: 0.0, yaw_max: 0.0 },
            ads_gun_kick: KickRange { pitch_min: 10.0, pitch_max: 10.0, yaw_min: 0.0, yaw_max: 0.0 },
            hip_gun_kick: KickRange { pitch_min: 20.0, pitch_max: 20.0, yaw_min: 5.0, yaw_max: 5.0 },
            ads_center_speed: 1500.0,
            ads_center_speed_raw: 1500.0,
            aim_down_sight: true,
            gun_max_pitch: 6.0,
            gun_max_yaw: 6.0,
            ..Default::default()
        }
    }

    #[test]
    fn random_matches_lcg() {
        let mut s = 0u32;
        assert_eq!(bg_random(&mut s), 19.0 / 32768.0);
        assert_eq!(s, 0x269ec3);
    }

    #[test]
    fn spread_decays_when_still() {
        let s = rifle();
        let d = RecoilDvars::default();
        let mut sc = 200.0;
        let mut ov = SpreadOverride::default();
        let i = AimSpreadInput { frametime: 0.05, speed: 190, ..Default::default() };
        adjust_aim_spread_scale(&mut sc, &mut ov, &s, &i, &d);
        // 4/s decay x 0.05 s x 255 = 51
        assert!((sc - 149.0).abs() < 1e-3, "{sc}");
    }

    #[test]
    fn spread_grows_in_air_and_snaps_to_zero_aimed() {
        let s = rifle();
        let d = RecoilDvars::default();
        let mut ov = SpreadOverride::default();
        let mut sc = 0.0;
        let air = AimSpreadInput { frametime: 0.05, in_air: true, speed: 190, ..Default::default() };
        adjust_aim_spread_scale(&mut sc, &mut ov, &s, &air, &d);
        assert!((sc - 2.56 * 0.05 * 255.0).abs() < 1e-3, "{sc}");
        let aimed = AimSpreadInput { frametime: 0.05, frac: 1.0, speed: 190, ..Default::default() };
        adjust_aim_spread_scale(&mut sc, &mut ov, &s, &aimed, &d);
        assert_eq!(sc, 0.0);
    }

    #[test]
    fn fire_add_caps() {
        let s = rifle();
        assert_eq!(add_firing_aim_spread(200.0, &s, 0.0), 255.0);
        assert_eq!(add_firing_aim_spread(10.0, &s, 1.0), 10.0);
        assert!((add_firing_aim_spread(10.0, &s, 0.5) - (10.0 + 0.6 * 255.0)).abs() < 1e-3);
    }

    #[test]
    fn spread_by_height() {
        let s = rifle();
        let d = RecoilDvars::default();
        let at = |h| spread_for_weapon(&s, &SpreadInput { view_height: h, ..Default::default() }, &d);
        assert_eq!(at(60.0), (3.0, 7.0));
        assert_eq!(at(40.0), (2.5, 6.0));
        let (pmin, pmax) = at(11.0);
        assert!((pmin - 2.0).abs() < 1e-5 && (pmax - 5.0).abs() < 1e-5);
    }

    #[test]
    fn hip_kick_sets_velocity_and_pushes_spring() {
        let s = rifle();
        let d = RecoilDvars::default();
        let mut k = KickState::default();
        let mut seed = 1;
        let i = FireRecoilInput {
            facts: &s,
            dual: None,
            frac: 0.0,
            restrict_kick_time: 0,
            held: false,
            now: 1000,
            charge_level: None,
            gas: false,
            weaponstate: state::FIRING,
            weaponstate_left: 0,
        };
        fire_recoil(&mut k, &i, &mut seed, &d);
        assert_eq!(k.view_avel, [-1.0, -0.5, 0.25]);
        assert_eq!(k.gun_speed, [20.0, 5.0]);
        assert_eq!(k.last_shot_ms, 1000);
    }

    #[test]
    fn view_kick_returns_to_zero() {
        let mut k = KickState { view_avel: [-30.0, 0.0, 0.0], ..Default::default() };
        let mut min = 0.0f32;
        for _ in 0..200 {
            settle_view_kick(&mut k, 16, 1500.0);
            min = min.min(k.view_angles[0]);
        }
        assert!(min < -0.1, "kicked up: {min}");
        assert_eq!(k.view_angles[0], 0.0);
        assert_eq!(k.view_avel[0], 0.0);
    }

    #[test]
    fn spring_rests() {
        let mut s = rifle();
        s.hip_spring = crate::pm::spread::GunSpring { accel: 800.0, speed_max: 2000.0, speed_decay: 16.0, static_decay: 20.0 };
        let mut k = KickState { gun_speed: [20.0, 5.0], ..Default::default() };
        gun_spring(&mut k, &s, 0.0, 0.005);
        assert!((k.gun_angles[0] - 0.1).abs() < 1e-6, "first 5 ms step moves 20 deg/s x 0.005");
        for _ in 0..200 {
            gun_spring(&mut k, &s, 0.0, 0.016);
        }
        assert_eq!(k.gun_angles, [0.0, 0.0]);
        assert_eq!(k.gun_speed, [0.0, 0.0]);
    }

    #[test]
    fn landing_shape() {
        assert_eq!(landing_offset(0, -8.0, 1.0), 0.0);
        assert_eq!(landing_offset(150, -8.0, 1.0), -8.0);
        assert_eq!(landing_offset(300, -8.0, 0.25), -1.0);
        assert_eq!(landing_offset(450, -8.0, 1.0), 0.0);
    }

    #[test]
    fn tracking_snaps_and_wraps() {
        assert_eq!(diff_track(1.0, 0.9995, 10.0, 0.01), 1.0);
        assert!((diff_track(1.0, 0.0, 10.0, 0.01) - 0.1).abs() < 1e-6);
        let a = diff_track_angle(-1.0, 0.0, 1000.0, 1.0);
        assert!((a - 359.0).abs() < 1e-3, "{a}");
        assert!((angle_wrap180(350.0) + 10.0).abs() < 1e-4);
    }

    fn scope() -> SpreadFacts {
        SpreadFacts {
            overlay: true,
            hip_idle_amount: 0.0,
            ads_idle_amount: 60.0,
            hip_idle_speed: 1.0,
            ads_idle_speed: 2.0,
            idle_crouch_factor: 0.5,
            idle_prone_factor: 0.25,
            ..rifle()
        }
    }

    #[test]
    fn idle_clocks_split_by_aim() {
        let s = scope();
        let mut c = IdleClocks::default();
        // From the hip: the gun's step only, at hip speed; holding breath counts half.
        assert_eq!(c.step(&s, 0.0, 1.0, 0.016), 16);
        assert_eq!(c.step(&s, 0.0, 3.0, 0.016), 16 + 32);
        // Aimed: both steps at the blended speed, on top of where the hip clock left off.
        assert_eq!(c.step(&s, 1.0, 1.0, 0.016), 48 + 32 + 32);
        assert_eq!(c.hip, 48);
        // A gun with no scope only takes the gun's step.
        let r = SpreadFacts { overlay: false, ..s };
        assert_eq!(step_idle_time(0, &r, 1.0, 1.0, 0.016), 32);
    }

    #[test]
    fn zero_idle_only_on_ads_weapons() {
        let s = SpreadFacts { hip_idle_amount: 100.0, ..scope() };
        let d = RecoilDvars { zero_idle_movement: true, ..Default::default() };
        // A weapon that cannot aim keeps its drift.
        let mut f = 1.0;
        let n = SpreadFacts { aim_down_sight: false, ..s };
        let hip = idle_angles(&mut f, &n, 0.0, false, false, 0.016, 2000, &d);
        assert!(hip[0] != 0.0);
        let mut f = 1.0;
        let ads = idle_angles(&mut f, &s, 0.5, false, false, 0.016, 2000, &d);
        assert_eq!(ads, [0.0; 3]);
    }

    #[test]
    fn camera_idle_rules() {
        let s = scope();
        let d = RecoilDvars::default();
        let base = CameraIdleInput {
            frac: 1.0,
            frametime: 0.016,
            idle_time: 1500,
            hold_breath: 1.0,
            quick_scope_scale: 1.0,
            aqs_sway_factor: 10.0,
            ..Default::default()
        };
        let mut f = 1.0;
        // Not aimed: nothing.
        assert_eq!(camera_idle_angles(&mut f, &s, &CameraIdleInput { frac: 0.0, ..base }, &d), [0.0; 2]);
        let [p, y] = camera_idle_angles(&mut f, &s, &base, &d);
        assert!((p - (1.5f32).sin() * 0.6).abs() < 1e-5, "{p}");
        assert!((y - (1.05f32).sin() * 0.6).abs() < 1e-5, "{y}");
        // Style 2: a quickscope adds a steady pitch and takes from the yaw.
        let q = CameraIdleInput { quick_scope_scale: 3.0, ..base };
        let [p2, y2] = camera_idle_angles(&mut f, &s, &q, &d);
        assert!((p2 - p - 0.2).abs() < 1e-5, "{p2}");
        assert!((y2 - (y - 0.2 * (1.05f32).sin())).abs() < 1e-5, "{y2}");
        // A non-ADS scoped weapon with no idle amount drifts by 80.
        let n = SpreadFacts { aim_down_sight: false, ..s };
        let [p3, _] = camera_idle_angles(&mut f, &n, &base, &d);
        assert!((p3 - (1.5f32).sin() * 0.8).abs() < 1e-5, "{p3}");
        // No scope picture: nothing.
        let r = SpreadFacts { overlay: false, ..s };
        assert_eq!(camera_idle_angles(&mut f, &r, &base, &d), [0.0; 2]);
    }

    #[test]
    fn quickscope_rises_then_settles() {
        let a = crate::pm::ads::AdsFacts { anti_quick_scope: true, aqs_time: 0.5, aqs_scale: 3.0, ..Default::default() };
        let mut q = QuickScope::default();
        q.step(&a, 0.0, 0.5, 16);
        assert_eq!(q.timer, 500);
        assert!(q.scale > 1.0);
        for _ in 0..200 {
            q.step(&a, 1.0, 1.0, 16);
        }
        assert_eq!(q.timer, 0);
        assert_eq!(q.scale, 1.0);
        let off = crate::pm::ads::AdsFacts::default();
        q.step(&off, 0.0, 1.0, 16);
        assert_eq!(q, QuickScope::default());
    }

    #[test]
    fn reticle_floor() {
        let s = rifle();
        let r = reticle_spread(&s, 0.0, 0.0, 0.0, 1.0, [0.0, 0.0], 0.75);
        assert_eq!(r, [8.0, 8.0]);
    }
}
