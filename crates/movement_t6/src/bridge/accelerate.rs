//! Pushing the player's velocity toward the speed the controls ask for.

use playerstate_t6::game::{PlayerState, pm_flags};

use crate::bridge::Pml;

/// Below this the wish speed counts as this much when working out how hard to
/// push.
const MIN_PUSH_SPEED: f32 = 100.0;

/// Accelerate toward `wishdir` at `wishspeed`. On a ladder the whole velocity
/// is steered toward the wish vector; otherwise only speed along `wishdir`
/// is added, up to `wishspeed`.
pub fn accelerate(ps: &mut PlayerState, pml: &Pml, wishdir: &[f32; 3], wishspeed: f32, accel: f32) {
    if ps.pm_flags & pm_flags::LADDER != 0 {
        steer_toward(&mut ps.velocity, pml.frametime, wishdir, wishspeed, accel);
    } else {
        add_speed_along(&mut ps.velocity, pml.frametime, wishdir, wishspeed, accel);
    }
}

/// Ladder rule: move the velocity straight at the wish vector.
fn steer_toward(
    velocity: &mut [f32; 3],
    frametime: f32,
    wishdir: &[f32; 3],
    wishspeed: f32,
    accel: f32,
) {
    let wish = [
        wishspeed * wishdir[0],
        wishdir[1] * wishspeed,
        wishspeed * wishdir[2],
    ];
    let mut to_wish = [
        wish[0] - velocity[0],
        wish[1] - velocity[1],
        wish[2] - velocity[2],
    ];
    let gap = normalize_in_place(&mut to_wish);
    let push = frametime * accel * wishspeed;
    let step = if gap < push { gap } else { push };
    for axis in 0..3 {
        velocity[axis] = to_wish[axis] * step + velocity[axis];
    }
}

/// Everywhere else: top the speed along the wish direction up to `wishspeed`.
fn add_speed_along(
    velocity: &mut [f32; 3],
    frametime: f32,
    wishdir: &[f32; 3],
    wishspeed: f32,
    accel: f32,
) {
    let along = velocity[2] * wishdir[2] + velocity[0] * wishdir[0] + velocity[1] * wishdir[1];
    let room = wishspeed - along;
    if !(room > 0.0) {
        return;
    }
    let push_speed = if wishspeed < MIN_PUSH_SPEED {
        MIN_PUSH_SPEED
    } else {
        wishspeed
    };
    let push = frametime * accel * push_speed;
    let step = if room < push { room } else { push };
    for axis in 0..3 {
        velocity[axis] = wishdir[axis] * step + velocity[axis];
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
    use crate::testing::pml;

    fn at_rest() -> PlayerState {
        PlayerState::ZERO
    }

    #[test]
    fn a_push_is_capped_by_the_time_step() {
        let mut ps = at_rest();
        accelerate(&mut ps, &pml(true), &[1.0, 0.0, 0.0], 190.0, 10.0);
        // 0.05 s * 10 * 190 = 95 units of speed this frame.
        assert!((ps.velocity[0] - 95.0).abs() < 1e-4);
        assert_eq!(ps.velocity[1], 0.0);
    }

    #[test]
    fn speed_already_at_the_wish_adds_nothing() {
        let mut ps = at_rest();
        ps.velocity = [190.0, 0.0, 0.0];
        accelerate(&mut ps, &pml(true), &[1.0, 0.0, 0.0], 190.0, 10.0);
        assert_eq!(ps.velocity, [190.0, 0.0, 0.0]);
    }

    #[test]
    fn a_slow_wish_still_pushes_as_if_it_were_a_hundred() {
        let mut ps = at_rest();
        accelerate(&mut ps, &pml(true), &[0.0, 1.0, 0.0], 40.0, 10.0);
        // The room is only 40, which is less than the 0.05 * 10 * 100 push.
        assert!((ps.velocity[1] - 40.0).abs() < 1e-4);
    }

    #[test]
    fn a_nan_wish_changes_nothing() {
        let mut ps = at_rest();
        accelerate(&mut ps, &pml(true), &[1.0, 0.0, 0.0], f32::NAN, 10.0);
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn on_a_ladder_the_velocity_is_steered_straight_at_the_wish() {
        let mut ps = at_rest();
        ps.pm_flags |= pm_flags::LADDER;
        ps.velocity = [0.0, 0.0, 50.0];
        accelerate(&mut ps, &pml(true), &[0.0, 0.0, 1.0], 100.0, 4.0);
        // Already below the wish speed: step = min(gap 50, 0.05 * 4 * 100 = 20).
        assert!((ps.velocity[2] - 70.0).abs() < 1e-4);
    }
}
