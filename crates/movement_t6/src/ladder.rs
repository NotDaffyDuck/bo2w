//! Black Ops II's ladders: PM_CheckLadderMove grabs and lets go of one,
//! PM_LadderMove climbs it. The grab is the legacy, with BO2's extras: a dead
//! player also leaves the ground this frame, the riot shield going up, down
//! or planted lets go, and a ladder on a moving thing makes that thing the
//! platform the climber rides (mover.rs). The climb differs more: the sideways direction stays the length the
//! ladder's face leaves it, the fast-climb perk climbs 1.75 times as fast,
//! sideways drift dies away when no sideways key is held, and the legs face
//! the ladder (at most 75 degrees from the view).
//!
//! Letting go in the air because the ladder is no longer there (not for
//! lying down, a fresh jump, the riot shield or dying) plays the legs' jump,
//! as a jump off one does (`BodyAnims`).
//!
//! Not here: a gun that grants the fast-climb perk, water and the console's
//! ladder controls. Without the BO2 gun brain the riot shield's states
//! cannot be seen.

use game_math::angle_subtract;
use crate::bridge::{
    CheckLadderContext, LADDER_JUMP_BLOCK_MS, LADDER_TRACE_DIST_AIR, LADDER_TRACE_DIST_WALK,
    LadderAttachBackend, LadderTraceHit, MoveBounds, Pml, SURF_LADDER, WalkMoveContext, accelerate,
    ladder_attract_velocity, step_slide_move,
};
use playerstate_t6::game::{ENTITYNUM_NONE, PM_TYPE_DEAD, PlayerState, UserCmd, pm_flags};

use crate::air::{air_move, cmd_scale};
use crate::frame::BodyAnims;
use crate::gjk::Bo2Collision;
use crate::view::bo2_vectoyaw;

/// The fast-climb perk (BO2's specialty_fastladderclimb, perk 17), in the
/// player's first perk word.
pub const PERK_FASTLADDERCLIMB: u32 = 1 << 17;

/// BO2's `perk_fastLadderClimbMultiplier`.
const FAST_LADDER_CLIMB_SCALE: f32 = 1.75;

/// A timer from a landing or a push (pm_flags 0x80 and 0x100, the ones
/// PM_DropTimers ends) keeps the player off ladders while it runs.
const TIMER_FLAGS: u32 = 0x180;

/// The prone view heights (PM_GetEffectiveStance): a prone player cannot
/// hold a ladder.
const PRONE_VIEW_HEIGHTS: [i32; 2] = [22, 11];

/// Looking up or down sets the climb: (pitch's sine + 0.25) * 2.5, -1 to 1.
const UPSCALE_BIAS: f32 = 0.25;
const UPSCALE_MUL: f32 = 2.5;

/// The keys' push: up and down at half the speed, sideways at a fifth.
const CLIMB_WISH_SCALE: f32 = 0.5;
const SIDE_WISH_SCALE: f32 = 0.2;
const LADDER_ACCEL: f32 = 9.0;

/// Sideways drift with no sideways key falls by 16 times the frame's time.
const SIDE_DAMPING: f32 = 16.0;

/// On a ladder the legs turn at most this far from the view.
const MOVEMENT_DIR_CAP: i32 = 75;

/// Which entity the last ladder trace hit, so a ladder on a moving thing
/// carries the climber.
pub(crate) trait LadderHitEntity {
    /// The entity the last `ladder_trace` hit (the world counts as one), or
    /// `None` when it hit no entity.
    fn last_hit_entity(&self) -> Option<i32>;
}

/// PM_CheckLadderMove: hold on, grab, or let go (and fall). True when a
/// player hanging on in the air lost the ladder: the legs play the jump.
pub(crate) fn check_ladder_move(
    ps: &mut PlayerState,
    pml: &mut Pml,
    context: CheckLadderContext,
    backend: &mut (impl LadderAttachBackend + LadderHitEntity),
) -> bool {
    // On the ground, or swimming, a fall off a ladder is over.
    if context.walking || ps.waterlevel > 3 {
        ps.pm_flags &= !pm_flags::LADDER_FALL;
    }
    if ps.pm_time != 0 && ps.pm_flags & pm_flags::LADDER == 0 && ps.pm_flags & TIMER_FLAGS != 0 {
        return false;
    }
    let dist = if context.walking {
        LADDER_TRACE_DIST_WALK
    } else {
        LADDER_TRACE_DIST_AIR
    };
    // Hanging on in the air, look for the ladder behind its face; otherwise
    // straight ahead.
    let hanging = ps.pm_flags & pm_flags::LADDER != 0 && ps.ground_entity_num == ENTITYNUM_NONE;
    let dir = if hanging {
        let v = ps.v_ladder_vec;
        [-v[0], -v[1], -v[2]]
    } else {
        let [x, y] = context.forward_xy;
        let mut len = libm::sqrtf(x * x + y * y);
        if len <= 0.0 {
            len = 1.0;
        }
        let inv = 1.0 / len;
        [x * inv, y * inv, 0.0]
    };
    if ps.pm_type >= PM_TYPE_DEAD {
        ps.ground_entity_num = ENTITYNUM_NONE;
        pml.walking = 0;
        pml.ground_plane = 0;
        pml.almost_ground_plane = 0;
        let_go(ps);
        return false;
    }
    if ps.pm_flags & pm_flags::LADDER_FALL != 0
        || PRONE_VIEW_HEIGHTS.contains(&ps.view_height_target)
        || context.server_time.wrapping_sub(ps.jump_time) < LADDER_JUMP_BLOCK_MS
        || crate::gun::riot_shield(ps)
    {
        let_go(ps);
        return false;
    }
    // On the ground, only walking into the ladder grabs it.
    let grabbed = backend
        .ladder_trace(ps.origin, dir, dist)
        .filter(|hit| is_ladder(hit) && (!context.walking || context.forwardmove > 0));
    let Some(hit) = grabbed else {
        let_go(ps);
        return hanging;
    };
    if ps.pm_flags & pm_flags::LADDER != 0 {
        ride(ps, backend);
        return false;
    }
    // A new ladder: its face is the one the player looks back at.
    ps.v_ladder_vec = hit.normal;
    let back = [-hit.normal[0], -hit.normal[1], -hit.normal[2]];
    if backend.ladder_trace(ps.origin, back, dist).is_some_and(|hit| is_ladder(&hit)) {
        ride(ps, backend);
        ps.pm_flags |= pm_flags::LADDER;
    } else {
        let_go(ps);
    }
    false
}

/// The ladder's entity becomes the mover the climber rides.
fn ride(ps: &mut PlayerState, backend: &impl LadderHitEntity) {
    if let Some(entity) = backend.last_hit_entity() {
        ps.mover_entity_num = entity;
    }
}

fn is_ladder(hit: &LadderTraceHit) -> bool {
    hit.fraction < 1.0 && hit.surface_flags & SURF_LADDER != 0
}

/// Off the ladder, the player falls (and cannot grab one until landing).
fn let_go(ps: &mut PlayerState) {
    if ps.pm_flags & pm_flags::LADDER != 0 {
        ps.pm_flags = (ps.pm_flags & !pm_flags::LADDER) | pm_flags::LADDER_FALL;
    }
}

/// PM_LadderMove: a jump pushes off; otherwise looking up or down with the
/// forward key climbs, the sideways keys move along the face, and the step
/// and slide runs without gravity.
pub(crate) fn ladder_move<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    cmd: &mut UserCmd,
    context: WalkMoveContext,
    bounds: MoveBounds,
    collision: &C,
    body: &mut BodyAnims,
) -> f32 {
    let zombies = context.feel.zombies;
    // Climbing keeps the ladder's mover from timing out.
    ps.mover_timestamp = cmd.server_time;
    if crate::walk::jump_check(ps, pml, cmd, &context) {
        body.jump = true;
        return air_move(ps, pml, cmd, context.air, bounds, collision, zombies, context.lunge);
    }

    let mut upscale = ((pml.forward[2] + UPSCALE_BIAS) * UPSCALE_MUL).clamp(-1.0, 1.0);

    let [fx, fy, _] = pml.forward;
    let inv = 1.0 / nonzero(libm::sqrtf(fx * fx + fy * fy));
    pml.forward = [fx * inv, fy * inv, 0.0];

    // The sideways direction, flat, then laid on the ladder's face; BO2 does
    // not make it whole again.
    let [rx, ry, _] = pml.right;
    let inv = 1.0 / nonzero(libm::sqrtf(rx * rx + ry * ry));
    pml.right = project_on_plane([rx * inv, ry * inv, 0.0], ps.v_ladder_vec);

    let scale = cmd_scale(ps, cmd, context.air.player_spectate_speed_scale);
    if ps.perks[0] & PERK_FASTLADDERCLIMB != 0 {
        upscale *= FAST_LADDER_CLIMB_SCALE;
    }

    let mut wish = [0.0_f32; 3];
    if cmd.forwardmove != 0 {
        wish[2] = upscale * CLIMB_WISH_SCALE * scale * f32::from(cmd.forwardmove);
    }
    if cmd.rightmove != 0 {
        let side = scale * SIDE_WISH_SCALE * f32::from(cmd.rightmove);
        wish[2] += pml.right[2] * side;
        wish[0] = pml.right[0] * side;
        wish[1] = pml.right[1] * side;
    }
    let wishspeed = libm::sqrtf(wish[1] * wish[1] + wish[0] * wish[0] + wish[2] * wish[2]);
    let inv = 1.0 / nonzero(wishspeed);
    let wishdir = [wish[0] * inv, wish[1] * inv, wish[2] * inv];
    accelerate(ps, pml, &wishdir, wishspeed, LADDER_ACCEL);

    // No forward key: the climb slows to a stop by gravity's amount.
    if cmd.forwardmove == 0 {
        let fall = ps.gravity as f32 * pml.frametime;
        let z = ps.velocity[2];
        ps.velocity[2] = if z <= 0.0 {
            let z = fall + z;
            if z < 0.0 { z } else { 0.0 }
        } else {
            let z = z - fall;
            if 0.0 < z { z } else { 0.0 }
        };
    }
    if cmd.rightmove == 0 {
        side_damping(ps, pml.right, pml.frametime);
    }
    if pml.walking == 0 {
        ladder_attract_velocity(ps);
    }

    let step = if collision.gjk_on() {
        crate::gjk::step::step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            false,
            zombies,
        )
    } else {
        step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            None,
            zombies,
        )
    };

    let facing = bo2_vectoyaw(ps.v_ladder_vec[0], ps.v_ladder_vec[1]) + 180.0;
    let dir = angle_subtract(facing, ps.viewangles[1]) as i32;
    let dir = if dir.abs() > MOVEMENT_DIR_CAP {
        if dir < 1 { -MOVEMENT_DIR_CAP } else { MOVEMENT_DIR_CAP }
    } else {
        dir
    };
    ps.movement_dir = i32::from(dir as i8);
    step
}

/// A zero (or broken) length divides as 1.
fn nonzero(len: f32) -> f32 {
    if 0.0 <= -len { 1.0 } else { len }
}

/// ProjectPointOnPlane: `point` less its part along the (unit) `normal`.
fn project_on_plane(point: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let d = -(point[1] * normal[1] + normal[0] * point[0] + point[2] * normal[2]);
    [
        normal[0] * d + point[0],
        d * normal[1] + point[1],
        d * normal[2] + point[2],
    ]
}

/// With no sideways key, the drift along the face falls by 16 times the
/// frame's time. Under one unit the step is a whole unit, so a drift under
/// one unit a second turns round (BO2 does this too).
fn side_damping(ps: &mut PlayerState, right: [f32; 3], frametime: f32) {
    let [x, y, _] = right;
    let len = nonzero(libm::sqrtf(y * y + x * x));
    let rx = x * (1.0 / len);
    let ry = y * (1.0 / len);
    let drift = ry * ps.velocity[1] + ps.velocity[0] * rx;
    if drift == 0.0 {
        return;
    }
    ps.velocity[0] += rx * -drift;
    ps.velocity[1] += ry * -drift;
    let mut slow = frametime * drift * SIDE_DAMPING;
    if slow.abs() < drift.abs() {
        if slow.abs() < 1.0 {
            slow = if 0.0 <= slow { 1.0 } else { -1.0 };
        }
        ps.velocity[0] += rx * (drift - slow);
        ps.velocity[1] += ry * (drift - slow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use crate::bridge::{
        AirMoveContext, Bo2Feel, CmdScaleWalkContext, CollisionBackend, GroundTraceInput,
        JumpLaunchContext,
    };
    use trace_t6::Trace;

    /// A ladder face every probe hits (or none), remembering where each
    /// probe looked.
    struct Probe {
        hit: Option<LadderTraceHit>,
        dirs: Vec<[f32; 3]>,
        entity: Option<i32>,
    }

    impl LadderAttachBackend for Probe {
        fn ladder_trace(&mut self, _origin: [f32; 3], dir: [f32; 3], _dist: f32) -> Option<LadderTraceHit> {
            self.dirs.push(dir);
            self.hit
        }
    }

    impl LadderHitEntity for Probe {
        fn last_hit_entity(&self) -> Option<i32> {
            self.entity
        }
    }

    /// A ladder face looking along -x, half way along the probe.
    const FACE: LadderTraceHit = LadderTraceHit {
        fraction: 0.5,
        normal: [-1.0, 0.0, 0.0],
        surface_flags: SURF_LADDER,
    };

    fn probe(hit: Option<LadderTraceHit>) -> Probe {
        Probe {
            hit,
            dirs: Vec::new(),
            entity: None,
        }
    }

    fn check(ps: &mut PlayerState, pml: &mut Pml, walking: bool, forwardmove: i8, probe: &mut Probe) -> bool {
        let context = CheckLadderContext {
            server_time: 10_000,
            walking,
            forward_xy: [2.0, 0.0],
            forwardmove,
        };
        check_ladder_move(ps, pml, context, probe)
    }

    fn on_ladder() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.v_ladder_vec = FACE.normal;
        ps.view_height_target = 60;
        ps
    }

    #[test]
    fn walking_into_a_ladder_grabs_it() {
        let mut ps = PlayerState::ZERO;
        ps.view_height_target = 60;
        let mut pml = crate::testing::pml(true);
        let mut rail = probe(Some(FACE));
        check(&mut ps, &mut pml, true, 127, &mut rail);
        assert_ne!(ps.pm_flags & pm_flags::LADDER, 0);
        assert_eq!(ps.v_ladder_vec, FACE.normal);
        // Ahead first, then back at the face.
        assert_eq!(rail.dirs, [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    }

    #[test]
    fn standing_still_on_the_ground_does_not_grab() {
        let mut ps = PlayerState::ZERO;
        ps.view_height_target = 60;
        let mut pml = crate::testing::pml(true);
        check(&mut ps, &mut pml, true, 0, &mut probe(Some(FACE)));
        assert_eq!(ps.pm_flags & pm_flags::LADDER, 0);
    }

    #[test]
    fn hanging_on_looks_behind_the_face() {
        let mut ps = on_ladder();
        let mut pml = crate::testing::pml(false);
        let mut rail = probe(Some(FACE));
        check(&mut ps, &mut pml, false, 0, &mut rail);
        assert_ne!(ps.pm_flags & pm_flags::LADDER, 0);
        assert_eq!(rail.dirs, [[1.0, 0.0, 0.0]]);
    }

    #[test]
    fn the_riot_shield_lets_go_but_a_detonator_does_not() {
        for state in crate::gun::t6::RIOTSHIELD_PREPARE..=crate::gun::t6::RIOTSHIELD_END {
            let mut ps = on_ladder();
            ps.weapon_state_t6 = state;
            let mut pml = crate::testing::pml(false);
            check(&mut ps, &mut pml, false, 0, &mut probe(Some(FACE)));
            assert_eq!(ps.pm_flags & pm_flags::LADDER, 0, "{state}");
            assert_ne!(ps.pm_flags & pm_flags::LADDER_FALL, 0, "{state}");
        }
        let mut ps = on_ladder();
        ps.weapon_state_t6 = crate::gun::t6::DETONATING;
        let mut pml = crate::testing::pml(false);
        check(&mut ps, &mut pml, false, 0, &mut probe(Some(FACE)));
        assert_ne!(ps.pm_flags & pm_flags::LADDER, 0);
    }

    #[test]
    fn losing_the_ladder_in_the_air_plays_the_jump() {
        let mut ps = on_ladder();
        let mut pml = crate::testing::pml(false);
        assert!(check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        assert_ne!(ps.pm_flags & pm_flags::LADDER_FALL, 0);
        // Holding on: none.
        let mut ps = on_ladder();
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(Some(FACE))));
        // On the ground when it goes, or never on one: none.
        let mut ps = on_ladder();
        ps.ground_entity_num = 0;
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.view_height_target = 60;
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        // The riot shield, lying down, a fresh jump or dying let go without it.
        let mut ps = on_ladder();
        ps.weapon_state_t6 = crate::gun::t6::RIOTSHIELD_PREPARE;
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        let mut ps = on_ladder();
        ps.view_height_target = PRONE_VIEW_HEIGHTS[0];
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        let mut ps = on_ladder();
        ps.jump_time = 10_000;
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        let mut ps = on_ladder();
        ps.pm_type = PM_TYPE_DEAD;
        assert!(!check(&mut ps, &mut pml, false, 0, &mut probe(None)));
        assert_eq!(ps.pm_flags & pm_flags::LADDER, 0);
    }

    #[test]
    fn a_ladder_on_a_moving_thing_is_ridden() {
        // Grabbing one.
        let mut ps = PlayerState::ZERO;
        ps.view_height_target = 60;
        let mut pml = crate::testing::pml(false);
        let mut rail = probe(Some(FACE));
        rail.entity = Some(40);
        check(&mut ps, &mut pml, false, 0, &mut rail);
        assert_ne!(ps.pm_flags & pm_flags::LADDER, 0);
        assert_eq!(ps.mover_entity_num, 40);
        // Holding on keeps it.
        let mut ps = on_ladder();
        let mut pml = crate::testing::pml(false);
        check(&mut ps, &mut pml, false, 0, &mut rail);
        assert_eq!(ps.mover_entity_num, 40);
        // A trace that hit no entity leaves the mover alone.
        let mut ps = on_ladder();
        ps.mover_entity_num = 41;
        let mut pml = crate::testing::pml(false);
        check(&mut ps, &mut pml, false, 0, &mut probe(Some(FACE)));
        assert_eq!(ps.mover_entity_num, 41);
    }

    #[test]
    fn climbing_stamps_the_movers_time() {
        let mut ps = climber();
        ps.mover_timestamp = 0;
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert_eq!(ps.mover_timestamp, 10_000);
    }

    #[test]
    fn prone_and_a_fresh_jump_let_go() {
        for (height, jump_time) in [(11, 0), (22, 0), (60, 9_800)] {
            let mut ps = on_ladder();
            ps.view_height_target = height;
            ps.jump_time = jump_time;
            let mut pml = crate::testing::pml(false);
            check(&mut ps, &mut pml, false, 0, &mut probe(Some(FACE)));
            assert_eq!(ps.pm_flags & pm_flags::LADDER, 0, "{height} {jump_time}");
        }
    }

    #[test]
    fn the_dead_let_go_and_leave_the_ground() {
        let mut ps = on_ladder();
        ps.pm_type = PM_TYPE_DEAD;
        ps.ground_entity_num = 1022;
        let mut pml = crate::testing::pml(true);
        check(&mut ps, &mut pml, true, 127, &mut probe(Some(FACE)));
        assert_eq!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert_eq!((pml.walking, pml.ground_plane, pml.almost_ground_plane), (0, 0, 0));
        assert_ne!(ps.pm_flags & pm_flags::LADDER_FALL, 0);
    }

    #[test]
    fn a_running_timer_skips_the_check() {
        let mut ps = PlayerState::ZERO;
        ps.view_height_target = 60;
        ps.pm_time = 100;
        ps.pm_flags = 0x80;
        let mut pml = crate::testing::pml(true);
        let mut rail = probe(Some(FACE));
        check(&mut ps, &mut pml, true, 127, &mut rail);
        assert!(rail.dirs.is_empty());
    }

    #[test]
    fn side_drift_dies_away() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [100.0, 0.0, 0.0];
        side_damping(&mut ps, [1.0, 0.0, 0.0], 0.05);
        assert!((ps.velocity[0] - 20.0).abs() < 1e-4, "{}", ps.velocity[0]);
        // A long frame stops it dead.
        ps.velocity = [100.0, 0.0, 0.0];
        side_damping(&mut ps, [1.0, 0.0, 0.0], 0.1);
        assert_eq!(ps.velocity[0], 0.0);
        // Under one unit the drift turns round.
        ps.velocity = [0.5, 0.0, 0.0];
        side_damping(&mut ps, [1.0, 0.0, 0.0], 0.05);
        assert!((ps.velocity[0] + 0.5).abs() < 1e-6, "{}", ps.velocity[0]);
        // Up and down are left alone.
        ps.velocity = [0.0, 0.0, 50.0];
        side_damping(&mut ps, [1.0, 0.0, 0.0], 0.05);
        assert_eq!(ps.velocity, [0.0, 0.0, 50.0]);
    }

    /// Nothing to hit anywhere.
    struct Open;

    impl Bo2Collision for Open {}

    impl CollisionBackend for Open {
        fn trace(&self, _input: GroundTraceInput) -> Trace {
            Trace {
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    fn walk_context() -> WalkMoveContext {
        let air = AirMoveContext {
            player_spectate_speed_scale: 1.0,
            shellshock_gravity_scale: 1.0,
            shellshock_gravity_bias: 0.0,
        };
        WalkMoveContext {
            cmd_scale: CmdScaleWalkContext {
                player_back_speed_scale: 0.7,
                player_strafe_speed_scale: 0.8,
                player_sprint_speed_scale: 1.5,
                player_last_stand_crawl_speed_scale: 0.15,
                weapon_move_speed_scale: 1.0,
                weapon_ads_move_speed_scale: 1.0,
                shellshock_movement_scale: 1.0,
                prone_lerp_ms: 0,
            },
            weapon_move_scale: 1.0,
            old_buttons: 0,
            jump: JumpLaunchContext {
                jump_height: 39.0,
                dive: false,
                crouch_jump_scale: 1.0,
                jump_ladder_push_vel: 128.0,
            },
            air,
            feel: Bo2Feel::BO2_MP,
            lunge: None,
        }
    }

    const BOUNDS: MoveBounds = MoveBounds {
        mins: [-15.0, -15.0, 0.0],
        maxs: [15.0, 15.0, 70.0],
        tracemask: 1,
    };

    /// One climbing frame: returns the player and the frame's directions.
    fn climb(ps: &mut PlayerState, forward: [f32; 3], right: [f32; 3], forwardmove: i8, rightmove: i8) -> Pml {
        let mut pml = crate::testing::pml(false);
        pml.forward = forward;
        pml.right = right;
        let mut cmd = UserCmd {
            server_time: 10_000,
            forwardmove,
            rightmove,
            ..UserCmd::default()
        };
        ladder_move(ps, &mut pml, &mut cmd, walk_context(), BOUNDS, &Open, &mut BodyAnims::default());
        pml
    }

    fn climber() -> PlayerState {
        let mut ps = on_ladder();
        ps.speed = 190;
        ps.gravity = 800;
        ps
    }

    #[test]
    fn the_perk_climbs_175_times_as_fast() {
        // Looking well up: the climb is at its full rate.
        let up = [0.6, 0.0, 0.8];
        let mut plain = climber();
        climb(&mut plain, up, [0.0, -1.0, 0.0], 127, 0);
        let mut quick = climber();
        quick.perks[0] = PERK_FASTLADDERCLIMB;
        climb(&mut quick, up, [0.0, -1.0, 0.0], 127, 0);
        assert!(plain.velocity[2] > 0.0);
        let ratio = quick.velocity[2] / plain.velocity[2];
        assert!((ratio - FAST_LADDER_CLIMB_SCALE).abs() < 1e-4, "{ratio}");
    }

    #[test]
    fn looking_down_climbs_down() {
        let mut ps = climber();
        climb(&mut ps, [0.6, 0.0, -0.8], [0.0, -1.0, 0.0], 127, 0);
        assert!(ps.velocity[2] < 0.0);
    }

    #[test]
    fn the_sideways_direction_keeps_its_length_on_the_face() {
        let mut ps = climber();
        let pml = climb(&mut ps, [0.8, 0.6, 0.0], [0.6, -0.8, 0.0], 0, 0);
        assert!(pml.right[0].abs() < 1e-6, "{:?}", pml.right);
        assert!((pml.right[1] + 0.8).abs() < 1e-6, "{:?}", pml.right);
    }

    #[test]
    fn the_legs_face_the_ladder_within_75_degrees() {
        let mut ps = climber();
        // The ladder's yaw comes out a hair under 180 in single precision
        // and the angle is cut toward zero (30 would give -29), so test off
        // the whole number.
        ps.viewangles[1] = 30.5;
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert_eq!(ps.movement_dir, -30);
        ps.viewangles[1] = 100.0;
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert_eq!(ps.movement_dir, -MOVEMENT_DIR_CAP);
        ps.viewangles[1] = -120.0;
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert_eq!(ps.movement_dir, MOVEMENT_DIR_CAP);
    }

    #[test]
    fn no_forward_key_slows_the_climb_to_a_stop() {
        let mut ps = climber();
        ps.velocity = [0.0, 0.0, 30.0];
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert_eq!(ps.velocity[2], 0.0);
        ps.velocity = [0.0, 0.0, -50.0];
        climb(&mut ps, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], 0, 0);
        assert!((ps.velocity[2] + 10.0).abs() < 1e-4, "{}", ps.velocity[2]);
    }
}
