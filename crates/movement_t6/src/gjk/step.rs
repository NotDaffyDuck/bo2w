//! Black Ops II's step-and-slide move in physics mode 1 (PM_StepSlideMove):
//! the slide, then, if it met something or the ground is sloped, the same
//! slide again from up to a step higher, kept when it got farther; then down
//! onto the ground again. Steps on the ground cost speed and can sound a
//! footstep. Its traces are the GJK body's (player trace up, ground trace
//! down).

use crate::bridge::{SolidProbe, BELLY_CLEARANCE_PROBE, BELLY_FOOT_REACH, TickLocals, ProneRules, leap};
use playerstate_t6::game::{NO_ENTITY_SLOT, MOVE_MODE_DOWNED, AvatarState, movement_state_bits};

use super::Bo2Collision;
use super::slide::slide_move;
use super::trace::{floor_probe, player_trace};
use crate::prone_room::check_prone;
use crate::gait_cycle::{footstep_event, should_make_footsteps};
use crate::project::project_velocity;

const STAIR_RISE_MAX: f32 = 18.0;
const BELLY_STAIR_RISE: f32 = 10.0;
/// The step up looks one unit past the step.
const STAIR_LIFT_MARGIN: f32 = 1.0;
/// Ground facing up less than this is sloped: the move tries a step on it
/// even when the slide met nothing.
const STAIR_FLOOR_UPRIGHTNESS: f32 = 0.9;
/// Walking, the step down looks this much farther down.
const FOOTED_DROP_MARGIN: f32 = 9.0;
/// The step must gain at least this much along the move.
const STAIR_ADVANCE_TOLERANCE: f32 = 0.001;
/// Entity numbers below this are players: the step never lands on one.
const MAX_CLIENTS: u16 = playerstate_t6::game::BODY_ENTITY_FIRST_SLOT as u16;
/// movement_mode below this (Black Ops II MP's own test): the stair step counts.
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
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    solids: &C,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    mask: u32,
    gravity: bool,
    zombies: bool,
) -> f32 {
    let was_footed = if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        leap::clear_state(avatar);
        false
    } else if tick_vars.standing_on_floor != 0 {
        true
    } else {
        if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer != 0 {
            leap::clear_state(avatar);
        }
        false
    };
    let from_origin = avatar.origin;
    let initial_speed_vec = avatar.velocity;

    let bumped = slide_move(avatar, tick_vars, solids, box_lo, box_hi, mask, gravity);
    if !bumped && avatar.movement_mode == MOVE_MODE_DOWNED && avatar.movement_state_bits & crate::ground::PMF_SLIDING != 0 {
        return 0.0;
    }
    let mut step_size = if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 { BELLY_STAIR_RISE } else { STAIR_RISE_MAX };
    let mut jump_step = false;
    if avatar.standing_on == NO_ENTITY_SLOT {
        if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer != 0 {
            leap::clear_state(avatar);
        }
        let rung_ascent_dir = avatar.movement_state_bits & movement_state_bits::LADDER != 0 && avatar.velocity[2] > 0.0;
        if bumped && avatar.movement_state_bits & movement_state_bits::JUMPING != 0 {
            match leap::step_room(avatar, from_origin[2]) {
                Some(height) if height < 1.0 => return 0.0,
                Some(height) => {
                    step_size = height;
                    jump_step = true;
                }
                None if !rung_ascent_dir => return 0.0,
                None => {}
            }
        } else if !rung_ascent_dir {
            return 0.0;
        }
    }
    let lowered_spot = avatar.origin;
    let descent_speed_vec = avatar.velocity;

    let sloped = tick_vars.standing_on_floor != 0 && f32::from_bits(tick_vars.floor_probe[3]) < STAIR_FLOOR_UPRIGHTNESS;
    let mut stair_rise = 0.0;
    if bumped || sloped {
        let top = [from_origin[0], from_origin[1], from_origin[2] + step_size + STAIR_LIFT_MARGIN];
        let up = player_trace(solids, from_origin, top, box_lo, box_hi, mask);
        let rise = (step_size + STAIR_LIFT_MARGIN) * up.fraction - STAIR_LIFT_MARGIN;
        if rise >= 1.0 {
            stair_rise = rise;
            avatar.origin = [from_origin[0], from_origin[1], from_origin[2] + rise];
            avatar.velocity = initial_speed_vec;
            slide_move(avatar, tick_vars, solids, box_lo, box_hi, mask, gravity);
        }
    }

    if was_footed || stair_rise != 0.0 {
        let mut drop = stair_rise;
        if was_footed {
            drop += FOOTED_DROP_MARGIN;
        }
        let end = [avatar.origin[0], avatar.origin[1], avatar.origin[2] - drop];
        let drop_hit = floor_probe(solids, avatar.origin, end, box_lo, box_hi, mask);
        if trace_t6::trace_get_entity_hit_id(&drop_hit) < MAX_CLIENTS {
            // Never onto another player: the step is taken back.
            avatar.origin = lowered_spot;
            avatar.velocity = descent_speed_vec;
            return 0.0;
        }
        if 1.0 <= drop_hit.fraction {
            if stair_rise != 0.0 {
                avatar.origin[2] -= stair_rise;
            }
        } else if drop_hit.walkable {
            avatar.origin = drop_hit.finish_location(avatar.origin, end);
            project_velocity(&mut avatar.velocity, &drop_hit.normal);
        } else {
            avatar.origin = lowered_spot;
            avatar.velocity = descent_speed_vec;
            return 0.0;
        }
    }

    let gained =
        initial_speed_vec[0] * (avatar.origin[0] - from_origin[0]) + initial_speed_vec[1] * (avatar.origin[1] - from_origin[1]);
    let flat = initial_speed_vec[0] * (lowered_spot[0] - from_origin[0])
        + initial_speed_vec[1] * (lowered_spot[1] - from_origin[1]);
    if gained <= flat + STAIR_ADVANCE_TOLERANCE || (jump_step && above_max(avatar)) {
        // The step got no farther (or a jump's step ends past the top of
        // the jump): the plain slide stands, kept on the ground.
        avatar.origin = lowered_spot;
        avatar.velocity = descent_speed_vec;
        if was_footed {
            let end = [avatar.origin[0], avatar.origin[1], avatar.origin[2] - FOOTED_DROP_MARGIN];
            let drop_hit = player_trace(solids, avatar.origin, end, box_lo, box_hi, mask);
            if 1.0 > drop_hit.fraction {
                avatar.origin = drop_hit.finish_location(avatar.origin, end);
                project_velocity(&mut avatar.velocity, &drop_hit.normal);
            }
        }
    }
    if jump_step {
        clamp_velocity(avatar, lowered_spot);
    }
    if was_footed
        && avatar.movement_mode < STAIR_PM_TYPE_LIMIT
        && verify_prone_position(avatar, solids, from_origin, initial_speed_vec, zombies)
    {
        return stair_step(avatar, tick_vars, from_origin, lowered_spot, step_size);
    }
    0.0
}

/// BO2's Jump_IsPlayerAboveMax: at or past the top of the jump.
fn above_max(avatar: &AvatarState) -> bool {
    avatar.takeoff_height + JUMP_HEIGHT <= avatar.origin[2]
}

/// BO2's Jump_ClampVelocity: a jump that rose from `before` climbs no
/// faster than takes it to the top of the jump.
fn clamp_velocity(avatar: &mut AvatarState, before: [f32; 3]) {
    if avatar.origin[2] - before[2] > 0.0 {
        let room = avatar.takeoff_height + JUMP_HEIGHT - avatar.origin[2];
        if room < 0.1 {
            avatar.velocity[2] = 0.0;
            return;
        }
        let top = libm::sqrtf(avatar.gravity as f32 * room * 2.0);
        if top < avatar.velocity[2] {
            avatar.velocity[2] = top;
        }
    }
}

/// BO2 after a move on the ground: a prone player must still have room to
/// lie where they ended up. If not, they go back to where they started; if
/// that spot is no better, the move stands. False when the move was taken
/// back.
fn verify_prone_position<C: SolidProbe>(
    avatar: &mut AvatarState,
    solids: &C,
    old_origin: [f32; 3],
    old_velocity: [f32; 3],
    zombies: bool,
) -> bool {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0 || prone_fits(avatar, solids, zombies) {
        return true;
    }
    let (new_origin, new_velocity) = (avatar.origin, avatar.velocity);
    avatar.origin = old_origin;
    avatar.velocity = old_velocity;
    if prone_fits(avatar, solids, zombies) {
        return false;
    }
    avatar.origin = new_origin;
    avatar.velocity = new_velocity;
    true
}

fn prone_fits<C: SolidProbe>(avatar: &mut AvatarState, solids: &C, zombies: bool) -> bool {
    let (mut torso, mut waist) = (avatar.f_torso_pitch, avatar.f_waist_pitch);
    let fits = check_prone(
        solids,
        avatar.origin,
        PRONE_VERIFY_SIZE,
        BELLY_CLEARANCE_PROBE,
        avatar.prone_direction,
        true,
        true,
        true,
        ProneRules::for_player(avatar, zombies),
        BELLY_FOOT_REACH,
        Some(&mut torso),
        Some(&mut waist),
    );
    avatar.f_torso_pitch = torso;
    avatar.f_waist_pitch = waist;
    fits
}

/// BO2 on a stair: a step up or down on the ground costs speed by its
/// height, and one of more than 3 units moves the walk rhythm on, which can
/// sound a footstep. Returns the step's height for the camera (0 when it
/// does not count).
fn stair_step(avatar: &mut AvatarState, tick_vars: &TickLocals, start_origin: [f32; 3], lowered_spot: [f32; 3], step_size: f32) -> f32 {
    let rise = avatar.origin[2] - lowered_spot[2];
    if libm::fabsf(rise) <= STEP_SLOW_MIN_RISE {
        return 0.0;
    }
    let delta = libm::rint(rise as f64 + STEP_ROUND_BIAS) as i32;
    if delta == 0 {
        return 0.0;
    }
    let scale = (1.0 - libm::fabsf(avatar.origin[2] - start_origin[2]) / step_size) * STEP_SLOW_RANGE + STEP_SLOW_FLOOR;
    for v in &mut avatar.velocity {
        *v *= scale;
    }
    // The step's footstep goes by the speed the stair left.
    let ground_velocity = libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1]);
    let delta = delta.abs();
    if delta > 3 && avatar.standing_on != NO_ENTITY_SLOT {
        let beats = (delta / 2).min(4);
        let old = avatar.sway_phase;
        avatar.sway_phase = ((beats as f32 * STEP_BOB_PER_UNIT + STEP_BOB_BASE + old as f32) as i32) & 0xff;
        let footsteps = should_make_footsteps(avatar, ground_velocity);
        footstep_event(avatar, tick_vars.floor_probe[4], old, avatar.sway_phase, footsteps, ground_velocity);
    }
    rise
}

#[cfg(test)]
mod tests {
    use super::super::hull::player_hull;
    use super::super::testkit::{MASK, MAXS, MINS, World, block, floor, other_player};
    use super::super::{Shape, add};
    use super::*;
    use crate::testing::tick_vars;
    use trace_t6::ENTITYNUM_WORLD;

    const REST_Z: f32 = 0.125;
    const REACH: f32 = 14.625 / 0.965_925_8 + 0.5;

    fn standing(x: f32, vx: f32) -> AvatarState {
        AvatarState {
            origin: [x, 0.0, REST_Z],
            velocity: [vx, 0.0, 0.0],
            gravity: 800,
            standing_on: i32::from(ENTITYNUM_WORLD),
            ..AvatarState::ZERO
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
        let mut avatar = standing(-20.0, 600.0);
        let rise = step_slide_move(&mut avatar, &tick_vars(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(avatar.origin[2], 12.0 + REST_Z, 0.01), "{:?}", avatar.origin);
        assert!(near(avatar.origin[0], 10.0, 0.01), "{:?}", avatar.origin);
        assert!(near(rise, 12.0, 0.01), "{rise}");
        let scale = (1.0 - 12.0 / 18.0) * 0.8 + f32::from_bits(0x3e4c_cccc);
        assert!(near(avatar.velocity[0], 600.0 * scale, 0.1), "{:?}", avatar.velocity);
        // 12 units moves the walk rhythm on four beats.
        assert_eq!(avatar.sway_phase, 12);
    }

    /// 24 units is more than a step: stopped at its face, nothing climbed.
    #[test]
    fn a_step_too_high() {
        let world = World::new(alloc::vec![floor(), block([0.0, -256.0, -16.0], [256.0, 256.0, 24.0])]);
        let mut avatar = standing(-20.0, 600.0);
        let rise = step_slide_move(&mut avatar, &tick_vars(true), &world, MINS, MAXS, MASK, false, false);
        assert_eq!(rise, 0.0);
        assert!(near(avatar.origin[0], -REACH, 0.05), "{:?}", avatar.origin);
        assert!(near(avatar.origin[2], REST_Z, 0.01), "{:?}", avatar.origin);
    }

    /// Off a 6-unit ledge: down onto the lower floor, which costs speed
    /// too.
    #[test]
    fn down_a_ledge() {
        let world = World::new(alloc::vec![
            block([-256.0, -256.0, -16.0], [0.0, 256.0, 0.0]),
            block([0.0, -256.0, -22.0], [256.0, 256.0, -6.0]),
        ]);
        let mut avatar = standing(-20.0, 1000.0);
        let rise = step_slide_move(&mut avatar, &tick_vars(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(avatar.origin[2], REST_Z - 6.0, 0.01), "{:?}", avatar.origin);
        assert!(near(rise, -6.0, 0.01), "{rise}");
        let scale = (1.0 - 6.0 / 18.0) * 0.8 + f32::from_bits(0x3e4c_cccc);
        assert!(near(avatar.velocity[0], 1000.0 * scale, 0.1), "{:?}", avatar.velocity);
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
        lying.lo_corner = add(lower, lo);
        lying.hi_corner = add(lower, hi);
        let ledge = || {
            alloc::vec![
                block([-256.0, -256.0, -16.0], [0.0, 256.0, 0.0]),
                block([0.0, -256.0, -22.0], [256.0, 256.0, -6.0]),
            ]
        };
        let mut shapes = ledge();
        shapes.push(lying);
        let world = World::new(shapes);
        let mut avatar = standing(-20.0, 1000.0);
        let rise = step_slide_move(&mut avatar, &tick_vars(true), &world, MINS, MAXS, MASK, false, false);
        assert_eq!(rise, 0.0);
        assert!(near(avatar.origin[0], 30.0, 0.01), "{:?}", avatar.origin);
        assert!(near(avatar.origin[2], REST_Z, 0.01), "{:?}", avatar.origin);
        assert_eq!(avatar.velocity, [1000.0, 0.0, 0.0]);

        // The same ledge with nobody there: down onto the lower floor.
        let world = World::new(ledge());
        let mut avatar = standing(-20.0, 1000.0);
        step_slide_move(&mut avatar, &tick_vars(true), &world, MINS, MAXS, MASK, false, false);
        assert!(near(avatar.origin[2], REST_Z - 6.0, 0.01), "{:?}", avatar.origin);
    }

    /// In the air, meeting nothing: just the fall, no step.
    #[test]
    fn falling_never_steps() {
        let world = World::new(alloc::vec![floor()]);
        let mut avatar = standing(0.0, 100.0);
        avatar.origin[2] = 100.0;
        avatar.standing_on = NO_ENTITY_SLOT;
        let rise = step_slide_move(&mut avatar, &tick_vars(false), &world, MINS, MAXS, MASK, true, false);
        assert_eq!(rise, 0.0);
        assert!(near(avatar.origin[2], 99.0, 1e-3), "{:?}", avatar.origin);
        assert!(near(avatar.velocity[2], -40.0, 1e-3), "{:?}", avatar.velocity);
    }
}
