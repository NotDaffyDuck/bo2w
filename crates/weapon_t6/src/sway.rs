//! Gun sway: the gun lags behind the view as it turns, on four springs (slide across, slide
//! up, pitch, yaw) that chase the turn rate.

/// The turn rate is measured per 60 Hz frame.
const FRAMES_PER_SEC: f32 = 60.0;
/// A spring this close to its target lands on it.
const SNAP: f32 = 0.001;
/// The peak of the landing scale's smoothstep (3t² − 2t³).
const SMOOTHSTEP_PEAK: f32 = 3.0;

/// A weapon's sway settings, hip or ADS.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponSwayParams {
    /// The largest turn rate (degrees a frame) the gun follows.
    pub max_angle: f32,
    /// How fast the springs chase their target.
    pub lerp_speed: f32,
    pub pitch_scale: f32,
    pub yaw_scale: f32,
    pub horiz_scale: f32,
    pub vert_scale: f32,
}

/// The four springs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwaySpringState {
    pub horiz: f32,
    pub vert: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// What the springs move the gun by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwayContribution {
    pub origin: [f32; 3],
    pub angles: [f32; 3],
}

/// `angle` brought into -180..=180.
fn wrap_180(mut angle: f32) -> f32 {
    while angle > 180.0 {
        angle -= 360.0;
    }
    while angle < -180.0 {
        angle += 360.0;
    }
    angle
}

/// `current` moved toward `target` by `speed * dt` of the gap, landing on it rather than
/// passing it.
fn chase(target: f32, current: f32, speed: f32, dt: f32) -> f32 {
    let gap = target - current;
    let step = gap * speed * dt;
    if gap.abs() <= SNAP || step.abs() >= gap.abs() {
        target
    } else {
        current + step
    }
}

/// [`chase`] for an angle, the short way round.
fn chase_angle(target: f32, current: f32, speed: f32, dt: f32) -> f32 {
    let near_target = current + wrap_180(target - current);
    wrap_180(chase(near_target, current, speed, dt))
}

/// The settings `frac` of the way from hip to ADS.
pub fn lerp_sway_params(
    hip: WeaponSwayParams,
    ads: WeaponSwayParams,
    frac: f32,
) -> WeaponSwayParams {
    let mix = |from: f32, to: f32| from + (to - from) * frac;
    WeaponSwayParams {
        max_angle: mix(hip.max_angle, ads.max_angle),
        lerp_speed: mix(hip.lerp_speed, ads.lerp_speed),
        pitch_scale: mix(hip.pitch_scale, ads.pitch_scale),
        yaw_scale: mix(hip.yaw_scale, ads.yaw_scale),
        horiz_scale: mix(hip.horiz_scale, ads.horiz_scale),
        vert_scale: mix(hip.vert_scale, ads.vert_scale),
    }
}

/// How much a shell shock or hard landing scales the sway: `shell_shock_scale` at the
/// start, easing back to 1 as `remaining_ms` of `duration_ms` runs out.
pub fn sway_shellshock_landing_scale(
    remaining_ms: i32,
    duration_ms: i32,
    shell_shock_scale: f32,
) -> f32 {
    if remaining_ms <= 0 {
        return 1.0;
    }
    let left = if duration_ms > 0 && remaining_ms < duration_ms {
        remaining_ms as f32 / duration_ms as f32
    } else {
        1.0
    };
    let ease = (SMOOTHSTEP_PEAK - (left + left)) * left * left;
    (shell_shock_scale - 1.0) * ease + 1.0
}

/// Steps the springs `dt_secs` on, chasing how far the view turned since the last frame
/// (`landing_scale` from [`sway_shellshock_landing_scale`]).
pub fn calculate_weapon_movement_sway(
    springs: &mut SwaySpringState,
    view_angles: [f32; 3],
    prev_view_angles: [f32; 3],
    params: WeaponSwayParams,
    landing_scale: f32,
    dt_secs: f32,
) {
    let frames = dt_secs * FRAMES_PER_SEC;
    if dt_secs == 0.0 || frames == 0.0 {
        return;
    }
    let per_frame = 1.0 / frames;
    let limit = params.max_angle.abs();
    let rate = |axis: usize| {
        let turned = wrap_180(view_angles[axis] - prev_view_angles[axis]) * per_frame;
        if turned > limit {
            limit
        } else if turned < -limit {
            -limit
        } else {
            turned
        }
    };
    let (pitch_rate, yaw_rate) = (rate(0), rate(1));
    let speed = params.lerp_speed;
    let target = |scale: f32, turn: f32| landing_scale * scale * turn;

    springs.horiz = chase(
        target(params.horiz_scale, yaw_rate),
        springs.horiz,
        speed,
        dt_secs,
    );
    springs.vert = chase(
        target(params.vert_scale, pitch_rate),
        springs.vert,
        speed,
        dt_secs,
    );
    springs.pitch = chase_angle(
        target(params.pitch_scale, pitch_rate),
        springs.pitch,
        speed,
        dt_secs,
    );
    springs.yaw = chase_angle(
        target(params.yaw_scale, yaw_rate),
        springs.yaw,
        speed,
        dt_secs,
    );
}

/// The springs as a gun move: slid left by `horiz` and up by `vert`, turned back against
/// the pitch and yaw springs.
pub fn sway_contribution(springs: SwaySpringState) -> SwayContribution {
    SwayContribution {
        origin: [0.0, -springs.horiz, springs.vert],
        angles: [-springs.pitch, -springs.yaw, 0.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> WeaponSwayParams {
        WeaponSwayParams {
            max_angle: 4.0,
            lerp_speed: 6.0,
            pitch_scale: 1.0,
            yaw_scale: 1.0,
            horiz_scale: 0.5,
            vert_scale: 0.5,
        }
    }

    #[test]
    fn the_gun_chases_a_turn_and_holds_it_to_the_max() {
        let mut springs = SwaySpringState::default();
        let dt = 1.0 / 60.0;
        calculate_weapon_movement_sway(&mut springs, [0.0, 2.0, 0.0], [0.0; 3], params(), 1.0, dt);
        assert!((springs.yaw - 0.2).abs() < 1e-5, "{springs:?}");
        assert!((springs.horiz - 0.1).abs() < 1e-5);
        assert_eq!(springs.pitch, 0.0);

        let mut fast = SwaySpringState::default();
        calculate_weapon_movement_sway(&mut fast, [0.0, 50.0, 0.0], [0.0; 3], params(), 1.0, dt);
        assert!((fast.yaw - 0.4).abs() < 1e-5, "{fast:?}");
    }

    #[test]
    fn a_turn_across_the_seam_is_short() {
        let mut springs = SwaySpringState::default();
        calculate_weapon_movement_sway(
            &mut springs,
            [0.0, -179.0, 0.0],
            [0.0, 179.0, 0.0],
            params(),
            1.0,
            1.0 / 60.0,
        );
        assert!((springs.yaw - 0.2).abs() < 1e-4, "{springs:?}");
    }

    #[test]
    fn no_time_moves_nothing() {
        let mut springs = SwaySpringState::default();
        calculate_weapon_movement_sway(&mut springs, [9.0, 9.0, 0.0], [0.0; 3], params(), 1.0, 0.0);
        assert_eq!(springs, SwaySpringState::default());
    }

    #[test]
    fn the_landing_scale_eases_back_to_one() {
        assert_eq!(sway_shellshock_landing_scale(0, 1000, 3.0), 1.0);
        assert_eq!(sway_shellshock_landing_scale(1000, 1000, 3.0), 3.0);
        assert_eq!(sway_shellshock_landing_scale(500, 1000, 3.0), 2.0);
    }

    #[test]
    fn params_mix_from_hip_to_ads() {
        let ads = WeaponSwayParams {
            max_angle: 8.0,
            ..params()
        };
        assert_eq!(lerp_sway_params(params(), ads, 0.5).max_angle, 6.0);
        let springs = SwaySpringState {
            horiz: 1.0,
            vert: 2.0,
            pitch: 3.0,
            yaw: 4.0,
        };
        let sway = sway_contribution(springs);
        assert_eq!(
            (sway.origin, sway.angles),
            ([0.0, -1.0, 2.0], [-3.0, -4.0, 0.0])
        );
    }
}
