//! Where the viewmodel gun sits and which way it points: the stance and movement offsets,
//! sway, kick, bob, idle drift, a hit's flinch and a lean, summed into one move and one turn.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use game_math::{euler_basis, lean_amount};

use super::kick::{ViewmodelKickReturn, ViewmodelKickPoseState, gun_kick_offset};
use crate::ps_flags::{ent_bit, movement_state_bits};
use crate::state;
use super::sway::{DriftReturnState, drift_offset};

/// `eye_height_goal` of a prone player and of a crouched one.
pub(crate) const EYE_LEVEL_LYING: i32 = 0x0b;
pub(crate) const EYE_LEVEL_CROUCHED: i32 = 0x28;

/// TODO(fact): BO2's bg_weaponBobAmplitudeBase default was not found; MW2's value kept.
pub(crate) const SWING_AMP_BASE: f32 = 0.16;
/// The roll bob swings this many times as wide as the side bob.
pub(crate) const SWING_AMP_ROLL: f32 = 1.5;
/// The bob cycle runs this share of a half turn ahead of the step counter.
pub(crate) const SWING_DELAY: f32 = 0.25;
/// The roll runs this far (radians) behind the side bob.
pub(crate) const SWING_RISE_PHASE: f32 = 0.471_238_94;
/// The widest a bob swings.
const SWING_CAP: f32 = 8.0;
/// Gun bob swing (side, up) for each way of moving, per unit of bob speed.
struct GunSwings {
    standing: [f32; 2],
    ducked: [f32; 2],
    prone: [f32; 2],
    sprinting: [f32; 2],
}

const GUN_SWINGS: GunSwings = GunSwings {
    standing: [0.055, 0.025],
    ducked: [0.045, 0.025],
    prone: [0.02, 0.005],
    sprinting: [0.02, 0.014],
};
/// bo2zm: bg_weaponBobAmplitudeDtp, gun bob during a dive to prone
/// (0.002 until read from real BO2).
const BOB_AMP_DTP: [f32; 2] = [0.002, 0.002];
/// bo2mp: BO2's gun bob for a swimmer (bg_weaponBobAmplitudeSwimming): not speed scaled,
/// no ADS fade.
const BOB_AMP_SWIMMING: [f32; 2] = [4.0, 4.0];

/// Idle drift: the clock runs in milliseconds, three sines at their own pace move the
/// gun, and a gun with no idle setting sways by [`IDLE_AMOUNT_DEFAULT`] at speed 1.
pub(crate) const WEAPON_IDLE_TIME_MS_SCALE: f32 = 1000.0;
pub(crate) const WEAPON_IDLE_FACTOR_LERP: f32 = 0.5;
pub(crate) const REST_YAW_HZ: f32 = 0.0007;
pub(crate) const REST_PITCH_HZ: f32 = 0.001;
pub(crate) const GUN_REST_WAVE_FACTOR: f32 = 0.01;
const IDLE_ROLL_FREQ: f32 = 0.0005;
const IDLE_AMOUNT_DEFAULT: f32 = 80.0;

/// A hit flinches the gun: it turns away for `DEFLECT` ms, then eases back over `RETURN`
/// ms, both stretched toward the full time as the sights come up.
const GUN_DAMAGE_DEFLECT_MS: f32 = 100.0;
const GUN_DAMAGE_RETURN_MS: f32 = 400.0;
const GUN_DAMAGE_ADS_HALF: f32 = 0.5;
/// How much of the flinch a full-screen scope takes away.
const GUN_DAMAGE_OVERLAY_MIX: f32 = 0.75;

/// bo2mp: BO2 pushes the gun this far along its right vector at full lean (scaled by 1 minus posFrac).
const VIEWMODEL_LEAN_DIST: f32 = 1.6;

/// bo2mp: BO2 takes this times the lean off the gun's roll.
const VIEWMODEL_LEAN_ROLL: f32 = 2.0;

/// The off hand's gun of a dual wield sits this many times as far out as the right.
const DUAL_WIELD_LEFT_SCALE: f32 = 2.0;

/// bo2zm: Black Ops II's dive bit, set in the air and sliding
/// (movement_t6::bridge::PMF_DIVE).
pub(crate) const PMF_DIVE_ANY: u32 = 0x0020_0000;

/// bo2zm: Black Ops II's idle sway does not count a dive as prone (the dive
/// drops the view to prone from the frame after take-off).
pub(crate) fn idle_e_flags(e_flags: u32, movement_state_bits: u32) -> u32 {
    if movement_state_bits & PMF_DIVE_ANY != 0 {
        e_flags & !ent_bit::LYING_FLAT
    } else {
        e_flags
    }
}

/// The turn the walk gives the gun, in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunStrideSwayState {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

/// Where the gun rests when crouched or prone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunPostureRestShiftFeed {
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    /// How far the gun tips (pitch) at the sights.
    pub aimed_pitch: f32,
    /// How long raising or lowering the night vision takes, ms.
    pub goggles_equip_duration: i32,
}

/// A weapon's offsets (move, turn) for walking, strafing, crouching and going prone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunMotionShiftFeed {
    pub stand_move: [f32; 3],
    pub stand_rot: [f32; 3],
    pub strafe_move: [f32; 3],
    pub strafe_rot: [f32; 3],
    pub ducked_move: [f32; 3],
    pub ducked_rot: [f32; 3],
    pub prone_move: [f32; 3],
    pub prone_rot: [f32; 3],
    /// How fast the offset eases toward its target.
    pub pos_move_rate: f32,
    pub pos_prone_move_rate: f32,
    /// The slowest ground speed that counts as moving, by stance.
    pub stand_move_min_speed: f32,
    pub ducked_move_min_speed: f32,
    pub prone_move_min_speed: f32,
    pub pos_rot_rate: f32,
    pub pos_prone_rot_rate: f32,
    /// bo2zm: Black Ops II's gun offset and turn while a climb keeps it in
    /// hand (`vMantleOfs`, `vMantleRot`).
    pub mantle_ofs: [f32; 3],
    pub mantle_rot: [f32; 3],
}

/// How the player is moving, for the movement offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunMotionPhysics {
    pub ground_velocity: f32,
    pub speed: f32,
    pub velocity: [f32; 3],
    pub look_angles: [f32; 3],
    pub gun_phase: i32,
    pub gun_phase_alt: i32,
    pub movement_state_bits: u32,
    pub step_millis_elapsed: f32,
    /// bo2zm: a Black Ops II climb is keeping the gun in hand.
    pub bo2_mantle: bool,
}

/// How much of the bob the sights keep.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunStrideSwayFeed {
    pub ads_bob_factor: f32,
}

/// A weapon's idle drift settings, at the hip and at the sights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunRestDriftFeed {
    pub ads_idle_amount: f32,
    pub hip_idle_amount: f32,
    pub ads_idle_speed: f32,
    pub hip_idle_speed: f32,
    /// How much of the drift is left when crouched, and when prone.
    pub idle_crouch_factor: f32,
    pub idle_prone_factor: f32,
}

/// What the bob waveform needs to know.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunStrideWaveFeed {
    /// The step counter: 0..=255 for half a turn of the bob.
    pub sway_phase: u8,
    pub ground_velocity: f32,
    pub eye_height_goal: i32,
    pub movement_state_bits: u32,
    pub gun_raise_share: f32,
    /// bo2zm: the Black Ops II gun's own bob. None = the MW2 gun bob.
    pub bo2_gun: Option<Bo2GunBob>,
    /// bo2mp: the player's water level (BO2 swims from SWIM_WATER_LEVEL up).
    pub waterlevel: i32,
}

/// bo2zm: Black Ops II per-gun bob from the weapon file: vSprintBob while
/// sprinting, vDtpBob during a dive to prone (side, up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bo2GunBob {
    pub sprint: [f32; 2],
    pub dtp: [f32; 2],
}

/// How far into a stance change the offsets fade out, as shares of the change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostureShiftBlendTuning {
    pub blend_begin_share: f32,
    pub blend_finish_share: f32,
}

impl Default for PostureShiftBlendTuning {
    fn default() -> Self {
        Self {
            blend_begin_share: 0.0,
            blend_finish_share: 1.0,
        }
    }
}

/// Everything the placement remembers from frame to frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GunPoseState {
    pub motion_tilt: [f32; 3],
    pub motion_shift: [f32; 3],
    pub unfaded_angles: [f32; 3],
    pub sway_springs: DriftReturnState,
    pub gun_recoil: ViewmodelKickPoseState,
    pub bob: GunStrideSwayState,
    /// The idle clock, ms, and the share of the drift left by the stance.
    pub gun_idle_clock: i32,
    pub previous_idle_ratio: f32,
    pub rest_drift_tilt: [f32; 3],
    /// The last hit's flinch: when it was taken, its two kicks, and the turn it gives.
    pub hit_flinch_stamp: i32,
    pub hurt_stamp_ms: i32,
    pub hurt_kick_pitch: f32,
    pub hurt_kick_roll: f32,
    pub hit_flinch_tilt: [f32; 3],
}

/// The player-state fields the placement reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunPosePlayerFeed {
    pub ent_bits: u32,
    pub gun_raise_share: f32,
    pub gun_timer_ms: i32,
    pub scoped_aiming: bool,
    pub hud_crosshair_layer: i32,
    pub swap_in_progress: bool,
    pub peek_amount: f32,
}

/// What the placement moves the gun by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunPoseShare {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

pub const POSE_BUILD_STAGE_TOTAL: usize = 6;

/// The parts of [`weapon_placement_assemble`], in the order it does them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GunPoseBuildStage {
    PostureMotionTiltBase,
    Sway,
    GunRecoil,
    Bob,
    CommitTilt,
    CommitPosition,
}

fn add_scaled(out: &mut [f32; 3], v: [f32; 3], factor_k: f32) {
    for (o, x) in out.iter_mut().zip(v) {
        *o += x * factor_k;
    }
}

fn blend_triple(start: [f32; 3], goal: [f32; 3], amount: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| start[i] + (goal[i] - start[i]) * amount)
}

/// How much of the stance offsets is left: all of it outside a stance change, none once
/// the change is `blend_finish_share` through.
fn stance_fade(
    in_change: bool,
    elapsed: i32,
    span: i32,
    fade_cfg: PostureShiftBlendTuning,
) -> f32 {
    if !in_change || span <= 0 {
        return 1.0;
    }
    let progress = elapsed as f32 / span as f32;
    let (from_frac, to_frac) = (fade_cfg.blend_begin_share, fade_cfg.blend_finish_share);
    if progress <= from_frac {
        1.0
    } else if progress >= to_frac {
        0.0
    } else if to_frac - from_frac <= 0.0 {
        0.0
    } else {
        1.0 - (progress - from_frac) / (to_frac - from_frac)
    }
}

/// Adds the crouched or prone resting offset to `origin`, and the sights' tip to `pitch`.
fn add_stance_offset(
    view_ps: GunPosePlayerFeed,
    rest_ofs: GunPostureRestShiftFeed,
    fade_cfg: PostureShiftBlendTuning,
    tip: &mut f32,
    rest_pos: &mut [f32; 3],
) {
    let keep = stance_fade(
        view_ps.swap_in_progress,
        view_ps.gun_timer_ms,
        rest_ofs.goggles_equip_duration,
        fade_cfg,
    );
    if view_ps.ent_bits & ent_bit::CROUCH_BIT != 0 {
        add_scaled(rest_pos, rest_ofs.ducked_ofs, keep);
    } else if view_ps.ent_bits & ent_bit::LYING_FLAT != 0 {
        add_scaled(rest_pos, rest_ofs.prone_ofs, keep);
    }
    if view_ps.scoped_aiming {
        *tip += rest_ofs.aimed_pitch * view_ps.gun_raise_share;
    }
}

/// How much of the bob the sights leave: all of it at the hip.
pub(crate) fn swing_damp_aimed(ads_frac: f32, keep_share: f32) -> f32 {
    if ads_frac <= 0.0 {
        return 1.0;
    }
    1.0 - ads_frac * (1.0 - keep_share)
}

/// Takes the walk's bob as the gun's, less what the sights take.
fn take_bob(
    place: &mut GunPoseState,
    view_ps: GunPosePlayerFeed,
    bob_cfg: GunStrideSwayFeed,
    walk_wave: GunStrideSwayState,
) {
    place.bob = walk_wave;
    if view_ps.gun_raise_share > 0.0 {
        let factor_k = swing_damp_aimed(view_ps.gun_raise_share, bob_cfg.ads_bob_factor);
        place.bob.pitch *= factor_k;
        place.bob.yaw *= factor_k;
        place.bob.roll *= factor_k;
    }
}

/// How wide the bob swings along `axis` (0 side, 1 up) at bob `speed`.
fn bob_amplitude(
    height_target: i32,
    move_flags: u32,
    ads_frac: f32,
    bob_speed: f32,
    dim: usize,
    gun_bob: Option<Bo2GunBob>,
) -> f32 {
    let hip_share = 1.0 - ads_frac;
    let dive_bob = gun_bob.filter(|_| move_flags & PMF_DIVE_ANY != 0);
    let factor_k = if let Some(per_gun) = dive_bob {
        BOB_AMP_DTP[dim] * per_gun.dtp[dim]
    } else if height_target == EYE_LEVEL_LYING {
        GUN_SWINGS.prone[dim]
    } else if height_target == EYE_LEVEL_CROUCHED {
        hip_share * GUN_SWINGS.ducked[dim]
    } else if move_flags & self::movement_state_bits::SPRINTING == 0 {
        hip_share * GUN_SWINGS.standing[dim]
    } else {
        GUN_SWINGS.sprinting[dim] * gun_bob.map_or(1.0, |per_gun| per_gun.sprint[dim])
    };
    let swing = bob_speed * factor_k;
    if swing > SWING_CAP {
        SWING_CAP
    } else {
        swing
    }
}

/// The up bob's shape at `cycle`, before its amplitude.
fn bob_up_shape(phase: f32) -> f32 {
    (libm::sinf(phase * 4.0 + FRAC_PI_2) * 0.2 + libm::sinf(phase + phase)) * 0.75
}

/// The gun's bob turn for a step counter and speed.
pub fn calculate_weapon_movement_bob_waveform(wf: GunStrideWaveFeed) -> GunStrideSwayState {
    let half_cycle = (f32::from(wf.sway_phase) / 255.0) * PI;
    let phase = TAU + half_cycle + half_cycle + TAU + SWING_DELAY * PI;
    let walk_speed = SWING_AMP_BASE * wf.ground_velocity;
    let swing_for = |walk_speed: f32, axis: usize| {
        bob_amplitude(
            wf.eye_height_goal,
            wf.movement_state_bits,
            wf.gun_raise_share,
            walk_speed,
            axis,
            wf.bo2_gun,
        )
    };

    let (bob_pitch, bob_yaw, mut bob_roll);
    // bo2mp: a BO2 swimmer's gun bobs by the raw swimming amount, whatever the speed.
    if wf.bo2_gun.is_some() && wf.waterlevel >= crate::view_bob::SWIM_WATER_LEVEL {
        let [side_amp, up_amp] = BOB_AMP_SWIMMING.map(|a| a.min(SWING_CAP));
        bob_pitch = -bob_up_shape(phase) * up_amp;
        bob_yaw = -libm::sinf(phase) * side_amp;
        bob_roll = libm::sinf(phase - SWING_RISE_PHASE) * side_amp;
    } else {
        bob_pitch = -(bob_up_shape(phase) * swing_for(walk_speed, 1));
        bob_yaw = -(libm::sinf(phase) * swing_for(walk_speed, 0));
        bob_roll = libm::sinf(phase - SWING_RISE_PHASE)
            * swing_for(SWING_AMP_ROLL * walk_speed, 0);
    }
    // The gun only ever rolls one way: a roll the other way is none.
    if !(bob_roll <= 0.0) {
        bob_roll = 0.0;
    }
    GunStrideSwayState {
        pitch: bob_pitch,
        yaw: bob_yaw,
        roll: bob_roll,
    }
}

/// Adds the bob to `angles`; a full-screen scope takes away what the sights have taken.
fn add_bob_to_angles(turn: &mut [f32; 3], walk: GunStrideSwayState, ads_frac: f32, scope_flag: i32) {
    let factor_k = if scope_flag != 0 { 1.0 - ads_frac } else { 1.0 };
    turn[0] += walk.pitch * factor_k;
    turn[1] += walk.yaw * factor_k;
    turn[2] += walk.roll * factor_k;
}

/// Turns `origin` by the bob: onto the bob's forward, left and up.
pub fn weapon_bob_rotate_origin(pos: &mut [f32; 3], walk: GunStrideSwayState) {
    let (fwd, rgt, upv) = euler_basis([walk.pitch, walk.yaw, walk.roll]);
    let basis = [fwd, rgt.map(|x| -x), upv];
    let before = *pos;
    for (row, slot) in pos.iter_mut().enumerate() {
        *slot = before[0] * basis[0][row] + before[1] * basis[1][row] + before[2] * basis[2][row];
    }
}

/// How much of the movement offsets the sights leave: none from half way up.
fn movement_scale(ads_frac: f32) -> f32 {
    let remaining = 1.0 - ads_frac - ads_frac;
    if remaining > 0.0 { remaining } else { 0.0 }
}

/// bo2mp: BO2's gun lean, the lean fraction curve, or None when the player is not leaning or is
/// fully down the sights. TODO(fact): the roll may take the raw lean fraction, not the curve.
fn viewmodel_lean(view_ps: GunPosePlayerFeed) -> Option<f32> {
    if view_ps.peek_amount == 0.0 || view_ps.gun_raise_share == 1.0 {
        return None;
    }
    Some(lean_amount(view_ps.peek_amount))
}

/// The gun's angles up to BO2's lean step (movement, sway, lean roll): the lean origin follows
/// their right vector. TODO(fact): which step's angles BO2 takes the right vector of.
fn angles_to_lean(place: &GunPoseState, view_ps: GunPosePlayerFeed) -> [f32; 3] {
    let factor_k = movement_scale(view_ps.gun_raise_share);
    let sway_now = drift_offset(place.sway_springs);
    let mut turn = [0.0; 3];
    if factor_k > 0.0 {
        add_scaled(&mut turn, place.motion_tilt, factor_k);
    }
    turn[0] += sway_now.angles[0];
    turn[1] += sway_now.angles[1];
    if let Some(lean_k) = viewmodel_lean(view_ps) {
        turn[2] -= VIEWMODEL_LEAN_ROLL * lean_k;
    }
    turn
}

/// Where the gun is moved to: the movement offset, the sway's slide, the lean's push, all
/// turned by the bob.
fn weapon_placement_apply_origin(
    place: &GunPoseState,
    view_ps: GunPosePlayerFeed,
) -> [f32; 3] {
    let mut pos = [0.0_f32; 3];
    let factor_k = movement_scale(view_ps.gun_raise_share);
    if factor_k > 0.0 {
        add_scaled(&mut pos, place.motion_shift, factor_k);
    }
    let sway_now = drift_offset(place.sway_springs);
    pos[1] -= sway_now.at_pos[1];
    pos[2] += sway_now.at_pos[2];
    if let Some(lean_k) = viewmodel_lean(view_ps) {
        let (_, rgt, _) = euler_basis(angles_to_lean(place, view_ps));
        add_scaled(
            &mut pos,
            rgt,
            (1.0 - view_ps.gun_raise_share) * lean_k * VIEWMODEL_LEAN_DIST,
        );
    }
    weapon_bob_rotate_origin(&mut pos, place.bob);
    pos
}

/// The gun's idle drift (amount, speed) for the sights' progress.
pub(crate) fn weapon_idle_amount_speed(
    view_ps: GunPosePlayerFeed,
    idle_cfg: GunRestDriftFeed,
) -> (f32, f32) {
    if view_ps.scoped_aiming {
        let drift_amount = (idle_cfg.ads_idle_amount - idle_cfg.hip_idle_amount)
            * view_ps.gun_raise_share
            + idle_cfg.hip_idle_amount;
        let drift_speed = (idle_cfg.ads_idle_speed - idle_cfg.hip_idle_speed)
            * view_ps.gun_raise_share
            + idle_cfg.hip_idle_speed;
        return (drift_amount, drift_speed);
    }
    if idle_cfg.hip_idle_amount != 0.0 {
        return (idle_cfg.hip_idle_amount, idle_cfg.hip_idle_speed);
    }
    (IDLE_AMOUNT_DEFAULT, 1.0)
}

/// How far the idle clock moves in a frame, ms.
pub(crate) fn idle_clock_step(idle_speed: f32, frame_secs: f32) -> i32 {
    libm::roundf(idle_speed * WEAPON_IDLE_TIME_MS_SCALE * frame_secs) as i32
}

/// `last` moved toward the share of the drift this stance keeps, by half a frame's time
/// at most.
pub(crate) fn step_idle_factor(
    prev: f32,
    ent_flags: u32,
    idle_cfg: GunRestDriftFeed,
    frame_secs: f32,
) -> f32 {
    let goal = if ent_flags & ent_bit::LYING_FLAT != 0 {
        idle_cfg.idle_prone_factor
    } else if ent_flags & ent_bit::CROUCH_BIT != 0 {
        idle_cfg.idle_crouch_factor
    } else {
        1.0
    };
    if prev == goal {
        return prev;
    }
    let max_move = frame_secs * WEAPON_IDLE_FACTOR_LERP;
    if goal <= prev {
        let moved = prev - max_move;
        if moved < goal { goal } else { moved }
    } else {
        let moved = prev + max_move;
        if moved > goal { goal } else { moved }
    }
}

/// Runs the idle clock on and adds the drift to `angles`.
fn add_idle_sway(
    place: &mut GunPoseState,
    view_ps: GunPosePlayerFeed,
    idle_cfg: GunRestDriftFeed,
    frame_secs: f32,
    turn: &mut [f32; 3],
) {
    let (drift_amount, drift_speed) = weapon_idle_amount_speed(view_ps, idle_cfg);
    place.gun_idle_clock = place
        .gun_idle_clock
        .wrapping_add(idle_clock_step(drift_speed, frame_secs));
    place.previous_idle_ratio = step_idle_factor(
        place.previous_idle_ratio,
        view_ps.ent_bits,
        idle_cfg,
        frame_secs,
    );

    let mut strength = place.previous_idle_ratio * drift_amount;
    if view_ps.hud_crosshair_layer != 0 {
        strength *= 1.0 - view_ps.gun_raise_share;
    }
    let clock = place.gun_idle_clock as f32;
    let sin_gain = strength * GUN_REST_WAVE_FACTOR;
    let idle_turn = [
        libm::sinf(clock * REST_PITCH_HZ) * sin_gain,
        libm::sinf(clock * REST_YAW_HZ) * sin_gain,
        libm::sinf(clock * IDLE_ROLL_FREQ) * sin_gain,
    ];
    place.rest_drift_tilt = idle_turn;
    for (slot, add) in turn.iter_mut().zip(idle_turn) {
        *slot += add;
    }
}

/// The turn a hit's flinch gives the gun `time - hurt_stamp_ms` ms after the hit.
pub fn weapon_damage_kick_angles(
    now: i32,
    hit_time: i32,
    dmg_pitch: f32,
    dmg_roll: f32,
    ads_frac: f32,
    scope_flag: i32,
) -> [f32; 3] {
    if hit_time == 0 {
        return [0.0; 3];
    }
    let mut stretch = GUN_DAMAGE_ADS_HALF + ads_frac * GUN_DAMAGE_ADS_HALF;
    let out_ms = GUN_DAMAGE_DEFLECT_MS * stretch;
    let back_ms = GUN_DAMAGE_RETURN_MS * stretch;
    if ads_frac != 0.0 && scope_flag != 0 {
        stretch *= 1.0 - ads_frac * GUN_DAMAGE_OVERLAY_MIX;
    }
    let since = now.wrapping_sub(hit_time) as f32;
    let factor_k = if out_ms <= since {
        let back_left = 1.0 - (since - out_ms) / back_ms;
        if back_left <= 0.0 {
            return [0.0; 3];
        }
        (1.0 - lean_amount(1.0 - back_left)) * stretch
    } else {
        lean_amount(since / out_ms) * stretch
    };
    [
        factor_k * dmg_pitch * GUN_DAMAGE_ADS_HALF,
        -(dmg_roll * factor_k),
        factor_k * dmg_roll * GUN_DAMAGE_ADS_HALF,
    ]
}

/// The gun's turn: movement, sway, lean, the unfaded angles, bob and kick.
fn weapon_placement_apply_angles(
    place: &GunPoseState,
    view_ps: GunPosePlayerFeed,
) -> [f32; 3] {
    let mut turn = angles_to_lean(place, view_ps);
    for (slot, extra) in turn.iter_mut().zip(place.unfaded_angles) {
        *slot += extra;
    }
    add_bob_to_angles(
        &mut turn,
        place.bob,
        view_ps.gun_raise_share,
        view_ps.hud_crosshair_layer,
    );
    turn[0] += place.gun_recoil.nod_offset;
    turn[1] += place.gun_recoil.yaw_shift;
    turn
}

/// Adds what moving does to the gun: the stance's move and turn, scaled by how far over the
/// slowest counted speed the player is, and the strafe's by how much of the speed is sideways.
fn add_movement_targets(
    view_ps: GunPosePlayerFeed,
    motion: GunMotionPhysics,
    move_ofs: GunMotionShiftFeed,
    floor_speed: f32,
    pos: &mut [f32; 3],
    turn_out: &mut [f32; 3],
) {
    let (shift_v, turn_v) = if view_ps.ent_bits & ent_bit::LYING_FLAT != 0 {
        (move_ofs.prone_move, move_ofs.prone_rot)
    } else if view_ps.ent_bits & ent_bit::CROUCH_BIT != 0 {
        (move_ofs.ducked_move, move_ofs.ducked_rot)
    } else {
        (move_ofs.stand_move, move_ofs.stand_rot)
    };
    let mut over_frac = (motion.ground_velocity - floor_speed) / (motion.speed - floor_speed);
    if over_frac >= 1.0 {
        over_frac = 1.0;
    }
    let hip_share = 1.0 - view_ps.gun_raise_share;
    add_scaled(pos, shift_v, over_frac);
    add_scaled(turn_out, turn_v, over_frac * hip_share);

    let (fwd, _, _) = euler_basis(motion.look_angles);
    let side_part =
        (fwd[0] * motion.velocity[0] + motion.velocity[1] * fwd[1]) * over_frac / motion.ground_velocity;
    add_scaled(pos, move_ofs.strafe_move, side_part);
    add_scaled(turn_out, move_ofs.strafe_rot, side_part * hip_share);
}

/// A weapon state that is a reload.
fn is_reload_state(ws: i32) -> bool {
    matches!(
        ws,
        state::RELOADING_RIGHT
            | state::RELOADING_LEFT
            | state::RELOADING
            | state::RELOAD_QUICK
            | state::RELOAD_QUICK_EMPTY
    )
}

/// Works out where the stance and the movement want the gun, and eases toward it.
fn base_stance_movement_angles(
    place: &mut GunPoseState,
    view_ps: GunPosePlayerFeed,
    rest_ofs: GunPostureRestShiftFeed,
    fade_cfg: PostureShiftBlendTuning,
    move_ofs: GunMotionShiftFeed,
    motion: GunMotionPhysics,
) {
    let crouched = view_ps.ent_bits & ent_bit::CROUCH_BIT != 0;
    let lying = view_ps.ent_bits & ent_bit::LYING_FLAT != 0;
    let floor_speed = if lying {
        move_ofs.prone_move_min_speed
    } else if crouched {
        move_ofs.ducked_move_min_speed
    } else {
        move_ofs.stand_move_min_speed
    };
    let in_motion = motion.ground_velocity > floor_speed && motion.speed > floor_speed;
    let mid_reload =
        is_reload_state(motion.gun_phase) || is_reload_state(motion.gun_phase_alt);
    let nv_busy = matches!(
        motion.gun_phase,
        crate::state::NIGHTVISION_WEAR | crate::state::NIGHTVISION_REMOVE
    );
    let laddering = motion.movement_state_bits & movement_state_bits::LADDER != 0;

    let mut pos_goal = [0.0_f32; 3];
    let mut turn_goal = [0.0_f32; 3];

    if (crouched || lying) && (!in_motion || !nv_busy || !lying) {
        let mut tip = 0.0_f32;
        add_stance_offset(view_ps, rest_ofs, fade_cfg, &mut tip, &mut pos_goal);
        turn_goal[0] += tip;
    }
    if in_motion && !laddering && !mid_reload && !nv_busy {
        add_movement_targets(
            view_ps,
            motion,
            move_ofs,
            floor_speed,
            &mut pos_goal,
            &mut turn_goal,
        );
    }
    if motion.bo2_mantle {
        for i in 0..3 {
            pos_goal[i] += move_ofs.mantle_ofs[i];
            turn_goal[i] += move_ofs.mantle_rot[i];
        }
    }

    let (slide_rate, spin_rate) = if lying {
        (move_ofs.pos_prone_move_rate, move_ofs.pos_prone_rot_rate)
    } else {
        (move_ofs.pos_move_rate, move_ofs.pos_rot_rate)
    };
    let ease_share = |rate: f32| {
        let step_k = rate * motion.step_millis_elapsed;
        if step_k > 1.0 { 1.0 } else { step_k }
    };
    place.motion_shift = blend_triple(place.motion_shift, pos_goal, ease_share(slide_rate));
    place.motion_tilt = blend_triple(place.motion_tilt, turn_goal, ease_share(spin_rate));
}

/// One frame of the gun's placement: eases the stance and movement offsets, steps the kick,
/// takes the bob, then sums the turn and the move.
#[allow(clippy::too_many_arguments)]
pub fn compose_gun_pose(
    place: &mut GunPoseState,
    view_ps: GunPosePlayerFeed,
    rest_ofs: GunPostureRestShiftFeed,
    fade_cfg: PostureShiftBlendTuning,
    move_ofs: GunMotionShiftFeed,
    motion: GunMotionPhysics,
    bob_cfg: GunStrideSwayFeed,
    idle_cfg: GunRestDriftFeed,
    walk_bob: Option<GunStrideSwayState>,
    hip_spring: ViewmodelKickReturn,
    ads_spring: ViewmodelKickReturn,
    pitch_cap: f32,
    yaw_cap: f32,
    step_secs: f32,
    order_out: &mut [GunPoseBuildStage; POSE_BUILD_STAGE_TOTAL],
) -> GunPoseShare {
    *order_out = [
        GunPoseBuildStage::PostureMotionTiltBase,
        GunPoseBuildStage::Sway,
        GunPoseBuildStage::GunRecoil,
        GunPoseBuildStage::Bob,
        GunPoseBuildStage::CommitTilt,
        GunPoseBuildStage::CommitPosition,
    ];

    base_stance_movement_angles(place, view_ps, rest_ofs, fade_cfg, move_ofs, motion);
    gun_kick_offset(
        &mut place.gun_recoil,
        step_secs,
        view_ps.gun_raise_share,
        view_ps.scoped_aiming,
        hip_spring,
        ads_spring,
        pitch_cap,
        yaw_cap,
    );
    if let Some(walk_wave) = walk_bob {
        take_bob(place, view_ps, bob_cfg, walk_wave);
    }

    let mut turn = weapon_placement_apply_angles(place, view_ps);
    let idle_view = GunPosePlayerFeed {
        ent_bits: idle_e_flags(view_ps.ent_bits, motion.movement_state_bits),
        ..view_ps
    };
    add_idle_sway(place, idle_view, idle_cfg, motion.step_millis_elapsed, &mut turn);
    let hit_turn = weapon_damage_kick_angles(
        place.hit_flinch_stamp,
        place.hurt_stamp_ms,
        place.hurt_kick_pitch,
        place.hurt_kick_roll,
        view_ps.gun_raise_share,
        view_ps.hud_crosshair_layer,
    );
    place.hit_flinch_tilt = hit_turn;
    for (a, f) in turn.iter_mut().zip(hit_turn) {
        *a += f;
    }

    let pos = weapon_placement_apply_origin(place, view_ps);
    GunPoseShare {
        origin: pos,
        angles: turn,
    }
}

/// How far out along the right vector a dual wielded gun sits: the left hand's is further.
#[must_use]
pub fn dual_wield_gun_offset(hand_idx: i32, rgt: [f32; 3], gap: f32) -> [f32; 3] {
    let hand_reach = gap
        * if hand_idx == 0 {
            1.0
        } else {
            DUAL_WIELD_LEFT_SCALE
        };
    rgt.map(|x| x * hand_reach)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lean_ps(peek_amount: f32, gun_raise_share: f32) -> GunPosePlayerFeed {
        GunPosePlayerFeed {
            peek_amount,
            gun_raise_share,
            ..Default::default()
        }
    }

    #[test]
    fn a_stance_change_fades_the_resting_offset_out() {
        let globals = PostureShiftBlendTuning {
            blend_begin_share: 0.25,
            blend_finish_share: 0.75,
        };
        assert_eq!(stance_fade(false, 50, 100, globals), 1.0);
        assert_eq!(stance_fade(true, 50, 0, globals), 1.0);
        assert_eq!(stance_fade(true, 20, 100, globals), 1.0);
        assert_eq!(stance_fade(true, 50, 100, globals), 0.5);
        assert_eq!(stance_fade(true, 80, 100, globals), 0.0);
        let flat = PostureShiftBlendTuning {
            blend_begin_share: 0.5,
            blend_finish_share: 0.5,
        };
        // On the edge the start wins; past it the offset is gone.
        assert_eq!(stance_fade(true, 50, 100, flat), 1.0);
        assert_eq!(stance_fade(true, 60, 100, flat), 0.0);
    }

    #[test]
    fn crouching_rests_the_gun_and_the_sights_tip_it() {
        let rest_ofs = GunPostureRestShiftFeed {
            ducked_ofs: [1.0, 2.0, 3.0],
            prone_ofs: [4.0, 5.0, 6.0],
            aimed_pitch: 8.0,
            goggles_equip_duration: 0,
        };
        let globals = PostureShiftBlendTuning::default();
        let (mut pitch, mut origin) = (0.0, [0.0; 3]);
        let view_ps = GunPosePlayerFeed {
            ent_bits: ent_bit::CROUCH_BIT | ent_bit::LYING_FLAT,
            gun_raise_share: 0.5,
            scoped_aiming: true,
            ..Default::default()
        };
        add_stance_offset(view_ps, rest_ofs, globals, &mut pitch, &mut origin);
        assert_eq!((origin, pitch), ([1.0, 2.0, 3.0], 4.0));
        let prone = GunPosePlayerFeed {
            ent_bits: ent_bit::LYING_FLAT,
            ..view_ps
        };
        let mut origin = [0.0; 3];
        add_stance_offset(prone, rest_ofs, globals, &mut 0.0, &mut origin);
        assert_eq!(origin, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn a_bob_with_no_turn_leaves_the_origin_alone() {
        let mut origin = [3.0, -2.0, 5.0];
        weapon_bob_rotate_origin(&mut origin, GunStrideSwayState::default());
        assert_eq!(origin, [3.0, -2.0, 5.0]);
    }

    #[test]
    fn the_sights_take_the_movement_offsets_by_half_way_up() {
        assert_eq!(movement_scale(0.0), 1.0);
        assert_eq!(movement_scale(0.25), 0.5);
        assert_eq!(movement_scale(0.5), 0.0);
        assert_eq!(movement_scale(1.0), 0.0);
    }

    #[test]
    fn a_hit_flinches_the_gun_and_it_eases_back() {
        assert_eq!(
            weapon_damage_kick_angles(500, 0, 10.0, 10.0, 0.0, 0),
            [0.0; 3]
        );
        let at_start = weapon_damage_kick_angles(1000, 1000, 10.0, 10.0, 0.0, 0);
        assert_eq!(at_start, [0.0, 0.0, 0.0]);
        let peak = weapon_damage_kick_angles(1050, 1000, 10.0, 6.0, 0.0, 0);
        assert!(peak[0] > 0.0 && peak[1] < 0.0 && peak[2] > 0.0, "{peak:?}");
        let gone = weapon_damage_kick_angles(1000 + 50 + 200 + 1, 1000, 10.0, 6.0, 0.0, 0);
        assert_eq!(gone, [0.0; 3]);
    }

    #[test]
    fn the_idle_share_moves_toward_the_stance_without_passing_it() {
        let idle_cfg = GunRestDriftFeed {
            idle_crouch_factor: 0.5,
            idle_prone_factor: 0.1,
            ..Default::default()
        };
        assert_eq!(step_idle_factor(1.0, 0, idle_cfg, 0.5), 1.0);
        assert_eq!(step_idle_factor(1.0, ent_bit::CROUCH_BIT, idle_cfg, 0.5), 0.75);
        assert_eq!(step_idle_factor(1.0, ent_bit::CROUCH_BIT, idle_cfg, 4.0), 0.5);
        assert_eq!(step_idle_factor(0.0, 0, idle_cfg, 0.5), 0.25);
        assert_eq!(step_idle_factor(0.9, ent_bit::LYING_FLAT, idle_cfg, 8.0), 0.1);
        assert_eq!(idle_clock_step(1.0, 0.05), 50);
    }

    #[test]
    fn a_gun_with_no_idle_setting_drifts_by_the_default() {
        let view_ps = GunPosePlayerFeed::default();
        assert_eq!(
            weapon_idle_amount_speed(view_ps, GunRestDriftFeed::default()),
            (80.0, 1.0)
        );
        let hip = GunRestDriftFeed {
            hip_idle_amount: 30.0,
            hip_idle_speed: 2.0,
            ..Default::default()
        };
        assert_eq!(weapon_idle_amount_speed(view_ps, hip), (30.0, 2.0));
        let ads = GunRestDriftFeed {
            ads_idle_amount: 10.0,
            ads_idle_speed: 4.0,
            ..hip
        };
        let half = GunPosePlayerFeed {
            scoped_aiming: true,
            gun_raise_share: 0.5,
            ..view_ps
        };
        assert_eq!(weapon_idle_amount_speed(half, ads), (20.0, 3.0));
    }

    #[test]
    fn a_still_player_gets_no_placement() {
        let mut place = GunPoseState::default();
        let mut steps = [GunPoseBuildStage::Bob; POSE_BUILD_STAGE_TOTAL];
        let out = compose_gun_pose(
            &mut place,
            GunPosePlayerFeed::default(),
            GunPostureRestShiftFeed::default(),
            PostureShiftBlendTuning::default(),
            GunMotionShiftFeed::default(),
            GunMotionPhysics::default(),
            GunStrideSwayFeed::default(),
            GunRestDriftFeed::default(),
            None,
            ViewmodelKickReturn::default(),
            ViewmodelKickReturn::default(),
            0.0,
            0.0,
            0.0,
            &mut steps,
        );
        assert_eq!(out, GunPoseShare::default());
        assert_eq!(
            steps[0],
            GunPoseBuildStage::PostureMotionTiltBase
        );
        assert_eq!(steps[5], GunPoseBuildStage::CommitPosition);
    }

    #[test]
    fn a_dual_wielded_left_gun_sits_twice_as_far_out() {
        let right = [0.0, 1.0, 0.0];
        assert_eq!(
            dual_wield_gun_offset(0, right, 3.0),
            [0.0, 3.0, 0.0]
        );
        assert_eq!(
            dual_wield_gun_offset(1, right, 3.0),
            [0.0, 6.0, 0.0]
        );
    }

    mod bo2_gun_bob {
        use super::*;

        fn side(movement_state_bits: u32, bo2_gun: Option<Bo2GunBob>) -> f32 {
            bob_amplitude(60, movement_state_bits, 0.0, 20.0, 0, bo2_gun)
        }

        #[test]
        fn rifles_do_not_bob_while_sprinting_in_bo2() {
            let rifle = Some(Bo2GunBob::default());
            let pistol = Some(Bo2GunBob {
                sprint: [1.0, 1.0],
                dtp: [1.0, 1.0],
            });
            let sprint = movement_state_bits::SPRINTING;
            assert_eq!(side(sprint, rifle), 0.0);
            assert!((side(sprint, pistol) - 0.02 * 20.0).abs() < 1e-6);
            assert!((side(sprint, None) - 0.02 * 20.0).abs() < 1e-6);
        }

        #[test]
        fn diving_uses_the_gun_dive_bob_in_bo2() {
            let pistol = Some(Bo2GunBob {
                sprint: [1.0, 1.0],
                dtp: [1.0, 1.0],
            });
            assert!((side(PMF_DIVE_ANY, pistol) - 0.002 * 20.0).abs() < 1e-6);
            assert_eq!(side(PMF_DIVE_ANY, Some(Bo2GunBob::default())), 0.0);
            // MW2 gun bob ignores the dive.
            assert!((side(PMF_DIVE_ANY, None) - 0.055 * 20.0).abs() < 1e-6);
        }

        #[test]
        fn a_prone_gun_bobs_the_prone_amount_and_a_fast_one_stops_at_the_cap() {
            let prone = bob_amplitude(EYE_LEVEL_LYING, 0, 0.0, 10.0, 1, None);
            assert!((prone - 0.005 * 10.0).abs() < 1e-6);
            assert_eq!(bob_amplitude(0, 0, 0.0, 1000.0, 0, None), 8.0);
        }
    }

    mod bo2_lean {
        use super::*;

        #[test]
        fn a_lean_moves_and_rolls_the_gun_and_fades_with_ads() {
            let place = GunPoseState::default();
            // Full right lean at hip: curve 1.0, 1.6 to the right (minus y), roll down 2.
            let hip = lean_ps(1.0, 0.0);
            let origin = weapon_placement_apply_origin(&place, hip);
            let angles = weapon_placement_apply_angles(&place, hip);
            assert!((angles[2] + 2.0).abs() < 1e-5);
            let hand_reach =
                (origin[0] * origin[0] + origin[1] * origin[1] + origin[2] * origin[2]).sqrt();
            assert!((hand_reach - 1.6).abs() < 1e-4);
            assert!(origin[1] < -1.5);
            // Half way down the sights the push halves; the roll does not.
            let half = weapon_placement_apply_origin(&place, lean_ps(1.0, 0.5));
            assert!((half[1] - origin[1] * 0.5).abs() < 1e-5);
            assert!(
                (weapon_placement_apply_angles(&place, lean_ps(1.0, 0.5))[2] + 2.0).abs() < 1e-5
            );
            // Fully down the sights, or not leaning: nothing.
            assert_eq!(
                weapon_placement_apply_angles(&place, lean_ps(1.0, 1.0))[2],
                0.0
            );
            assert_eq!(
                weapon_placement_apply_origin(&place, lean_ps(0.0, 0.0)),
                [0.0; 3]
            );
        }

        #[test]
        fn a_swimmer_s_gun_bobs_by_the_raw_amount_at_any_speed_and_ads() {
            let swim = |ground_velocity, gun_raise_share| {
                calculate_weapon_movement_bob_waveform(GunStrideWaveFeed {
                    sway_phase: 40,
                    ground_velocity,
                    gun_raise_share,
                    bo2_gun: Some(Bo2GunBob::default()),
                    waterlevel: 4,
                    ..Default::default()
                })
            };
            let slow = swim(0.0, 0.0);
            assert_eq!(slow, swim(250.0, 0.0));
            assert_eq!(slow, swim(0.0, 0.8));
            assert!(slow.yaw.abs() > 0.5);
            // Waist deep, or the MW2 gun bob: no swimming bob.
            let wade = GunStrideWaveFeed {
                sway_phase: 40,
                waterlevel: 3,
                bo2_gun: Some(Bo2GunBob::default()),
                ..Default::default()
            };
            assert_eq!(calculate_weapon_movement_bob_waveform(wade).yaw, 0.0);
            let mw2 = GunStrideWaveFeed {
                sway_phase: 40,
                waterlevel: 4,
                ..Default::default()
            };
            assert_eq!(calculate_weapon_movement_bob_waveform(mw2).yaw, 0.0);
        }
    }
}
