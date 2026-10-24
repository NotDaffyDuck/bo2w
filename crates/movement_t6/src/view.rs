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

use game_math::{angle_normalize_360, angle_subtract};
use crate::bridge::{MantleCapViewContext, SHORT2ANGLE, ViewAngleClamp, mantle};
use playerstate_t6::game::{
    ENTITYNUM_NONE, PM_TYPE_DEAD, PM_TYPE_INTERMISSION, PlayerState, UserCmd, pm_flags,
};

/// BO2's `bg_ladder_yawcap`: on a ladder in the air, the view stays this
/// close to facing the ladder.
pub(crate) const LADDER_YAWCAP: f32 = 100.0;

/// linkFlags 4: a link that lets its player look around, dead or alive.
const LINK_CAN_LOOK: u32 = 4;

const PM_TYPE_NOCLIP: i32 = 2;
const PM_TYPE_UFO: i32 = 3;

/// One frame of the view: the command's angles, then whatever holds them.
pub(crate) fn update_view_angles(ps: &mut PlayerState, cmd: &UserCmd, clamp: ViewAngleClamp) {
    if ps.pm_type == PM_TYPE_INTERMISSION {
        return;
    }
    if ps.pm_type >= PM_TYPE_DEAD && ps.link_flags & LINK_CAN_LOOK == 0 {
        return;
    }
    clamp_angles(ps, cmd, clamp);
    if ps.pm_type == PM_TYPE_NOCLIP || ps.pm_type == PM_TYPE_UFO {
        return;
    }
    if ps.pm_flags & pm_flags::MANTLE != 0 {
        mantle::cap_view(ps, MantleCapViewContext::default());
        return;
    }
    if ps.pm_flags & pm_flags::LADDER != 0 && ps.ground_entity_num == ENTITYNUM_NONE {
        ladder_clamp(ps);
    }
}

/// PM_UpdateViewAngles_Clamp: pitch and yaw follow the command plus the
/// player's offset; a pitch past its limit moves the offset instead.
fn clamp_angles(ps: &mut PlayerState, cmd: &UserCmd, clamp: ViewAngleClamp) {
    for axis in 0..2 {
        let commanded = cmd.angles[axis] as f32 * SHORT2ANGLE;
        let mut angle = angle_subtract(commanded + ps.delta_angles[axis], 0.0);
        if axis == 0 && (clamp.pitch_down < angle || angle < -clamp.pitch_up) {
            angle = if clamp.pitch_down < angle {
                clamp.pitch_down
            } else {
                -clamp.pitch_up
            };
            ps.delta_angles[0] = angle - commanded;
        }
        ps.viewangles[axis] = angle_subtract(angle, 0.0);
    }
}

/// PM_UpdateViewAngles_LadderClamp: the yaw stays within the cap of facing
/// the ladder; past it, the view sits on the cap and the offset takes up the
/// difference.
fn ladder_clamp(ps: &mut PlayerState) {
    let facing = bo2_vectoyaw(ps.v_ladder_vec[0], ps.v_ladder_vec[1]) + 180.0;
    let delta = angle_subtract(facing, ps.viewangles[1]);
    let correction = if LADDER_YAWCAP < delta {
        delta - LADDER_YAWCAP
    } else if delta < -LADDER_YAWCAP {
        delta + LADDER_YAWCAP
    } else {
        return;
    };
    ps.delta_angles[1] += correction;
    let edge = if correction <= 0.0 {
        facing + LADDER_YAWCAP
    } else {
        facing - LADDER_YAWCAP
    };
    ps.viewangles[1] = angle_normalize_360(edge);
}

/// BO2's vectoyaw: the arctangent in double precision, then degrees in
/// single precision, 0 to 360.
pub(crate) fn bo2_vectoyaw(x: f32, y: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let degrees = libm::atan2(f64::from(y), f64::from(x)) as f32 * 180.0 * 0.318_309_87;
    if degrees < 0.0 { degrees + 360.0 } else { degrees }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::ANGLE2SHORT;
    use playerstate_t6::game::PM_TYPE_DEAD_LINKED;

    const CLAMP: ViewAngleClamp = ViewAngleClamp {
        pitch_up: 85.0,
        pitch_down: 85.0,
        unclamped_pitch_bit: false,
    };

    fn cmd(pitch: f32, yaw: f32, roll: f32) -> UserCmd {
        UserCmd {
            angles: [
                (pitch * ANGLE2SHORT) as i32,
                (yaw * ANGLE2SHORT) as i32,
                (roll * ANGLE2SHORT) as i32,
            ],
            ..UserCmd::default()
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn pitch_stops_at_the_limit_and_moves_the_offset() {
        let mut ps = PlayerState::ZERO;
        update_view_angles(&mut ps, &cmd(89.0, 10.0, 0.0), CLAMP);
        assert!(near(ps.viewangles[0], 85.0));
        assert!(near(ps.delta_angles[0] + 89.0, 85.0));
        assert!(near(ps.viewangles[1], 10.0));
    }

    #[test]
    fn roll_never_follows_the_command() {
        let mut ps = PlayerState::ZERO;
        ps.viewangles[2] = 3.0;
        update_view_angles(&mut ps, &cmd(0.0, 0.0, 40.0), CLAMP);
        assert_eq!(ps.viewangles[2], 3.0);
    }

    #[test]
    fn dead_and_match_end_hold_the_view() {
        for pm_type in [PM_TYPE_DEAD, PM_TYPE_DEAD_LINKED, PM_TYPE_INTERMISSION] {
            let mut ps = PlayerState::ZERO;
            ps.pm_type = pm_type;
            update_view_angles(&mut ps, &cmd(10.0, 50.0, 0.0), CLAMP);
            assert_eq!(ps.viewangles, [0.0; 3], "pm_type {pm_type}");
        }
        // A link that lets the dead look turns the view.
        let mut ps = PlayerState::ZERO;
        ps.pm_type = PM_TYPE_DEAD_LINKED;
        ps.link_flags = LINK_CAN_LOOK;
        update_view_angles(&mut ps, &cmd(10.0, 50.0, 0.0), CLAMP);
        assert!(near(ps.viewangles[1], 50.0));
    }

    #[test]
    fn ladder_in_the_air_holds_the_yaw_near_the_ladder() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        // The ladder's face points along -x, so the player faces +x (yaw 0).
        ps.v_ladder_vec = [-1.0, 0.0, 0.0];
        update_view_angles(&mut ps, &cmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(ps.viewangles[1], 100.0), "{}", ps.viewangles[1]);
        // The offset holds the view there on the next frame too.
        update_view_angles(&mut ps, &cmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(ps.viewangles[1], 100.0), "{}", ps.viewangles[1]);
        // Within the cap the view is free.
        let mut ps2 = ps;
        ps2.delta_angles = [0.0; 3];
        update_view_angles(&mut ps2, &cmd(0.0, -60.0, 0.0), CLAMP);
        assert!(near(ps2.viewangles[1], -60.0), "{}", ps2.viewangles[1]);
        // On the ground at the foot of the ladder the view is free.
        let mut ps3 = ps;
        ps3.delta_angles = [0.0; 3];
        ps3.ground_entity_num = 1022;
        update_view_angles(&mut ps3, &cmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(ps3.viewangles[1], 150.0), "{}", ps3.viewangles[1]);
    }

    #[test]
    fn noclip_only_clamps() {
        let mut ps = PlayerState::ZERO;
        ps.pm_type = PM_TYPE_NOCLIP;
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.v_ladder_vec = [-1.0, 0.0, 0.0];
        update_view_angles(&mut ps, &cmd(0.0, 150.0, 0.0), CLAMP);
        assert!(near(ps.viewangles[1], 150.0));
    }
}
