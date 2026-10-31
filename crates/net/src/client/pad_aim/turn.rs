//! The right stick's turn rate, which builds up toward the stick's push instead of jumping to it.

use super::view_math::blend;
use crate::client::local_input::PlayerControlSnapshot;

/// Fastest turn, degrees a second, with the stick fully pushed and the sights down.
const YAW_RATE: f32 = 260.0;
const PITCH_RATE: f32 = 90.0;
/// The same with the sights raised.
const YAW_RATE_ADS: f32 = 90.0;
const PITCH_RATE_ADS: f32 = 55.0;
/// How fast the rate builds toward a bigger push, degrees a second each second at full zoom.
const BUILD_UP: f32 = 1200.0;

/// Move the stored turn rate (pitch, yaw) toward this frame's stick push and return the angle to
/// turn by over `dt` seconds. A push that grows builds the rate up at `BUILD_UP`; easing off or
/// reversing takes effect at once.
pub(super) fn stick_turn(
    input: &mut PlayerControlSnapshot,
    dt: f32,
    ads_lerp: f32,
    zoom: f32,
) -> [f32; 2] {
    let sensitivity = input.pad_sensitivity * blend(1.0, input.pad_ads_sensitivity, ads_lerp);
    let top_yaw = blend(YAW_RATE, YAW_RATE_ADS, ads_lerp) * zoom * sensitivity;
    let top_pitch = blend(PITCH_RATE, PITCH_RATE_ADS, ads_lerp) * zoom * sensitivity;
    let goals = [-input.pad_look[1] * top_pitch, -input.pad_look[0] * top_yaw];

    for (rate, goal) in input.pad_turn_rate.iter_mut().zip(goals) {
        let growing = goal.abs() > rate.abs() && goal.signum() == rate.signum();
        let starting = *rate == 0.0 && goal != 0.0;
        *rate = if growing || starting {
            let step = BUILD_UP * zoom * dt;
            if goal > *rate {
                (*rate + step).min(goal)
            } else {
                (*rate - step).max(goal)
            }
        } else {
            goal
        };
    }
    [input.pad_turn_rate[0] * dt, input.pad_turn_rate[1] * dt]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pushed(look: [f32; 2]) -> PlayerControlSnapshot {
        PlayerControlSnapshot {
            pad_look: look,
            ..PlayerControlSnapshot::default()
        }
    }

    #[test]
    fn a_push_from_rest_builds_one_step_at_a_time() {
        let mut input = pushed([1.0, 0.0]);
        let delta = stick_turn(&mut input, 0.01, 0.0, 1.0);
        assert!((input.pad_turn_rate[1] - -12.0).abs() < 1e-4);
        assert!((delta[1] - -0.12).abs() < 1e-5);
        assert_eq!(delta[0], 0.0);
        stick_turn(&mut input, 0.01, 0.0, 1.0);
        assert!((input.pad_turn_rate[1] - -24.0).abs() < 1e-4);
    }

    #[test]
    fn the_rate_stops_at_the_top_speed() {
        let mut input = pushed([1.0, 0.0]);
        for _ in 0..100 {
            stick_turn(&mut input, 0.1, 0.0, 1.0);
        }
        assert!((input.pad_turn_rate[1] - -260.0).abs() < 1e-3);
    }

    #[test]
    fn letting_go_stops_the_turn_at_once() {
        let mut input = pushed([1.0, 0.0]);
        input.pad_turn_rate = [0.0, -200.0];
        input.pad_look = [0.0, 0.0];
        let delta = stick_turn(&mut input, 0.01, 0.0, 1.0);
        assert_eq!(input.pad_turn_rate, [0.0, 0.0]);
        assert_eq!(delta, [0.0, 0.0]);
    }

    #[test]
    fn pushing_up_looks_up_and_the_sights_slow_the_top_speed() {
        let mut input = pushed([0.0, 1.0]);
        input.pad_turn_rate = [-10_000.0, 0.0];
        // Past the top speed and the same sign: it snaps back to the goal, which is the top speed.
        stick_turn(&mut input, 0.01, 1.0, 1.0);
        assert!((input.pad_turn_rate[0] - -55.0).abs() < 1e-4);
    }

    #[test]
    fn zoom_scales_the_speed() {
        let mut input = pushed([1.0, 0.0]);
        input.pad_turn_rate = [0.0, -1000.0];
        stick_turn(&mut input, 0.01, 0.0, 0.5);
        assert!((input.pad_turn_rate[1] - -130.0).abs() < 1e-3);
    }
}
