//! Black Ops II's step-and-slide move in physics mode 1 (PM_StepSlideMove):
//! the slide, then, if it met something or the ground is sloped, the same
//! slide again from up to a step higher, kept when it got farther; then down
//! onto the ground again. Steps on the ground cost speed and can sound a
//! footstep. Its traces are the GJK body's (player trace up, ground trace
//! down).

use crate::bridge::{CollisionBackend, PRONE_CHECK_HEIGHT, PRONE_FEET_DIST, Pml, ProneRules, jump};
use playerstate_t6::game::{ENTITYNUM_NONE, PM_TYPE_LAST_STAND, PlayerState, pm_flags};

use super::Bo2Collision;
use super::slide::slide_move;
use super::trace::{ground_trace, player_trace};
use crate::check_prone::check_prone;
use crate::footstep::{footstep_event, should_make_footsteps};
use crate::project::project_velocity;

const STEP_SIZE: f32 = 18.0;
const PRONE_STEP_SIZE: f32 = 10.0;
/// The step up looks one unit past the step.
const STEP_UP_EXTRA: f32 = 1.0;
/// Ground facing up less than this is sloped: the move tries a step on it
/// even when the slide met nothing.
const STEP_GROUND_NORMAL_Z: f32 = 0.9;
/// Walking, the step down looks this much farther down.
const WALKING_STEP_DOWN_EXTRA: f32 = 9.0;
/// The step must gain at least this much along the move.
const STEP_PROGRESS_EPSILON: f32 = 0.001;
/// Entity numbers below this are players: the step never lands on one.
const MAX_CLIENTS: u16 = playerstate_t6::game::PLAYER_CORPSE_ENTITY_BASE as u16;
/// pm_type below this (Black Ops II MP's own test): the stair step counts.
const STAIR_PM_TYPE_LIMIT: i32 = 10;
/// The room a prone player needs after the move.
const PRONE_VERIFY_SIZE: f32 = 15.0;
/// Highest a jump climbs over its start.
const JUMP_HEIGHT: f32 = 39.0;

/// The stair step: a rise of more than this slows the player, by how much
/// of a full step it was (a full step leaves the floor's share).
const STEP_SLOW_MIN_RISE: f32 = 0.5;
const STEP_ROUND_BIAS: f64 = 9.313_225_746_154_785e-10;
const STEP_SLOW_RANGE: f32 = 0.8;
const STEP_SLOW_FLOOR: f32 = f32::from_bits(0x3e4c_cccc);
/// The walk rhythm a stair step moves on: this much plus this much per two
/// units (four at most).
const STEP_BOB_PER_UNIT: f32 = 1.25;
const STEP_BOB_BASE: f32 = 7.0;

/// BO2's PM_StepSlideMove (mode 1). `gravity`: in the air. Returns how far
/// a stair step moved the player up or down (0 when there was none): the
/// camera eases over it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn step_slide_move<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &Pml,
    collision: &C,
    mins: [f32; 3],
    maxs: [f32; 3],
    mask: u32,
    gravity: bool,
    zombies: bool,
) -> f32 {
    let had_ground = if ps.pm_flags & pm_flags::LADDER != 0 {
        jump::clear_state(ps);
        false
    } else if pml.ground_plane != 0 {
        true
    } else {
        if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time != 0 {
            jump::clear_state(ps);
        }
        false
    };
    let start_origin = ps.origin;
    let start_velocity = ps.velocity;

    let bumped = slide_move(ps, pml, collision, mins, maxs, mask, gravity);
    if !bumped && ps.pm_type == PM_TYPE_LAST_STAND && ps.pm_flags & crate::ground::PMF_SLIDING != 0 {
        return 0.0;
    }
    let mut step_size = if ps.pm_flags & pm_flags::PRONE != 0 { PRONE_STEP_SIZE } else { STEP_SIZE };
    let mut jump_step = false;
    if ps.ground_entity_num == ENTITYNUM_NONE {
        if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time != 0 {
            jump::clear_state(ps);
        }
        let ladder_up = ps.pm_flags & pm_flags::LADDER != 0 && ps.velocity[2] > 0.0;
        if bumped && ps.pm_flags & pm_flags::JUMPING != 0 {
            match jump::get_step_height(ps, start_origin) {
                Some(height) if height < 1.0 => return 0.0,
                Some(height) => {
                    step_size = height;
                    jump_step = true;
                }
                None if !ladder_up => return 0.0,
                None => {}
            }
        } else if !ladder_up {
            return 0.0;
        }
    }
    let down_origin = ps.origin;
    let down_velocity = ps.velocity;

    let sloped = pml.ground_plane != 0 && f32::from_bits(pml.ground_trace[3]) < STEP_GROUND_NORMAL_Z;
    let mut step_height = 0.0;
    if bumped || sloped {
        let top = [start_origin[0], start_origin[1], start_origin[2] + step_size + STEP_UP_EXTRA];
        let up = player_trace(collision, start_origin, top, mins, maxs, mask);
        let rise = (step_size + STEP_UP_EXTRA) * up.fraction - STEP_UP_EXTRA;
        if rise >= 1.0 {
            step_height = rise;
            ps.origin = [start_origin[0], start_origin[1], start_origin[2] + rise];
            ps.velocity = start_velocity;
            slide_move(ps, pml, collision, mins, maxs, mask, gravity);
        }
    }

    if had_ground || step_height != 0.0 {
        let mut drop = step_height;
        if had_ground {
            drop += WALKING_STEP_DOWN_EXTRA;
        }
        let end = [ps.origin[0], ps.origin[1], ps.origin[2] - drop];
        let down = ground_trace(collision, ps.origin, end, mins, maxs, mask);
        if trace_t6::trace_get_entity_hit_id(&down) < MAX_CLIENTS {
            // Never onto another player: the step is taken back.
            ps.origin = down_origin;
            ps.velocity = down_velocity;
            return 0.0;
        }
        if down.fraction >= 1.0 {
            if step_height != 0.0 {
                ps.origin[2] -= step_height;
            }
        } else if down.walkable {
            ps.origin = down.end_pos(ps.origin, end);
            project_velocity(&mut ps.velocity, &down.normal);
        } else {
            ps.origin = down_origin;
            ps.velocity = down_velocity;
            return 0.0;
        }
    }

    let gained =
        start_velocity[0] * (ps.origin[0] - start_origin[0]) + start_velocity[1] * (ps.origin[1] - start_origin[1]);
    let flat = start_velocity[0] * (down_origin[0] - start_origin[0])
        + start_velocity[1] * (down_origin[1] - start_origin[1]);
    if gained <= flat + STEP_PROGRESS_EPSILON || (jump_step && above_max(ps)) {
        // The step got no farther (or a jump's step ends past the top of
        // the jump): the plain slide stands, kept on the ground.
        ps.origin = down_origin;
        ps.velocity = down_velocity;
        if had_ground {
            let end = [ps.origin[0], ps.origin[1], ps.origin[2] - WALKING_STEP_DOWN_EXTRA];
            let down = player_trace(collision, ps.origin, end, mins, maxs, mask);
            if down.fraction < 1.0 {
                ps.origin = down.end_pos(ps.origin, end);
                project_velocity(&mut ps.velocity, &down.normal);
            }
        }
    }
    if jump_step {
        clamp_velocity(ps, down_origin);
    }
    if had_ground
        && ps.pm_type < STAIR_PM_TYPE_LIMIT
        && verify_prone_position(ps, collision, start_origin, start_velocity, zombies)
    {
        return stair_step(ps, pml, start_origin, down_origin, step_size);
    }
    0.0
}

/// BO2's Jump_IsPlayerAboveMax: at or past the top of the jump.
fn above_max(ps: &PlayerState) -> bool {
    ps.jump_origin_z + JUMP_HEIGHT <= ps.origin[2]
}

/// BO2's Jump_ClampVelocity: a jump that rose from `before` climbs no
/// faster than takes it to the top of the jump.
fn clamp_velocity(ps: &mut PlayerState, before: [f32; 3]) {
    if ps.origin[2] - before[2] > 0.0 {
        let room = ps.jump_origin_z + JUMP_HEIGHT - ps.origin[2];
        if room < 0.1 {
            ps.velocity[2] = 0.0;
            return;
        }
        let top = libm::sqrtf(ps.gravity as f32 * room * 2.0);
        if top < ps.velocity[2] {
            ps.velocity[2] = top;
        }
    }
}

/// BO2 after a move on the ground: a prone player must still have room to
/// lie where they ended up. If not, they go back to where they started; if
/// that spot is no better, the move stands. False when the move was taken
/// back.
fn verify_prone_position<C: CollisionBackend>(
    ps: &mut PlayerState,
    collision: &C,
    old_origin: [f32; 3],
    old_velocity: [f32; 3],
    zombies: bool,
) -> bool {
    if ps.pm_flags & pm_flags::PRONE == 0 || prone_fits(ps, collision, zombies) {
        return true;
    }
    let (new_origin, new_velocity) = (ps.origin, ps.velocity);
    ps.origin = old_origin;
    ps.velocity = old_velocity;
    if prone_fits(ps, collision, zombies) {
        return false;
    }
    ps.origin = new_origin;
    ps.velocity = new_velocity;
    true
}

fn prone_fits<C: CollisionBackend>(ps: &mut PlayerState, collision: &C, zombies: bool) -> bool {
    let (mut torso, mut waist) = (ps.f_torso_pitch, ps.f_waist_pitch);
    let fits = check_prone(
        collision,
        ps.origin,
        PRONE_VERIFY_SIZE,
        PRONE_CHECK_HEIGHT,
        ps.prone_direction,
        true,
        true,
        true,
        ProneRules::for_player(ps, zombies),
        PRONE_FEET_DIST,
        Some(&mut torso),
        Some(&mut waist),
    );
    ps.f_torso_pitch = torso;
    ps.f_waist_pitch = waist;
    fits
}

/// BO2 on a stair: a step up or down on the ground costs speed by its
/// height, and one of more than 3 units moves the walk rhythm on, which can
/// sound a footstep. Returns the step's height for the camera (0 when it
/// does not count).
fn stair_step(ps: &mut PlayerState, pml: &Pml, start_origin: [f32; 3], down_origin: [f32; 3], step_size: f32) -> f32 {
    let rise = ps.origin[2] - down_origin[2];
    if libm::fabsf(rise) <= STEP_SLOW_MIN_RISE {
        return 0.0;
    }
    let delta = libm::rint(rise as f64 + STEP_ROUND_BIAS) as i32;
    if delta == 0 {
        return 0.0;
    }
    let scale = (1.0 - libm::fabsf(ps.origin[2] - start_origin[2]) / step_size) * STEP_SLOW_RANGE + STEP_SLOW_FLOOR;
    for v in &mut ps.velocity {
        *v *= scale;
    }
    // The step's footstep goes by the speed the stair left.
    let xyspeed = libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1]);
    let delta = delta.abs();
    if delta > 3 && ps.ground_entity_num != ENTITYNUM_NONE {
        let beats = (delta / 2).min(4);
        let old = ps.bob_cycle;
        ps.bob_cycle = ((beats as f32 * STEP_BOB_PER_UNIT + STEP_BOB_BASE + old as f32) as i32) & 0xff;
        let footsteps = should_make_footsteps(ps, xyspeed);
        footstep_event(ps, pml.ground_trace[4], old, ps.bob_cycle, footsteps, xyspeed);
    }
    rise
}

#[cfg(test)]
mod tests {
    use super::super::hull::player_hull;
    use super::super::testkit::{MASK, MAXS, MINS, World, block, floor, other_player};
    use super::super::{Shape, add};
    use super::*;
    use crate::testing::pml;
    use trace_t6::ENTITYNUM_WORLD;

    const REST_Z: f32 = 0.125;
    const REACH: f32 = 14.625 / 0.965_925_8 + 0.5;

    fn standing(x: f32, vx: f32) -> PlayerState {
        PlayerState {
            origin: [x, 0.0, REST_Z],
            velocity: [vx, 0.0, 0.0],
            gravity: 800,
            ground_entity_num: i32::from(ENTITYNUM_WORLD),
            ..PlayerState::ZERO
        }
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    /// A 12-unit stair: up onto it, slowed by its height (two thirds of a
    /// step leaves 0.47 of the speed), the rise handed to the camera.
    #[test]
    fn up_a_stair() {
        let world = World::new(alloc::vec![floor(), block([0.0, -256.0, -16.0], [256.0, 256.0, 12.0])]);
        let mut ps = standing(-20.0, 600.0);
        let rise = step_slide_move(&mut ps, &pml(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(ps.origin[2], 12.0 + REST_Z, 0.01), "{:?}", ps.origin);
        assert!(near(ps.origin[0], 10.0, 0.01), "{:?}", ps.origin);
        assert!(near(rise, 12.0, 0.01), "{rise}");
        let scale = (1.0 - 12.0 / 18.0) * 0.8 + f32::from_bits(0x3e4c_cccc);
        assert!(near(ps.velocity[0], 600.0 * scale, 0.1), "{:?}", ps.velocity);
        // 12 units moves the walk rhythm on four beats.
        assert_eq!(ps.bob_cycle, 12);
    }

    /// 24 units is more than a step: stopped at its face, nothing climbed.
    #[test]
    fn a_step_too_high() {
        let world = World::new(alloc::vec![floor(), block([0.0, -256.0, -16.0], [256.0, 256.0, 24.0])]);
        let mut ps = standing(-20.0, 600.0);
        let rise = step_slide_move(&mut ps, &pml(true), &world, MINS, MAXS, MASK, false, false);
        assert_eq!(rise, 0.0);
        assert!(near(ps.origin[0], -REACH, 0.05), "{:?}", ps.origin);
        assert!(near(ps.origin[2], REST_Z, 0.01), "{:?}", ps.origin);
    }

    /// Off a 6-unit ledge: down onto the lower floor, which costs speed
    /// too.
    #[test]
    fn down_a_ledge() {
        let world = World::new(alloc::vec![
            block([-256.0, -256.0, -16.0], [0.0, 256.0, 0.0]),
            block([0.0, -256.0, -22.0], [256.0, 256.0, -6.0]),
        ]);
        let mut ps = standing(-20.0, 1000.0);
        let rise = step_slide_move(&mut ps, &pml(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(ps.origin[2], REST_Z - 6.0, 0.01), "{:?}", ps.origin);
        assert!(near(rise, -6.0, 0.01), "{rise}");
        let scale = (1.0 - 6.0 / 18.0) * 0.8 + f32::from_bits(0x3e4c_cccc);
        assert!(near(ps.velocity[0], 1000.0 * scale, 0.1), "{:?}", ps.velocity);
    }

    /// Off a ledge with a player lying on the floor below, inside the
    /// step down's reach: the walker never lands on them, it goes over the
    /// edge at its own height (BO2 takes the step back).
    #[test]
    fn never_steps_onto_a_player() {
        let lower = [30.0, 0.0, -6.0];
        let mut lying = other_player(lower, 3);
        let hull = player_hull(MINS, [15.0, 15.0, 2.0], 0.0);
        let (lo, hi) = hull.calc_aabb();
        lying.shape = Shape::Player { hull };
        lying.mins = add(lower, lo);
        lying.maxs = add(lower, hi);
        let ledge = || {
            alloc::vec![
                block([-256.0, -256.0, -16.0], [0.0, 256.0, 0.0]),
                block([0.0, -256.0, -22.0], [256.0, 256.0, -6.0]),
            ]
        };
        let mut shapes = ledge();
        shapes.push(lying);
        let world = World::new(shapes);
        let mut ps = standing(-20.0, 1000.0);
        let rise = step_slide_move(&mut ps, &pml(true), &world, MINS, MAXS, MASK, false, false);
        assert_eq!(rise, 0.0);
        assert!(near(ps.origin[0], 30.0, 0.01), "{:?}", ps.origin);
        assert!(near(ps.origin[2], REST_Z, 0.01), "{:?}", ps.origin);
        assert_eq!(ps.velocity, [1000.0, 0.0, 0.0]);

        // The same ledge with nobody there: down onto the lower floor.
        let world = World::new(ledge());
        let mut ps = standing(-20.0, 1000.0);
        step_slide_move(&mut ps, &pml(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(ps.origin[2], REST_Z - 6.0, 0.01), "{:?}", ps.origin);
    }

    /// In the air, meeting nothing: just the fall, no step.
    #[test]
    fn falling_never_steps() {
        let world = World::new(alloc::vec![floor()]);
        let mut ps = standing(0.0, 100.0);
        ps.origin[2] = 100.0;
        ps.ground_entity_num = ENTITYNUM_NONE;
        let rise = step_slide_move(&mut ps, &pml(false), &world, MINS, MAXS, MASK, true, false);
        assert_eq!(rise, 0.0);
        assert!(near(ps.origin[2], 99.0, 1e-3), "{:?}", ps.origin);
        assert!(near(ps.velocity[2], -40.0, 1e-3), "{:?}", ps.velocity);
    }
}
