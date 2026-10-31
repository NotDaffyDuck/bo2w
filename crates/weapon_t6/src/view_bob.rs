//! What the walk, a landing, a hit and the scope's drift do to the camera: the head bob,
//! the landing dip, the flinch, and the hit feedback the server sends.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use game_math::{euler_basis, lean_amount, dir_to_euler};

use crate::env::perk;
use super::placement::{
    PMF_DIVE_ANY, EYE_LEVEL_CROUCHED, EYE_LEVEL_LYING, SWING_AMP_BASE,
    SWING_AMP_ROLL, SWING_DELAY, SWING_RISE_PHASE, REST_PITCH_HZ,
    GUN_REST_WAVE_FACTOR, REST_YAW_HZ, GunRestDriftFeed, GunPosePlayerFeed,
    idle_clock_step, idle_e_flags, step_idle_factor, swing_damp_aimed,
    weapon_idle_amount_speed,
};
use crate::ps_flags::{ent_bit, movement_state_bits};

/// Hits report this yaw and pitch byte when they have no direction.
pub const HIT_FLINCH_NO_DIRECTION: u32 = 255;

/// The gap the feet are lifted off the ground by the landing dip.
pub const CAMERA_SWAY_LOWEST_DROP: f32 = 8.0;

/// How much of the camera's landing dip the gun takes.
pub const VIEWMODEL_TOUCHDOWN_FACTOR: f32 = 0.25;

/// The `e_flags` bits that put the player on a turret or in a vehicle.
pub const EFLAGS_TURRET_VEHICLE: u32 = ent_bit::TURRET_MASK | ent_bit::IN_VEHICLE;

/// bo2zm: from this water level up the player swims.
pub const SWIM_WATER_LEVEL: i32 = 4;

/// The widest the head bob swings.
const BOB_MAX: f32 = 8.0;
/// Head bob swing (side, up) per unit of speed for each way of moving. Standing
/// and ducked hold two: from the hip, then aiming down the sights.
struct HeadSwings {
    standing: [[f32; 2]; 2],
    ducked: [[f32; 2]; 2],
    prone: [f32; 2],
    sprinting: [f32; 2],
}

const HEAD_SWINGS: HeadSwings = HeadSwings {
    standing: [[0.007, 0.007], [0.007, 0.007]],
    ducked: [[0.0075, 0.0075], [0.0075, 0.0075]],
    prone: [0.02, 0.005],
    sprinting: [0.02, 0.014],
};
/// bo2zm: Black Ops' head bob when prone (bg_viewBobAmplitudeProne).
const AMP_PRONE_BO2: [f32; 2] = [0.08, 0.04];
/// bo2zm: Black Ops II's head bob through a dive, in the air and sliding
/// (bg_viewBobAmplitudeDtp), whatever the view height.
const AMP_DTP: [f32; 2] = [0.002, 0.002];
/// bo2zm: Black Ops II's head bob for a swimmer (bg_viewBobAmplitudeSwimming), the same
/// at any speed, even treading water.
const AMP_SWIMMING: [f32; 2] = [3.0, 2.0];
/// Lightweight leaves this share of the MW2 head bob.
const LIGHTWEIGHT_SCALE: f32 = 0.75;
const SIDE: usize = 0;
const UP: usize = 1;

/// A landing pushes the view down for `LAND_DEFLECT_MS`, lets it up over `LAND_RETURN_MS`,
/// and is over `LAND_END_MS` after the touchdown.
const LAND_DEFLECT_MS: f32 = 150.0;
const LAND_RETURN_MS: f32 = 300.0;
const LAND_END_MS: f32 = 450.0;
/// A fall under `LAND_DIP_FALL_IN` units dips nothing; from there, `LAND_DIP_SCALE` steps
/// of dip for every `LAND_DIP_SPAN_IN` units, from `LAND_DIP_SCALE` up, at most `LAND_DIP_MAX`.
const LAND_DIP_FALL_IN: f32 = 12.0;
const LAND_DIP_SPAN_IN: f32 = 26.0;
const LAND_DIP_SCALE: f32 = 4.0;
const LAND_DIP_MAX: i32 = 24;

/// MW2's flinch: 0.2 degrees for each point of damage, from `KICK_MIN` to `KICK_MAX`.
const KICK_SCALE: f32 = 0.2;
const KICK_MIN: f32 = 5.0;
const KICK_MAX: f32 = 90.0;
/// bo2zm: Black Ops' smallest hit flinch (bg_viewKickMin).
const KICK_MIN_BO2: f32 = 0.5;
/// bo2zm: Black Ops II's hit flinch per percent of health a frame's hits took (its
/// bg_viewKickScale).
const KICK_SCALE_BO2: f32 = 0.175;
/// bo2zm: the share of the hit flinch Toughness leaves (perk_damageKickReduction).
const KICK_TOUGHNESS_SHARE: f32 = 0.25;
/// bo2zm: the most a frame's hits count for, in percent of health.
const DAMAGE_COUNT_MAX_BO2: i32 = 127;
/// The hit direction's bytes: 0..=255 for a full turn.
const DIRECTION_BYTE_MAX: f32 = 255.0;
const FULL_TURN_DEGREES: f32 = 360.0;
const DEGREES_TO_TURNS: f32 = 0.002_777_777_8;
const DIRECTION_BYTE_TURN: f32 = 256.0;

/// The flinch's view kick: out for `DEFLECT` ms, then back over `RETURN` ms; the sights
/// halve it.
const FLINCH_DEFLECT_MS: f32 = 100.0;
const FLINCH_RETURN_MS: f32 = 400.0;
const FLINCH_ADS_HALF: f32 = 0.5;

/// What the head bob reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraHeightSwayFeed {
    /// The step counter: 0..=255 for half a turn of the bob.
    pub sway_phase: u8,
    pub ground_velocity: f32,
    pub eye_height_goal: i32,
    pub movement_state_bits: u32,
    pub gun_raise_share: f32,
    /// The player's two perk words (Lightweight shrinks the legacy head bob).
    pub perks: [u32; 2],
    /// bo2zm: Black Ops II camera rules (prone bob, no Lightweight shrink).
    pub bo2: bool,
    /// bo2zm: how deep in water the player stands (0 to 6).
    pub waterlevel: i32,
}

/// The head's bob, up (`vertical`) and across (`horizontal`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraHeightSway {
    pub vertical: f32,
    pub horizontal: f32,
}

/// What the camera angles read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraTiltSwayFeed {
    pub org: CameraHeightSwayFeed,
    pub ent_bits: u32,
    pub hud_crosshair_layer: i32,
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub time: i32,
    pub hurt_stamp_ms: i32,
    pub hurt_kick_pitch: f32,
    pub hurt_kick_roll: f32,
    pub scoped_aiming: bool,
    pub idle: GunRestDriftFeed,
    pub step_millis_elapsed: f32,
    pub steady_aim_factor: f32,
    pub gun_idle_clock: i32,
    pub view_prev_sway_ratio: f32,
}

/// The turn the camera gets, and the scope drift's clock to carry to the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraTiltSway {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub camera_rest_nod: f32,
    pub camera_rest_turn: f32,
    pub gun_idle_clock: i32,
    pub view_prev_sway_ratio: f32,
}

/// A hit's kick: the pitch and roll it gives the view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitFlinchResponse {
    pub hurt_kick_pitch: f32,
    pub hurt_kick_roll: f32,
}

/// The bob cycle of the head: where in the step the player is, in radians.
fn head_cycle(sway_phase: u8) -> f32 {
    let half_turns = (f32::from(sway_phase) / 255.0) * PI;
    half_turns + half_turns + TAU
}

/// How wide the head swings along `axis` (`SIDE` or `UP`).
fn head_amplitude(org: CameraHeightSwayFeed, axis: usize) -> f32 {
    let bo2 = org.bo2;
    if bo2 && org.waterlevel >= SWIM_WATER_LEVEL {
        return AMP_SWIMMING[axis].min(BOB_MAX);
    }
    let ads = org.gun_raise_share;
    let hip_to_aim = |[hip, aim]: [[f32; 2]; 2]| (1.0 - ads) * hip[axis] + ads * aim[axis];
    let scale = if bo2 && org.movement_state_bits & PMF_DIVE_ANY != 0 {
        AMP_DTP[axis]
    } else if org.eye_height_goal == EYE_LEVEL_LYING {
        if bo2 {
            AMP_PRONE_BO2[axis]
        } else {
            HEAD_SWINGS.prone[axis]
        }
    } else if org.eye_height_goal == EYE_LEVEL_CROUCHED {
        hip_to_aim(HEAD_SWINGS.ducked)
    } else if org.movement_state_bits & movement_state_bits::SPRINTING == 0 {
        hip_to_aim(HEAD_SWINGS.standing)
    } else {
        HEAD_SWINGS.sprinting[axis]
    };
    let mut amp = scale * org.ground_velocity;
    if !bo2 && perk::held(org.perks, perk::MOVEFASTER) {
        amp *= LIGHTWEIGHT_SCALE;
    }
    if amp > BOB_MAX {
        amp = BOB_MAX;
    }
    amp
}

/// The head's up and down bob at `cycle`.
fn bob_up(cycle: f32, org: CameraHeightSwayFeed) -> f32 {
    (libm::sinf(cycle * 4.0 + FRAC_PI_2) * 0.2 + libm::sinf(cycle + cycle))
        * 0.75
        * head_amplitude(org, UP)
}

/// The head's side to side bob at `cycle`.
fn bob_side(cycle: f32, org: CameraHeightSwayFeed) -> f32 {
    libm::sinf(cycle) * head_amplitude(org, SIDE)
}

/// The height of a fall, from the speed the player touched down at. None without gravity,
/// or when the two heights cannot be joined by a fall.
pub fn hard_landing_drop(grav: i32, prev_z: f32, now_z: f32, prev_vz: f32) -> Option<f32> {
    if grav == 0 {
        return None;
    }
    let drop_h = prev_z - now_z;
    let vz = prev_vz;
    let acc = -(grav as f32);
    let half_acc = acc * 0.5;
    let under_root = vz * vz - 4.0 * half_acc * drop_h;
    if under_root < 0.0 {
        return None;
    }
    let twice_half = half_acc * 2.0;
    if twice_half == 0.0 {
        return None;
    }
    let air_time = (-vz - libm::sqrtf(under_root)) / twice_half;
    let hit_speed = (air_time * acc + vz) * -1.0;
    Some((hit_speed * hit_speed) / ((grav as f32) * 2.0))
}

/// How many steps the view dips on touching down from `fall_height`. BO2 drops the
/// fraction where MW2 rounds.
pub fn crash_land_view_dip(fall_in: f32, bo2: bool) -> i32 {
    if !(fall_in > LAND_DIP_FALL_IN) {
        return 0;
    }
    let dip_raw = (fall_in - LAND_DIP_FALL_IN) / LAND_DIP_SPAN_IN * LAND_DIP_SCALE + LAND_DIP_SCALE;
    let dip_steps = if bo2 {
        dip_raw as i32
    } else {
        libm::roundf(dip_raw) as i32
    };
    if dip_steps > LAND_DIP_MAX {
        LAND_DIP_MAX
    } else if dip_steps < 0 {
        0
    } else {
        dip_steps
    }
}

/// How far into its dip the view is `delta_ms` after the landing: 0 to 1 and back to 0. None
/// before the touchdown and once it is over.
fn land_weight(since_ms: f32) -> Option<f32> {
    if !(since_ms > 0.0) {
        return None;
    }
    if since_ms < LAND_DEFLECT_MS {
        return Some(since_ms / LAND_DEFLECT_MS);
    }
    if since_ms >= LAND_END_MS {
        return None;
    }
    Some(1.0 - (since_ms - LAND_DEFLECT_MS) / LAND_RETURN_MS)
}

/// The view's drop `delta_ms` after a landing that dips `touchdown_dip`.
pub fn land_origin_z(since_ms: f32, dip_amount: f32) -> f32 {
    land_weight(since_ms).map_or(0.0, |w| w * dip_amount)
}

/// The gun's drop `delta_ms` after a landing that dips the view `touchdown_dip`.
pub fn viewweapon_land_origin_z(since_ms: i32, dip_amount: f32) -> f32 {
    let deflect_ms = LAND_DEFLECT_MS as i32;
    let end_ms = LAND_END_MS as i32;
    if since_ms < deflect_ms {
        return dip_amount * VIEWMODEL_TOUCHDOWN_FACTOR * (since_ms as f32) / LAND_DEFLECT_MS;
    }
    if since_ms < end_ms {
        return dip_amount * VIEWMODEL_TOUCHDOWN_FACTOR * ((end_ms - since_ms) as f32) / LAND_RETURN_MS;
    }
    0.0
}

/// The head's bob: none standing still, but a BO2 swimmer bobs treading water.
pub fn view_org_bob(head: CameraHeightSwayFeed) -> CameraHeightSway {
    let treading = head.bo2 && head.waterlevel >= SWIM_WATER_LEVEL;
    if !(head.ground_velocity > 0.0) && !treading {
        return CameraHeightSway::default();
    }
    let head_phase = head_cycle(head.sway_phase);
    CameraHeightSway {
        vertical: bob_up(head_phase, head),
        horizontal: bob_side(head_phase, head),
    }
}

fn with_speed(head: CameraHeightSwayFeed, speed_in: f32) -> CameraHeightSwayFeed {
    CameraHeightSwayFeed {
        ground_velocity: speed_in,
        ..head
    }
}

/// Adds the gun's own bob to the camera as the scope comes up, so the scope bobs with it.
fn add_overlay_bob(cam: &mut CameraTiltSway, frame: CameraTiltSwayFeed) {
    if frame.hud_crosshair_layer == 0 {
        return;
    }
    let phase = head_cycle(frame.org.sway_phase) + TAU + SWING_DELAY * PI;
    let scope_speed = SWING_AMP_BASE * frame.org.ground_velocity;
    let scaled_in = with_speed(frame.org, scope_speed);
    let mut p_bob = -bob_up(phase, scaled_in);
    let mut y_bob = -bob_side(phase, scaled_in);
    let mut r_bob = bob_side(
        phase - SWING_RISE_PHASE,
        with_speed(frame.org, SWING_AMP_ROLL * scope_speed),
    );
    // The scope only ever rolls one way: a roll the other way is none.
    if !(r_bob <= 0.0) {
        r_bob = 0.0;
    }
    let ads = frame.org.gun_raise_share;
    if ads != 0.0 {
        let kept = swing_damp_aimed(ads, frame.ads_bob_factor);
        p_bob *= kept;
        y_bob *= kept;
        r_bob *= kept;
    }
    cam.pitch += ads * p_bob;
    cam.yaw += ads * y_bob;
    cam.roll += ads * r_bob;
}

/// Takes the head's bob off the camera as the sights come up, by the weapon's own factor.
fn take_ads_bob(cam: &mut CameraTiltSway, frame: CameraTiltSwayFeed) {
    let ads = frame.org.gun_raise_share;
    if !(ads > 0.0) || frame.ent_bits & EFLAGS_TURRET_VEHICLE != 0 {
        return;
    }
    if !(frame.ads_view_bob_mult > 0.0) {
        return;
    }
    let cut = frame.ads_view_bob_mult * ads;
    let phase = head_cycle(frame.org.sway_phase);
    cam.pitch -= bob_up(phase, frame.org) * cut;
    cam.yaw -= bob_side(phase, frame.org) * cut;
}

/// MW2's flinch size for `damage_count`, at least `kick_min`.
fn kick_size(dmg_count: i32, min_kick: f32) -> f32 {
    let raw_kick = dmg_count as f32 * KICK_SCALE;
    if raw_kick < min_kick {
        min_kick
    } else if raw_kick > KICK_MAX {
        KICK_MAX
    } else {
        raw_kick
    }
}

/// How far a hit's travel direction (as bytes) points along the view's forward and right.
fn hit_along_view(look_ang: [f32; 3], yaw_b: u32, pitch_b: u32) -> (f32, f32) {
    let hit_pitch = (pitch_b as f32 / DIRECTION_BYTE_MAX) * FULL_TURN_DEGREES;
    let hit_yaw = (yaw_b as f32 / DIRECTION_BYTE_MAX) * FULL_TURN_DEGREES;
    let (hit_dir, _, _) = euler_basis([hit_pitch, hit_yaw, 0.0]);
    let (camera_ahead, view_rgt, _) = euler_basis(look_ang);
    let across = hit_dir[0] * view_rgt[0] + hit_dir[1] * view_rgt[1] + hit_dir[2] * view_rgt[2];
    let along = hit_dir[0] * camera_ahead[0] + hit_dir[1] * camera_ahead[1] + hit_dir[2] * camera_ahead[2];
    (along, across)
}

/// MW2's flinch from a hit's travel direction, with the smallest flinch given
/// (`KICK_MIN_BO2` for Black Ops II).
fn damage_kick_with_min(
    yaw_b: u32,
    pitch_b: u32,
    dmg_count: i32,
    view_ang: [f32; 3],
    min_kick: f32,
) -> HitFlinchResponse {
    let flinch = kick_size(dmg_count, min_kick);
    if yaw_b == HIT_FLINCH_NO_DIRECTION && pitch_b == HIT_FLINCH_NO_DIRECTION {
        return HitFlinchResponse {
            hurt_kick_pitch: -flinch,
            hurt_kick_roll: 0.0,
        };
    }
    let (along, across) = hit_along_view(view_ang, yaw_b, pitch_b);
    HitFlinchResponse {
        hurt_kick_pitch: flinch * along,
        hurt_kick_roll: -flinch * across,
    }
}

/// MW2's flinch from a hit's travel direction (yaw and pitch bytes; 255/255 for none).
pub fn damage_feedback_kick(
    yaw_b: u32,
    pitch_b: u32,
    dmg_count: i32,
    view_ang: [f32; 3],
) -> HitFlinchResponse {
    damage_kick_with_min(yaw_b, pitch_b, dmg_count, view_ang, KICK_MIN)
}

/// bo2zm: Black Ops II's hit flinch size: 0.175 a percent of health, a quarter of that
/// with Toughness, kept between 0.5 and 90 degrees.
fn kick_size_bo2(dmg_count: i32, toughness: bool) -> f32 {
    let mut flinch = dmg_count as f32 * KICK_SCALE_BO2;
    if toughness {
        flinch *= KICK_TOUGHNESS_SHARE;
    }
    if flinch < KICK_MIN_BO2 {
        KICK_MIN_BO2
    } else if flinch > KICK_MAX {
        KICK_MAX
    } else {
        flinch
    }
}

/// bo2zm: Black Ops II's hit flinch from the hit's travel direction (yaw and pitch bytes).
/// 255/255 = no direction: the view tips up. 0/0 keeps the last flinch's angles (None);
/// the flinch still restarts. The roll leans the other way from MW2's.
pub fn damage_feedback_kick_bo2(
    yaw_b: u32,
    pitch_b: u32,
    dmg_count: i32,
    view_ang: [f32; 3],
    toughness: bool,
) -> Option<HitFlinchResponse> {
    let flinch = kick_size_bo2(dmg_count, toughness);
    if yaw_b == HIT_FLINCH_NO_DIRECTION && pitch_b == HIT_FLINCH_NO_DIRECTION {
        return Some(HitFlinchResponse {
            hurt_kick_pitch: -flinch,
            hurt_kick_roll: 0.0,
        });
    }
    if yaw_b == 0 && pitch_b == 0 {
        return None;
    }
    let (along, across) = hit_along_view(view_ang, yaw_b, pitch_b);
    Some(HitFlinchResponse {
        hurt_kick_pitch: flinch * along,
        hurt_kick_roll: flinch * across,
    })
}

/// bo2zm: what Black Ops II sends for a frame's hits: the share of max health they took in
/// percent (at most 127), and the last hit's travel direction as yaw and pitch bytes
/// (255/255 with no direction). None when there is nothing to send.
pub fn damage_feedback_bo2(
    hp_lost: i32,
    hp_max: i32,
    hit_dir: Option<[f32; 3]>,
) -> Option<(i32, u32, u32)> {
    if hp_lost < 1 || hp_max < 1 {
        return None;
    }
    let pct = (hp_lost.saturating_mul(100) / hp_max).min(DAMAGE_COUNT_MAX_BO2);
    let Some(hit_dir) = hit_dir else {
        return Some((pct, HIT_FLINCH_NO_DIRECTION, HIT_FLINCH_NO_DIRECTION));
    };
    let dir_ang = dir_to_euler(hit_dir);
    let as_byte = |deg: f32| ((deg * DEGREES_TO_TURNS * DIRECTION_BYTE_TURN) as i32 as u32) & 0xff;
    Some((pct, as_byte(dir_ang[1]), as_byte(dir_ang[0])))
}

/// Adds the flinch of the last hit to the camera: out fast, back slowly.
fn add_flinch(cam: &mut CameraTiltSway, frame: CameraTiltSwayFeed) {
    if frame.hurt_stamp_ms == 0 {
        return;
    }
    let ads = frame.org.gun_raise_share;
    let mut stretch = 1.0 - ads * FLINCH_ADS_HALF;
    if ads != 0.0 && frame.hud_crosshair_layer != 0 {
        stretch = (ads * FLINCH_ADS_HALF + 1.0) * stretch;
    }
    let since = frame.time.wrapping_sub(frame.hurt_stamp_ms) as f32;
    let w = if since < FLINCH_DEFLECT_MS {
        lean_amount(since / FLINCH_DEFLECT_MS)
    } else {
        let back_left = 1.0 - (since - FLINCH_DEFLECT_MS) / FLINCH_RETURN_MS;
        if !(back_left > 0.0) {
            return;
        }
        1.0 - lean_amount(1.0 - back_left)
    };
    let kick_k = w * stretch;
    cam.pitch += kick_k * frame.hurt_kick_pitch;
    cam.roll += kick_k * frame.hurt_kick_roll;
}

/// The camera's turn from the last hit alone.
pub fn view_damage_angles(frame: CameraTiltSwayFeed) -> CameraTiltSway {
    let mut cam = CameraTiltSway::default();
    add_flinch(&mut cam, frame);
    cam
}

/// Runs the scope's drift on and adds it to the camera.
fn add_camera_idle(cam: &mut CameraTiltSway, frame: CameraTiltSwayFeed) {
    let ads = frame.org.gun_raise_share;
    if frame.hud_crosshair_layer == 0 || ads == 0.0 {
        return;
    }
    let view_ps = GunPosePlayerFeed {
        ent_bits: frame.ent_bits,
        gun_raise_share: ads,
        scoped_aiming: frame.scoped_aiming,
        hud_crosshair_layer: frame.hud_crosshair_layer,
        ..Default::default()
    };
    let (drift_amt, drift_spd) = weapon_idle_amount_speed(view_ps, frame.idle);
    // bo2mp: holding the breath slows the scope's drift in time as well as in size (Black Ops'
    // view idle: the time step is scaled by `holdBreathScale`).
    cam.gun_idle_clock = cam.gun_idle_clock.wrapping_add(idle_clock_step(
        frame.steady_aim_factor * drift_spd,
        frame.step_millis_elapsed,
    ));
    cam.view_prev_sway_ratio = step_idle_factor(
        cam.view_prev_sway_ratio,
        idle_e_flags(frame.ent_bits, frame.org.movement_state_bits),
        frame.idle,
        frame.step_millis_elapsed,
    );

    let strength = cam.view_prev_sway_ratio
        * drift_amt
        * ads
        * frame.steady_aim_factor
        * GUN_REST_WAVE_FACTOR;
    let clock = cam.gun_idle_clock as f32;
    let idle_yaw = libm::sinf(clock * REST_YAW_HZ) * strength;
    let idle_pitch = libm::sinf(clock * REST_PITCH_HZ) * strength;
    cam.camera_rest_turn = idle_yaw;
    cam.camera_rest_nod = idle_pitch;
    cam.yaw += idle_yaw;
    cam.pitch += idle_pitch;
}

/// The camera's turn for the frame: the flinch, the scope's drift and the bob.
pub fn view_angle_bob(frame: CameraTiltSwayFeed) -> CameraTiltSway {
    let mut cam = CameraTiltSway {
        gun_idle_clock: frame.gun_idle_clock,
        view_prev_sway_ratio: frame.view_prev_sway_ratio,
        ..Default::default()
    };
    add_flinch(&mut cam, frame);
    add_camera_idle(&mut cam, frame);
    add_overlay_bob(&mut cam, frame);
    take_ads_bob(&mut cam, frame);
    cam
}

/// bo2mp: the walk bob part of [`view_angle_bob`] alone (the scope's bob and the ADS bob,
/// pitch / yaw / roll), for a caller that puts in its own or moves it into the aim.
pub fn view_walk_bob_angles(frame: CameraTiltSwayFeed) -> [f32; 3] {
    let mut cam = CameraTiltSway::default();
    add_overlay_bob(&mut cam, frame);
    take_ads_bob(&mut cam, frame);
    [cam.pitch, cam.yaw, cam.roll]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak of the side to side sway.
    fn side_peak(org: CameraHeightSwayFeed) -> f32 {
        bob_side(FRAC_PI_2, org)
    }

    fn prone(bo2: bool, perks: [u32; 2]) -> CameraHeightSwayFeed {
        CameraHeightSwayFeed {
            ground_velocity: 20.0,
            eye_height_goal: 0x0b,
            perks,
            bo2,
            ..CameraHeightSwayFeed::default()
        }
    }

    #[test]
    fn a_hit_flinches_by_its_share_of_health() {
        // 0.175 a percent: 40 of 100 health = 7 degrees.
        assert!((kick_size_bo2(40, false) - 7.0).abs() < 1e-5);
        // Toughness leaves a quarter.
        assert!((kick_size_bo2(40, true) - 1.75).abs() < 1e-5);
        // At least 0.5, at most 90.
        assert_eq!(kick_size_bo2(1, false), 0.5);
        assert_eq!(kick_size_bo2(1, true), 0.5);
        assert_eq!(kick_size_bo2(127, false), 127.0 * 0.175);
        assert_eq!(kick_size_bo2(1000, false), 90.0);
    }

    #[test]
    fn a_hit_rolls_the_view_the_way_bo2_does() {
        // Looking along +x; a hit travelling +y (yaw byte 64, about 90.4).
        let hit = damage_feedback_kick_bo2(64, 0, 40, [0.0; 3], false).unwrap();
        assert!((hit.hurt_kick_roll + 7.0).abs() < 0.01, "{hit:?}");
        assert!(hit.hurt_kick_pitch.abs() < 0.1, "{hit:?}");
        // MW2 rolls the other way.
        let mw2 = damage_feedback_kick(64, 0, 35, [0.0; 3]);
        assert!(mw2.hurt_kick_roll > 6.9, "{mw2:?}");
        // A hit travelling the way he looks (yaw byte 0, pitch byte 1).
        let ahead = damage_feedback_kick_bo2(0, 1, 40, [0.0; 3], false).unwrap();
        assert!((ahead.hurt_kick_pitch - 7.0).abs() < 0.01, "{ahead:?}");
        // No direction: the view tips up by the whole flinch.
        let none = damage_feedback_kick_bo2(255, 255, 40, [0.0; 3], true).unwrap();
        assert_eq!((none.hurt_kick_pitch, none.hurt_kick_roll), (-1.75, 0.0));
        // 0/0 keeps the last flinch's angles.
        assert_eq!(damage_feedback_kick_bo2(0, 0, 40, [0.0; 3], false), None);
    }

    #[test]
    fn the_server_sends_the_share_and_the_travel_direction() {
        // 30 of 150 health = 20 percent, travelling +y = yaw 90 = byte 64.
        assert_eq!(
            damage_feedback_bo2(30, 150, Some([0.0, 1.0, 0.0])),
            Some((20, 64, 0))
        );
        // Travelling -x and 45 degrees down: yaw 180 = 128, pitch 45 = 32.
        assert_eq!(
            damage_feedback_bo2(10, 100, Some([-1.0, 0.0, -1.0])),
            Some((10, 128, 32))
        );
        // At most 127 percent; no direction = 255/255.
        assert_eq!(damage_feedback_bo2(500, 100, None), Some((127, 255, 255)));
        // A share under one percent still counts as a hit.
        assert_eq!(damage_feedback_bo2(1, 250, None), Some((0, 255, 255)));
        // No damage or no max health: nothing.
        assert_eq!(damage_feedback_bo2(0, 100, None), None);
        assert_eq!(damage_feedback_bo2(10, 0, None), None);
    }

    #[test]
    fn small_hits_flinch_less_in_bo2() {
        let hit = |min| {
            damage_kick_with_min(
                HIT_FLINCH_NO_DIRECTION,
                HIT_FLINCH_NO_DIRECTION,
                10,
                [0.0; 3],
                min,
            )
        };
        // 10 damage x 0.2 = 2: Black Ops keeps 2, MW2 raises it to 5.
        assert_eq!(hit(KICK_MIN_BO2).hurt_kick_pitch, -2.0);
        assert_eq!(hit(KICK_MIN).hurt_kick_pitch, -5.0);
    }

    #[test]
    fn the_landing_dip_drops_the_fraction_in_bo2() {
        // 22 units: 5.54 steps of dip.
        assert_eq!(crash_land_view_dip(22.0, true), 5);
        assert_eq!(crash_land_view_dip(22.0, false), 6);
        assert_eq!(crash_land_view_dip(500.0, true), 24);
        assert_eq!(crash_land_view_dip(12.0, true), 0);
    }

    #[test]
    fn a_fall_from_rest_lands_from_the_height_it_fell() {
        let height = hard_landing_drop(800, 100.0, 0.0, 0.0).unwrap();
        assert!((height - 100.0).abs() < 1e-3, "{height}");
        assert_eq!(hard_landing_drop(0, 100.0, 0.0, 0.0), None);
        // Rising faster than it can ever come down from: no such fall.
        assert_eq!(hard_landing_drop(800, 0.0, 100.0, 0.0), None);
    }

    #[test]
    fn the_view_dips_in_fast_and_comes_back_up() {
        assert_eq!(land_origin_z(0.0, 8.0), 0.0);
        assert_eq!(land_origin_z(75.0, 8.0), 4.0);
        assert_eq!(land_origin_z(150.0, 8.0), 8.0);
        assert_eq!(land_origin_z(300.0, 8.0), 4.0);
        assert_eq!(land_origin_z(450.0, 8.0), 0.0);
        assert_eq!(viewweapon_land_origin_z(75, 8.0), 1.0);
        assert_eq!(viewweapon_land_origin_z(150, 8.0), 2.0);
        assert_eq!(viewweapon_land_origin_z(450, 8.0), 0.0);
    }

    #[test]
    fn prone_head_bob_is_black_ops_size() {
        assert!((side_peak(prone(true, [0; 2])) - 0.08 * 20.0).abs() < 1e-4);
        assert!((side_peak(prone(false, [0; 2])) - 0.02 * 20.0).abs() < 1e-4);
    }

    #[test]
    fn a_swimmer_bobs_by_its_own_amount_at_any_speed() {
        let swim = |bo2, ground_velocity| CameraHeightSwayFeed {
            waterlevel: 4,
            ground_velocity,
            ..prone(bo2, [0; 2])
        };
        assert!((side_peak(swim(true, 20.0)) - 3.0).abs() < 1e-4);
        assert!((side_peak(swim(true, 200.0)) - 3.0).abs() < 1e-4);
        // MW2 has no swimming.
        assert!((side_peak(swim(false, 20.0)) - 0.02 * 20.0).abs() < 1e-4);
        // Treading water still bobs; standing still on land does not.
        let still = CameraHeightSwayFeed {
            sway_phase: 64,
            ..swim(true, 0.0)
        };
        assert!(view_org_bob(still).vertical != 0.0);
        let dry = CameraHeightSwayFeed {
            waterlevel: 3,
            ..still
        };
        assert_eq!(view_org_bob(dry), CameraHeightSway::default());
    }

    #[test]
    fn lightweight_does_not_shrink_the_bo2_head_bob() {
        // Move-faster is perk 40: bit 8 of the second word.
        let lw = [0, 1 << 8];
        assert_eq!(side_peak(prone(true, lw)), side_peak(prone(true, [0; 2])));
        assert!(side_peak(prone(false, lw)) < side_peak(prone(false, [0; 2])));
        // The first word's bit 8 is another perk.
        assert_eq!(
            side_peak(prone(false, [1 << 8, 0])),
            side_peak(prone(false, [0; 2]))
        );
    }

    #[test]
    fn a_dive_bobs_the_dive_amount_in_bo2_only() {
        let dive = |bo2| CameraHeightSwayFeed {
            movement_state_bits: PMF_DIVE_ANY,
            ..prone(bo2, [0; 2])
        };
        assert!((side_peak(dive(true)) - 0.002 * 20.0).abs() < 1e-5);
        assert!((side_peak(dive(false)) - 0.02 * 20.0).abs() < 1e-4);
    }

    #[test]
    fn the_flinch_goes_out_fast_and_is_gone_after_half_a_second() {
        let hit = |time| CameraTiltSwayFeed {
            time,
            hurt_stamp_ms: 1000,
            hurt_kick_pitch: 4.0,
            hurt_kick_roll: -2.0,
            ..Default::default()
        };
        let none = view_damage_angles(CameraTiltSwayFeed::default());
        assert_eq!(none, CameraTiltSway::default());
        let early = view_damage_angles(hit(1050));
        assert!((early.pitch - 0.75 * 4.0).abs() < 1e-6 && (early.roll + 0.75 * 2.0).abs() < 1e-6);
        let peak = view_damage_angles(hit(1100));
        assert!((peak.pitch - 4.0).abs() < 1e-6);
        assert_eq!(view_damage_angles(hit(1500)), CameraTiltSway::default());
    }

    #[test]
    fn the_scope_drifts_only_while_the_sights_are_up_and_slows_with_the_breath() {
        let scoped = |steady_aim_factor| CameraTiltSwayFeed {
            org: CameraHeightSwayFeed {
                gun_raise_share: 1.0,
                ..CameraHeightSwayFeed::default()
            },
            hud_crosshair_layer: 1,
            scoped_aiming: true,
            idle: GunRestDriftFeed {
                ads_idle_amount: 10.0,
                ads_idle_speed: 1.0,
                ..GunRestDriftFeed::default()
            },
            step_millis_elapsed: 0.05,
            steady_aim_factor,
            view_prev_sway_ratio: 1.0,
            ..Default::default()
        };
        let free = view_angle_bob(scoped(1.0));
        assert_eq!(free.gun_idle_clock, 50);
        assert!(
            free.camera_rest_turn > 0.0 && free.camera_rest_nod > 0.0,
            "{free:?}"
        );
        assert_eq!(view_angle_bob(scoped(0.5)).gun_idle_clock, 25);

        let hip = CameraTiltSwayFeed {
            hud_crosshair_layer: 0,
            ..scoped(1.0)
        };
        let out = view_angle_bob(hip);
        assert_eq!(
            (out.camera_rest_turn, out.camera_rest_nod, out.gun_idle_clock),
            (0.0, 0.0, 0)
        );
    }

    #[test]
    fn the_ads_bob_is_taken_off_the_camera_unless_on_a_turret() {
        let moving = CameraTiltSwayFeed {
            org: CameraHeightSwayFeed {
                sway_phase: 40,
                ground_velocity: 200.0,
                gun_raise_share: 1.0,
                ..CameraHeightSwayFeed::default()
            },
            ads_view_bob_mult: 0.5,
            ..Default::default()
        };
        let [pitch, yaw, _] = view_walk_bob_angles(moving);
        assert!(pitch != 0.0 && yaw != 0.0);
        let turret = CameraTiltSwayFeed {
            ent_bits: EFLAGS_TURRET_VEHICLE,
            ..moving
        };
        assert_eq!(view_walk_bob_angles(turret), [0.0; 3]);
        let no_mult = CameraTiltSwayFeed {
            ads_view_bob_mult: 0.0,
            ..moving
        };
        assert_eq!(view_walk_bob_angles(no_mult), [0.0; 3]);
    }
}
