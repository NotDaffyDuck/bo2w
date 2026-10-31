//! Black Ops II's moving platforms (Pmove's mover step and
//! BG_AdjustPositionForMover). Before each command moves the player, the
//! platform they ride (a lift, a turning floor, a ladder on a moving thing)
//! carries them, and their view turns with it, by however far it moved
//! since the last command.
//!
//! This is the only thing that carries a player: BO2's server pushes props,
//! corpses and dropped things off a moving brush (G_MoverPush) but never a
//! player, so nothing here doubles a push.
//!
//! The rider is picked from the ground: standing on a non-player entity makes
//! it the mover and stamps the time. Ladders and ledge climbs pick theirs in
//! `ladder` and `mantle` and stamp the time each frame. A player who steps or
//! jumps off is still carried for 2 s (`DISSOCIATE_MOVER_DELAY`), so a jump on
//! a lift lands on the lift.

use game_math::degree_gap;
use game_math::euler_to_rows;
use game_math::rows_to_euler;
use playerstate_t6::game::{
    AvatarState, InputCommand, movement_state_bits,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::BODY_ENTITY_FIRST_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD_TETHERED;
use playerstate_t6::game::MOVE_MODE_WALK_ATTACHED;

use crate::gjk::{Bo2Collision, dot};

/// A player who left the mover is still carried this long (ms).
const DISSOCIATE_MOVER_DELAY: i32 = 2000;

/// The first entity number that is not a player (BO2 has 18 clients); only
/// those can be ridden.
const FIRST_RIDEABLE: i32 = BODY_ENTITY_FIRST_SLOT;

/// How much of the mover's pitch turns the view (BO2's
/// bg_movingPlatformPitchScale, 0 by default: a tilting floor never tips the
/// view, only its turning does).
const PITCH_SCALE: f32 = 0.0;

const PM_TYPE_NOCLIP: i32 = 2;
const PM_TYPE_UFO: i32 = 3;

/// The world as an entity number (it never moves).
const WORLD: i32 = trace_t6::ENTITYNUM_WORLD as i32;

/// Before the move: pick the mover the player rides, then carry them and
/// turn their view by how it moved from the last command to this one.
pub(crate) fn pre_step<C: Bo2Collision>(avatar: &mut AvatarState, ucmd: &InputCommand, solids: &C) {
    // A ladder or a ledge climb keeps the mover it picked.
    if avatar.movement_state_bits & (movement_state_bits::MANTLE | movement_state_bits::LADDER) == 0 {
        choose_mover(avatar, ucmd.authority_clock_ms);
    }
    let mut angles = avatar.look_angles;
    adjust_position_for_mover(
        solids,
        avatar.mover_entity_num,
        avatar.last_cmd_ms,
        ucmd.authority_clock_ms,
        &mut avatar.origin,
        &mut angles,
    );
    avatar.view_offset_angles[1] += degree_gap(angles[1], avatar.look_angles[1]);
    avatar.view_offset_angles[0] += PITCH_SCALE * degree_gap(angles[0], avatar.look_angles[0]);
}

/// The mover from the ground under the player, kept 2 s after they leave it.
fn choose_mover(avatar: &mut AvatarState, authority_clock_ms: i32) {
    let ground = avatar.standing_on;
    if ground == WORLD {
        avatar.mover_entity_num = NO_ENTITY_SLOT;
    } else {
        if ground != NO_ENTITY_SLOT && ground >= FIRST_RIDEABLE {
            if avatar.mover_entity_num != ground {
                avatar.mover_entity_num = ground;
                avatar.mover_timestamp = authority_clock_ms;
            } else if authority_clock_ms > avatar.mover_timestamp {
                avatar.mover_timestamp = authority_clock_ms;
            }
        }
        let expired = avatar.mover_timestamp.wrapping_add(DISSOCIATE_MOVER_DELAY) < authority_clock_ms;
        let linked = avatar.movement_mode == MOVE_MODE_WALK_ATTACHED || avatar.movement_mode == MOVE_KIND_DEAD_TETHERED;
        if expired || linked {
            avatar.mover_entity_num = NO_ENTITY_SLOT;
        }
    }
    // Flying never rides anything.
    if avatar.movement_mode == PM_TYPE_NOCLIP || avatar.movement_mode == PM_TYPE_UFO {
        avatar.mover_entity_num = NO_ENTITY_SLOT;
    }
}

/// BO2's BG_AdjustPositionForMover: `origin` and the view `angles` keep
/// their place on the mover between its pose at `from` and at `to`. No
/// mover, the world, or a mover with no path: nothing moves.
pub(crate) fn adjust_position_for_mover<C: Bo2Collision>(
    solids: &C,
    mover: i32,
    from: i32,
    to: i32,
    at_pos: &mut [f32; 3],
    view_ang: &mut [f32; 3],
) {
    if mover == NO_ENTITY_SLOT || mover == WORLD {
        return;
    }
    let (Some((from_origin, from_angles)), Some((to_origin, to_angles))) =
        (solids.mover_pose(mover, from), solids.mover_pose(mover, to))
    else {
        return;
    };
    let from_axis = euler_to_rows(from_angles);
    let to_axis = euler_to_rows(to_angles);
    let offset = [
        at_pos[0] - from_origin[0],
        at_pos[1] - from_origin[1],
        at_pos[2] - from_origin[2],
    ];
    let carried = to_world(to_axis, to_local(from_axis, offset));
    *at_pos = [
        to_origin[0] + carried[0],
        to_origin[1] + carried[1],
        to_origin[2] + carried[2],
    ];
    let view = euler_to_rows(*view_ang);
    let turned = view.map(|v| to_world(to_axis, to_local(from_axis, v)));
    *view_ang = rows_to_euler(turned);
}

/// `v` in the mover's own axes.
fn to_local(axis: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(v, axis[0]), dot(v, axis[1]), dot(v, axis[2])]
}

/// A vector given in the mover's axes, back in the world.
fn to_world(axis: [[f32; 3]; 3], local_pt: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| local_pt[0] * axis[0][i] + local_pt[1] * axis[1][i] + local_pt[2] * axis[2][i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{SolidProbe, SweepAsk};
    use trace_t6::Trace;
    use playerstate_t6::game::MOVE_KIND_DEAD_TETHERED;

    const LIFT: i32 = 40;

    /// Entity 40 rises 1 unit per 5 ms and turns `turn_per_ms` degrees of
    /// yaw per ms, from the origin at time 1000. Nothing else moves.
    struct Lift {
        rise_per_ms: f32,
        turn_per_ms: f32,
    }

    impl SolidProbe for Lift {
        fn trace(&self, _input: SweepAsk) -> Trace {
            Trace::new()
        }
    }

    impl Bo2Collision for Lift {
        fn mover_pose(&self, ent: i32, time: i32) -> Option<([f32; 3], [f32; 3])> {
            (ent == LIFT).then(|| {
                let t = (time - 1000) as f32;
                ([0.0, 0.0, self.rise_per_ms * t], [0.0, self.turn_per_ms * t, 0.0])
            })
        }
    }

    const RISING: Lift = Lift {
        rise_per_ms: 0.2,
        turn_per_ms: 0.0,
    };

    fn rider(ground: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.last_cmd_ms = 1000;
        avatar.standing_on = ground;
        avatar.origin = [5.0, 5.0, 0.0];
        avatar
    }

    fn ucmd(authority_clock_ms: i32) -> InputCommand {
        InputCommand {
            authority_clock_ms,
            ..InputCommand::default()
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn a_rising_lift_carries_the_player_up() {
        let mut avatar = rider(LIFT);
        pre_step(&mut avatar, &ucmd(1050), &RISING);
        assert_eq!(avatar.mover_entity_num, LIFT);
        assert_eq!(avatar.mover_timestamp, 1050);
        assert!(close(avatar.origin, [5.0, 5.0, 10.0]), "{:?}", avatar.origin);
        assert_eq!(avatar.view_offset_angles, [0.0; 3]);
    }

    #[test]
    fn a_turning_floor_swings_the_player_round_and_turns_the_view() {
        let spinner = Lift {
            rise_per_ms: 0.0,
            turn_per_ms: 90.0 / 50.0,
        };
        let mut avatar = rider(LIFT);
        avatar.origin = [100.0, 0.0, 0.0];
        avatar.look_angles = [10.0, 0.0, 0.0];
        pre_step(&mut avatar, &ucmd(1050), &spinner);
        assert!(close(avatar.origin, [0.0, 100.0, 0.0]), "{:?}", avatar.origin);
        assert!((avatar.view_offset_angles[1] - 90.0).abs() < 1e-3, "{:?}", avatar.view_offset_angles);
        // The view's pitch never follows the floor (scale 0).
        assert_eq!(avatar.view_offset_angles[0], 0.0);
    }

    #[test]
    fn a_jump_off_still_rides_for_two_seconds_then_lets_go() {
        let mut avatar = rider(NO_ENTITY_SLOT);
        avatar.mover_entity_num = LIFT;
        avatar.mover_timestamp = 1000;
        avatar.last_cmd_ms = 2950;
        pre_step(&mut avatar, &ucmd(3000), &RISING);
        assert_eq!(avatar.mover_entity_num, LIFT);
        assert!(close(avatar.origin, [5.0, 5.0, 10.0]), "{:?}", avatar.origin);
        // The stamp is not renewed in the air.
        assert_eq!(avatar.mover_timestamp, 1000);

        avatar.last_cmd_ms = 3000;
        pre_step(&mut avatar, &ucmd(3001), &RISING);
        assert_eq!(avatar.mover_entity_num, NO_ENTITY_SLOT);
        assert!(close(avatar.origin, [5.0, 5.0, 10.0]), "{:?}", avatar.origin);
    }

    #[test]
    fn landing_on_the_world_lets_go_at_once() {
        let mut avatar = rider(WORLD);
        avatar.mover_entity_num = LIFT;
        avatar.mover_timestamp = 1000;
        pre_step(&mut avatar, &ucmd(1050), &RISING);
        assert_eq!(avatar.mover_entity_num, NO_ENTITY_SLOT);
        assert_eq!(avatar.origin, [5.0, 5.0, 0.0]);
    }

    #[test]
    fn standing_on_a_player_keeps_the_old_mover_without_renewing_it() {
        let mut avatar = rider(3);
        avatar.mover_entity_num = LIFT;
        avatar.mover_timestamp = 900;
        pre_step(&mut avatar, &ucmd(1050), &RISING);
        assert_eq!(avatar.mover_entity_num, LIFT);
        assert_eq!(avatar.mover_timestamp, 900);
    }

    #[test]
    fn linked_and_flying_players_ride_nothing() {
        for movement_mode in [MOVE_MODE_WALK_ATTACHED, MOVE_KIND_DEAD_TETHERED, PM_TYPE_NOCLIP, PM_TYPE_UFO] {
            let mut avatar = rider(LIFT);
            avatar.movement_mode = movement_mode;
            pre_step(&mut avatar, &ucmd(1050), &RISING);
            assert_eq!(avatar.mover_entity_num, NO_ENTITY_SLOT, "pm_type {movement_mode}");
            assert_eq!(avatar.origin, [5.0, 5.0, 0.0], "pm_type {movement_mode}");
        }
    }

    #[test]
    fn a_ladder_keeps_its_mover_even_off_the_ground() {
        let mut avatar = rider(NO_ENTITY_SLOT);
        avatar.movement_state_bits |= movement_state_bits::LADDER;
        avatar.mover_entity_num = LIFT;
        // Long expired, but a climb keeps the mover it picked.
        avatar.mover_timestamp = -10_000;
        pre_step(&mut avatar, &ucmd(1050), &RISING);
        assert_eq!(avatar.mover_entity_num, LIFT);
        assert!(close(avatar.origin, [5.0, 5.0, 10.0]), "{:?}", avatar.origin);
    }

    #[test]
    fn something_with_no_path_carries_nothing() {
        let mut avatar = rider(41);
        pre_step(&mut avatar, &ucmd(1050), &RISING);
        // It is still the mover (BO2 picks it), but it has no pose to follow.
        assert_eq!(avatar.mover_entity_num, 41);
        assert_eq!(avatar.origin, [5.0, 5.0, 0.0]);
    }
}
