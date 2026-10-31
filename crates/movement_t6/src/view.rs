//! Black Ops II's PM_UpdateViewAngles: the mouse turns the view and the pitch
//! stops at its limits. The roll never follows the command. The dead (unless
//! a link lets them look) and the end-of-match screen hold the view still.
//! Noclip and UFO stop at the pitch limits. Climbing a ledge holds the yaw
//! near the ledge, and hanging on a ladder in the air holds it near the
//! ladder. Lying down is held to the body's turn afterwards (prone_turn).
//!
//! Not here: a link's or a turret's view range (BO2 keeps it in the player
//! state; ours has no such fields, and nothing in bo2mp sets one), the lock
//! onto an entity (`player_viewLockEnt`, 0 by default) and leaning (BO2 MP
//! has no lean keys).

use game_math::wrap_degrees;
use game_math::degree_gap;
use crate::bridge::{ClimbLookLimitArgs, WORD16_TO_DEGREES, LookRotationLimit, mantle};
use playerstate_t6::game::{
    NO_ENTITY_SLOT, MOVE_MODE_INTERLUDE, AvatarState, InputCommand, movement_state_bits,
};
use playerstate_t6::game::MOVE_KIND_DEAD;

/// BO2's `bg_ladder_yawcap`: on a ladder in the air, the view stays this
/// close to facing the ladder.
pub(crate) const LADDER_YAWCAP: f32 = 100.0;

/// linkFlags 4: a link that lets its player look around, dead or alive.
const LINK_CAN_LOOK: u32 = 4;

const PM_TYPE_NOCLIP: i32 = 2;
const PM_TYPE_UFO: i32 = 3;

/// One frame of the view: the command's angles, then whatever holds them.
pub(crate) fn update_view_angles(avatar: &mut AvatarState, ucmd: &InputCommand, clamp: LookRotationLimit) {
    if avatar.movement_mode == MOVE_MODE_INTERLUDE {
        return;
    }
    if avatar.movement_mode >= MOVE_KIND_DEAD && avatar.attach_bits & LINK_CAN_LOOK == 0 {
        return;
    }
    clamp_angles(avatar, ucmd, clamp);
    if avatar.movement_mode == PM_TYPE_NOCLIP || avatar.movement_mode == PM_TYPE_UFO {
        return;
    }
    if avatar.movement_state_bits & movement_state_bits::MANTLE != 0 {
        mantle::cap_view(avatar, ClimbLookLimitArgs::default());
        return;
    }
    if avatar.movement_state_bits & movement_state_bits::LADDER != 0 && avatar.standing_on == NO_ENTITY_SLOT {
        ladder_clamp(avatar);
    }
}

/// PM_UpdateViewAngles_Clamp: pitch and yaw follow the command plus the
/// player's offset; a pitch past its limit moves the offset instead.
fn clamp_angles(avatar: &mut AvatarState, ucmd: &InputCommand, clamp: LookRotationLimit) {
    for axis in 0_usize..2 {
        let commanded = ucmd.angles[axis] as f32 * WORD16_TO_DEGREES;
        let mut angle = degree_gap(commanded + avatar.view_offset_angles[axis], 0.0);
        if axis == 0 && (clamp.tilt_lower_limit < angle || angle < -clamp.tilt_upper_limit) {
            angle = if clamp.tilt_lower_limit < angle {
                clamp.tilt_lower_limit
            } else {
                -clamp.tilt_upper_limit
            };
            avatar.view_offset_angles[0] = angle - commanded;
        }
        avatar.look_angles[axis] = degree_gap(angle, 0.0);
    }
}

/// PM_UpdateViewAngles_LadderClamp: the yaw stays within the cap of facing
/// the ladder; past it, the view sits on the cap and the offset takes up the
/// difference.
fn ladder_clamp(avatar: &mut AvatarState) {
    let facing = bo2_vectoyaw(avatar.ladder_normal_dir[0], avatar.ladder_normal_dir[1]) + 180.0;
    let delta = degree_gap(facing, avatar.look_angles[1]);
    let correction = if LADDER_YAWCAP < delta {
        delta - LADDER_YAWCAP
    } else if delta < -LADDER_YAWCAP {
        delta + LADDER_YAWCAP
    } else {
        return;
    };
    avatar.view_offset_angles[1] += correction;
    let edge = if correction <= 0.0 {
        facing + LADDER_YAWCAP
    } else {
        facing - LADDER_YAWCAP
    };
    avatar.look_angles[1] = wrap_degrees(edge);
}

/// BO2's vectoyaw: the arctangent in double precision, then degrees in
/// single precision, 0 to 360.
pub(crate) fn bo2_vectoyaw(dx: f32, dy: f32) -> f32 {
    if dx == 0.0 && dy == 0.0 {
        return 0.0;
    }
    let degrees = libm::atan2(f64::from(dy), f64::from(dx)) as f32 * 180.0 * 0.318_309_87;
    if degrees < 0.0 { degrees + 360.0 } else { degrees }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::DEGREES_TO_WORD16;
    use playerstate_t6::game::MOVE_KIND_DEAD_TETHERED;

    const CLAMP: LookRotationLimit = LookRotationLimit {
        tilt_upper_limit: 85.0,
        tilt_lower_limit: 85.0,
        free_tilt_flag: false,
    };

    fn ucmd(pitch: f32, yaw: f32, roll: f32) -> InputCommand {
        InputCommand {
            angles: [
                (pitch * DEGREES_TO_WORD16) as i32,
                (yaw * DEGREES_TO_WORD16) as i32,
                (roll * DEGREES_TO_WORD16) as i32,
            ],
            ..InputCommand::default()
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn pitch_stops_at_the_limit_and_moves_the_offset() {
        let mut avatar = AvatarState::ZERO;
        update_view_angles(&mut avatar, &ucmd(89.0, 10.0, 0.0), CLAMP);
        assert!(near(avatar.look_angles[0], 85.0));
        assert!(near(avatar.view_offset_angles[0] + 89.0, 85.0));
        assert!(near(avatar.look_angles[1], 10.0));
    }

    #[test]
    fn roll_never_follows_the_command() {
        let mut avatar = AvatarState::ZERO;
        avatar.look_angles[2] = 3.0;
        update_view_angles(&mut avatar, &ucmd(0.0, 0.0, 40.0), CLAMP);
        assert_eq!(avatar.look_angles[2], 3.0);
    }

    #[test]
    fn dead_and_match_end_hold_the_view() {
        for movement_mode in [MOVE_KIND_DEAD, MOVE_KIND_DEAD_TETHERED, MOVE_MODE_INTERLUDE] {
            let mut avatar = AvatarState::ZERO;
            avatar.movement_mode = movement_mode;
            update_view_angles(&mut avatar, &ucmd(10.0, 50.0, 0.0), CLAMP);
            assert_eq!(avatar.look_angles, [0.0; 3], "pm_type {movement_mode}");
        }
        // A link that lets the dead look turns the view.
        let mut avatar = AvatarState::ZERO;
        avatar.movement_mode = MOVE_KIND_DEAD_TETHERED;
        avatar.attach_bits = LINK_CAN_LOOK;
        update_view_angles(&mut avatar, &ucmd(10.0, 50.0, 0.0), CLAMP);
        assert!(near(avatar.look_angles[1], 50.0));
    }

    #[test]
    fn ladder_in_the_air_holds_the_yaw_near_the_ladder() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        // The ladder's face points along -x, so the player faces +x (yaw 0).
        avatar.ladder_normal_dir = [-1.0, 0.0, 0.0];
        update_view_angles(&mut avatar, &ucmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(avatar.look_angles[1], 100.0), "{}", avatar.look_angles[1]);
        // The offset holds the view there on the next frame too.
        update_view_angles(&mut avatar, &ucmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(avatar.look_angles[1], 100.0), "{}", avatar.look_angles[1]);
        // Within the cap the view is free.
        let mut ps2 = avatar;
        ps2.view_offset_angles = [0.0; 3];
        update_view_angles(&mut ps2, &ucmd(0.0, -60.0, 0.0), CLAMP);
        assert!(near(ps2.look_angles[1], -60.0), "{}", ps2.look_angles[1]);
        // On the ground at the foot of the ladder the view is free.
        let mut ps3 = avatar;
        ps3.view_offset_angles = [0.0; 3];
        ps3.standing_on = 1022;
        update_view_angles(&mut ps3, &ucmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(ps3.look_angles[1], 150.0), "{}", ps3.look_angles[1]);
    }

    #[test]
    fn noclip_only_clamps() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_mode = PM_TYPE_NOCLIP;
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.ladder_normal_dir = [-1.0, 0.0, 0.0];
        update_view_angles(&mut avatar, &ucmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(avatar.look_angles[1], 150.0));
    }
}
