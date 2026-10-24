//! Black Ops II's PM_SetMovementDir: which way the legs move against where
//! the player looks, -90 to 90 (the body animations read it). Lying prone it
//! is the body's heading; on a ladder it faces the ladder; otherwise it is
//! the way the player actually moved this frame.

use game_math::{angle_subtract, vec_to_yaw};
use crate::bridge::Pml;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, UserCmd, pm_flags};

use crate::feel::EF_TURRET;
use crate::project::normalize;
use crate::view::bo2_vectoyaw;

/// Moving less than this many units a second does not count as moving.
const MIN_MOVE_SPEED: f32 = 5.0;

/// The legs turn at most this far from the view.
const MAX_MOVEMENT_DIR: i32 = 90;

pub(crate) fn set_movement_dir(ps: &mut PlayerState, pml: &Pml, cmd: &UserCmd) {
    let yaw = ps.viewangles[1];
    let dir = if ps.pm_flags & pm_flags::PRONE != 0 && ps.e_flags & EF_TURRET == 0 {
        angle_subtract(ps.prone_direction, yaw) as i32
    } else if ps.pm_flags & pm_flags::LADDER != 0 {
        let ladder_yaw = bo2_vectoyaw(ps.v_ladder_vec[0], ps.v_ladder_vec[1]);
        angle_subtract(ladder_yaw + 180.0, yaw) as i32
    } else {
        let mut moved = [
            ps.origin[0] - pml.previous_origin[0],
            ps.origin[1] - pml.previous_origin[1],
            ps.origin[2] - pml.previous_origin[2],
        ];
        ps.movement_dir = 0;
        if cmd.forwardmove == 0 && cmd.rightmove == 0 {
            return;
        }
        if ps.ground_entity_num == ENTITYNUM_NONE {
            return;
        }
        let distance = crate::project::length(&moved);
        if distance == 0.0 || distance <= pml.frametime * MIN_MOVE_SPEED {
            return;
        }
        normalize(&mut moved);
        let dir = angle_subtract(vec_to_yaw(moved[0], moved[1]), yaw) as i32;
        if cmd.forwardmove < 0 {
            // Backing up, the legs face the other way.
            angle_subtract(dir as f32 + 180.0, 0.0) as i32
        } else {
            dir
        }
    };
    let dir = if dir.abs() > MAX_MOVEMENT_DIR {
        if dir >= 1 { MAX_MOVEMENT_DIR } else { -MAX_MOVEMENT_DIR }
    } else {
        dir
    };
    ps.movement_dir = i32::from(dir as i8);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pml() -> Pml {
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: 0.05,
            msec: 50,
            walking: 1,
            ground_plane: 1,
            almost_ground_plane: 1,
            ground_trace: [0; 11],
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }

    fn moving(forwardmove: i8, rightmove: i8, moved: [f32; 3]) -> i32 {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 0;
        ps.origin = moved;
        ps.movement_dir = 33;
        let cmd = UserCmd {
            forwardmove,
            rightmove,
            ..UserCmd::default()
        };
        set_movement_dir(&mut ps, &pml(), &cmd);
        ps.movement_dir
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
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::PRONE;
        ps.prone_direction = 30.0;
        ps.viewangles[1] = 0.0;
        set_movement_dir(&mut ps, &pml(), &UserCmd::default());
        near(ps.movement_dir, 30);
        ps.prone_direction = 170.0;
        set_movement_dir(&mut ps, &pml(), &UserCmd::default());
        assert_eq!(ps.movement_dir, 90);
    }
}
