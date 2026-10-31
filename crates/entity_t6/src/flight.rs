//! BO2's `trajectory_t` and the two functions that read it:
//! `BG_EvaluateTrajectory` (where the entity is) and
//! `BG_EvaluateTrajectoryDelta` (how fast it moves).

/// BO2's `trType_t`.
pub mod path_kind {
    pub const AT_REST: u8 = 0;
    pub const BLEND: u8 = 1;
    pub const INTERPOLATE_MOVER: u8 = 2;
    pub const STRAIGHT: u8 = 3;
    pub const STRAIGHT_THEN_STOP: u8 = 4;
    pub const SINE: u8 = 5;
    pub const FALLING: u8 = 6;
    pub const LOW_GRAVITY: u8 = 7;
    pub const ACCELERATE: u8 = 8;
    pub const DECELERATE: u8 = 9;
    pub const PHYSICS: u8 = 10;
    pub const XDOLL: u8 = 11;
    pub const LIMP: u8 = 12;
    pub const FIRST_RAGDOLL: u8 = LIMP;
    pub const LIMP_FALLING: u8 = 13;
    pub const RAGDOLL_INTERPOLATE: u8 = 14;
    pub const LIMP_LAST: u8 = RAGDOLL_INTERPOLATE;
    pub const MOON_GRAVITY: u8 = 15;
    pub const COUNT: u8 = 16;
}

/// BO2's `trajectory_t`, in BO2's field order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trajectory {
    pub tr_type: u8,
    /// Start time, in ms.
    pub began_at: i32,
    /// Length in ms, for the types that stop or swing.
    pub span_ms: i32,
    pub base_point: [f32; 3],
    pub rate: [f32; 3],
}

/// The three gravity dvars the falling types read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gravity {
    /// `bg_gravity`.
    pub base_pull: f32,
    /// `bg_lowGravity`.
    pub low: f32,
    /// `bg_moonGravity`.
    pub moon: f32,
}

impl Gravity {
    /// BO2's dvar defaults.
    pub const BO2: Self = Self {
        base_pull: 800.0,
        low: 400.0,
        moon: 136.0,
    };
}

impl Default for Gravity {
    fn default() -> Self {
        Self::BO2
    }
}

/// A trajectory type BO2's function has no case for: BO2 drops the game
/// with "unknown trType".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadMotionKind(pub u8);

const MSEC_TO_SEC: f32 = 0.001;

fn mad(base: [f32; 3], scale: f32, dir: [f32; 3]) -> [f32; 3] {
    [
        dir[0] * scale + base[0],
        dir[1] * scale + base[1],
        dir[2] * scale + base[2],
    ]
}

fn scaled(v: [f32; 3], factor: f32) -> [f32; 3] {
    v.map(|c| c * factor)
}

fn length(v: [f32; 3]) -> f32 {
    libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    match length(v) {
        mag if mag == 0.0 => v,
        mag => scaled(v, 1.0 / mag),
    }
}

impl Trajectory {
    fn seconds_since_start(&self, when_ms: i32) -> f32 {
        (when_ms - self.began_at) as f32 * MSEC_TO_SEC
    }

    /// The end time the stopping types clamp to.
    fn clamped(&self, when_ms: i32) -> i32 {
        when_ms.min(self.span_ms + self.began_at)
    }

    /// BO2's `BG_EvaluateTrajectory`: the position at `when_ms` (ms).
    pub fn position_at(&self, when_ms: i32, pulls: Gravity) -> Result<[f32; 3], BadMotionKind> {
        use path_kind::*;
        let base = self.base_point;
        let delta = self.rate;
        Ok(match self.tr_type {
            AT_REST | BLEND | RAGDOLL_INTERPOLATE | PHYSICS => base,
            STRAIGHT | XDOLL | LIMP => mad(base, self.seconds_since_start(when_ms), delta),
            STRAIGHT_THEN_STOP => {
                let dt = (self.clamped(when_ms) - self.began_at) as f32 * MSEC_TO_SEC;
                mad(base, dt.max(0.0), delta)
            }
            SINE => {
                if self.span_ms == 0 {
                    return Ok(base);
                }
                let phase = (when_ms - self.began_at) as f32 / self.span_ms as f32
                    * core::f32::consts::PI
                    * 2.0;
                mad(base, libm::sinf(phase), delta)
            }
            FALLING | LIMP_FALLING | LOW_GRAVITY | MOON_GRAVITY => {
                let g = match self.tr_type {
                    LOW_GRAVITY => pulls.low,
                    MOON_GRAVITY => pulls.moon,
                    _ => pulls.base_pull,
                };
                let dt = self.seconds_since_start(when_ms);
                let mut out = mad(base, dt, delta);
                out[2] -= dt * g * 0.5 * dt;
                out
            }
            ACCELERATE | DECELERATE => {
                if self.span_ms == 0 {
                    return Ok(base);
                }
                let dt = (self.clamped(when_ms) - self.began_at) as f32 * MSEC_TO_SEC;
                let rate = length(delta) / (self.span_ms as f32 * MSEC_TO_SEC);
                let dir = normalized(delta);
                if self.tr_type == ACCELERATE {
                    mad(base, rate * 0.5 * dt * dt, dir)
                } else {
                    mad(mad(base, dt, delta), rate * -0.5 * dt * dt, dir)
                }
            }
            other => return Err(BadMotionKind(other)),
        })
    }

    /// BO2's `BG_EvaluateTrajectoryDelta`: the velocity at `when_ms` (ms).
    ///
    /// The accelerate and decelerate cases are BO2's as its code reads:
    /// delta times dt squared and delta times dt, with no normalize.
    pub fn velocity_at(
        &self,
        when_ms: i32,
        pulls: Gravity,
    ) -> Result<[f32; 3], BadMotionKind> {
        use path_kind::*;
        let delta = self.rate;
        let past_end = self.span_ms + self.began_at < when_ms;
        Ok(match self.tr_type {
            AT_REST | BLEND => [0.0; 3],
            STRAIGHT => delta,
            STRAIGHT_THEN_STOP => {
                if past_end {
                    [0.0; 3]
                } else {
                    delta
                }
            }
            SINE => {
                let phase = (when_ms - self.began_at) as f32 / self.span_ms as f32
                    * core::f32::consts::PI
                    * 2.0;
                scaled(delta, libm::cosf(phase) * 0.5)
            }
            FALLING | LOW_GRAVITY | MOON_GRAVITY => {
                let g = match self.tr_type {
                    LOW_GRAVITY => pulls.low,
                    MOON_GRAVITY => pulls.moon,
                    _ => pulls.base_pull,
                };
                let mut out = delta;
                out[2] -= g * self.seconds_since_start(when_ms);
                out
            }
            ACCELERATE => {
                if past_end {
                    [0.0; 3]
                } else {
                    let dt = self.seconds_since_start(when_ms);
                    scaled(delta, dt * dt)
                }
            }
            DECELERATE => {
                if past_end {
                    [0.0; 3]
                } else {
                    scaled(delta, self.seconds_since_start(when_ms))
                }
            }
            other => return Err(BadMotionKind(other)),
        })
    }
}

/// BO2's `BG_EvaluateTrajectory` with the gravity dvars at their defaults.
///
/// # Panics
///
/// On a type BO2's function has no case for, as BO2 drops the game with
/// "unknown trType".
#[must_use]
pub fn evaluate_trajectory(tr: &Trajectory, when_ms: i32) -> [f32; 3] {
    tr.position_at(when_ms, Gravity::BO2).expect("unknown trType")
}

/// BO2's `BG_EvaluateTrajectoryDelta` with the gravity dvars at their
/// defaults.
///
/// # Panics
///
/// On a type BO2's function has no case for ("unknown trType").
#[must_use]
pub fn evaluate_trajectory_delta(tr: &Trajectory, when_ms: i32) -> [f32; 3] {
    tr.velocity_at(when_ms, Gravity::BO2).expect("unknown trType")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(tr_type: u8) -> Trajectory {
        Trajectory {
            tr_type,
            began_at: 1000,
            span_ms: 500,
            base_point: [10.0, 20.0, 30.0],
            rate: [100.0, 0.0, 200.0],
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn numbers_are_bo2s() {
        assert_eq!(path_kind::STRAIGHT, 3);
        assert_eq!(path_kind::FALLING, 6);
        assert_eq!(path_kind::MOON_GRAVITY, 15);
        assert_eq!(path_kind::COUNT, 16);
    }

    #[test]
    fn still_types_stay_at_base() {
        let g = Gravity::BO2;
        for t in [0, 1, 10, 14] {
            assert_eq!(tr(t).position_at(9999, g), Ok([10.0, 20.0, 30.0]));
        }
    }

    #[test]
    fn linear_and_stop() {
        let g = Gravity::BO2;
        assert!(close(tr(3).position_at(2000, g).unwrap(), [110.0, 20.0, 230.0]));
        // Stops at tr_time + tr_duration, and never runs backwards.
        assert!(close(tr(4).position_at(2000, g).unwrap(), [60.0, 20.0, 130.0]));
        assert!(close(tr(4).position_at(500, g).unwrap(), [10.0, 20.0, 30.0]));
        assert_eq!(tr(4).velocity_at(1600, g), Ok([0.0; 3]));
        assert_eq!(tr(4).velocity_at(1500, g), Ok([100.0, 0.0, 200.0]));
    }

    #[test]
    fn gravity_uses_the_matching_dvar() {
        let g = Gravity::BO2;
        let one_second = |t| tr(t).position_at(2000, g).unwrap()[2];
        assert!((one_second(6) - (230.0 - 400.0)).abs() < 1e-3);
        assert!((one_second(13) - (230.0 - 400.0)).abs() < 1e-3);
        assert!((one_second(7) - (230.0 - 200.0)).abs() < 1e-3);
        assert!((one_second(15) - (230.0 - 68.0)).abs() < 1e-3);
        assert!((tr(6).velocity_at(2000, g).unwrap()[2] - (200.0 - 800.0)).abs() < 1e-3);
        // BO2 has no velocity case for ragdoll gravity.
        assert_eq!(tr(13).velocity_at(2000, g), Err(BadMotionKind(13)));
    }

    #[test]
    fn sine_swings_over_its_duration() {
        let g = Gravity::BO2;
        let quarter = tr(5).position_at(1125, g).unwrap();
        assert!(close(quarter, [110.0, 20.0, 230.0]));
        let mut zero = tr(5);
        zero.span_ms = 0;
        assert_eq!(zero.position_at(1125, g), Ok([10.0, 20.0, 30.0]));
        assert!(close(tr(5).velocity_at(1000, g).unwrap(), [50.0, 0.0, 100.0]));
    }

    #[test]
    fn accelerate_covers_half_its_delta_in_its_duration() {
        let g = Gravity::BO2;
        let mut a = tr(8);
        a.rate = [100.0, 0.0, 0.0];
        // rate 200/s/s over 0.5 s: 0.5 * 200 * 0.25 = 25.
        assert!(close(a.position_at(1500, g).unwrap(), [35.0, 20.0, 30.0]));
        assert!(close(a.position_at(5000, g).unwrap(), [35.0, 20.0, 30.0]));
        let mut d = a;
        d.tr_type = 9;
        assert!(close(d.position_at(1500, g).unwrap(), [10.0 + 50.0 - 25.0, 20.0, 30.0]));
    }

    #[test]
    fn interpolate_mover_is_unknown_to_bo2s_math() {
        let g = Gravity::BO2;
        assert_eq!(tr(2).position_at(0, g), Err(BadMotionKind(2)));
        assert_eq!(tr(2).velocity_at(0, g), Err(BadMotionKind(2)));
        assert_eq!(tr(16).position_at(0, g), Err(BadMotionKind(16)));
    }
}
