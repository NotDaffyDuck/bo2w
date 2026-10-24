//! How far a landing fell.

use playerstate_t6::game::PlayerState;

use crate::bridge::Pml;

/// The height the player fell through, reconstructed from this frame's drop
/// and the speed the frame began with: solve `dist = vel*t + a*t^2/2` for the
/// time to land, take the speed at that moment, and turn it back into a height
/// with `v^2 / 2g`. `None` when gravity is off or the frame never reached the
/// ground.
pub fn crash_land_fall_height(ps: &PlayerState, pml: &Pml) -> Option<f32> {
    if ps.gravity == 0 {
        return None;
    }
    let gravity = ps.gravity as f32;
    let dropped = pml.previous_origin[2] - ps.origin[2];
    let start_speed = pml.previous_velocity[2];
    let accel = -gravity;
    let half_accel = accel * 0.5;
    let discriminant = start_speed * start_speed - 4.0 * half_accel * dropped;
    if discriminant < 0.0 {
        return None;
    }
    let two_a = half_accel * 2.0;
    if two_a == 0.0 {
        return None;
    }
    let time_to_land = (-start_speed - libm::sqrtf(discriminant)) / two_a;
    let landing_speed = -(time_to_land * accel + start_speed);
    Some((landing_speed * landing_speed) / (gravity * 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pml;

    fn after_dropping(from_z: f32, to_z: f32, vz: f32, gravity: i32) -> Option<f32> {
        let mut ps = PlayerState::ZERO;
        ps.gravity = gravity;
        ps.origin[2] = to_z;
        let mut frame = pml(false);
        frame.previous_origin[2] = from_z;
        frame.previous_velocity[2] = vz;
        crash_land_fall_height(&ps, &frame)
    }

    #[test]
    fn a_standing_start_drop_recovers_the_height() {
        let fell = after_dropping(100.0, 0.0, 0.0, 800).unwrap();
        assert!((fell - 100.0).abs() < 1e-3, "{fell}");
    }

    #[test]
    fn no_gravity_means_no_fall() {
        assert_eq!(after_dropping(100.0, 0.0, 0.0, 0), None);
    }

    #[test]
    fn rising_further_than_the_speed_allows_is_not_a_landing() {
        assert_eq!(after_dropping(0.0, 500.0, 10.0, 800), None);
    }
}
