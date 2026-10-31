//! How far a landing fell.

use playerstate_t6::game::AvatarState;

use crate::bridge::TickLocals;

/// The height the player fell through, reconstructed from this frame's drop
/// and the speed the frame began with: solve `dist = motion_vec*t + a*t^2/2` for the
/// time to land, take the speed at that moment, and turn it back into a height
/// with `v^2 / 2g`. `None` when gravity is off or the frame never reached the
/// ground.
pub fn hard_landing_drop(avatar: &AvatarState, tick_vars: &TickLocals) -> Option<f32> {
    if avatar.gravity == 0 {
        return None;
    }
    let gravity = avatar.gravity as f32;
    let dropped = tick_vars.prior_tick_position[2] - avatar.origin[2];
    let start_speed = tick_vars.prior_tick_speed_vec[2];
    let speed_up_rate = -gravity;
    let half_accel = speed_up_rate * 0.5;
    let discriminant = start_speed * start_speed - 4.0 * half_accel * dropped;
    if discriminant < 0.0 {
        return None;
    }
    let two_a = half_accel * 2.0;
    if 0.0 == two_a {
        return None;
    }
    let time_to_land = (-start_speed - libm::sqrtf(discriminant)) / two_a;
    let landing_speed = -(time_to_land * speed_up_rate + start_speed);
    Some((landing_speed * landing_speed) / (gravity * 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;

    fn after_dropping(from_z: f32, to_z: f32, vz: f32, gravity: i32) -> Option<f32> {
        let mut avatar = AvatarState::ZERO;
        avatar.gravity = gravity;
        avatar.origin[2] = to_z;
        let mut frame = tick_vars(false);
        frame.prior_tick_position[2] = from_z;
        frame.prior_tick_speed_vec[2] = vz;
        hard_landing_drop(&avatar, &frame)
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
