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
/// weap_flags bit: the kick is scaled by `recoil_scale` percent.
const WEAP_FLAG_RECOIL_SCALE: u32 = 0x400;
/// weap_flags bit: the pitch kick is five times as hard.
const WEAP_FLAG_PITCH_X5: u32 = 0x200;
const PITCH_X5: f32 = 5.0;
/// A gun spring nearer rest than this offset and speed stops.
const GUN_REST_OFFSET: f32 = 0.25;
const GUN_REST_SPEED: f32 = 1.0;

/// How hard a shot kicks the view (degrees a second, picked between min and max).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewKickRange {
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub yaw_min: f32,
    pub yaw_max: f32,
}

/// How hard a shot kicks the gun (degrees a second, picked between min and max).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunKickRange {
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub yaw_min: f32,
    pub yaw_max: f32,
}

/// The gun's kick: how far it is turned and how fast it is turning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunRecoilPlacementState {
    pub pitch_offset: f32,
    pub pitch_speed: f32,
    pub yaw_offset: f32,
    pub yaw_speed: f32,
}

/// The spring that pulls a kicked gun back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunKickSpring {
    /// The pull toward centre.
    pub accel: f32,
    pub speed_max: f32,
    /// The share of its speed lost each second.
    pub speed_decay: f32,
    /// Speed lost each second regardless.
    pub static_decay: f32,
}

/// What one shot adds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireRecoilImpulse {
    /// To the view's turn speed (pitch, yaw, roll).
    pub kick_avel: [f32; 3],
    /// To the gun's turn speed (pitch, yaw).
    pub gun_speed_delta: [f32; 2],
}

/// The player-state that scales a shot's kick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireRecoilPsScales {
    /// The kick is cut to `reduced_percent` for now.
    pub reduce_window_active: bool,
    pub reduced_percent: f32,
    pub weap_flags: u32,
    /// The kick's percent when the weap_flags ask for it.
    pub recoil_scale: i32,
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// What a shot's kick is scaled by.
pub fn fire_recoil_amplitude_scale(ps: FireRecoilPsScales) -> f32 {
    let reduced = if ps.reduce_window_active {
        ps.reduced_percent * 0.01
    } else {
        1.0
    };
    if ps.weap_flags & WEAP_FLAG_RECOIL_SCALE == 0 {
        reduced
    } else {
        reduced * (ps.recoil_scale as f32 / 100.0)
    }
}

/// The view kick: the ADS one only with the sights all the way up.
pub fn fire_recoil_view_range(
    weapon_pos_frac: f32,
    hip: ViewKickRange,
    ads: ViewKickRange,
) -> ViewKickRange {
    if weapon_pos_frac == 1.0 { ads } else { hip }
}

/// The gun kick: the ADS one as soon as the sights start up.
pub fn fire_recoil_gun_range(
    weapon_pos_frac: f32,
    hip: GunKickRange,
    ads: GunKickRange,
) -> GunKickRange {
    if weapon_pos_frac <= 0.0 { hip } else { ads }
}

/// One shot's kick, `unit01` picking within each range (view pitch, view yaw, gun pitch,
/// gun yaw). The view kicks up and rolls against its yaw.
pub fn weapon_fire_recoil(
    view: ViewKickRange,
    gun: GunKickRange,
    ps: FireRecoilPsScales,
    unit01: [f32; 4],
) -> FireRecoilImpulse {
    let scale = fire_recoil_amplitude_scale(ps);
    let pitch_scale = if ps.weap_flags & WEAP_FLAG_PITCH_X5 != 0 {
        PITCH_X5
    } else {
        1.0
    };
    let view_pitch = lerp(view.pitch_min, view.pitch_max, unit01[0]) * scale * pitch_scale;
    let view_yaw = lerp(view.yaw_min, view.yaw_max, unit01[1]) * scale;
    let gun_pitch = lerp(gun.pitch_min, gun.pitch_max, unit01[2]) * scale * pitch_scale;
    let gun_yaw = lerp(gun.yaw_min, gun.yaw_max, unit01[3]) * scale;
    FireRecoilImpulse {
        kick_avel: [-view_pitch, view_yaw, view_yaw * ROLL_PER_YAW],
        gun_speed_delta: [gun_pitch, gun_yaw],
    }
}

/// How fast the view kick centres: fixed with no weapon, else the hip or ADS speed.
pub fn kick_angles_center_speed(
    weapon_index: i32,
    weapon_pos_frac: f32,
    hip_center_speed: f32,
    ads_center_speed: f32,
) -> f32 {
    match weapon_index {
        0 => NO_WEAPON_CENTER_SPEED,
        _ if weapon_pos_frac <= ADS_CENTER_FRAC => hip_center_speed,
        _ => ads_center_speed,
    }
}

/// One axis of the view kick, `dt` seconds on. It is pulled back toward centre, slowed
/// while heading there, stopped on reaching or passing it, and held within the limit.
fn view_kick_axis(angle: &mut f32, speed: &mut f32, dt: f32, center_speed: f32) {
    if *angle == 0.0 && *speed == 0.0 {
        return;
    }
    if *angle != 0.0 {
        let pull = if *angle <= 0.0 {
            center_speed
        } else {
            -center_speed
        };
        *speed += pull * dt;
    }
    let mut moved = *speed * dt;
    if *angle * moved < 0.0 {
        moved *= VIEW_KICK_RETURN_SHARE;
    }
    let next = *angle + moved;
    if next * *angle < 0.0 || next == 0.0 {
        *angle = 0.0;
        *speed = 0.0;
    } else if next.abs() > VIEW_KICK_LIMIT {
        *angle = if next < 0.0 {
            -VIEW_KICK_LIMIT
        } else {
            VIEW_KICK_LIMIT
        };
        *speed = 0.0;
    } else {
        *angle = next;
    }
}

/// The view kick `frametime_ms` on, in 5 ms slices.
pub fn kick_angles(
    angles: &mut [f32; 3],
    avel: &mut [f32; 3],
    frametime_ms: i32,
    center_speed: f32,
) {
    let mut left = frametime_ms;
    while left > 0 {
        let dt = left.min(STEP_MS) as f32 * MS;
        for (angle, speed) in angles.iter_mut().zip(avel.iter_mut()) {
            view_kick_axis(angle, speed, dt, center_speed);
        }
        left -= STEP_MS;
    }
}

/// The gun spring `frac` of the way from hip to ADS.
pub fn lerp_gun_kick_spring(hip: GunKickSpring, ads: GunKickSpring, frac: f32) -> GunKickSpring {
    GunKickSpring {
        accel: lerp(hip.accel, ads.accel, frac),
        speed_max: lerp(hip.speed_max, ads.speed_max, frac),
        speed_decay: lerp(hip.speed_decay, ads.speed_decay, frac),
        static_decay: lerp(hip.static_decay, ads.static_decay, frac),
    }
}

/// One axis of the gun kick, `dt` seconds on: it turns at its speed (held within
/// `ofs_cap`), the spring pulls it back, and its speed decays.
pub fn gun_recoil_single_angle(
    offset: &mut f32,
    speed: &mut f32,
    dt: f32,
    ofs_cap: f32,
    spring: GunKickSpring,
) {
    if offset.abs() < GUN_REST_OFFSET && speed.abs() < GUN_REST_SPEED {
        *offset = 0.0;
        *speed = 0.0;
        return;
    }
    let cap = ofs_cap.abs();
    *offset += *speed * dt;
    if *offset > cap {
        *offset = cap;
        if *speed > 0.0 {
            *speed = 0.0;
        }
    } else if *offset < -cap {
        *offset = -cap;
        if *speed < 0.0 {
            *speed = 0.0;
        }
    }

    let pull = spring.accel * dt;
    if *offset < 0.0 {
        *speed += pull;
    } else if *offset > 0.0 {
        *speed -= pull;
    }
    *speed -= *speed * spring.speed_decay * dt;

    let drag = spring.static_decay * dt;
    if *speed <= 0.0 {
        *speed += drag;
        if *speed > 0.0 {
            *speed = 0.0;
        }
    } else {
        *speed -= drag;
        if *speed < 0.0 {
            *speed = 0.0;
        }
    }
    if *speed > spring.speed_max {
        *speed = spring.speed_max;
    } else if *speed < -spring.speed_max {
        *speed = -spring.speed_max;
    }
}

/// The gun kick `frametime_secs` on, in 5 ms slices, while `overlay_active`.
#[allow(clippy::too_many_arguments)]
pub fn calculate_weapon_position_gun_recoil(
    state: &mut GunRecoilPlacementState,
    frametime_secs: f32,
    weapon_pos_frac: f32,
    overlay_active: bool,
    hip: GunKickSpring,
    ads: GunKickSpring,
    gun_max_pitch: f32,
    gun_max_yaw: f32,
) {
    if !overlay_active || frametime_secs <= 0.0 {
        return;
    }
    let spring = lerp_gun_kick_spring(hip, ads, weapon_pos_frac);
    let mut left = frametime_secs;
    while left > 0.0 {
        let dt = left.min(GUN_STEP_SECS);
        gun_recoil_single_angle(
            &mut state.pitch_offset,
            &mut state.pitch_speed,
            dt,
            gun_max_pitch,
            spring,
        );
        gun_recoil_single_angle(
            &mut state.yaw_offset,
            &mut state.yaw_speed,
            dt,
            gun_max_yaw,
            spring,
        );
        left -= dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(min: f32, max: f32) -> ViewKickRange {
        ViewKickRange {
            pitch_min: min,
            pitch_max: max,
            yaw_min: min,
            yaw_max: max,
        }
    }

    #[test]
    fn a_shot_kicks_up_and_rolls_against_its_yaw() {
        let gun = GunKickRange {
            pitch_min: 1.0,
            pitch_max: 3.0,
            yaw_min: 0.0,
            yaw_max: 2.0,
        };
        let shot = weapon_fire_recoil(
            range(2.0, 4.0),
            gun,
            FireRecoilPsScales::default(),
            [0.5; 4],
        );
        assert_eq!(shot.kick_avel, [-3.0, 3.0, -1.5]);
        assert_eq!(shot.gun_speed_delta, [2.0, 1.0]);
    }

    #[test]
    fn the_kick_scales_down_and_a_flag_quintuples_pitch() {
        let ps = FireRecoilPsScales {
            reduce_window_active: true,
            reduced_percent: 50.0,
            weap_flags: WEAP_FLAG_RECOIL_SCALE | WEAP_FLAG_PITCH_X5,
            recoil_scale: 50,
        };
        assert_eq!(fire_recoil_amplitude_scale(ps), 0.25);
        let shot = weapon_fire_recoil(range(4.0, 4.0), GunKickRange::default(), ps, [0.0; 4]);
        assert_eq!(shot.kick_avel[0], -5.0);
        assert_eq!(shot.kick_avel[1], 1.0);
    }

    #[test]
    fn hip_and_ads_ranges_switch_at_different_heights() {
        let (hip, ads) = (range(1.0, 1.0), range(2.0, 2.0));
        assert_eq!(fire_recoil_view_range(0.9, hip, ads), hip);
        assert_eq!(fire_recoil_view_range(1.0, hip, ads), ads);
        let (gun_hip, gun_ads) = (
            GunKickRange::default(),
            GunKickRange {
                pitch_max: 1.0,
                ..GunKickRange::default()
            },
        );
        assert_eq!(fire_recoil_gun_range(0.0, gun_hip, gun_ads), gun_hip);
        assert_eq!(fire_recoil_gun_range(0.1, gun_hip, gun_ads), gun_ads);
        assert_eq!(kick_angles_center_speed(0, 1.0, 5.0, 9.0), 2400.0);
        assert_eq!(kick_angles_center_speed(3, 0.5, 5.0, 9.0), 5.0);
        assert_eq!(kick_angles_center_speed(3, 0.6, 5.0, 9.0), 9.0);
    }

    #[test]
    fn the_view_kick_is_held_to_ten_degrees_and_comes_back() {
        let mut angles = [0.0; 3];
        let mut avel = [-4000.0, 0.0, 0.0];
        kick_angles(&mut angles, &mut avel, 5, 0.0);
        assert_eq!((angles[0], avel[0]), (-10.0, 0.0));

        let mut angles = [-1.0, 0.0, 0.0];
        let mut avel = [0.0; 3];
        for _ in 0..2000 {
            kick_angles(&mut angles, &mut avel, 50, 100.0);
        }
        assert_eq!(angles, [0.0; 3]);
    }

    #[test]
    fn a_kicked_gun_settles_back() {
        let spring = GunKickSpring {
            accel: 400.0,
            speed_max: 200.0,
            speed_decay: 10.0,
            static_decay: 20.0,
        };
        let mut state = GunRecoilPlacementState {
            pitch_speed: 150.0,
            ..Default::default()
        };
        calculate_weapon_position_gun_recoil(&mut state, 0.05, 0.0, true, spring, spring, 5.0, 5.0);
        assert!(
            state.pitch_offset > 0.0 && state.pitch_offset <= 5.0,
            "{state:?}"
        );
        for _ in 0..200 {
            calculate_weapon_position_gun_recoil(
                &mut state, 0.05, 0.0, true, spring, spring, 5.0, 5.0,
            );
        }
        assert_eq!(state, GunRecoilPlacementState::default());

        let mut idle = GunRecoilPlacementState {
            pitch_speed: 150.0,
            ..Default::default()
        };
        calculate_weapon_position_gun_recoil(&mut idle, 0.05, 0.0, false, spring, spring, 5.0, 5.0);
        assert_eq!(idle.pitch_offset, 0.0);
    }
}
