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

use game_math::{angle_subtract, angles_to_axis, axis_to_angles};
use playerstate_t6::game::{
    ENTITYNUM_NONE, PLAYER_CORPSE_ENTITY_BASE, PM_TYPE_DEAD_LINKED, PM_TYPE_NORMAL_LINKED,
    PlayerState, UserCmd, pm_flags,
};

use crate::gjk::{Bo2Collision, dot};

/// A player who left the mover is still carried this long (ms).
const DISSOCIATE_MOVER_DELAY: i32 = 2000;

/// The first entity number that is not a player (BO2 has 18 clients); only
/// those can be ridden.
const FIRST_RIDEABLE: i32 = PLAYER_CORPSE_ENTITY_BASE;

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
pub(crate) fn pre_step<C: Bo2Collision>(ps: &mut PlayerState, cmd: &UserCmd, collision: &C) {
    // A ladder or a ledge climb keeps the mover it picked.
    if ps.pm_flags & (pm_flags::MANTLE | pm_flags::LADDER) == 0 {
        choose_mover(ps, cmd.server_time);
    }
    let mut angles = ps.viewangles;
    adjust_position_for_mover(
        collision,
        ps.mover_entity_num,
        ps.command_time,
        cmd.server_time,
        &mut ps.origin,
        &mut angles,
    );
    ps.delta_angles[1] += angle_subtract(angles[1], ps.viewangles[1]);
    ps.delta_angles[0] += PITCH_SCALE * angle_subtract(angles[0], ps.viewangles[0]);
}

/// The mover from the ground under the player, kept 2 s after they leave it.
fn choose_mover(ps: &mut PlayerState, server_time: i32) {
    let ground = ps.ground_entity_num;
    if ground == WORLD {
        ps.mover_entity_num = ENTITYNUM_NONE;
    } else {
        if ground != ENTITYNUM_NONE && ground >= FIRST_RIDEABLE {
            if ps.mover_entity_num != ground {
                ps.mover_entity_num = ground;
                ps.mover_timestamp = server_time;
            } else if server_time > ps.mover_timestamp {
                ps.mover_timestamp = server_time;
            }
        }
        let expired = ps.mover_timestamp.wrapping_add(DISSOCIATE_MOVER_DELAY) < server_time;
        let linked = ps.pm_type == PM_TYPE_NORMAL_LINKED || ps.pm_type == PM_TYPE_DEAD_LINKED;
        if expired || linked {
            ps.mover_entity_num = ENTITYNUM_NONE;
        }
    }
    // Flying never rides anything.
    if ps.pm_type == PM_TYPE_NOCLIP || ps.pm_type == PM_TYPE_UFO {
        ps.mover_entity_num = ENTITYNUM_NONE;
    }
}

/// BO2's BG_AdjustPositionForMover: `origin` and the view `angles` keep
/// their place on the mover between its pose at `from` and at `to`. No
/// mover, the world, or a mover with no path: nothing moves.
pub(crate) fn adjust_position_for_mover<C: Bo2Collision>(
    collision: &C,
    mover: i32,
    from: i32,
    to: i32,
    origin: &mut [f32; 3],
    angles: &mut [f32; 3],
) {
    if mover == ENTITYNUM_NONE || mover == WORLD {
        return;
    }
    let (Some((from_origin, from_angles)), Some((to_origin, to_angles))) =
        (collision.mover_pose(mover, from), collision.mover_pose(mover, to))
    else {
        return;
    };
    let from_axis = angles_to_axis(from_angles);
    let to_axis = angles_to_axis(to_angles);
    let offset = [
        origin[0] - from_origin[0],
        origin[1] - from_origin[1],
        origin[2] - from_origin[2],
    ];
    let carried = to_world(to_axis, to_local(from_axis, offset));
    *origin = [
        to_origin[0] + carried[0],
        to_origin[1] + carried[1],
        to_origin[2] + carried[2],
    ];
    let view = angles_to_axis(*angles);
    let turned = view.map(|v| to_world(to_axis, to_local(from_axis, v)));
    *angles = axis_to_angles(turned);
}

/// `v` in the mover's own axes.
fn to_local(axis: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(v, axis[0]), dot(v, axis[1]), dot(v, axis[2])]
}

/// A vector given in the mover's axes, back in the world.
fn to_world(axis: [[f32; 3]; 3], local: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| local[0] * axis[0][i] + local[1] * axis[1][i] + local[2] * axis[2][i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{CollisionBackend, GroundTraceInput};
    use trace_t6::Trace;

    const LIFT: i32 = 40;

    /// Entity 40 rises 1 unit per 5 ms and turns `turn_per_ms` degrees of
    /// yaw per ms, from the origin at time 1000. Nothing else moves.
    struct Lift {
        rise_per_ms: f32,
        turn_per_ms: f32,
    }

    impl CollisionBackend for Lift {
        fn trace(&self, _input: GroundTraceInput) -> Trace {
            Trace {
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    impl Bo2Collision for Lift {
        fn mover_pose(&self, entnum: i32, time: i32) -> Option<([f32; 3], [f32; 3])> {
            (entnum == LIFT).then(|| {
                let t = (time - 1000) as f32;
                ([0.0, 0.0, self.rise_per_ms * t], [0.0, self.turn_per_ms * t, 0.0])
            })
        }
    }

    const RISING: Lift = Lift {
        rise_per_ms: 0.2,
        turn_per_ms: 0.0,
    };

    fn rider(ground: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.command_time = 1000;
        ps.ground_entity_num = ground;
        ps.origin = [5.0, 5.0, 0.0];
        ps
    }

    fn cmd(server_time: i32) -> UserCmd {
        UserCmd {
            server_time,
            ..UserCmd::default()
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn a_rising_lift_carries_the_player_up() {
        let mut ps = rider(LIFT);
        pre_step(&mut ps, &cmd(1050), &RISING);
        assert_eq!(ps.mover_entity_num, LIFT);
        assert_eq!(ps.mover_timestamp, 1050);
        assert!(close(ps.origin, [5.0, 5.0, 10.0]), "{:?}", ps.origin);
        assert_eq!(ps.delta_angles, [0.0; 3]);
    }

    #[test]
    fn a_turning_floor_swings_the_player_round_and_turns_the_view() {
        let spinner = Lift {
            rise_per_ms: 0.0,
            turn_per_ms: 90.0 / 50.0,
        };
        let mut ps = rider(LIFT);
        ps.origin = [100.0, 0.0, 0.0];
        ps.viewangles = [10.0, 0.0, 0.0];
        pre_step(&mut ps, &cmd(1050), &spinner);
        assert!(close(ps.origin, [0.0, 100.0, 0.0]), "{:?}", ps.origin);
        assert!((ps.delta_angles[1] - 90.0).abs() < 1e-3, "{:?}", ps.delta_angles);
        // The view's pitch never follows the floor (scale 0).
        assert_eq!(ps.delta_angles[0], 0.0);
    }

    #[test]
    fn a_jump_off_still_rides_for_two_seconds_then_lets_go() {
        let mut ps = rider(ENTITYNUM_NONE);
        ps.mover_entity_num = LIFT;
        ps.mover_timestamp = 1000;
        ps.command_time = 2950;
        pre_step(&mut ps, &cmd(3000), &RISING);
        assert_eq!(ps.mover_entity_num, LIFT);
        assert!(close(ps.origin, [5.0, 5.0, 10.0]), "{:?}", ps.origin);
        // The stamp is not renewed in the air.
        assert_eq!(ps.mover_timestamp, 1000);

        ps.command_time = 3000;
        pre_step(&mut ps, &cmd(3001), &RISING);
        assert_eq!(ps.mover_entity_num, ENTITYNUM_NONE);
        assert!(close(ps.origin, [5.0, 5.0, 10.0]), "{:?}", ps.origin);
    }

    #[test]
    fn landing_on_the_world_lets_go_at_once() {
        let mut ps = rider(WORLD);
        ps.mover_entity_num = LIFT;
        ps.mover_timestamp = 1000;
        pre_step(&mut ps, &cmd(1050), &RISING);
        assert_eq!(ps.mover_entity_num, ENTITYNUM_NONE);
        assert_eq!(ps.origin, [5.0, 5.0, 0.0]);
    }

    #[test]
    fn standing_on_a_player_keeps_the_old_mover_without_renewing_it() {
        let mut ps = rider(3);
        ps.mover_entity_num = LIFT;
        ps.mover_timestamp = 900;
        pre_step(&mut ps, &cmd(1050), &RISING);
        assert_eq!(ps.mover_entity_num, LIFT);
        assert_eq!(ps.mover_timestamp, 900);
    }

    #[test]
    fn linked_and_flying_players_ride_nothing() {
        for pm_type in [PM_TYPE_NORMAL_LINKED, PM_TYPE_DEAD_LINKED, PM_TYPE_NOCLIP, PM_TYPE_UFO] {
            let mut ps = rider(LIFT);
            ps.pm_type = pm_type;
            pre_step(&mut ps, &cmd(1050), &RISING);
            assert_eq!(ps.mover_entity_num, ENTITYNUM_NONE, "pm_type {pm_type}");
            assert_eq!(ps.origin, [5.0, 5.0, 0.0], "pm_type {pm_type}");
        }
    }

    #[test]
    fn a_ladder_keeps_its_mover_even_off_the_ground() {
        let mut ps = rider(ENTITYNUM_NONE);
        ps.pm_flags |= pm_flags::LADDER;
        ps.mover_entity_num = LIFT;
        // Long expired, but a climb keeps the mover it picked.
        ps.mover_timestamp = -10_000;
        pre_step(&mut ps, &cmd(1050), &RISING);
        assert_eq!(ps.mover_entity_num, LIFT);
        assert!(close(ps.origin, [5.0, 5.0, 10.0]), "{:?}", ps.origin);
    }

    #[test]
    fn something_with_no_path_carries_nothing() {
        let mut ps = rider(41);
        pre_step(&mut ps, &cmd(1050), &RISING);
        // It is still the mover (BO2 picks it), but it has no pose to follow.
        assert_eq!(ps.mover_entity_num, 41);
        assert_eq!(ps.origin, [5.0, 5.0, 0.0]);
    }
}
