//! Pushing the player's velocity toward the speed the controls ask for.

use playerstate_t6::game::{AvatarState, movement_state_bits};

use crate::bridge::TickLocals;

/// Below this the wish speed counts as this much when working out how hard to
/// push.
const MIN_PUSH_SPEED: f32 = 100.0;

/// Accelerate toward `intended_heading` at `intended_pace`. On a ladder the whole velocity
/// is steered toward the wish vector; otherwise only speed along `intended_heading`
/// is added, up to `intended_pace`.
pub fn accelerate(avatar: &mut AvatarState, tick_vars: &TickLocals, intended_heading: &[f32; 3], intended_pace: f32, speed_up_rate: f32) {
    if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        steer_toward(&mut avatar.velocity, tick_vars.step_millis_elapsed, intended_heading, intended_pace, speed_up_rate);
    } else {
        add_speed_along(&mut avatar.velocity, tick_vars.step_millis_elapsed, intended_heading, intended_pace, speed_up_rate);
    }
}

/// Ladder rule: move the velocity straight at the wish vector.
fn steer_toward(
    vel: &mut [f32; 3],
    step_millis_elapsed: f32,
    intended_heading: &[f32; 3],
    intended_pace: f32,
    speed_up_rate: f32,
) {
    let wish = [
        intended_pace * intended_heading[0],
        intended_heading[1] * intended_pace,
        intended_pace * intended_heading[2],
    ];
    let mut to_wish = [
        wish[0] - vel[0],
        wish[1] - vel[1],
        wish[2] - vel[2],
    ];
    let gap = normalize_in_place(&mut to_wish);
    let push = step_millis_elapsed * speed_up_rate * intended_pace;
    let step = if gap < push { gap } else { push };
    for axis in 0_usize..3 {
        vel[axis] = to_wish[axis] * step + vel[axis];
    }
}

/// Everywhere else: top the speed along the wish direction up to `intended_pace`.
fn add_speed_along(
    vel: &mut [f32; 3],
    step_millis_elapsed: f32,
    intended_heading: &[f32; 3],
    intended_pace: f32,
    speed_up_rate: f32,
) {
    let along = vel[2] * intended_heading[2] + vel[0] * intended_heading[0] + vel[1] * intended_heading[1];
    let room = intended_pace - along;
    if !(room > 0.0) {
        return;
    }
    let push_speed = if intended_pace < MIN_PUSH_SPEED {
        MIN_PUSH_SPEED
    } else {
        intended_pace
    };
    let push = step_millis_elapsed * speed_up_rate * push_speed;
    let step = if room < push { room } else { push };
    for axis in 0_usize..3 {
        vel[axis] = intended_heading[axis] * step + vel[axis];
    }
}

/// Scale `v` to unit length and return what its length was; a zero vector is
/// left alone.
fn normalize_in_place(v: &mut [f32; 3]) -> f32 {
    let length = libm::sqrtf(v[2] * v[2] + v[0] * v[0] + v[1] * v[1]);
    let scale = 1.0 / if length <= 0.0 { 1.0 } else { length };
    for part in v.iter_mut() {
        *part = scale * *part;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;

    fn at_rest() -> AvatarState {
        AvatarState::ZERO
    }

    #[test]
    fn a_push_is_capped_by_the_time_step() {
        let mut avatar = at_rest();
        accelerate(&mut avatar, &tick_vars(true), &[1.0, 0.0, 0.0], 190.0, 10.0);
        // 0.05 s * 10 * 190 = 95 units of speed this frame.
        assert!((avatar.velocity[0] - 95.0).abs() < 1e-4);
        assert_eq!(avatar.velocity[1], 0.0);
    }

    #[test]
    fn speed_already_at_the_wish_adds_nothing() {
        let mut avatar = at_rest();
        avatar.velocity = [190.0, 0.0, 0.0];
        accelerate(&mut avatar, &tick_vars(true), &[1.0, 0.0, 0.0], 190.0, 10.0);
        assert_eq!(avatar.velocity, [190.0, 0.0, 0.0]);
    }

    #[test]
    fn a_slow_wish_still_pushes_as_if_it_were_a_hundred() {
        let mut avatar = at_rest();
        accelerate(&mut avatar, &tick_vars(true), &[0.0, 1.0, 0.0], 40.0, 10.0);
        // The room is only 40, which is less than the 0.05 * 10 * 100 push.
        assert!((avatar.velocity[1] - 40.0).abs() < 1e-4);
    }

    #[test]
    fn a_nan_wish_changes_nothing() {
        let mut avatar = at_rest();
        accelerate(&mut avatar, &tick_vars(true), &[1.0, 0.0, 0.0], f32::NAN, 10.0);
        assert_eq!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn on_a_ladder_the_velocity_is_steered_straight_at_the_wish() {
        let mut avatar = at_rest();
        avatar.movement_state_bits |= movement_state_bits::LADDER;
        avatar.velocity = [0.0_f32, 0.0, 50.0];
        accelerate(&mut avatar, &tick_vars(true), &[0.0, 0.0, 1.0], 100.0, 4.0);
        // Already below the wish speed: step = min(gap 50, 0.05 * 4 * 100 = 20).
        assert!((avatar.velocity[2] - 70.0).abs() < 1e-4);
    }
}
