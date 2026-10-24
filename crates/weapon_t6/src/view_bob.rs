//! What the walk, a landing, a hit and the scope's drift do to the camera: the head bob,
//! the landing dip, the flinch, and the hit feedback the server sends.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use game_math::{angle_vectors, get_lean_fraction, vect_to_angles};

use crate::placement::{
    PMF_DIVE_ANY, VIEWHEIGHT_TARGET_CROUCH, VIEWHEIGHT_TARGET_PRONE, WEAPON_BOB_AMPLITUDE_BASE,
    WEAPON_BOB_AMPLITUDE_ROLL, WEAPON_BOB_LAG, WEAPON_BOB_UP_PHASE, WEAPON_IDLE_PITCH_FREQ,
    WEAPON_IDLE_SIN_SCALE, WEAPON_IDLE_YAW_FREQ, WeaponIdleInputs, WeaponPlacementPsInputs,
    idle_clock_step, idle_e_flags, step_idle_factor, weapon_bob_ads_attenuation,
    weapon_idle_amount_speed,
};
use crate::ps_flags::pm_flags;

/// Hits report this yaw and pitch byte when they have no direction.
pub const VIEW_DAMAGE_UNDIRECTED: u32 = 255;

/// The gap the feet are lifted off the ground by the landing dip.
pub const VIEW_ORG_BOB_Z_MIN_OFS: f32 = 8.0;

/// How much of the camera's landing dip the gun takes.
pub const VIEWWEAPON_LAND_SCALE: f32 = 0.25;

/// `perks[0]` bits: Lightweight (shrinks the MW2 head bob), Toughness (shrinks the BO2 flinch).
pub const PERK_LIGHTWEIGHT_VIEW_BOB_BIT: u32 = 0x0100_0000;
pub const PERK_BULLETFLINCH: u32 = 1 << 5;

/// The `e_flags` bits that put the player on a turret or in a vehicle.
pub const EFLAGS_TURRET_VEHICLE: u32 = 0xc00;

/// bo2zm: from this water level up the player swims.
pub const SWIM_WATER_LEVEL: i32 = 4;

/// The widest the head bob swings.
const BOB_MAX: f32 = 8.0;
/// Head bob swing (side, up) per unit of speed for each way of moving.
const AMP_STANDING: [f32; 2] = [0.007, 0.007];
const AMP_STANDING_ADS: [f32; 2] = [0.007, 0.007];
const AMP_DUCKED: [f32; 2] = [0.0075, 0.0075];
const AMP_DUCKED_ADS: [f32; 2] = [0.0075, 0.0075];
const AMP_PRONE: [f32; 2] = [0.02, 0.005];
const AMP_SPRINTING: [f32; 2] = [0.02, 0.014];
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
pub struct ViewOrgBobInputs {
    /// The step counter: 0..=255 for half a turn of the bob.
    pub bob_cycle: u8,
    pub xyspeed: f32,
    pub view_height_target: i32,
    pub pm_flags: u32,
    pub weapon_pos_frac: f32,
    pub perks0: u32,
    /// bo2zm: Black Ops II camera rules (prone bob, no Lightweight shrink).
    pub bo2: bool,
    /// bo2zm: how deep in water the player stands (0 to 6).
    pub waterlevel: i32,
}

/// The head's bob, up (`vertical`) and across (`horizontal`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewOrgBob {
    pub vertical: f32,
    pub horizontal: f32,
}

/// What the camera angles read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewAngleBobInputs {
    pub org: ViewOrgBobInputs,
    pub e_flags: u32,
    pub overlay_reticle: i32,
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub time: i32,
    pub damage_time: i32,
    pub v_dmg_pitch: f32,
    pub v_dmg_roll: f32,
    pub aim_down_sight: bool,
    pub idle: WeaponIdleInputs,
    pub frametime: f32,
    pub hold_breath_scale: f32,
    pub weap_idle_time: i32,
    pub view_last_idle_factor: f32,
}

/// The turn the camera gets, and the scope drift's clock to carry to the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewAngleBob {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub cam_idle_pitch: f32,
    pub cam_idle_yaw: f32,
    pub weap_idle_time: i32,
    pub view_last_idle_factor: f32,
}

/// A hit's kick: the pitch and roll it gives the view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewDamageFeedback {
    pub v_dmg_pitch: f32,
    pub v_dmg_roll: f32,
}

/// The bob cycle of the head: where in the step the player is, in radians.
fn head_cycle(bob_cycle: u8) -> f32 {
    let half_turns = (f32::from(bob_cycle) / 255.0) * PI;
    half_turns + half_turns + TAU
}

/// How wide the head swings along `axis` (`SIDE` or `UP`).
fn head_amplitude(org: ViewOrgBobInputs, axis: usize) -> f32 {
    let bo2 = org.bo2;
    if bo2 && org.waterlevel >= SWIM_WATER_LEVEL {
        return AMP_SWIMMING[axis].min(BOB_MAX);
    }
    let ads = org.weapon_pos_frac;
    let scale = if bo2 && org.pm_flags & PMF_DIVE_ANY != 0 {
        AMP_DTP[axis]
    } else if org.view_height_target == VIEWHEIGHT_TARGET_PRONE {
        if bo2 {
            AMP_PRONE_BO2[axis]
        } else {
            AMP_PRONE[axis]
        }
    } else if org.view_height_target == VIEWHEIGHT_TARGET_CROUCH {
        (1.0 - ads) * AMP_DUCKED[axis] + ads * AMP_DUCKED_ADS[axis]
    } else if org.pm_flags & pm_flags::SPRINTING == 0 {
        (1.0 - ads) * AMP_STANDING[axis] + ads * AMP_STANDING_ADS[axis]
    } else {
        AMP_SPRINTING[axis]
    };
    let mut amp = scale * org.xyspeed;
    if !bo2 && org.perks0 & PERK_LIGHTWEIGHT_VIEW_BOB_BIT != 0 {
        amp *= LIGHTWEIGHT_SCALE;
    }
    if amp > BOB_MAX {
        amp = BOB_MAX;
    }
    amp
}

/// The head's up and down bob at `cycle`.
fn bob_up(cycle: f32, org: ViewOrgBobInputs) -> f32 {
    (libm::sinf(cycle * 4.0 + FRAC_PI_2) * 0.2 + libm::sinf(cycle + cycle))
        * 0.75
        * head_amplitude(org, UP)
}

/// The head's side to side bob at `cycle`.
fn bob_side(cycle: f32, org: ViewOrgBobInputs) -> f32 {
    libm::sinf(cycle) * head_amplitude(org, SIDE)
}

/// The height of a fall, from the speed the player touched down at. None without gravity,
/// or when the two heights cannot be joined by a fall.
pub fn crash_land_fall_height(
    gravity: i32,
    previous_origin_z: f32,
    origin_z: f32,
    previous_velocity_z: f32,
) -> Option<f32> {
    if gravity == 0 {
        return None;
    }
    let dropped = previous_origin_z - origin_z;
    let vel = previous_velocity_z;
    let accel = -(gravity as f32);
    let half_accel = accel * 0.5;
    let discriminant = vel * vel - 4.0 * half_accel * dropped;
    if discriminant < 0.0 {
        return None;
    }
    let two_a = half_accel * 2.0;
    if two_a == 0.0 {
        return None;
    }
    let fall_time = (-vel - libm::sqrtf(discriminant)) / two_a;
    let touchdown_speed = (fall_time * accel + vel) * -1.0;
    Some((touchdown_speed * touchdown_speed) / ((gravity as f32) * 2.0))
}

/// How many steps the view dips on touching down from `fall_height`. BO2 drops the
/// fraction where MW2 rounds.
pub fn crash_land_view_dip(fall_height: f32, bo2: bool) -> i32 {
    if !(fall_height > LAND_DIP_FALL_IN) {
        return 0;
    }
    let raw = (fall_height - LAND_DIP_FALL_IN) / LAND_DIP_SPAN_IN * LAND_DIP_SCALE + LAND_DIP_SCALE;
    let steps = if bo2 {
        raw as i32
    } else {
        libm::roundf(raw) as i32
    };
    if steps > LAND_DIP_MAX {
        LAND_DIP_MAX
    } else if steps < 0 {
        0
    } else {
        steps
    }
}

/// How far into its dip the view is `delta_ms` after the landing: 0 to 1 and back to 0. None
/// before the touchdown and once it is over.
fn land_weight(delta_ms: f32) -> Option<f32> {
    if !(delta_ms > 0.0) {
        return None;
    }
    if delta_ms < LAND_DEFLECT_MS {
        return Some(delta_ms / LAND_DEFLECT_MS);
    }
    if delta_ms >= LAND_END_MS {
        return None;
    }
    Some(1.0 - (delta_ms - LAND_DEFLECT_MS) / LAND_RETURN_MS)
}

/// The view's drop `delta_ms` after a landing that dips `land_change`.
pub fn land_origin_z(delta_ms: f32, land_change: f32) -> f32 {
    land_weight(delta_ms).map_or(0.0, |weight| weight * land_change)
}

/// The gun's drop `delta_ms` after a landing that dips the view `land_change`.
pub fn viewweapon_land_origin_z(delta_ms: i32, land_change: f32) -> f32 {
    let deflect = LAND_DEFLECT_MS as i32;
    let end = LAND_END_MS as i32;
    if delta_ms < deflect {
        return land_change * VIEWWEAPON_LAND_SCALE * (delta_ms as f32) / LAND_DEFLECT_MS;
    }
    if delta_ms < end {
        return land_change * VIEWWEAPON_LAND_SCALE * ((end - delta_ms) as f32) / LAND_RETURN_MS;
    }
    0.0
}

/// The head's bob: none standing still, but a BO2 swimmer bobs treading water.
pub fn view_org_bob(inputs: ViewOrgBobInputs) -> ViewOrgBob {
    let swimming = inputs.bo2 && inputs.waterlevel >= SWIM_WATER_LEVEL;
    if !(inputs.xyspeed > 0.0) && !swimming {
        return ViewOrgBob::default();
    }
    let cycle = head_cycle(inputs.bob_cycle);
    ViewOrgBob {
        vertical: bob_up(cycle, inputs),
        horizontal: bob_side(cycle, inputs),
    }
}

fn with_speed(org: ViewOrgBobInputs, xyspeed: f32) -> ViewOrgBobInputs {
    ViewOrgBobInputs { xyspeed, ..org }
}

/// Adds the gun's own bob to the camera as the scope comes up, so the scope bobs with it.
fn add_overlay_bob(angles: &mut ViewAngleBob, inputs: ViewAngleBobInputs) {
    if inputs.overlay_reticle == 0 {
        return;
    }
    let cycle = head_cycle(inputs.org.bob_cycle) + TAU + WEAPON_BOB_LAG * PI;
    let speed = WEAPON_BOB_AMPLITUDE_BASE * inputs.org.xyspeed;
    let at_speed = with_speed(inputs.org, speed);
    let mut pitch = -bob_up(cycle, at_speed);
    let mut yaw = -bob_side(cycle, at_speed);
    let mut roll = bob_side(
        cycle - WEAPON_BOB_UP_PHASE,
        with_speed(inputs.org, WEAPON_BOB_AMPLITUDE_ROLL * speed),
    );
    // The scope only ever rolls one way: a roll the other way is none.
    if !(roll <= 0.0) {
        roll = 0.0;
    }
    let frac = inputs.org.weapon_pos_frac;
    if frac != 0.0 {
        let keep = weapon_bob_ads_attenuation(frac, inputs.ads_bob_factor);
        pitch *= keep;
        yaw *= keep;
        roll *= keep;
    }
    angles.pitch += frac * pitch;
    angles.yaw += frac * yaw;
    angles.roll += frac * roll;
}

/// Takes the head's bob off the camera as the sights come up, by the weapon's own factor.
fn take_ads_bob(angles: &mut ViewAngleBob, inputs: ViewAngleBobInputs) {
    let frac = inputs.org.weapon_pos_frac;
    if !(frac > 0.0) || inputs.e_flags & EFLAGS_TURRET_VEHICLE != 0 {
        return;
    }
    if !(inputs.ads_view_bob_mult > 0.0) {
        return;
    }
    let scale = inputs.ads_view_bob_mult * frac;
    let cycle = head_cycle(inputs.org.bob_cycle);
    angles.pitch -= bob_up(cycle, inputs.org) * scale;
    angles.yaw -= bob_side(cycle, inputs.org) * scale;
}

/// MW2's flinch size for `damage_count`, at least `kick_min`.
fn kick_size(damage_count: i32, kick_min: f32) -> f32 {
    let scaled = damage_count as f32 * KICK_SCALE;
    if scaled < kick_min {
        kick_min
    } else if scaled > KICK_MAX {
        KICK_MAX
    } else {
        scaled
    }
}

/// How far a hit's travel direction (as bytes) points along the view's forward and right.
fn hit_along_view(yaw_byte: u32, pitch_byte: u32, viewangles: [f32; 3]) -> (f32, f32) {
    let pitch = (pitch_byte as f32 / DIRECTION_BYTE_MAX) * FULL_TURN_DEGREES;
    let yaw = (yaw_byte as f32 / DIRECTION_BYTE_MAX) * FULL_TURN_DEGREES;
    let (dir, _, _) = angle_vectors([pitch, yaw, 0.0]);
    let (forward, right, _) = angle_vectors(viewangles);
    let side = dir[0] * right[0] + dir[1] * right[1] + dir[2] * right[2];
    let ahead = dir[0] * forward[0] + dir[1] * forward[1] + dir[2] * forward[2];
    (ahead, side)
}

/// MW2's flinch from a hit's travel direction, with the smallest flinch given
/// (`KICK_MIN_BO2` for Black Ops II).
fn damage_kick_with_min(
    yaw_byte: u32,
    pitch_byte: u32,
    damage_count: i32,
    viewangles: [f32; 3],
    kick_min: f32,
) -> ViewDamageFeedback {
    let kick = kick_size(damage_count, kick_min);
    if yaw_byte == VIEW_DAMAGE_UNDIRECTED && pitch_byte == VIEW_DAMAGE_UNDIRECTED {
        return ViewDamageFeedback {
            v_dmg_pitch: -kick,
            v_dmg_roll: 0.0,
        };
    }
    let (ahead, side) = hit_along_view(yaw_byte, pitch_byte, viewangles);
    ViewDamageFeedback {
        v_dmg_pitch: kick * ahead,
        v_dmg_roll: -kick * side,
    }
}

/// MW2's flinch from a hit's travel direction (yaw and pitch bytes; 255/255 for none).
pub fn damage_feedback_kick(
    yaw_byte: u32,
    pitch_byte: u32,
    damage_count: i32,
    viewangles: [f32; 3],
) -> ViewDamageFeedback {
    damage_kick_with_min(yaw_byte, pitch_byte, damage_count, viewangles, KICK_MIN)
}

/// bo2zm: Black Ops II's hit flinch size: 0.175 a percent of health, a quarter of that
/// with Toughness, kept between 0.5 and 90 degrees.
fn kick_size_bo2(damage_count: i32, bullet_flinch: bool) -> f32 {
    let mut kick = damage_count as f32 * KICK_SCALE_BO2;
    if bullet_flinch {
        kick *= KICK_TOUGHNESS_SHARE;
    }
    if kick < KICK_MIN_BO2 {
        KICK_MIN_BO2
    } else if kick > KICK_MAX {
        KICK_MAX
    } else {
        kick
    }
}

/// bo2zm: Black Ops II's hit flinch from the hit's travel direction (yaw and pitch bytes).
/// 255/255 = no direction: the view tips up. 0/0 keeps the last flinch's angles (None);
/// the flinch still restarts. The roll leans the other way from MW2's.
pub fn damage_feedback_kick_bo2(
    yaw_byte: u32,
    pitch_byte: u32,
    damage_count: i32,
    viewangles: [f32; 3],
    bullet_flinch: bool,
) -> Option<ViewDamageFeedback> {
    let kick = kick_size_bo2(damage_count, bullet_flinch);
    if yaw_byte == VIEW_DAMAGE_UNDIRECTED && pitch_byte == VIEW_DAMAGE_UNDIRECTED {
        return Some(ViewDamageFeedback {
            v_dmg_pitch: -kick,
            v_dmg_roll: 0.0,
        });
    }
    if yaw_byte == 0 && pitch_byte == 0 {
        return None;
    }
    let (ahead, side) = hit_along_view(yaw_byte, pitch_byte, viewangles);
    Some(ViewDamageFeedback {
        v_dmg_pitch: kick * ahead,
        v_dmg_roll: kick * side,
    })
}

/// bo2zm: what Black Ops II sends for a frame's hits: the share of max health they took in
/// percent (at most 127), and the last hit's travel direction as yaw and pitch bytes
/// (255/255 with no direction). None when there is nothing to send.
pub fn damage_feedback_bo2(
    blood: i32,
    max_health: i32,
    dir: Option<[f32; 3]>,
) -> Option<(i32, u32, u32)> {
    if blood < 1 || max_health < 1 {
        return None;
    }
    let count = (blood.saturating_mul(100) / max_health).min(DAMAGE_COUNT_MAX_BO2);
    let Some(dir) = dir else {
        return Some((count, VIEW_DAMAGE_UNDIRECTED, VIEW_DAMAGE_UNDIRECTED));
    };
    let angles = vect_to_angles(dir);
    let to_byte =
        |angle: f32| ((angle * DEGREES_TO_TURNS * DIRECTION_BYTE_TURN) as i32 as u32) & 0xff;
    Some((count, to_byte(angles[1]), to_byte(angles[0])))
}

/// Adds the flinch of the last hit to the camera: out fast, back slowly.
fn add_flinch(angles: &mut ViewAngleBob, inputs: ViewAngleBobInputs) {
    if inputs.damage_time == 0 {
        return;
    }
    let frac = inputs.org.weapon_pos_frac;
    let mut factor = 1.0 - frac * FLINCH_ADS_HALF;
    if frac != 0.0 && inputs.overlay_reticle != 0 {
        factor = (frac * FLINCH_ADS_HALF + 1.0) * factor;
    }
    let delta = inputs.time.wrapping_sub(inputs.damage_time) as f32;
    let weight = if delta < FLINCH_DEFLECT_MS {
        get_lean_fraction(delta / FLINCH_DEFLECT_MS)
    } else {
        let left = 1.0 - (delta - FLINCH_DEFLECT_MS) / FLINCH_RETURN_MS;
        if !(left > 0.0) {
            return;
        }
        1.0 - get_lean_fraction(1.0 - left)
    };
    let scale = weight * factor;
    angles.pitch += scale * inputs.v_dmg_pitch;
    angles.roll += scale * inputs.v_dmg_roll;
}

/// The camera's turn from the last hit alone.
pub fn view_damage_angles(inputs: ViewAngleBobInputs) -> ViewAngleBob {
    let mut angles = ViewAngleBob::default();
    add_flinch(&mut angles, inputs);
    angles
}

/// Runs the scope's drift on and adds it to the camera.
fn add_camera_idle(angles: &mut ViewAngleBob, inputs: ViewAngleBobInputs) {
    let frac = inputs.org.weapon_pos_frac;
    if inputs.overlay_reticle == 0 || frac == 0.0 {
        return;
    }
    let ps = WeaponPlacementPsInputs {
        e_flags: inputs.e_flags,
        weapon_pos_frac: frac,
        aim_down_sight: inputs.aim_down_sight,
        overlay_reticle: inputs.overlay_reticle,
        ..WeaponPlacementPsInputs::default()
    };
    let (amount, speed) = weapon_idle_amount_speed(ps, inputs.idle);
    // bo2mp: holding the breath slows the scope's drift in time as well as in size (Black Ops'
    // view idle: the time step is scaled by `holdBreathScale`).
    angles.weap_idle_time = angles.weap_idle_time.wrapping_add(idle_clock_step(
        inputs.hold_breath_scale * speed,
        inputs.frametime,
    ));
    angles.view_last_idle_factor = step_idle_factor(
        angles.view_last_idle_factor,
        idle_e_flags(inputs.e_flags, inputs.org.pm_flags),
        inputs.idle,
        inputs.frametime,
    );

    let scale = angles.view_last_idle_factor
        * amount
        * frac
        * inputs.hold_breath_scale
        * WEAPON_IDLE_SIN_SCALE;
    let t = angles.weap_idle_time as f32;
    let yaw = libm::sinf(t * WEAPON_IDLE_YAW_FREQ) * scale;
    let pitch = libm::sinf(t * WEAPON_IDLE_PITCH_FREQ) * scale;
    angles.cam_idle_yaw = yaw;
    angles.cam_idle_pitch = pitch;
    angles.yaw += yaw;
    angles.pitch += pitch;
}

/// The camera's turn for the frame: the flinch, the scope's drift and the bob.
pub fn view_angle_bob(inputs: ViewAngleBobInputs) -> ViewAngleBob {
    let mut angles = ViewAngleBob {
        weap_idle_time: inputs.weap_idle_time,
        view_last_idle_factor: inputs.view_last_idle_factor,
        ..ViewAngleBob::default()
    };
    add_flinch(&mut angles, inputs);
    add_camera_idle(&mut angles, inputs);
    add_overlay_bob(&mut angles, inputs);
    take_ads_bob(&mut angles, inputs);
    angles
}

/// bo2mp: the walk bob part of [`view_angle_bob`] alone (the scope's bob and the ADS bob,
/// pitch / yaw / roll), for a caller that puts in its own or moves it into the aim.
pub fn view_walk_bob_angles(inputs: ViewAngleBobInputs) -> [f32; 3] {
    let mut angles = ViewAngleBob::default();
    add_overlay_bob(&mut angles, inputs);
    take_ads_bob(&mut angles, inputs);
    [angles.pitch, angles.yaw, angles.roll]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak of the side to side sway.
    fn side_peak(org: ViewOrgBobInputs) -> f32 {
        bob_side(FRAC_PI_2, org)
    }

    fn prone(bo2: bool, perks0: u32) -> ViewOrgBobInputs {
        ViewOrgBobInputs {
            xyspeed: 20.0,
            view_height_target: 0x0b,
            perks0,
            bo2,
            ..ViewOrgBobInputs::default()
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
        assert!((hit.v_dmg_roll + 7.0).abs() < 0.01, "{hit:?}");
        assert!(hit.v_dmg_pitch.abs() < 0.1, "{hit:?}");
        // MW2 rolls the other way.
        let mw2 = damage_feedback_kick(64, 0, 35, [0.0; 3]);
        assert!(mw2.v_dmg_roll > 6.9, "{mw2:?}");
        // A hit travelling the way he looks (yaw byte 0, pitch byte 1).
        let ahead = damage_feedback_kick_bo2(0, 1, 40, [0.0; 3], false).unwrap();
        assert!((ahead.v_dmg_pitch - 7.0).abs() < 0.01, "{ahead:?}");
        // No direction: the view tips up by the whole flinch.
        let none = damage_feedback_kick_bo2(255, 255, 40, [0.0; 3], true).unwrap();
        assert_eq!((none.v_dmg_pitch, none.v_dmg_roll), (-1.75, 0.0));
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
                VIEW_DAMAGE_UNDIRECTED,
                VIEW_DAMAGE_UNDIRECTED,
                10,
                [0.0; 3],
                min,
            )
        };
        // 10 damage x 0.2 = 2: Black Ops keeps 2, MW2 raises it to 5.
        assert_eq!(hit(KICK_MIN_BO2).v_dmg_pitch, -2.0);
        assert_eq!(hit(KICK_MIN).v_dmg_pitch, -5.0);
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
        let height = crash_land_fall_height(800, 100.0, 0.0, 0.0).unwrap();
        assert!((height - 100.0).abs() < 1e-3, "{height}");
        assert_eq!(crash_land_fall_height(0, 100.0, 0.0, 0.0), None);
        // Rising faster than it can ever come down from: no such fall.
        assert_eq!(crash_land_fall_height(800, 0.0, 100.0, 0.0), None);
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
        assert!((side_peak(prone(true, 0)) - 0.08 * 20.0).abs() < 1e-4);
        assert!((side_peak(prone(false, 0)) - 0.02 * 20.0).abs() < 1e-4);
    }

    #[test]
    fn a_swimmer_bobs_by_its_own_amount_at_any_speed() {
        let swim = |bo2, xyspeed| ViewOrgBobInputs {
            waterlevel: 4,
            xyspeed,
            ..prone(bo2, 0)
        };
        assert!((side_peak(swim(true, 20.0)) - 3.0).abs() < 1e-4);
        assert!((side_peak(swim(true, 200.0)) - 3.0).abs() < 1e-4);
        // MW2 has no swimming.
        assert!((side_peak(swim(false, 20.0)) - 0.02 * 20.0).abs() < 1e-4);
        // Treading water still bobs; standing still on land does not.
        let still = ViewOrgBobInputs {
            bob_cycle: 64,
            ..swim(true, 0.0)
        };
        assert!(view_org_bob(still).vertical != 0.0);
        let dry = ViewOrgBobInputs {
            waterlevel: 3,
            ..still
        };
        assert_eq!(view_org_bob(dry), ViewOrgBob::default());
    }

    #[test]
    fn lightweight_does_not_shrink_the_bo2_head_bob() {
        let lw = PERK_LIGHTWEIGHT_VIEW_BOB_BIT;
        assert_eq!(side_peak(prone(true, lw)), side_peak(prone(true, 0)));
        assert!(side_peak(prone(false, lw)) < side_peak(prone(false, 0)));
    }

    #[test]
    fn a_dive_bobs_the_dive_amount_in_bo2_only() {
        let dive = |bo2| ViewOrgBobInputs {
            pm_flags: PMF_DIVE_ANY,
            ..prone(bo2, 0)
        };
        assert!((side_peak(dive(true)) - 0.002 * 20.0).abs() < 1e-5);
        assert!((side_peak(dive(false)) - 0.02 * 20.0).abs() < 1e-4);
    }

    #[test]
    fn the_flinch_goes_out_fast_and_is_gone_after_half_a_second() {
        let hit = |time| ViewAngleBobInputs {
            time,
            damage_time: 1000,
            v_dmg_pitch: 4.0,
            v_dmg_roll: -2.0,
            ..ViewAngleBobInputs::default()
        };
        let none = view_damage_angles(ViewAngleBobInputs::default());
        assert_eq!(none, ViewAngleBob::default());
        let early = view_damage_angles(hit(1050));
        assert!((early.pitch - 0.75 * 4.0).abs() < 1e-6 && (early.roll + 0.75 * 2.0).abs() < 1e-6);
        let peak = view_damage_angles(hit(1100));
        assert!((peak.pitch - 4.0).abs() < 1e-6);
        assert_eq!(view_damage_angles(hit(1500)), ViewAngleBob::default());
    }

    #[test]
    fn the_scope_drifts_only_while_the_sights_are_up_and_slows_with_the_breath() {
        let scoped = |hold_breath_scale| ViewAngleBobInputs {
            org: ViewOrgBobInputs {
                weapon_pos_frac: 1.0,
                ..ViewOrgBobInputs::default()
            },
            overlay_reticle: 1,
            aim_down_sight: true,
            idle: WeaponIdleInputs {
                ads_idle_amount: 10.0,
                ads_idle_speed: 1.0,
                ..WeaponIdleInputs::default()
            },
            frametime: 0.05,
            hold_breath_scale,
            view_last_idle_factor: 1.0,
            ..ViewAngleBobInputs::default()
        };
        let free = view_angle_bob(scoped(1.0));
        assert_eq!(free.weap_idle_time, 50);
        assert!(
            free.cam_idle_yaw > 0.0 && free.cam_idle_pitch > 0.0,
            "{free:?}"
        );
        assert_eq!(view_angle_bob(scoped(0.5)).weap_idle_time, 25);

        let hip = ViewAngleBobInputs {
            overlay_reticle: 0,
            ..scoped(1.0)
        };
        let out = view_angle_bob(hip);
        assert_eq!(
            (out.cam_idle_yaw, out.cam_idle_pitch, out.weap_idle_time),
            (0.0, 0.0, 0)
        );
    }

    #[test]
    fn the_ads_bob_is_taken_off_the_camera_unless_on_a_turret() {
        let moving = ViewAngleBobInputs {
            org: ViewOrgBobInputs {
                bob_cycle: 40,
                xyspeed: 200.0,
                weapon_pos_frac: 1.0,
                ..ViewOrgBobInputs::default()
            },
            ads_view_bob_mult: 0.5,
            ..ViewAngleBobInputs::default()
        };
        let [pitch, yaw, _] = view_walk_bob_angles(moving);
        assert!(pitch != 0.0 && yaw != 0.0);
        let turret = ViewAngleBobInputs {
            e_flags: EFLAGS_TURRET_VEHICLE,
            ..moving
        };
        assert_eq!(view_walk_bob_angles(turret), [0.0; 3]);
        let no_mult = ViewAngleBobInputs {
            ads_view_bob_mult: 0.0,
            ..moving
        };
        assert_eq!(view_walk_bob_angles(no_mult), [0.0; 3]);
    }
}
