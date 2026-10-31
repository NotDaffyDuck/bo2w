//! Black Ops II's PM_SetMovementDir: which way the legs move against where
//! the player looks, -90 to 90 (the body animations read it). Lying prone it
//! is the body's heading; on a ladder it faces the ladder; otherwise it is
//! the way the player actually moved this frame.

use game_math::degree_gap;
use game_math::heading_of;
use crate::bridge::TickLocals;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, InputCommand, movement_state_bits};

use playerstate_t6::game::entity_flag_bits::TURRET_MASK;
use crate::project::scale_to_unit;
use crate::view::bo2_vectoyaw;

/// Moving less than this many units a second does not count as moving.
const MIN_MOVE_SPEED: f32 = 5.0;

/// The legs turn at most this far from the view.
const MAX_MOVEMENT_DIR: i32 = 90;

pub(crate) fn set_movement_dir(avatar: &mut AvatarState, tick_vars: &TickLocals, ucmd: &InputCommand) {
    let yaw = avatar.look_angles[1];
    let dir = if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 && avatar.e_flags & TURRET_MASK == 0 {
        degree_gap(avatar.prone_direction, yaw) as i32
    } else if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        let ladder_yaw = bo2_vectoyaw(avatar.ladder_normal_dir[0], avatar.ladder_normal_dir[1]);
        degree_gap(ladder_yaw + 180.0, yaw) as i32
    } else {
        let mut moved = [
            avatar.origin[0] - tick_vars.prior_tick_position[0],
            avatar.origin[1] - tick_vars.prior_tick_position[1],
            avatar.origin[2] - tick_vars.prior_tick_position[2],
        ];
        avatar.move_heading = 0;
        if ucmd.thrust_ahead == 0 && ucmd.thrust_lateral == 0 {
            return;
        }
        if avatar.standing_on == NO_ENTITY_SLOT {
            return;
        }
        let distance = crate::project::length(&moved);
        if distance == 0.0 || distance <= tick_vars.step_millis_elapsed * MIN_MOVE_SPEED {
            return;
        }
        scale_to_unit(&mut moved);
        let dir = degree_gap(heading_of(moved[0], moved[1]), yaw) as i32;
        if ucmd.thrust_ahead < 0 {
            // Backing up, the legs face the other way.
            degree_gap(dir as f32 + 180.0, 0.0) as i32
        } else {
            dir
        }
    };
    let dir = if dir.abs() > MAX_MOVEMENT_DIR {
        if dir >= 1 { MAX_MOVEMENT_DIR } else { -MAX_MOVEMENT_DIR }
    } else {
        dir
    };
    avatar.move_heading = i32::from(dir as i8);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_vars() -> TickLocals {
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: 1,
            standing_on_floor: 1,
            near_floor_contact: 1,
            floor_probe: [0; 11],
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    fn moving(thrust_ahead: i8, thrust_lateral: i8, moved: [f32; 3]) -> i32 {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 0;
        avatar.origin = moved;
        avatar.move_heading = 33;
        let ucmd = InputCommand {
            thrust_ahead,
            thrust_lateral,
            ..InputCommand::default()
        };
        set_movement_dir(&mut avatar, &tick_vars(), &ucmd);
        avatar.move_heading
    }

    /// BO2 truncates the angle to whole degrees, so a hair under can land
    /// one lower.
    fn near(got: i32, want: i32) {
        assert!((got - want).abs() <= 1, "got {got}, want {want}");
    }

    #[test]
    fn strafing_right_is_minus_90() {
        near(moving(0, 127, [0.0, -10.0, 0.0]), -90);
    }

    #[test]
    fn diagonal_forward_left_is_45() {
        near(moving(127, -127, [10.0, 10.0, 0.0]), 45);
    }

    #[test]
    fn backing_up_faces_forward() {
        near(moving(-127, 0, [-10.0, 0.0, 0.0]), 0);
        // Backing up to the left, the legs face front-right.
        near(moving(-127, -127, [-10.0, 10.0, 0.0]), -45);
    }

    #[test]
    fn no_keys_or_too_slow_is_zero() {
        assert_eq!(moving(0, 0, [10.0, 0.0, 0.0]), 0);
        // 0.05 s x 5 = 0.25 units is too little.
        assert_eq!(moving(127, 0, [0.25, 0.0, 0.0]), 0);
    }

    #[test]
    fn prone_uses_the_body_heading() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        avatar.prone_direction = 30.0;
        avatar.look_angles[1] = 0.0;
        set_movement_dir(&mut avatar, &tick_vars(), &InputCommand::default());
        near(avatar.move_heading, 30);
        avatar.prone_direction = 170.0;
        set_movement_dir(&mut avatar, &tick_vars(), &InputCommand::default());
        assert_eq!(avatar.move_heading, 90);
    }
}
