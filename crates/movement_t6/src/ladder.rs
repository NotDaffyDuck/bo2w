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

use game_math::degree_gap;
use crate::bridge::{
    RungGrabArgs, RUNG_LEAP_LOCKOUT_MS, RUNG_PROBE_REACH_AIRBORNE, RUNG_PROBE_REACH_FOOTED,
    RungGripSource, RungProbeContact, HullExtents, TickLocals, FLOOR_IS_RUNGS, FootedStepArgs, accelerate,
    ladder_attract_velocity, step_slide_move,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::AvatarState;
use playerstate_t6::game::InputCommand;
use playerstate_t6::game::movement_state_bits;

use crate::air::{air_move, cmd_scale};
use crate::frame::BodyAnims;
use crate::gjk::Bo2Collision;
use crate::view::bo2_vectoyaw;

/// The fast-climb perk (BO2's specialty_fastladderclimb, perk 17).
pub use playerstate_t6::perks::PERK_FASTLADDERCLIMB;

/// BO2's `perk_fastLadderClimbMultiplier`.
const FAST_LADDER_CLIMB_SCALE: f32 = 1.75;

/// A timer from a landing or a push (movement_state_bits 0x80 and 0x100, the ones
/// PM_DropTimers ends) keeps the player off ladders while it runs.
const TIMER_FLAGS: u32 = 0x180;

/// The prone view heights (PM_GetEffectiveStance): a prone player cannot
/// hold a ladder.
const PRONE_VIEW_HEIGHTS: [i32; 2] = [22, 11];

/// Pitch to climb rate: (sine of the pitch + 0.25) * 2.5, held to -1..1.
const PITCH_OFFSET: f32 = 0.25;
const PITCH_GAIN: f32 = 2.5;

/// How hard the keys push: half speed up and down, a fifth sideways, with
/// acceleration 9.
const UP_PUSH: f32 = 0.5;
const SIDE_PUSH: f32 = 0.2;
const CLIMB_ACCELERATION: f32 = 9.0;

/// Sideways drift with no sideways key falls by 16 times the frame's time.
const DRIFT_DECAY: f32 = 16.0;

/// On a ladder the legs turn at most this far from the view.
const MOVEMENT_DIR_CAP: i32 = 75;

/// Which entity the last ladder trace hit, so a ladder on a moving thing
/// carries the climber.
pub(crate) trait LadderHitEntity {
    /// The entity the last `rung_probe` hit (the world counts as one), or
    /// `None` when it hit no entity.
    fn last_hit_entity(&self) -> Option<i32>;
}

fn negated(v: [f32; 3]) -> [f32; 3] {
    v.map(|c| -c)
}

/// A zero (or broken) length divides as 1.
fn nonzero(len: f32) -> f32 {
    if 0.0 <= -len { 1.0 } else { len }
}

/// The x and y of a direction scaled to unit length on the floor (z = 0).
fn floor_unit(x: f32, y: f32) -> [f32; 3] {
    let k = 1.0 / nonzero(libm::sqrtf(x * x + y * y));
    [x * k, y * k, 0.0]
}

/// A trace that stopped on a ladder's face.
fn is_ladder(strike: &RungProbeContact) -> bool {
    strike.surf_bits & FLOOR_IS_RUNGS != 0 && strike.travelled < 1.0
}

/// Why a player cannot hold a ladder this frame, short of finding none.
fn barred(avatar: &AvatarState, authority_clock_ms: i32) -> bool {
    let lying = PRONE_VIEW_HEIGHTS.contains(&avatar.eye_height_goal);
    let just_jumped = authority_clock_ms.wrapping_sub(avatar.hop_start_ms) < RUNG_LEAP_LOCKOUT_MS;
    avatar.movement_state_bits & movement_state_bits::LADDER_FALL != 0 || lying || just_jumped || crate::gun::riot_shield(avatar)
}

/// PM_CheckLadderMove: hold on, grab, or let go (and fall). True when a
/// player hanging on in the air lost the ladder: the legs play the jump.
pub(crate) fn check_ladder_move(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    frame_ctx: RungGrabArgs,
    backend: &mut (impl RungGripSource + LadderHitEntity),
) -> bool {
    // Standing, or under water, ends a fall off a ladder.
    if avatar.waterlevel > 3 || frame_ctx.on_foot {
        avatar.movement_state_bits &= !movement_state_bits::LADDER_FALL;
    }
    let holding = avatar.movement_state_bits & movement_state_bits::LADDER != 0;
    let timer_running = avatar.move_timer != 0 && avatar.movement_state_bits & TIMER_FLAGS != 0;
    if timer_running && !holding {
        return false;
    }
    if avatar.movement_mode >= MOVE_KIND_DEAD {
        // The dead leave the ground as well as the ladder.
        avatar.standing_on = NO_ENTITY_SLOT;
        tick_vars.leave_ground();
        release(avatar);
        return false;
    }
    if barred(avatar, frame_ctx.authority_clock_ms) {
        release(avatar);
        return false;
    }
    let reach = match frame_ctx.on_foot {
        true => RUNG_PROBE_REACH_FOOTED,
        false => RUNG_PROBE_REACH_AIRBORNE,
    };
    // In the air on a ladder the probe goes into its face; otherwise along
    // the flat view.
    let airborne_on_it = holding && avatar.standing_on == NO_ENTITY_SLOT;
    let probe = match airborne_on_it {
        true => negated(avatar.ladder_normal_dir),
        false => floor_unit(frame_ctx.ahead_planar[0], frame_ctx.ahead_planar[1]),
    };
    let found = backend.rung_probe(avatar.origin, probe, reach).filter(is_ladder);
    // From the ground only pushing forward takes hold.
    let pushing = frame_ctx.thrust_ahead > 0 || !frame_ctx.on_foot;
    let face = match found {
        Some(strike) if pushing => strike.facing,
        _ => {
            release(avatar);
            return airborne_on_it;
        }
    };
    if !holding {
        // A fresh ladder: keep its face, and take it only if looking back
        // at that face finds the ladder too.
        avatar.ladder_normal_dir = face;
        let confirmed = backend.rung_probe(avatar.origin, negated(face), reach).is_some_and(|h| is_ladder(&h));
        if !confirmed {
            return false;
        }
        avatar.movement_state_bits |= movement_state_bits::LADDER;
    }
    // The ladder's entity becomes the mover the climber rides.
    if let Some(entity) = backend.last_hit_entity() {
        avatar.mover_entity_num = entity;
    }
    false
}

/// Off the ladder, the player falls (and cannot grab one until landing).
fn release(avatar: &mut AvatarState) {
    if avatar.movement_state_bits & movement_state_bits::LADDER == 0 {
        return;
    }
    avatar.movement_state_bits &= !movement_state_bits::LADDER;
    avatar.movement_state_bits |= movement_state_bits::LADDER_FALL;
}

/// PM_LadderMove: a jump pushes off; otherwise looking up or down with the
/// forward key climbs, the sideways keys move along the face, and the step
/// and slide runs without gravity.
pub(crate) fn ladder_move<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    ucmd: &mut InputCommand,
    frame_ctx: FootedStepArgs,
    bounds: HullExtents,
    solids: &C,
    body: &mut BodyAnims,
) -> f32 {
    let zombies = frame_ctx.feel.zombies;
    // Climbing keeps the ladder's mover from timing out.
    avatar.mover_timestamp = ucmd.authority_clock_ms;
    if crate::walk::jump_check(avatar, tick_vars, ucmd, &frame_ctx) {
        body.jump = true;
        return air_move(avatar, tick_vars, ucmd, frame_ctx.air, bounds, solids, zombies, frame_ctx.lunge);
    }

    let mut rate = ((tick_vars.fwd_axis[2] + PITCH_OFFSET) * PITCH_GAIN).clamp(-1.0, 1.0);
    tick_vars.fwd_axis = floor_unit(tick_vars.fwd_axis[0], tick_vars.fwd_axis[1]);
    // Sideways: flattened, then laid on the face, and BO2 leaves it short.
    tick_vars.right_axis = onto_plane(floor_unit(tick_vars.right_axis[0], tick_vars.right_axis[1]), avatar.ladder_normal_dir);

    let scale = cmd_scale(avatar, ucmd, frame_ctx.air.observer_pace_ratio);
    if avatar.has_perk_bit(PERK_FASTLADDERCLIMB) {
        rate *= FAST_LADDER_CLIMB_SCALE;
    }
    push_along_ladder(avatar, tick_vars, ucmd, rate, scale);

    if ucmd.thrust_ahead == 0 {
        // No forward key: gravity's worth of the climb is taken away, never
        // past a stop.
        let fall = avatar.gravity as f32 * tick_vars.step_millis_elapsed;
        let vz = avatar.velocity[2];
        let next = match vz <= 0.0 {
            true => Some(fall + vz).filter(|z| *z < 0.0),
            false => Some(vz - fall).filter(|z| *z > 0.0),
        };
        avatar.velocity[2] = next.unwrap_or(0.0);
    }
    if ucmd.thrust_lateral == 0 {
        side_damping(avatar, tick_vars.right_axis, tick_vars.step_millis_elapsed);
    }
    if tick_vars.walking == 0 {
        ladder_attract_velocity(avatar);
    }

    let HullExtents { lo_corner: box_lo, hi_corner: box_hi, collide_filter: mask } = bounds;
    let stepped = match solids.gjk_on() {
        true => crate::gjk::step::step_slide_move(avatar, tick_vars, solids, box_lo, box_hi, mask, false, zombies),
        false => step_slide_move(avatar, tick_vars, solids, box_lo, box_hi, mask, None, zombies),
    };

    // The legs turn toward the ladder, within the cap of the view.
    let toward_ladder = bo2_vectoyaw(avatar.ladder_normal_dir[0], avatar.ladder_normal_dir[1]) + 180.0;
    let turn = (degree_gap(toward_ladder, avatar.look_angles[1]) as i32).clamp(-MOVEMENT_DIR_CAP, MOVEMENT_DIR_CAP);
    avatar.move_heading = i32::from(turn as i8);
    stepped
}

/// The keys' wish: forward climbs at `rate`, sideways runs along the face;
/// accelerated toward at the ladder's rate.
fn push_along_ladder(avatar: &mut AvatarState, tick_vars: &mut TickLocals, ucmd: &InputCommand, rate: f32, factor: f32) {
    let climb = match ucmd.thrust_ahead {
        0 => 0.0,
        f => rate * UP_PUSH * factor * f32::from(f),
    };
    let mut wish = [0.0, 0.0, climb];
    if ucmd.thrust_lateral != 0 {
        let side = factor * SIDE_PUSH * f32::from(ucmd.thrust_lateral);
        let r = tick_vars.right_axis;
        wish = [r[0] * side, r[1] * side, wish[2] + r[2] * side];
    }
    let speed = libm::sqrtf(wish[0] * wish[0] + wish[1] * wish[1] + wish[2] * wish[2]);
    let k = 1.0 / nonzero(speed);
    accelerate(avatar, tick_vars, &wish.map(|c| c * k), speed, CLIMB_ACCELERATION);
}

/// ProjectPointOnPlane: `p` less its part along the (unit) `n`.
fn onto_plane(p: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let along = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
    let back = -along;
    [p[0] + back * n[0], p[1] + back * n[1], p[2] + back * n[2]]
}

/// With no sideways key, the drift along the face falls by 16 times the
/// frame's time. Under one unit the step is a whole unit, so a drift under
/// one unit a second turns round (BO2 does this too).
fn side_damping(avatar: &mut AvatarState, side_axis: [f32; 3], step_millis_elapsed: f32) {
    let axis = floor_unit(side_axis[0], side_axis[1]);
    let drift = axis[0] * avatar.velocity[0] + axis[1] * avatar.velocity[1];
    if drift == 0.0 {
        return;
    }
    let mut add_along = |amount: f32| {
        avatar.velocity[0] += axis[0] * amount;
        avatar.velocity[1] += axis[1] * amount;
    };
    // All of it goes, then what survives the decay comes back.
    add_along(-drift);
    let decay = step_millis_elapsed * drift * DRIFT_DECAY;
    let survives = decay.abs() < drift.abs();
    if survives {
        let step = match decay.abs() < 1.0 {
            false => decay,
            true if decay >= 0.0 => 1.0,
            true => -1.0,
        };
        add_along(drift - step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use crate::testing::{Open, tick_vars, walk_context};
    use crate::gun::t6::{DETONATING, RIOTSHIELD_END, RIOTSHIELD_PREPARE};

    /// A ladder face looking along -x, half way along the probe.
    const FACE: RungProbeContact = RungProbeContact {
        travelled: 0.5,
        facing: [-1.0, 0.0, 0.0],
        surf_bits: FLOOR_IS_RUNGS,
    };

    /// Answers every probe the same way and notes where each one looked.
    #[derive(Default)]
    struct Rail {
        answer: Option<RungProbeContact>,
        looked: Vec<[f32; 3]>,
        entity: Option<i32>,
    }

    impl Rail {
        fn face() -> Self {
            Rail { answer: Some(FACE), ..Rail::default() }
        }
    }

    impl RungGripSource for Rail {
        fn rung_probe(&mut self, _: [f32; 3], heading: [f32; 3], _: f32) -> Option<RungProbeContact> {
            self.looked.push(heading);
            self.answer
        }
    }

    impl LadderHitEntity for Rail {
        fn last_hit_entity(&self) -> Option<i32> {
            self.entity
        }
    }

    /// Runs the check at t = 10 s, looking along +x.
    fn grab(avatar: &mut AvatarState, on_foot: bool, thrust_ahead: i8, rail: &mut Rail) -> (bool, TickLocals) {
        let mut tick_vars = tick_vars(on_foot);
        let frame_ctx = RungGrabArgs { authority_clock_ms: 10_000, on_foot: on_foot, ahead_planar: [2.0, 0.0], thrust_ahead };
        let jumped = check_ladder_move(avatar, &mut tick_vars, frame_ctx, rail);
        (jumped, tick_vars)
    }

    fn standing() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.eye_height_goal = 60;
        avatar
    }

    fn hanging() -> AvatarState {
        let mut avatar = standing();
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.ladder_normal_dir = FACE.facing;
        avatar
    }

    fn holds(avatar: &AvatarState) -> bool {
        avatar.movement_state_bits & movement_state_bits::LADDER != 0
    }

    fn falling(avatar: &AvatarState) -> bool {
        avatar.movement_state_bits & movement_state_bits::LADDER_FALL != 0
    }

    #[test]
    fn walking_into_a_ladder_grabs_it() {
        let (mut avatar, mut rail) = (standing(), Rail::face());
        grab(&mut avatar, true, 127, &mut rail);
        assert!(holds(&avatar));
        assert_eq!(avatar.ladder_normal_dir, FACE.facing);
        // Ahead first, then back at the face.
        assert_eq!(rail.looked, [[1.0, 0.0, 0.0]; 2]);
    }

    #[test]
    fn standing_still_on_the_ground_does_not_grab() {
        let mut avatar = standing();
        grab(&mut avatar, true, 0, &mut Rail::face());
        assert!(!holds(&avatar));
    }

    #[test]
    fn hanging_on_looks_behind_the_face() {
        let (mut avatar, mut rail) = (hanging(), Rail::face());
        grab(&mut avatar, false, 0, &mut rail);
        assert!(holds(&avatar));
        assert_eq!(rail.looked, [[1.0, 0.0, 0.0]]);
    }

    #[test]
    fn the_riot_shield_lets_go_but_a_detonator_does_not() {
        for state in RIOTSHIELD_PREPARE..=RIOTSHIELD_END {
            let mut avatar = hanging();
            avatar.weapon_state_t6 = state;
            grab(&mut avatar, false, 0, &mut Rail::face());
            assert!(!holds(&avatar) && falling(&avatar), "{state}");
        }
        let mut avatar = hanging();
        avatar.weapon_state_t6 = DETONATING;
        grab(&mut avatar, false, 0, &mut Rail::face());
        assert!(holds(&avatar));
    }

    #[test]
    fn losing_the_ladder_in_the_air_plays_the_jump() {
        let gone = |avatar: &mut AvatarState| grab(avatar, false, 0, &mut Rail::default()).0;
        let mut avatar = hanging();
        assert!(gone(&mut avatar));
        assert!(falling(&avatar));
        // Still holding: no jump.
        assert!(!grab(&mut hanging(), false, 0, &mut Rail::face()).0);
        // Lost while on the ground, or never held: no jump.
        let mut grounded = hanging();
        grounded.standing_on = 0;
        assert!(!gone(&mut grounded));
        let mut never = standing();
        never.standing_on = NO_ENTITY_SLOT;
        assert!(!gone(&mut never));
        // The riot shield, lying down, a fresh jump or dying let go without it.
        let tweaks: [fn(&mut AvatarState); 4] = [
            |avatar| avatar.weapon_state_t6 = RIOTSHIELD_PREPARE,
            |avatar| avatar.eye_height_goal = PRONE_VIEW_HEIGHTS[0],
            |avatar| avatar.hop_start_ms = 10_000,
            |avatar| avatar.movement_mode = MOVE_KIND_DEAD,
        ];
        for tweak in tweaks {
            let mut avatar = hanging();
            tweak(&mut avatar);
            assert!(!gone(&mut avatar));
            assert!(!holds(&avatar));
        }
    }

    #[test]
    fn a_ladder_on_a_moving_thing_is_ridden() {
        let mut rail = Rail { entity: Some(40), ..Rail::face() };
        // Grabbing one.
        let mut avatar = standing();
        grab(&mut avatar, false, 0, &mut rail);
        assert!(holds(&avatar));
        assert_eq!(avatar.mover_entity_num, 40);
        // Holding on keeps it.
        let mut avatar = hanging();
        grab(&mut avatar, false, 0, &mut rail);
        assert_eq!(avatar.mover_entity_num, 40);
        // A trace that hit no entity leaves the mover alone.
        let mut avatar = hanging();
        avatar.mover_entity_num = 41;
        grab(&mut avatar, false, 0, &mut Rail::face());
        assert_eq!(avatar.mover_entity_num, 41);
    }

    #[test]
    fn prone_and_a_fresh_jump_let_go() {
        for (height, hop_start_ms) in [(11, 0), (22, 0), (60, 9_800)] {
            let mut avatar = hanging();
            avatar.eye_height_goal = height;
            avatar.hop_start_ms = hop_start_ms;
            grab(&mut avatar, false, 0, &mut Rail::face());
            assert!(!holds(&avatar), "{height} {hop_start_ms}");
        }
    }

    #[test]
    fn the_dead_let_go_and_leave_the_ground() {
        let mut avatar = hanging();
        avatar.movement_mode = MOVE_KIND_DEAD;
        avatar.standing_on = 1022;
        let (_, tick_vars) = grab(&mut avatar, true, 127, &mut Rail::face());
        assert_eq!(avatar.standing_on, NO_ENTITY_SLOT);
        assert_eq!([tick_vars.walking, tick_vars.standing_on_floor, tick_vars.near_floor_contact], [0; 3]);
        assert!(falling(&avatar));
    }

    #[test]
    fn a_running_timer_skips_the_check() {
        let mut avatar = standing();
        avatar.move_timer = 100;
        avatar.movement_state_bits = 0x80;
        let mut rail = Rail::face();
        grab(&mut avatar, true, 127, &mut rail);
        assert!(rail.looked.is_empty());
    }

    #[test]
    fn side_drift_dies_away() {
        let along_x = [1.0, 0.0, 0.0];
        let damped = |velocity: [f32; 3], step_millis_elapsed: f32| {
            let mut avatar = AvatarState::ZERO;
            avatar.velocity = velocity;
            side_damping(&mut avatar, along_x, step_millis_elapsed);
            avatar.velocity
        };
        let v = damped([100.0, 0.0, 0.0], 0.05);
        assert!((v[0] - 20.0).abs() < 1e-4, "{}", v[0]);
        // A long frame stops it dead.
        assert_eq!(damped([100.0, 0.0, 0.0], 0.1)[0], 0.0);
        // Under one unit the drift turns round.
        let v = damped([0.5, 0.0, 0.0], 0.05);
        assert!((v[0] + 0.5).abs() < 1e-6, "{}", v[0]);
        // Up and down are left alone.
        assert_eq!(damped([0.0, 0.0, 50.0], 0.05), [0.0, 0.0, 50.0]);
    }

    /// The climber: hanging on, 190 speed, 800 gravity.
    fn climber() -> AvatarState {
        let mut avatar = hanging();
        avatar.speed = 190;
        avatar.gravity = 800_i32;
        avatar
    }

    const LEFT: [f32; 3] = [0.0, -1.0, 0.0];

    /// One climbing frame with these view directions and keys; gives back
    /// the frame's directions afterwards.
    fn climb(avatar: &mut AvatarState, fwd: [f32; 3], rgt: [f32; 3], keys: [i8; 2]) -> TickLocals {
        let mut tick_vars = tick_vars(false);
        tick_vars.fwd_axis = fwd;
        tick_vars.right_axis = rgt;
        let mut ucmd = InputCommand {
            authority_clock_ms: 10_000,
            thrust_ahead: keys[0],
            thrust_lateral: keys[1],
            ..InputCommand::default()
        };
        let bounds = HullExtents {
            lo_corner: [-15.0_f32, -15.0, 0.0],
            hi_corner: [15.0_f32, 15.0, 70.0],
            collide_filter: 1,
        };
        ladder_move(avatar, &mut tick_vars, &mut ucmd, walk_context(), bounds, &Open, &mut BodyAnims::default());
        tick_vars
    }

    #[test]
    fn climbing_stamps_the_movers_time() {
        let mut avatar = climber();
        avatar.mover_timestamp = 0;
        climb(&mut avatar, [1.0, 0.0, 0.0], LEFT, [0, 0]);
        assert_eq!(avatar.mover_timestamp, 10_000);
    }

    #[test]
    fn the_perk_climbs_175_times_as_fast() {
        // Looking well up: the climb is at its full rate.
        let up = [0.6, 0.0, 0.8];
        let mut plain = climber();
        let mut quick = climber();
        quick.set_perk(PERK_FASTLADDERCLIMB, true);
        for avatar in [&mut plain, &mut quick] {
            climb(avatar, up, LEFT, [127, 0]);
        }
        assert!(plain.velocity[2] > 0.0);
        let ratio = quick.velocity[2] / plain.velocity[2];
        assert!((ratio - FAST_LADDER_CLIMB_SCALE).abs() < 1e-4, "{ratio}");
    }

    #[test]
    fn looking_down_climbs_down() {
        let mut avatar = climber();
        climb(&mut avatar, [0.6, 0.0, -0.8], LEFT, [127, 0]);
        assert!(avatar.velocity[2] < 0.0);
    }

    #[test]
    fn the_sideways_direction_keeps_its_length_on_the_face() {
        let tick_vars = climb(&mut climber(), [0.8, 0.6, 0.0], [0.6, -0.8, 0.0], [0, 0]);
        assert!(tick_vars.right_axis[0].abs() < 1e-6, "{:?}", tick_vars.right_axis);
        assert!((tick_vars.right_axis[1] + 0.8).abs() < 1e-6, "{:?}", tick_vars.right_axis);
    }

    #[test]
    fn the_legs_face_the_ladder_within_75_degrees() {
        // The ladder's yaw comes out a hair under 180 in single precision
        // and the angle is cut toward zero (30 would give -29), so test off
        // the whole number.
        let mut avatar = climber();
        for (yaw, legs) in [(30.5, -30), (100.0, -MOVEMENT_DIR_CAP), (-120.0, MOVEMENT_DIR_CAP)] {
            avatar.look_angles[1] = yaw;
            climb(&mut avatar, [1.0, 0.0, 0.0], LEFT, [0, 0]);
            assert_eq!(avatar.move_heading, legs, "{yaw}");
        }
    }

    #[test]
    fn no_forward_key_slows_the_climb_to_a_stop() {
        let mut avatar = climber();
        avatar.velocity = [0.0_f32, 0.0, 30.0];
        climb(&mut avatar, [1.0, 0.0, 0.0], LEFT, [0, 0]);
        assert_eq!(avatar.velocity[2], 0.0);
        avatar.velocity = [0.0_f32, 0.0, -50.0];
        climb(&mut avatar, [1.0, 0.0, 0.0], LEFT, [0, 0]);
        assert!((avatar.velocity[2] + 10.0).abs() < 1e-4, "{}", avatar.velocity[2]);
    }
}
