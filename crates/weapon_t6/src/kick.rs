//! Recoil: the kick a shot gives the view and the gun, and the springs that bring both
//! back to centre.

/// Kicks and gun springs are stepped in slices of this many ms.
const STEP_MS: i32 = 5;
const MS: f32 = 0.001;
const GUN_STEP_SECS: f32 = 0.005;
/// The view kick is held within this many degrees of centre.
const VIEW_KICK_LIMIT: f32 = 10.0;
/// A view kick heading back toward centre moves at this share of its speed.
const VIEW_KICK_RETURN_SHARE: f32 = 0.06;
/// Sights further up than this centre the view at the ADS speed.
const ADS_CENTER_FRAC: f32 = 0.5;
/// How fast the view centres with no weapon.
const NO_WEAPON_CENTER_SPEED: f32 = 2400.0;
/// A shot rolls the view by this much of its yaw kick.
const ROLL_PER_YAW: f32 = -0.5;
/// weapon_state_bits bit: the kick is scaled by `recoil_scale` percent.
const WEAP_FLAG_RECOIL_SCALE: u32 = 0x400;
/// weapon_state_bits bit: the pitch kick is five times as hard.
const WEAP_FLAG_PITCH_X5: u32 = 0x200;
const PITCH_X5: f32 = 5.0;
/// A gun spring nearer rest than this offset and speed stops.
const GUN_REST_OFFSET: f32 = 0.25;
const GUN_REST_SPEED: f32 = 1.0;

/// How hard a shot kicks the view (degrees a second, picked between min and max).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraKickBounds {
    pub pitch_lower_bound: f32,
    pub pitch_upper_bound: f32,
    pub turn_floor: f32,
    pub turn_ceiling: f32,
}

/// How hard a shot kicks the gun (degrees a second, picked between min and max).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewmodelKickBounds {
    pub pitch_lower_bound: f32,
    pub pitch_upper_bound: f32,
    pub turn_floor: f32,
    pub turn_ceiling: f32,
}

/// The gun's kick: how far it is turned and how fast it is turning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewmodelKickPoseState {
    pub nod_offset: f32,
    pub nod_rate: f32,
    pub yaw_shift: f32,
    pub heading_turn_rate: f32,
}

/// The spring that pulls a kicked gun back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewmodelKickReturn {
    /// The pull toward centre.
    pub speed_up_rate: f32,
    pub velocity_cap: f32,
    /// The share of its speed lost each second.
    pub velocity_falloff: f32,
    /// Speed lost each second regardless.
    pub resting_falloff: f32,
}

/// What one shot adds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotKickPush {
    /// To the view's turn speed (pitch, yaw, roll).
    pub punch_spin_rate: [f32; 3],
    /// To the gun's turn speed (pitch, yaw).
    pub viewmodel_velocity_step: [f32; 2],
}

/// The player-state that scales a shot's kick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShotKickPlayerFactors {
    /// The kick is cut to `dampened_share` for now.
    pub damping_spell_on: bool,
    pub dampened_share: f32,
    pub weapon_state_bits: u32,
    /// The kick's percent when the weapon_state_bits ask for it.
    pub kick_pct: i32,
}

fn lerp(start: f32, end: f32, amount: f32) -> f32 {
    start + (end - start) * amount
}

/// What a shot's kick is scaled by.
pub fn shot_recoil_gain(ps: ShotKickPlayerFactors) -> f32 {
    let reduced = if ps.damping_spell_on {
        ps.dampened_share * 0.01
    } else {
        1.0
    };
    if ps.weapon_state_bits & WEAP_FLAG_RECOIL_SCALE == 0 {
        reduced
    } else {
        reduced * (ps.kick_pct as f32 / 100.0)
    }
}

/// The view kick: the ADS one only with the sights all the way up.
pub fn shot_kick_camera_bounds(
    ads_frac: f32,
    hip_range: CameraKickBounds,
    ads_range: CameraKickBounds,
) -> CameraKickBounds {
    if ads_frac == 1.0 {
        ads_range
    } else {
        hip_range
    }
}

/// The gun kick: the ADS one as soon as the sights start up.
pub fn shot_kick_viewmodel_bounds(
    ads_frac: f32,
    hip_range: ViewmodelKickBounds,
    ads_range: ViewmodelKickBounds,
) -> ViewmodelKickBounds {
    if ads_frac <= 0.0 {
        hip_range
    } else {
        ads_range
    }
}

/// One shot's kick, `rolls` picking within each range (view pitch, view yaw, gun pitch,
/// gun yaw). The view kicks up and rolls against its yaw.
pub fn weapon_fire_recoil(
    view_range: CameraKickBounds,
    gun_range: ViewmodelKickBounds,
    scales: ShotKickPlayerFactors,
    rolls: [f32; 4],
) -> ShotKickPush {
    let amp = shot_recoil_gain(scales);
    let pitch_mult = if scales.weapon_state_bits & WEAP_FLAG_PITCH_X5 != 0 {
        PITCH_X5
    } else {
        1.0
    };
    let view_pitch = lerp(view_range.pitch_lower_bound, view_range.pitch_upper_bound, rolls[0]) * amp * pitch_mult;
    let look_heading = lerp(view_range.turn_floor, view_range.turn_ceiling, rolls[1]) * amp;
    let weapon_tilt_angle = lerp(gun_range.pitch_lower_bound, gun_range.pitch_upper_bound, rolls[2]) * amp * pitch_mult;
    let weapon_swing_angle = lerp(gun_range.turn_floor, gun_range.turn_ceiling, rolls[3]) * amp;
    ShotKickPush {
        punch_spin_rate: [-view_pitch, look_heading, look_heading * ROLL_PER_YAW],
        viewmodel_velocity_step: [weapon_tilt_angle, weapon_swing_angle],
    }
}

/// How fast the view kick centres: fixed with no weapon, else the hip or ADS speed.
pub fn recoil_tilt_return_rate(
    weapon_idx: i32,
    ads_frac: f32,
    hip_pull: f32,
    ads_pull: f32,
) -> f32 {
    match weapon_idx {
        0 => NO_WEAPON_CENTER_SPEED,
        _ if ads_frac <= ADS_CENTER_FRAC => hip_pull,
        _ => ads_pull,
    }
}

/// One axis of the view kick, `dt` seconds on. It is pulled back toward centre, slowed
/// while heading there, stopped on reaching or passing it, and held within the limit.
fn view_kick_axis(ang: &mut f32, rate: &mut f32, step: f32, pull_rate: f32) {
    if *ang == 0.0 && *rate == 0.0 {
        return;
    }
    if *ang != 0.0 {
        let toward = if *ang <= 0.0 { pull_rate } else { -pull_rate };
        *rate += toward * step;
    }
    let mut travel = *rate * step;
    if *ang * travel < 0.0 {
        travel *= VIEW_KICK_RETURN_SHARE;
    }
    let landed = *ang + travel;
    if landed * *ang < 0.0 || landed == 0.0 {
        *ang = 0.0;
        *rate = 0.0;
    } else if landed.abs() > VIEW_KICK_LIMIT {
        *ang = if landed < 0.0 {
            -VIEW_KICK_LIMIT
        } else {
            VIEW_KICK_LIMIT
        };
        *rate = 0.0;
    } else {
        *ang = landed;
    }
}

/// The view kick `step_duration_ms` on, in 5 ms slices.
pub fn recoil_tilt(
    view_angs: &mut [f32; 3],
    view_vels: &mut [f32; 3],
    frame_ms: i32,
    pull_rate: f32,
) {
    let mut countdown_rest = frame_ms;
    while countdown_rest > 0 {
        let slice = countdown_rest.min(STEP_MS) as f32 * MS;
        for (ang, rate) in view_angs.iter_mut().zip(view_vels.iter_mut()) {
            view_kick_axis(ang, rate, slice, pull_rate);
        }
        countdown_rest -= STEP_MS;
    }
}

/// The gun spring `frac` of the way from hip to ADS.
pub fn blend_gun_kick_spring(
    hip_spring: ViewmodelKickReturn,
    ads_spring: ViewmodelKickReturn,
    mix: f32,
) -> ViewmodelKickReturn {
    ViewmodelKickReturn {
        speed_up_rate: lerp(hip_spring.speed_up_rate, ads_spring.speed_up_rate, mix),
        velocity_cap: lerp(hip_spring.velocity_cap, ads_spring.velocity_cap, mix),
        velocity_falloff: lerp(hip_spring.velocity_falloff, ads_spring.velocity_falloff, mix),
        resting_falloff: lerp(hip_spring.resting_falloff, ads_spring.resting_falloff, mix),
    }
}

/// One axis of the gun kick, `dt` seconds on: it turns at its speed (held within
/// `ofs_cap`), the spring pulls it back, and its speed decays.
pub fn viewmodel_kick_one_axis(
    pos: &mut f32,
    motion_vec: &mut f32,
    step: f32,
    pos_limit: f32,
    coil: ViewmodelKickReturn,
) {
    if pos.abs() < GUN_REST_OFFSET && motion_vec.abs() < GUN_REST_SPEED {
        *pos = 0.0;
        *motion_vec = 0.0;
        return;
    }
    let limit = pos_limit.abs();
    *pos += *motion_vec * step;
    if *pos > limit {
        *pos = limit;
        if *motion_vec > 0.0 {
            *motion_vec = 0.0;
        }
    } else if *pos < -limit {
        *pos = -limit;
        if *motion_vec < 0.0 {
            *motion_vec = 0.0;
        }
    }

    let restore = coil.speed_up_rate * step;
    if *pos < 0.0 {
        *motion_vec += restore;
    } else if *pos > 0.0 {
        *motion_vec -= restore;
    }
    *motion_vec -= *motion_vec * coil.velocity_falloff * step;

    let friction = coil.resting_falloff * step;
    if *motion_vec <= 0.0 {
        *motion_vec += friction;
        if *motion_vec > 0.0 {
            *motion_vec = 0.0;
        }
    } else {
        *motion_vec -= friction;
        if *motion_vec < 0.0 {
            *motion_vec = 0.0;
        }
    }
    if *motion_vec > coil.velocity_cap {
        *motion_vec = coil.velocity_cap;
    } else if *motion_vec < -coil.velocity_cap {
        *motion_vec = -coil.velocity_cap;
    }
}

/// The gun kick `step_duration_seconds` on, in 5 ms slices, while `overlay_active`.
#[allow(clippy::too_many_arguments)]
pub fn gun_kick_offset(
    recoil: &mut ViewmodelKickPoseState,
    frame_secs: f32,
    ads_frac: f32,
    scope_up: bool,
    hip_spring: ViewmodelKickReturn,
    ads_spring: ViewmodelKickReturn,
    pitch_limit: f32,
    yaw_limit: f32,
) {
    if !scope_up || frame_secs <= 0.0 {
        return;
    }
    let blended = blend_gun_kick_spring(hip_spring, ads_spring, ads_frac);
    let mut countdown_rest = frame_secs;
    while countdown_rest > 0.0 {
        let slice = countdown_rest.min(GUN_STEP_SECS);
        let mut advance = |pos: &mut f32, motion_vec: &mut f32, limit: f32| {
            viewmodel_kick_one_axis(pos, motion_vec, slice, limit, blended);
        };
        advance(
            &mut recoil.nod_offset,
            &mut recoil.nod_rate,
            pitch_limit,
        );
        advance(&mut recoil.yaw_shift, &mut recoil.heading_turn_rate, yaw_limit);
        countdown_rest -= slice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(min: f32, max: f32) -> CameraKickBounds {
        CameraKickBounds {
            pitch_lower_bound: min,
            pitch_upper_bound: max,
            turn_floor: min,
            turn_ceiling: max,
        }
    }

    #[test]
    fn a_shot_kicks_up_and_rolls_against_its_yaw() {
        let gun = ViewmodelKickBounds {
            pitch_lower_bound: 1.0,
            pitch_upper_bound: 3.0,
            turn_floor: 0.0,
            turn_ceiling: 2.0,
        };
        let shot = weapon_fire_recoil(
            range(2.0, 4.0),
            gun,
            ShotKickPlayerFactors::default(),
            [0.5; 4],
        );
        assert_eq!(shot.punch_spin_rate, [-3.0, 3.0, -1.5]);
        assert_eq!(shot.viewmodel_velocity_step, [2.0, 1.0]);
    }

    #[test]
    fn the_kick_scales_down_and_a_flag_quintuples_pitch() {
        let ps = ShotKickPlayerFactors {
            damping_spell_on: true,
            dampened_share: 50.0,
            weapon_state_bits: WEAP_FLAG_RECOIL_SCALE | WEAP_FLAG_PITCH_X5,
            kick_pct: 50,
        };
        assert_eq!(shot_recoil_gain(ps), 0.25);
        let shot = weapon_fire_recoil(range(4.0, 4.0), ViewmodelKickBounds::default(), ps, [0.0; 4]);
        assert_eq!(shot.punch_spin_rate[0], -5.0);
        assert_eq!(shot.punch_spin_rate[1], 1.0);
    }

    #[test]
    fn hip_and_ads_ranges_switch_at_different_heights() {
        let (hip, ads) = (range(1.0, 1.0), range(2.0, 2.0));
        assert_eq!(shot_kick_camera_bounds(0.9, hip, ads), hip);
        assert_eq!(shot_kick_camera_bounds(1.0, hip, ads), ads);
        let (gun_hip, gun_ads) = (
            ViewmodelKickBounds::default(),
            ViewmodelKickBounds {
                pitch_upper_bound: 1.0,
                ..ViewmodelKickBounds::default()
            },
        );
        assert_eq!(shot_kick_viewmodel_bounds(0.0, gun_hip, gun_ads), gun_hip);
        assert_eq!(shot_kick_viewmodel_bounds(0.1, gun_hip, gun_ads), gun_ads);
        assert_eq!(recoil_tilt_return_rate(0, 1.0, 5.0, 9.0), 2400.0);
        assert_eq!(recoil_tilt_return_rate(3, 0.5, 5.0, 9.0), 5.0);
        assert_eq!(recoil_tilt_return_rate(3, 0.6, 5.0, 9.0), 9.0);
    }

    #[test]
    fn the_view_kick_is_held_to_ten_degrees_and_comes_back() {
        let mut angles = [0.0; 3];
        let mut avel = [-4000.0, 0.0, 0.0];
        recoil_tilt(&mut angles, &mut avel, 5, 0.0);
        assert_eq!((angles[0], avel[0]), (-10.0, 0.0));

        let mut angles = [-1.0, 0.0, 0.0];
        let mut avel = [0.0; 3];
        for _ in 0..2000 {
            recoil_tilt(&mut angles, &mut avel, 50, 100.0);
        }
        assert_eq!(angles, [0.0; 3]);
    }

    #[test]
    fn a_kicked_gun_settles_back() {
        let spring = ViewmodelKickReturn {
            speed_up_rate: 400.0,
            velocity_cap: 200.0,
            velocity_falloff: 10.0,
            resting_falloff: 20.0,
        };
        let mut state = ViewmodelKickPoseState {
            nod_rate: 150.0,
            ..Default::default()
        };
        gun_kick_offset(&mut state, 0.05, 0.0, true, spring, spring, 5.0, 5.0);
        assert!(
            state.nod_offset > 0.0 && state.nod_offset <= 5.0,
            "{state:?}"
        );
        for _ in 0..200 {
            gun_kick_offset(
                &mut state, 0.05, 0.0, true, spring, spring, 5.0, 5.0,
            );
        }
        assert_eq!(state, ViewmodelKickPoseState::default());

        let mut idle = ViewmodelKickPoseState {
            nod_rate: 150.0,
            ..Default::default()
        };
        gun_kick_offset(&mut idle, 0.05, 0.0, false, spring, spring, 5.0, 5.0);
        assert_eq!(idle.nod_offset, 0.0);
    }
}
