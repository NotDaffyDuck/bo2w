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
pub struct GunDriftTuning {
    /// The largest turn rate (degrees a frame) the gun follows.
    pub widest_cone: f32,
    /// How fast the springs chase their target.
    pub ease_rate: f32,
    pub tilt_multiplier: f32,
    pub turn_factor: f32,
    pub sideways_factor: f32,
    pub upward_factor: f32,
}

/// The four springs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriftReturnState {
    pub lateral_drift: f32,
    pub rise: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// What the springs move the gun by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriftShare {
    pub at_pos: [f32; 3],
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
fn chase(target: f32, now_val: f32, speed: f32, secs: f32) -> f32 {
    let gap = target - now_val;
    let step = gap * speed * secs;
    if gap.abs() <= SNAP || step.abs() >= gap.abs() {
        target
    } else {
        now_val + step
    }
}

/// [`chase`] for an angle, the short way round.
fn chase_angle(target: f32, now_val: f32, speed: f32, secs: f32) -> f32 {
    let near_target = now_val + wrap_180(target - now_val);
    wrap_180(chase(near_target, now_val, speed, secs))
}

/// The settings `frac` of the way from hip to ADS.
pub fn blend_drift_tuning(
    hip: GunDriftTuning,
    ads: GunDriftTuning,
    frac: f32,
) -> GunDriftTuning {
    let mix = |from: f32, to: f32| from + (to - from) * frac;
    GunDriftTuning {
        widest_cone: mix(hip.widest_cone, ads.widest_cone),
        ease_rate: mix(hip.ease_rate, ads.ease_rate),
        tilt_multiplier: mix(hip.tilt_multiplier, ads.tilt_multiplier),
        turn_factor: mix(hip.turn_factor, ads.turn_factor),
        sideways_factor: mix(hip.sideways_factor, ads.sideways_factor),
        upward_factor: mix(hip.upward_factor, ads.upward_factor),
    }
}

/// How much a shell shock or hard landing scales the sway: `shell_shock_scale` at the
/// start, easing back to 1 as `ms_to_go` of `duration_ms` runs out.
pub fn daze_touchdown_factor(
    ms_to_go: i32,
    shock_ms: i32,
    shock_scale: f32,
) -> f32 {
    if ms_to_go <= 0 {
        return 1.0;
    }
    let left = if shock_ms > 0 && ms_to_go < shock_ms {
        ms_to_go as f32 / shock_ms as f32
    } else {
        1.0
    };
    let ease = (SMOOTHSTEP_PEAK - (left + left)) * left * left;
    (shock_scale - 1.0) * ease + 1.0
}

/// Steps the springs `dt_secs` on, chasing how far the view turned since the last frame
/// (`landing_scale` from [`sway_shellshock_landing_scale`]).
pub fn calculate_weapon_movement_sway(
    spring: &mut DriftReturnState,
    look_rotation: [f32; 3],
    earlier_look_angles: [f32; 3],
    params: GunDriftTuning,
    land_scale: f32,
    step_secs: f32,
) {
    let frames = step_secs * FRAMES_PER_SEC;
    if step_secs == 0.0 || frames == 0.0 {
        return;
    }
    let per_frame = 1.0 / frames;
    let limit = params.widest_cone.abs();
    let rate = |axis: usize| {
        let turned = wrap_180(look_rotation[axis] - earlier_look_angles[axis]) * per_frame;
        if turned > limit {
            limit
        } else if turned < -limit {
            -limit
        } else {
            turned
        }
    };
    let (pitch_speed, yaw_speed) = (rate(0), rate(1));
    let speed = params.ease_rate;
    let target = |scale: f32, turn: f32| land_scale * scale * turn;

    spring.lateral_drift = chase(
        target(params.sideways_factor, yaw_speed),
        spring.lateral_drift,
        speed,
        step_secs,
    );
    spring.rise = chase(
        target(params.upward_factor, pitch_speed),
        spring.rise,
        speed,
        step_secs,
    );
    spring.pitch = chase_angle(
        target(params.tilt_multiplier, pitch_speed),
        spring.pitch,
        speed,
        step_secs,
    );
    spring.yaw = chase_angle(
        target(params.turn_factor, yaw_speed),
        spring.yaw,
        speed,
        step_secs,
    );
}

/// The springs as a gun move: slid left by `lateral_drift` and up by `vert`, turned back against
/// the pitch and yaw springs.
pub fn drift_offset(spr: DriftReturnState) -> DriftShare {
    DriftShare {
        at_pos: [0f32, -spr.lateral_drift, spr.rise],
        angles: [-spr.pitch, -spr.yaw, 0f32],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> GunDriftTuning {
        GunDriftTuning {
            widest_cone: 4.0,
            ease_rate: 6.0,
            tilt_multiplier: 1.0,
            turn_factor: 1.0,
            sideways_factor: 0.5,
            upward_factor: 0.5,
        }
    }

    #[test]
    fn the_gun_chases_a_turn_and_holds_it_to_the_max() {
        let mut springs = DriftReturnState::default();
        let dt = 1.0 / 60.0;
        calculate_weapon_movement_sway(&mut springs, [0.0, 2.0, 0.0], [0.0; 3], params(), 1.0, dt);
        assert!((springs.yaw - 0.2).abs() < 1e-5, "{springs:?}");
        assert!((springs.lateral_drift - 0.1).abs() < 1e-5);
        assert_eq!(springs.pitch, 0.0);

        let mut fast = DriftReturnState::default();
        calculate_weapon_movement_sway(&mut fast, [0.0, 50.0, 0.0], [0.0; 3], params(), 1.0, dt);
        assert!((fast.yaw - 0.4).abs() < 1e-5, "{fast:?}");
    }

    #[test]
    fn a_turn_across_the_seam_is_short() {
        let mut springs = DriftReturnState::default();
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
        let mut springs = DriftReturnState::default();
        calculate_weapon_movement_sway(&mut springs, [9.0, 9.0, 0.0], [0.0; 3], params(), 1.0, 0.0);
        assert_eq!(springs, DriftReturnState::default());
    }

    #[test]
    fn the_landing_scale_eases_back_to_one() {
        assert_eq!(daze_touchdown_factor(0, 1000, 3.0), 1.0);
        assert_eq!(daze_touchdown_factor(1000, 1000, 3.0), 3.0);
        assert_eq!(daze_touchdown_factor(500, 1000, 3.0), 2.0);
    }

    #[test]
    fn params_mix_from_hip_to_ads() {
        let ads = GunDriftTuning {
            widest_cone: 8.0,
            ..params()
        };
        assert_eq!(blend_drift_tuning(params(), ads, 0.5).widest_cone, 6.0);
        let springs = DriftReturnState {
            lateral_drift: 1.0,
            rise: 2.0,
            pitch: 3.0,
            yaw: 4.0,
        };
        let sway = drift_offset(springs);
        assert_eq!(
            (sway.at_pos, sway.angles),
            ([0.0, -1.0, 2.0], [-3.0, -4.0, 0.0])
        );
    }
}
