//! Where the viewmodel gun sits and which way it points: the stance and movement offsets,
//! sway, kick, bob, idle drift, a hit's flinch and a lean, summed into one move and one turn.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use game_math::{angle_vectors, get_lean_fraction};

use crate::kick::{GunKickSpring, GunRecoilPlacementState, calculate_weapon_position_gun_recoil};
use crate::ps_flags::{eflags, pm_flags};
use crate::state;
use crate::sway::{SwaySpringState, sway_contribution};

/// `view_height_target` of a prone player and of a crouched one.
pub(crate) const VIEWHEIGHT_TARGET_PRONE: i32 = 0x0b;
pub(crate) const VIEWHEIGHT_TARGET_CROUCH: i32 = 0x28;

/// TODO(fact): BO2's bg_weaponBobAmplitudeBase default was not found; MW2's value kept.
pub(crate) const WEAPON_BOB_AMPLITUDE_BASE: f32 = 0.16;
/// The roll bob swings this many times as wide as the side bob.
pub(crate) const WEAPON_BOB_AMPLITUDE_ROLL: f32 = 1.5;
/// The bob cycle runs this share of a half turn ahead of the step counter.
pub(crate) const WEAPON_BOB_LAG: f32 = 0.25;
/// The roll runs this far (radians) behind the side bob.
pub(crate) const WEAPON_BOB_UP_PHASE: f32 = 0.471_238_94;
/// The widest a bob swings.
const WEAPON_BOB_MAX: f32 = 8.0;

/// Gun bob swing (side, up) for each way of moving, per unit of bob speed.
const BOB_AMP_STANDING: [f32; 2] = [0.055, 0.025];
const BOB_AMP_DUCKED: [f32; 2] = [0.045, 0.025];
const BOB_AMP_PRONE: [f32; 2] = [0.02, 0.005];
const BOB_AMP_SPRINTING: [f32; 2] = [0.02, 0.014];
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
pub(crate) const WEAPON_IDLE_YAW_FREQ: f32 = 0.0007;
pub(crate) const WEAPON_IDLE_PITCH_FREQ: f32 = 0.001;
pub(crate) const WEAPON_IDLE_SIN_SCALE: f32 = 0.01;
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
pub(crate) fn idle_e_flags(e_flags: u32, pm_flags: u32) -> u32 {
    if pm_flags & PMF_DIVE_ANY != 0 {
        e_flags & !eflags::PRONE
    } else {
        e_flags
    }
}

/// The turn the walk gives the gun, in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponBobState {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

/// Where the gun rests when crouched or prone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponStanceStaticOfsInputs {
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    /// How far the gun tips (pitch) at the sights.
    pub ads_aim_pitch: f32,
    /// How long raising or lowering the night vision takes, ms.
    pub night_vision_wear_time: i32,
}

/// A weapon's offsets (move, turn) for walking, strafing, crouching and going prone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponMovementOfsInputs {
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
pub struct WeaponMovementKinematics {
    pub xyspeed: f32,
    pub speed: f32,
    pub velocity: [f32; 3],
    pub viewangles: [f32; 3],
    pub weaponstate: i32,
    pub weaponstate_secondary: i32,
    pub pm_flags: u32,
    pub frametime: f32,
    /// bo2zm: a Black Ops II climb is keeping the gun in hand.
    pub bo2_mantle: bool,
}

/// How much of the bob the sights keep.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponBobInputs {
    pub ads_bob_factor: f32,
}

/// A weapon's idle drift settings, at the hip and at the sights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponIdleInputs {
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
pub struct WeaponBobWaveformInputs {
    /// The step counter: 0..=255 for half a turn of the bob.
    pub bob_cycle: u8,
    pub xyspeed: f32,
    pub view_height_target: i32,
    pub pm_flags: u32,
    pub weapon_pos_frac: f32,
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
pub struct StanceTransitionFadeGlobals {
    pub fade_start_frac: f32,
    pub fade_end_frac: f32,
}

impl Default for StanceTransitionFadeGlobals {
    fn default() -> Self {
        Self {
            fade_start_frac: 0.0,
            fade_end_frac: 1.0,
        }
    }
}

/// Everything the placement remembers from frame to frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponPlacementState {
    pub movement_angles: [f32; 3],
    pub movement_origin: [f32; 3],
    pub unfaded_angles: [f32; 3],
    pub sway_springs: SwaySpringState,
    pub gun_recoil: GunRecoilPlacementState,
    pub bob: WeaponBobState,
    /// The idle clock, ms, and the share of the drift left by the stance.
    pub weap_idle_time: i32,
    pub last_idle_factor: f32,
    pub idle_sway_angles: [f32; 3],
    /// The last hit's flinch: when it was taken, its two kicks, and the turn it gives.
    pub damage_kick_time: i32,
    pub damage_time: i32,
    pub v_dmg_pitch: f32,
    pub v_dmg_roll: f32,
    pub damage_kick_angles: [f32; 3],
}

/// The player-state fields the placement reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponPlacementPsInputs {
    pub e_flags: u32,
    pub weapon_pos_frac: f32,
    pub weapon_time: i32,
    pub aim_down_sight: bool,
    pub overlay_reticle: i32,
    pub weapon_transition_active: bool,
    pub lean_fraction: f32,
}

/// What the placement moves the gun by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponPlacementContribution {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

pub const PLACEMENT_ASSEMBLE_STEP_COUNT: usize = 6;

/// The parts of [`weapon_placement_assemble`], in the order it does them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponPlacementAssembleStep {
    BaseStanceMovementAngles,
    Sway,
    GunRecoil,
    Bob,
    ApplyAngles,
    ApplyOrigin,
}

fn add_scaled(out: &mut [f32; 3], v: [f32; 3], scale: f32) {
    for (o, x) in out.iter_mut().zip(v) {
        *o += x * scale;
    }
}

fn lerp3(from: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * t)
}

/// How much of the stance offsets is left: all of it outside a stance change, none once
/// the change is `fade_end_frac` through.
fn stance_fade(
    active: bool,
    time: i32,
    wear_time: i32,
    globals: StanceTransitionFadeGlobals,
) -> f32 {
    if !active || wear_time <= 0 {
        return 1.0;
    }
    let t = time as f32 / wear_time as f32;
    let (start, end) = (globals.fade_start_frac, globals.fade_end_frac);
    if t <= start {
        1.0
    } else if t >= end {
        0.0
    } else if end - start <= 0.0 {
        0.0
    } else {
        1.0 - (t - start) / (end - start)
    }
}

/// Adds the crouched or prone resting offset to `origin`, and the sights' tip to `pitch`.
fn add_stance_offset(
    ps: WeaponPlacementPsInputs,
    stance: WeaponStanceStaticOfsInputs,
    fade_globals: StanceTransitionFadeGlobals,
    pitch: &mut f32,
    origin: &mut [f32; 3],
) {
    let fade = stance_fade(
        ps.weapon_transition_active,
        ps.weapon_time,
        stance.night_vision_wear_time,
        fade_globals,
    );
    if ps.e_flags & eflags::DUCK != 0 {
        add_scaled(origin, stance.ducked_ofs, fade);
    } else if ps.e_flags & eflags::PRONE != 0 {
        add_scaled(origin, stance.prone_ofs, fade);
    }
    if ps.aim_down_sight {
        *pitch += stance.ads_aim_pitch * ps.weapon_pos_frac;
    }
}

/// How much of the bob the sights leave: all of it at the hip.
pub(crate) fn weapon_bob_ads_attenuation(weapon_pos_frac: f32, ads_bob_factor: f32) -> f32 {
    if weapon_pos_frac <= 0.0 {
        return 1.0;
    }
    1.0 - weapon_pos_frac * (1.0 - ads_bob_factor)
}

/// Takes the walk's bob as the gun's, less what the sights take.
fn take_bob(
    state: &mut WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
    bob_inputs: WeaponBobInputs,
    waveform: WeaponBobState,
) {
    state.bob = waveform;
    if ps.weapon_pos_frac > 0.0 {
        let scale = weapon_bob_ads_attenuation(ps.weapon_pos_frac, bob_inputs.ads_bob_factor);
        state.bob.pitch *= scale;
        state.bob.yaw *= scale;
        state.bob.roll *= scale;
    }
}

/// How wide the bob swings along `axis` (0 side, 1 up) at bob `speed`.
fn bob_amplitude(
    view_height_target: i32,
    pm_flags: u32,
    weapon_pos_frac: f32,
    speed: f32,
    axis: usize,
    bo2_gun: Option<Bo2GunBob>,
) -> f32 {
    let hip = 1.0 - weapon_pos_frac;
    let dive_gun = bo2_gun.filter(|_| pm_flags & PMF_DIVE_ANY != 0);
    let scale = if let Some(gun) = dive_gun {
        BOB_AMP_DTP[axis] * gun.dtp[axis]
    } else if view_height_target == VIEWHEIGHT_TARGET_PRONE {
        BOB_AMP_PRONE[axis]
    } else if view_height_target == VIEWHEIGHT_TARGET_CROUCH {
        hip * BOB_AMP_DUCKED[axis]
    } else if pm_flags & self::pm_flags::SPRINTING == 0 {
        hip * BOB_AMP_STANDING[axis]
    } else {
        BOB_AMP_SPRINTING[axis] * bo2_gun.map_or(1.0, |gun| gun.sprint[axis])
    };
    let amp = speed * scale;
    if amp > WEAPON_BOB_MAX {
        WEAPON_BOB_MAX
    } else {
        amp
    }
}

/// The up bob's shape at `cycle`, before its amplitude.
fn bob_up_shape(cycle: f32) -> f32 {
    (libm::sinf(cycle * 4.0 + FRAC_PI_2) * 0.2 + libm::sinf(cycle + cycle)) * 0.75
}

/// The gun's bob turn for a step counter and speed.
pub fn calculate_weapon_movement_bob_waveform(inputs: WeaponBobWaveformInputs) -> WeaponBobState {
    let half_turns = (f32::from(inputs.bob_cycle) / 255.0) * PI;
    let cycle = TAU + half_turns + half_turns + TAU + WEAPON_BOB_LAG * PI;
    let speed = WEAPON_BOB_AMPLITUDE_BASE * inputs.xyspeed;
    let amplitude = |speed: f32, axis: usize| {
        bob_amplitude(
            inputs.view_height_target,
            inputs.pm_flags,
            inputs.weapon_pos_frac,
            speed,
            axis,
            inputs.bo2_gun,
        )
    };

    let (pitch, yaw, mut roll);
    // bo2mp: a BO2 swimmer's gun bobs by the raw swimming amount, whatever the speed.
    if inputs.bo2_gun.is_some() && inputs.waterlevel >= crate::view_bob::SWIM_WATER_LEVEL {
        let [side, up] = BOB_AMP_SWIMMING.map(|a| a.min(WEAPON_BOB_MAX));
        pitch = -bob_up_shape(cycle) * up;
        yaw = -libm::sinf(cycle) * side;
        roll = libm::sinf(cycle - WEAPON_BOB_UP_PHASE) * side;
    } else {
        pitch = -(bob_up_shape(cycle) * amplitude(speed, 1));
        yaw = -(libm::sinf(cycle) * amplitude(speed, 0));
        roll = libm::sinf(cycle - WEAPON_BOB_UP_PHASE)
            * amplitude(WEAPON_BOB_AMPLITUDE_ROLL * speed, 0);
    }
    // The gun only ever rolls one way: a roll the other way is none.
    if !(roll <= 0.0) {
        roll = 0.0;
    }
    WeaponBobState { pitch, yaw, roll }
}

/// Adds the bob to `angles`; a full-screen scope takes away what the sights have taken.
fn add_bob_to_angles(
    angles: &mut [f32; 3],
    bob: WeaponBobState,
    weapon_pos_frac: f32,
    overlay_reticle: i32,
) {
    let scale = if overlay_reticle != 0 {
        1.0 - weapon_pos_frac
    } else {
        1.0
    };
    angles[0] += bob.pitch * scale;
    angles[1] += bob.yaw * scale;
    angles[2] += bob.roll * scale;
}

/// Turns `origin` by the bob: onto the bob's forward, left and up.
pub fn weapon_bob_rotate_origin(origin: &mut [f32; 3], bob: WeaponBobState) {
    let (forward, right, up) = angle_vectors([bob.pitch, bob.yaw, bob.roll]);
    let axes = [forward, right.map(|x| -x), up];
    let src = *origin;
    for (j, out) in origin.iter_mut().enumerate() {
        *out = src[0] * axes[0][j] + src[1] * axes[1][j] + src[2] * axes[2][j];
    }
}

/// How much of the movement offsets the sights leave: none from half way up.
fn movement_scale(weapon_pos_frac: f32) -> f32 {
    let left = 1.0 - weapon_pos_frac - weapon_pos_frac;
    if left > 0.0 { left } else { 0.0 }
}

/// bo2mp: BO2's gun lean, the lean fraction curve, or None when the player is not leaning or is
/// fully down the sights. TODO(fact): the roll may take the raw lean fraction, not the curve.
fn viewmodel_lean(ps: WeaponPlacementPsInputs) -> Option<f32> {
    if ps.lean_fraction == 0.0 || ps.weapon_pos_frac == 1.0 {
        return None;
    }
    Some(get_lean_fraction(ps.lean_fraction))
}

/// The gun's angles up to BO2's lean step (movement, sway, lean roll): the lean origin follows
/// their right vector. TODO(fact): which step's angles BO2 takes the right vector of.
fn angles_to_lean(state: &WeaponPlacementState, ps: WeaponPlacementPsInputs) -> [f32; 3] {
    let scale = movement_scale(ps.weapon_pos_frac);
    let sway = sway_contribution(state.sway_springs);
    let mut angles = [0.0; 3];
    if scale > 0.0 {
        add_scaled(&mut angles, state.movement_angles, scale);
    }
    angles[0] += sway.angles[0];
    angles[1] += sway.angles[1];
    if let Some(lean) = viewmodel_lean(ps) {
        angles[2] -= VIEWMODEL_LEAN_ROLL * lean;
    }
    angles
}

/// Where the gun is moved to: the movement offset, the sway's slide, the lean's push, all
/// turned by the bob.
fn weapon_placement_apply_origin(
    state: &WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
) -> [f32; 3] {
    let mut out = [0.0_f32; 3];
    let scale = movement_scale(ps.weapon_pos_frac);
    if scale > 0.0 {
        add_scaled(&mut out, state.movement_origin, scale);
    }
    let sway = sway_contribution(state.sway_springs);
    out[1] -= sway.origin[1];
    out[2] += sway.origin[2];
    if let Some(lean) = viewmodel_lean(ps) {
        let (_, right, _) = angle_vectors(angles_to_lean(state, ps));
        add_scaled(
            &mut out,
            right,
            (1.0 - ps.weapon_pos_frac) * lean * VIEWMODEL_LEAN_DIST,
        );
    }
    weapon_bob_rotate_origin(&mut out, state.bob);
    out
}

/// The gun's idle drift (amount, speed) for the sights' progress.
pub(crate) fn weapon_idle_amount_speed(
    ps: WeaponPlacementPsInputs,
    idle: WeaponIdleInputs,
) -> (f32, f32) {
    if ps.aim_down_sight {
        let amount = (idle.ads_idle_amount - idle.hip_idle_amount) * ps.weapon_pos_frac
            + idle.hip_idle_amount;
        let speed =
            (idle.ads_idle_speed - idle.hip_idle_speed) * ps.weapon_pos_frac + idle.hip_idle_speed;
        return (amount, speed);
    }
    if idle.hip_idle_amount != 0.0 {
        return (idle.hip_idle_amount, idle.hip_idle_speed);
    }
    (IDLE_AMOUNT_DEFAULT, 1.0)
}

/// How far the idle clock moves in a frame, ms.
pub(crate) fn idle_clock_step(speed: f32, frametime: f32) -> i32 {
    libm::roundf(speed * WEAPON_IDLE_TIME_MS_SCALE * frametime) as i32
}

/// `last` moved toward the share of the drift this stance keeps, by half a frame's time
/// at most.
pub(crate) fn step_idle_factor(
    last: f32,
    e_flags: u32,
    idle: WeaponIdleInputs,
    frametime: f32,
) -> f32 {
    let target = if e_flags & eflags::PRONE != 0 {
        idle.idle_prone_factor
    } else if e_flags & eflags::DUCK != 0 {
        idle.idle_crouch_factor
    } else {
        1.0
    };
    if last == target {
        return last;
    }
    let step = frametime * WEAPON_IDLE_FACTOR_LERP;
    if target <= last {
        let next = last - step;
        if next < target { target } else { next }
    } else {
        let next = last + step;
        if next > target { target } else { next }
    }
}

/// Runs the idle clock on and adds the drift to `angles`.
fn add_idle_sway(
    state: &mut WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
    idle: WeaponIdleInputs,
    frametime: f32,
    angles: &mut [f32; 3],
) {
    let (amount, speed) = weapon_idle_amount_speed(ps, idle);
    state.weap_idle_time = state
        .weap_idle_time
        .wrapping_add(idle_clock_step(speed, frametime));
    state.last_idle_factor = step_idle_factor(state.last_idle_factor, ps.e_flags, idle, frametime);

    let mut scaled = state.last_idle_factor * amount;
    if ps.overlay_reticle != 0 {
        scaled *= 1.0 - ps.weapon_pos_frac;
    }
    let t = state.weap_idle_time as f32;
    let s = scaled * WEAPON_IDLE_SIN_SCALE;
    let drift = [
        libm::sinf(t * WEAPON_IDLE_PITCH_FREQ) * s,
        libm::sinf(t * WEAPON_IDLE_YAW_FREQ) * s,
        libm::sinf(t * IDLE_ROLL_FREQ) * s,
    ];
    state.idle_sway_angles = drift;
    for (a, d) in angles.iter_mut().zip(drift) {
        *a += d;
    }
}

/// The turn a hit's flinch gives the gun `time - damage_time` ms after the hit.
pub fn weapon_damage_kick_angles(
    time: i32,
    damage_time: i32,
    v_dmg_pitch: f32,
    v_dmg_roll: f32,
    weapon_pos_frac: f32,
    overlay_reticle: i32,
) -> [f32; 3] {
    if damage_time == 0 {
        return [0.0; 3];
    }
    let mut factor = GUN_DAMAGE_ADS_HALF + weapon_pos_frac * GUN_DAMAGE_ADS_HALF;
    let deflect = GUN_DAMAGE_DEFLECT_MS * factor;
    let ret = GUN_DAMAGE_RETURN_MS * factor;
    if weapon_pos_frac != 0.0 && overlay_reticle != 0 {
        factor *= 1.0 - weapon_pos_frac * GUN_DAMAGE_OVERLAY_MIX;
    }
    let delta = time.wrapping_sub(damage_time) as f32;
    let scale = if deflect <= delta {
        let left = 1.0 - (delta - deflect) / ret;
        if left <= 0.0 {
            return [0.0; 3];
        }
        (1.0 - get_lean_fraction(1.0 - left)) * factor
    } else {
        get_lean_fraction(delta / deflect) * factor
    };
    [
        scale * v_dmg_pitch * GUN_DAMAGE_ADS_HALF,
        -(v_dmg_roll * scale),
        scale * v_dmg_roll * GUN_DAMAGE_ADS_HALF,
    ]
}

/// The gun's turn: movement, sway, lean, the unfaded angles, bob and kick.
fn weapon_placement_apply_angles(
    state: &WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
) -> [f32; 3] {
    let mut out = angles_to_lean(state, ps);
    for (o, u) in out.iter_mut().zip(state.unfaded_angles) {
        *o += u;
    }
    add_bob_to_angles(&mut out, state.bob, ps.weapon_pos_frac, ps.overlay_reticle);
    out[0] += state.gun_recoil.pitch_offset;
    out[1] += state.gun_recoil.yaw_offset;
    out
}

/// Adds what moving does to the gun: the stance's move and turn, scaled by how far over the
/// slowest counted speed the player is, and the strafe's by how much of the speed is sideways.
fn add_movement_targets(
    ps: WeaponPlacementPsInputs,
    kinematics: WeaponMovementKinematics,
    movement: WeaponMovementOfsInputs,
    min_speed: f32,
    origin: &mut [f32; 3],
    angles: &mut [f32; 3],
) {
    let (shift, turn) = if ps.e_flags & eflags::PRONE != 0 {
        (movement.prone_move, movement.prone_rot)
    } else if ps.e_flags & eflags::DUCK != 0 {
        (movement.ducked_move, movement.ducked_rot)
    } else {
        (movement.stand_move, movement.stand_rot)
    };
    let mut speed_frac = (kinematics.xyspeed - min_speed) / (kinematics.speed - min_speed);
    if speed_frac >= 1.0 {
        speed_frac = 1.0;
    }
    let hip = 1.0 - ps.weapon_pos_frac;
    add_scaled(origin, shift, speed_frac);
    add_scaled(angles, turn, speed_frac * hip);

    let (forward, _, _) = angle_vectors(kinematics.viewangles);
    let along = (forward[0] * kinematics.velocity[0] + kinematics.velocity[1] * forward[1])
        * speed_frac
        / kinematics.xyspeed;
    add_scaled(origin, movement.strafe_move, along);
    add_scaled(angles, movement.strafe_rot, along * hip);
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
    state: &mut WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
    stance: WeaponStanceStaticOfsInputs,
    fade_globals: StanceTransitionFadeGlobals,
    movement: WeaponMovementOfsInputs,
    kinematics: WeaponMovementKinematics,
) {
    let duck = ps.e_flags & eflags::DUCK != 0;
    let prone = ps.e_flags & eflags::PRONE != 0;
    let min_speed = if prone {
        movement.prone_move_min_speed
    } else if duck {
        movement.ducked_move_min_speed
    } else {
        movement.stand_move_min_speed
    };
    let moving = kinematics.xyspeed > min_speed && kinematics.speed > min_speed;
    let reloading = is_reload_state(kinematics.weaponstate)
        || is_reload_state(kinematics.weaponstate_secondary);
    let night_vision = matches!(
        kinematics.weaponstate,
        crate::state::NIGHTVISION_WEAR | crate::state::NIGHTVISION_REMOVE
    );
    let on_ladder = kinematics.pm_flags & pm_flags::LADDER != 0;

    let mut origin_target = [0.0_f32; 3];
    let mut angles_target = [0.0_f32; 3];

    if (duck || prone) && (!moving || !night_vision || !prone) {
        let mut pitch = 0.0_f32;
        add_stance_offset(ps, stance, fade_globals, &mut pitch, &mut origin_target);
        angles_target[0] += pitch;
    }
    if moving && !on_ladder && !reloading && !night_vision {
        add_movement_targets(
            ps,
            kinematics,
            movement,
            min_speed,
            &mut origin_target,
            &mut angles_target,
        );
    }
    if kinematics.bo2_mantle {
        for i in 0..3 {
            origin_target[i] += movement.mantle_ofs[i];
            angles_target[i] += movement.mantle_rot[i];
        }
    }

    let (move_rate, turn_rate) = if prone {
        (movement.pos_prone_move_rate, movement.pos_prone_rot_rate)
    } else {
        (movement.pos_move_rate, movement.pos_rot_rate)
    };
    let share = |rate: f32| {
        let t = rate * kinematics.frametime;
        if t > 1.0 { 1.0 } else { t }
    };
    state.movement_origin = lerp3(state.movement_origin, origin_target, share(move_rate));
    state.movement_angles = lerp3(state.movement_angles, angles_target, share(turn_rate));
}

/// One frame of the gun's placement: eases the stance and movement offsets, steps the kick,
/// takes the bob, then sums the turn and the move.
#[allow(clippy::too_many_arguments)]
pub fn weapon_placement_assemble(
    state: &mut WeaponPlacementState,
    ps: WeaponPlacementPsInputs,
    stance: WeaponStanceStaticOfsInputs,
    fade_globals: StanceTransitionFadeGlobals,
    movement: WeaponMovementOfsInputs,
    kinematics: WeaponMovementKinematics,
    bob_inputs: WeaponBobInputs,
    idle: WeaponIdleInputs,
    bob_waveform: Option<WeaponBobState>,
    gun_spring_hip: GunKickSpring,
    gun_spring_ads: GunKickSpring,
    gun_max_pitch: f32,
    gun_max_yaw: f32,
    dt_secs: f32,
    steps_out: &mut [WeaponPlacementAssembleStep; PLACEMENT_ASSEMBLE_STEP_COUNT],
) -> WeaponPlacementContribution {
    use WeaponPlacementAssembleStep as Step;
    *steps_out = [
        Step::BaseStanceMovementAngles,
        Step::Sway,
        Step::GunRecoil,
        Step::Bob,
        Step::ApplyAngles,
        Step::ApplyOrigin,
    ];

    base_stance_movement_angles(state, ps, stance, fade_globals, movement, kinematics);
    calculate_weapon_position_gun_recoil(
        &mut state.gun_recoil,
        dt_secs,
        ps.weapon_pos_frac,
        ps.aim_down_sight,
        gun_spring_hip,
        gun_spring_ads,
        gun_max_pitch,
        gun_max_yaw,
    );
    if let Some(waveform) = bob_waveform {
        take_bob(state, ps, bob_inputs, waveform);
    }

    let mut angles = weapon_placement_apply_angles(state, ps);
    let idle_ps = WeaponPlacementPsInputs {
        e_flags: idle_e_flags(ps.e_flags, kinematics.pm_flags),
        ..ps
    };
    add_idle_sway(state, idle_ps, idle, kinematics.frametime, &mut angles);
    let flinch = weapon_damage_kick_angles(
        state.damage_kick_time,
        state.damage_time,
        state.v_dmg_pitch,
        state.v_dmg_roll,
        ps.weapon_pos_frac,
        ps.overlay_reticle,
    );
    state.damage_kick_angles = flinch;
    for (a, f) in angles.iter_mut().zip(flinch) {
        *a += f;
    }

    let origin = weapon_placement_apply_origin(state, ps);
    WeaponPlacementContribution { origin, angles }
}

/// How far out along the right vector a dual wielded gun sits: the left hand's is further.
#[must_use]
pub fn dual_wield_view_model_origin_add(hand: i32, right: [f32; 3], offset: f32) -> [f32; 3] {
    let reach = offset
        * if hand == 0 {
            1.0
        } else {
            DUAL_WIELD_LEFT_SCALE
        };
    right.map(|x| x * reach)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lean_ps(lean_fraction: f32, weapon_pos_frac: f32) -> WeaponPlacementPsInputs {
        WeaponPlacementPsInputs {
            lean_fraction,
            weapon_pos_frac,
            ..Default::default()
        }
    }

    #[test]
    fn a_stance_change_fades_the_resting_offset_out() {
        let globals = StanceTransitionFadeGlobals {
            fade_start_frac: 0.25,
            fade_end_frac: 0.75,
        };
        assert_eq!(stance_fade(false, 50, 100, globals), 1.0);
        assert_eq!(stance_fade(true, 50, 0, globals), 1.0);
        assert_eq!(stance_fade(true, 20, 100, globals), 1.0);
        assert_eq!(stance_fade(true, 50, 100, globals), 0.5);
        assert_eq!(stance_fade(true, 80, 100, globals), 0.0);
        let flat = StanceTransitionFadeGlobals {
            fade_start_frac: 0.5,
            fade_end_frac: 0.5,
        };
        // On the edge the start wins; past it the offset is gone.
        assert_eq!(stance_fade(true, 50, 100, flat), 1.0);
        assert_eq!(stance_fade(true, 60, 100, flat), 0.0);
    }

    #[test]
    fn crouching_rests_the_gun_and_the_sights_tip_it() {
        let stance = WeaponStanceStaticOfsInputs {
            ducked_ofs: [1.0, 2.0, 3.0],
            prone_ofs: [4.0, 5.0, 6.0],
            ads_aim_pitch: 8.0,
            night_vision_wear_time: 0,
        };
        let globals = StanceTransitionFadeGlobals::default();
        let (mut pitch, mut origin) = (0.0, [0.0; 3]);
        let ps = WeaponPlacementPsInputs {
            e_flags: eflags::DUCK | eflags::PRONE,
            weapon_pos_frac: 0.5,
            aim_down_sight: true,
            ..Default::default()
        };
        add_stance_offset(ps, stance, globals, &mut pitch, &mut origin);
        assert_eq!((origin, pitch), ([1.0, 2.0, 3.0], 4.0));
        let prone = WeaponPlacementPsInputs {
            e_flags: eflags::PRONE,
            ..ps
        };
        let mut origin = [0.0; 3];
        add_stance_offset(prone, stance, globals, &mut 0.0, &mut origin);
        assert_eq!(origin, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn a_bob_with_no_turn_leaves_the_origin_alone() {
        let mut origin = [3.0, -2.0, 5.0];
        weapon_bob_rotate_origin(&mut origin, WeaponBobState::default());
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
        let idle = WeaponIdleInputs {
            idle_crouch_factor: 0.5,
            idle_prone_factor: 0.1,
            ..Default::default()
        };
        assert_eq!(step_idle_factor(1.0, 0, idle, 0.5), 1.0);
        assert_eq!(step_idle_factor(1.0, eflags::DUCK, idle, 0.5), 0.75);
        assert_eq!(step_idle_factor(1.0, eflags::DUCK, idle, 4.0), 0.5);
        assert_eq!(step_idle_factor(0.0, 0, idle, 0.5), 0.25);
        assert_eq!(step_idle_factor(0.9, eflags::PRONE, idle, 8.0), 0.1);
        assert_eq!(idle_clock_step(1.0, 0.05), 50);
    }

    #[test]
    fn a_gun_with_no_idle_setting_drifts_by_the_default() {
        let ps = WeaponPlacementPsInputs::default();
        assert_eq!(
            weapon_idle_amount_speed(ps, WeaponIdleInputs::default()),
            (80.0, 1.0)
        );
        let hip = WeaponIdleInputs {
            hip_idle_amount: 30.0,
            hip_idle_speed: 2.0,
            ..Default::default()
        };
        assert_eq!(weapon_idle_amount_speed(ps, hip), (30.0, 2.0));
        let ads = WeaponIdleInputs {
            ads_idle_amount: 10.0,
            ads_idle_speed: 4.0,
            ..hip
        };
        let half = WeaponPlacementPsInputs {
            aim_down_sight: true,
            weapon_pos_frac: 0.5,
            ..ps
        };
        assert_eq!(weapon_idle_amount_speed(half, ads), (20.0, 3.0));
    }

    #[test]
    fn a_still_player_gets_no_placement() {
        let mut state = WeaponPlacementState::default();
        let mut steps = [WeaponPlacementAssembleStep::Bob; PLACEMENT_ASSEMBLE_STEP_COUNT];
        let out = weapon_placement_assemble(
            &mut state,
            WeaponPlacementPsInputs::default(),
            WeaponStanceStaticOfsInputs::default(),
            StanceTransitionFadeGlobals::default(),
            WeaponMovementOfsInputs::default(),
            WeaponMovementKinematics::default(),
            WeaponBobInputs::default(),
            WeaponIdleInputs::default(),
            None,
            GunKickSpring::default(),
            GunKickSpring::default(),
            0.0,
            0.0,
            0.0,
            &mut steps,
        );
        assert_eq!(out, WeaponPlacementContribution::default());
        assert_eq!(
            steps[0],
            WeaponPlacementAssembleStep::BaseStanceMovementAngles
        );
        assert_eq!(steps[5], WeaponPlacementAssembleStep::ApplyOrigin);
    }

    #[test]
    fn a_dual_wielded_left_gun_sits_twice_as_far_out() {
        let right = [0.0, 1.0, 0.0];
        assert_eq!(
            dual_wield_view_model_origin_add(0, right, 3.0),
            [0.0, 3.0, 0.0]
        );
        assert_eq!(
            dual_wield_view_model_origin_add(1, right, 3.0),
            [0.0, 6.0, 0.0]
        );
    }

    mod bo2_gun_bob {
        use super::*;

        fn side(pm_flags: u32, bo2_gun: Option<Bo2GunBob>) -> f32 {
            bob_amplitude(60, pm_flags, 0.0, 20.0, 0, bo2_gun)
        }

        #[test]
        fn rifles_do_not_bob_while_sprinting_in_bo2() {
            let rifle = Some(Bo2GunBob::default());
            let pistol = Some(Bo2GunBob {
                sprint: [1.0, 1.0],
                dtp: [1.0, 1.0],
            });
            let sprint = pm_flags::SPRINTING;
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
            let prone = bob_amplitude(VIEWHEIGHT_TARGET_PRONE, 0, 0.0, 10.0, 1, None);
            assert!((prone - 0.005 * 10.0).abs() < 1e-6);
            assert_eq!(bob_amplitude(0, 0, 0.0, 1000.0, 0, None), 8.0);
        }
    }

    mod bo2_lean {
        use super::*;

        #[test]
        fn a_lean_moves_and_rolls_the_gun_and_fades_with_ads() {
            let state = WeaponPlacementState::default();
            // Full right lean at hip: curve 1.0, 1.6 to the right (minus y), roll down 2.
            let hip = lean_ps(1.0, 0.0);
            let origin = weapon_placement_apply_origin(&state, hip);
            let angles = weapon_placement_apply_angles(&state, hip);
            assert!((angles[2] + 2.0).abs() < 1e-5);
            let reach =
                (origin[0] * origin[0] + origin[1] * origin[1] + origin[2] * origin[2]).sqrt();
            assert!((reach - 1.6).abs() < 1e-4);
            assert!(origin[1] < -1.5);
            // Half way down the sights the push halves; the roll does not.
            let half = weapon_placement_apply_origin(&state, lean_ps(1.0, 0.5));
            assert!((half[1] - origin[1] * 0.5).abs() < 1e-5);
            assert!(
                (weapon_placement_apply_angles(&state, lean_ps(1.0, 0.5))[2] + 2.0).abs() < 1e-5
            );
            // Fully down the sights, or not leaning: nothing.
            assert_eq!(
                weapon_placement_apply_angles(&state, lean_ps(1.0, 1.0))[2],
                0.0
            );
            assert_eq!(
                weapon_placement_apply_origin(&state, lean_ps(0.0, 0.0)),
                [0.0; 3]
            );
        }

        #[test]
        fn a_swimmer_s_gun_bobs_by_the_raw_amount_at_any_speed_and_ads() {
            let swim = |xyspeed, weapon_pos_frac| {
                calculate_weapon_movement_bob_waveform(WeaponBobWaveformInputs {
                    bob_cycle: 40,
                    xyspeed,
                    weapon_pos_frac,
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
            let wade = WeaponBobWaveformInputs {
                bob_cycle: 40,
                waterlevel: 3,
                bo2_gun: Some(Bo2GunBob::default()),
                ..Default::default()
            };
            assert_eq!(calculate_weapon_movement_bob_waveform(wade).yaw, 0.0);
            let mw2 = WeaponBobWaveformInputs {
                bob_cycle: 40,
                waterlevel: 4,
                ..Default::default()
            };
            assert_eq!(calculate_weapon_movement_bob_waveform(mw2).yaw, 0.0);
        }
    }
}
