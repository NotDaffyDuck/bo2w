//! BO2's `trajectory_t` and the two functions that read it:
//! `BG_EvaluateTrajectory` (where the entity is) and
//! `BG_EvaluateTrajectoryDelta` (how fast it moves).

/// BO2's `trType_t`.
pub mod tr_type {
    pub const STATIONARY: u8 = 0;
    pub const INTERPOLATE: u8 = 1;
    pub const INTERPOLATE_MOVER: u8 = 2;
    pub const LINEAR: u8 = 3;
    pub const LINEAR_STOP: u8 = 4;
    pub const SINE: u8 = 5;
    pub const GRAVITY: u8 = 6;
    pub const LOW_GRAVITY: u8 = 7;
    pub const ACCELERATE: u8 = 8;
    pub const DECELERATE: u8 = 9;
    pub const PHYSICS: u8 = 10;
    pub const XDOLL: u8 = 11;
    pub const RAGDOLL: u8 = 12;
    pub const FIRST_RAGDOLL: u8 = RAGDOLL;
    pub const RAGDOLL_GRAVITY: u8 = 13;
    pub const RAGDOLL_INTERPOLATE: u8 = 14;
    pub const LAST_RAGDOLL: u8 = RAGDOLL_INTERPOLATE;
    pub const MOON_GRAVITY: u8 = 15;
    pub const COUNT: u8 = 16;
}

/// BO2's `trajectory_t`, in BO2's field order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trajectory {
    pub tr_type: u8,
    /// Start time, in ms.
    pub tr_time: i32,
    /// Length in ms, for the types that stop or swing.
    pub tr_duration: i32,
    pub tr_base: [f32; 3],
    pub tr_delta: [f32; 3],
}

/// The three gravity dvars the falling types read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gravity {
    /// `bg_gravity`.
    pub gravity: f32,
    /// `bg_lowGravity`.
    pub low: f32,
    /// `bg_moonGravity`.
    pub moon: f32,
}

impl Gravity {
    /// BO2's dvar defaults.
    pub const BO2: Self = Self {
        gravity: 800.0,
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
pub struct UnknownTrType(pub u8);

const MSEC_TO_SEC: f32 = 0.001;

fn mad(base: [f32; 3], scale: f32, dir: [f32; 3]) -> [f32; 3] {
    [
        dir[0] * scale + base[0],
        dir[1] * scale + base[1],
        dir[2] * scale + base[2],
    ]
}

fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn length(v: [f32; 3]) -> f32 {
    libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len == 0.0 {
        return v;
    }
    scale(v, 1.0 / len)
}

impl Trajectory {
    fn seconds_since_start(&self, at_time: i32) -> f32 {
        (at_time - self.tr_time) as f32 * MSEC_TO_SEC
    }

    /// The end time the stopping types clamp to.
    fn clamped(&self, at_time: i32) -> i32 {
        at_time.min(self.tr_duration + self.tr_time)
    }

    /// BO2's `BG_EvaluateTrajectory`: the position at `at_time` (ms).
    pub fn evaluate(&self, at_time: i32, gravity: Gravity) -> Result<[f32; 3], UnknownTrType> {
        use tr_type::*;
        let base = self.tr_base;
        let delta = self.tr_delta;
        Ok(match self.tr_type {
            STATIONARY | INTERPOLATE | RAGDOLL_INTERPOLATE | PHYSICS => base,
            LINEAR | XDOLL | RAGDOLL => mad(base, self.seconds_since_start(at_time), delta),
            LINEAR_STOP => {
                let dt = (self.clamped(at_time) - self.tr_time) as f32 * MSEC_TO_SEC;
                mad(base, dt.max(0.0), delta)
            }
            SINE => {
                if self.tr_duration == 0 {
                    return Ok(base);
                }
                let phase = (at_time - self.tr_time) as f32 / self.tr_duration as f32
                    * core::f32::consts::PI
                    * 2.0;
                mad(base, libm::sinf(phase), delta)
            }
            GRAVITY | RAGDOLL_GRAVITY | LOW_GRAVITY | MOON_GRAVITY => {
                let g = match self.tr_type {
                    LOW_GRAVITY => gravity.low,
                    MOON_GRAVITY => gravity.moon,
                    _ => gravity.gravity,
                };
                let dt = self.seconds_since_start(at_time);
                let mut out = mad(base, dt, delta);
                out[2] -= dt * g * 0.5 * dt;
                out
            }
            ACCELERATE | DECELERATE => {
                if self.tr_duration == 0 {
                    return Ok(base);
                }
                let dt = (self.clamped(at_time) - self.tr_time) as f32 * MSEC_TO_SEC;
                let rate = length(delta) / (self.tr_duration as f32 * MSEC_TO_SEC);
                let dir = normalized(delta);
                if self.tr_type == ACCELERATE {
                    mad(base, rate * 0.5 * dt * dt, dir)
                } else {
                    mad(mad(base, dt, delta), rate * -0.5 * dt * dt, dir)
                }
            }
            other => return Err(UnknownTrType(other)),
        })
    }

    /// BO2's `BG_EvaluateTrajectoryDelta`: the velocity at `at_time` (ms).
    ///
    /// The accelerate and decelerate cases are BO2's as its code reads:
    /// delta times dt squared and delta times dt, with no normalize.
    pub fn evaluate_delta(
        &self,
        at_time: i32,
        gravity: Gravity,
    ) -> Result<[f32; 3], UnknownTrType> {
        use tr_type::*;
        let delta = self.tr_delta;
        let past_end = self.tr_duration + self.tr_time < at_time;
        Ok(match self.tr_type {
            STATIONARY | INTERPOLATE => [0.0; 3],
            LINEAR => delta,
            LINEAR_STOP => {
                if past_end {
                    [0.0; 3]
                } else {
                    delta
                }
            }
            SINE => {
                let phase = (at_time - self.tr_time) as f32 / self.tr_duration as f32
                    * core::f32::consts::PI
                    * 2.0;
                scale(delta, libm::cosf(phase) * 0.5)
            }
            GRAVITY | LOW_GRAVITY | MOON_GRAVITY => {
                let g = match self.tr_type {
                    LOW_GRAVITY => gravity.low,
                    MOON_GRAVITY => gravity.moon,
                    _ => gravity.gravity,
                };
                let mut out = delta;
                out[2] -= g * self.seconds_since_start(at_time);
                out
            }
            ACCELERATE => {
                if past_end {
                    [0.0; 3]
                } else {
                    let dt = self.seconds_since_start(at_time);
                    scale(delta, dt * dt)
                }
            }
            DECELERATE => {
                if past_end {
                    [0.0; 3]
                } else {
                    scale(delta, self.seconds_since_start(at_time))
                }
            }
            other => return Err(UnknownTrType(other)),
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
pub fn evaluate_trajectory(tr: &Trajectory, at_time: i32) -> [f32; 3] {
    tr.evaluate(at_time, Gravity::BO2).expect("unknown trType")
}

/// BO2's `BG_EvaluateTrajectoryDelta` with the gravity dvars at their
/// defaults.
///
/// # Panics
///
/// On a type BO2's function has no case for ("unknown trType").
#[must_use]
pub fn evaluate_trajectory_delta(tr: &Trajectory, at_time: i32) -> [f32; 3] {
    tr.evaluate_delta(at_time, Gravity::BO2).expect("unknown trType")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(tr_type: u8) -> Trajectory {
        Trajectory {
            tr_type,
            tr_time: 1000,
            tr_duration: 500,
            tr_base: [10.0, 20.0, 30.0],
            tr_delta: [100.0, 0.0, 200.0],
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn numbers_are_bo2s() {
        assert_eq!(tr_type::LINEAR, 3);
        assert_eq!(tr_type::GRAVITY, 6);
        assert_eq!(tr_type::MOON_GRAVITY, 15);
        assert_eq!(tr_type::COUNT, 16);
    }

    #[test]
    fn still_types_stay_at_base() {
        let g = Gravity::BO2;
        for t in [0, 1, 10, 14] {
            assert_eq!(tr(t).evaluate(9999, g), Ok([10.0, 20.0, 30.0]));
        }
    }

    #[test]
    fn linear_and_stop() {
        let g = Gravity::BO2;
        assert!(close(tr(3).evaluate(2000, g).unwrap(), [110.0, 20.0, 230.0]));
        // Stops at tr_time + tr_duration, and never runs backwards.
        assert!(close(tr(4).evaluate(2000, g).unwrap(), [60.0, 20.0, 130.0]));
        assert!(close(tr(4).evaluate(500, g).unwrap(), [10.0, 20.0, 30.0]));
        assert_eq!(tr(4).evaluate_delta(1600, g), Ok([0.0; 3]));
        assert_eq!(tr(4).evaluate_delta(1500, g), Ok([100.0, 0.0, 200.0]));
    }

    #[test]
    fn gravity_uses_the_matching_dvar() {
        let g = Gravity::BO2;
        let one_second = |t| tr(t).evaluate(2000, g).unwrap()[2];
        assert!((one_second(6) - (230.0 - 400.0)).abs() < 1e-3);
        assert!((one_second(13) - (230.0 - 400.0)).abs() < 1e-3);
        assert!((one_second(7) - (230.0 - 200.0)).abs() < 1e-3);
        assert!((one_second(15) - (230.0 - 68.0)).abs() < 1e-3);
        assert!((tr(6).evaluate_delta(2000, g).unwrap()[2] - (200.0 - 800.0)).abs() < 1e-3);
        // BO2 has no velocity case for ragdoll gravity.
        assert_eq!(tr(13).evaluate_delta(2000, g), Err(UnknownTrType(13)));
    }

    #[test]
    fn sine_swings_over_its_duration() {
        let g = Gravity::BO2;
        let quarter = tr(5).evaluate(1125, g).unwrap();
        assert!(close(quarter, [110.0, 20.0, 230.0]));
        let mut zero = tr(5);
        zero.tr_duration = 0;
        assert_eq!(zero.evaluate(1125, g), Ok([10.0, 20.0, 30.0]));
        assert!(close(tr(5).evaluate_delta(1000, g).unwrap(), [50.0, 0.0, 100.0]));
    }

    #[test]
    fn accelerate_covers_half_its_delta_in_its_duration() {
        let g = Gravity::BO2;
        let mut a = tr(8);
        a.tr_delta = [100.0, 0.0, 0.0];
        // rate 200/s/s over 0.5 s: 0.5 * 200 * 0.25 = 25.
        assert!(close(a.evaluate(1500, g).unwrap(), [35.0, 20.0, 30.0]));
        assert!(close(a.evaluate(5000, g).unwrap(), [35.0, 20.0, 30.0]));
        let mut d = a;
        d.tr_type = 9;
        assert!(close(d.evaluate(1500, g).unwrap(), [10.0 + 50.0 - 25.0, 20.0, 30.0]));
    }

    #[test]
    fn interpolate_mover_is_unknown_to_bo2s_math() {
        let g = Gravity::BO2;
        assert_eq!(tr(2).evaluate(0, g), Err(UnknownTrType(2)));
        assert_eq!(tr(2).evaluate_delta(0, g), Err(UnknownTrType(2)));
        assert_eq!(tr(16).evaluate(0, g), Err(UnknownTrType(16)));
    }
}
