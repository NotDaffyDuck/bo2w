//! Black Ops II's PM_Footsteps: the walk rhythm (bobCycle) that drives the
//! head bob and the footstep sounds. On the ground the rhythm runs with the
//! speed against the stance's top speed, and a step sounds each time it
//! passes a quarter mark into the second or fourth quarter, if the player
//! moves at least 10 units a second. Crouched and lying down sound their own
//! steps; standing still (under 1 unit a second) puts the rhythm back to the
//! start. On a ladder the rungs sound only off the ground, at the climbing
//! speed's own pace.
//!
//! Crouched steps (run and walk) and a ladder's hand-over-hand rungs are
//! BO2's own events; they play its fly_step_crouch_run,
//! crouch_walk and ladder_hand sounds.
//!
//! In water: chest deep and over (level 4 up) the rhythm runs at the
//! swimming pace, keys or no keys, moving or not, and standing still no
//! longer puts it back; from level 5 no step sounds; and any water makes the
//! ground sound as water. Off the ground in water the ground's rules run
//! (the ladder's need dry feet).
//!
//! First of all, a living player's ground pitch joins the slope average
//! (`slope.rs`), in any move.
//!
//! Not here: vehicle rhythms (bo2mp has none), the slide
//! flag (nothing sets it), the sprint-rhythm exception for otherFlags
//! 0x1000000, and the leg animations PM_Footsteps also picks (the sim picks
//! them once per command).

use crate::bridge::{
    Bo2Feel, StrideScaleArgs, RUNG_MATERIAL, TickLocals, queue_forecast_happening,
    footsteps_bob_cycle, is_in_air,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::AvatarState;
use playerstate_t6::game::InputCommand;
use playerstate_t6::game::movement_state_bits;

use crate::dive;
use playerstate_t6::game::entity_flag_bits::TURRET_MASK;
use crate::slope::{PitchHistory, calc_player_pitch};

/// player_moveThreshhold: slower than this does not count as moving.
const PLAYER_MOVE_THRESHHOLD: f32 = 2.0;
/// player_footstepsThreshhold: slower than this makes no footsteps.
const PLAYER_FOOTSTEPS_THRESHHOLD: f32 = 10.0;
/// player_runThreshhold: this fast or faster sounds as running.
const PLAYER_RUN_THRESHHOLD: f32 = 110.0;
/// Under this speed, standing still puts the rhythm back to the start.
const STOP_SPEED: f32 = 1.0;

/// A ladder's rungs stay quiet for this long after a jump.
const LADDER_JUMP_QUIET_MS: i32 = 299;
/// Ladder rhythm per unit of climbing speed (1 / 95.25), and its pace.
const LADDER_CLIMB_PER_UNIT: f32 = 0.010_498_688;
const RUNG_SWAY: f32 = 0.45;
/// Walking (or leaning) on a ladder: 1 / 38.1 per unit, at its own pace.
const LADDER_WALK_PER_UNIT: f32 = 0.026_246_719;
const LADDER_WALK_BOB: f32 = 0.35;

/// The rhythm's quarter marks: a step can sound when it crosses one.
const QUARTER_MASK: i32 = !0x3f;
/// The rhythm is in its second or fourth quarter: a step's moment.
const STEP_HALF: i32 = 0x7f;
const STEP_QUARTER: i32 = 0x40;

/// A surface marked to make no footsteps (it still sounds, as surface 0).
const FLOOR_SILENT_FEET: u32 = 0x2000;

const SIGNAL_DASH_STEP: i32 = entity_t6::event::FOOTSTEP_SPRINT as i32;
const SIGNAL_JOG_STEP: i32 = entity_t6::event::FOOTSTEP_RUN as i32;
const SIGNAL_STROLL_STEP: i32 = entity_t6::event::FOOTSTEP_WALK as i32;
const SIGNAL_CRAWL_STEP: i32 = entity_t6::event::FOOTSTEP_PRONE as i32;
const EV_FOOTSTEP_CROUCH_RUN: i32 = entity_t6::event::FOOTSTEP_CROUCHRUN as i32;
const EV_FOOTSTEP_CROUCH_WALK: i32 = entity_t6::event::FOOTSTEP_CROUCHWALK as i32;
const EV_LADDER_HAND_GRAB: i32 = entity_t6::event::LADDER_HAND_GRAB as i32;

/// Set after a foot on a ladder: the next rung is a hand. BO2's 0x4000000,
/// which ours gives to the dive.
const PMF_LADDER_HAND: u32 = 0x1000_0000;

/// bg_weaponBobFrequencySwimming: the rhythm's pace swimming.
const SWIM_BOB: f32 = 0.1;
/// From this water level the swimming pace takes over.
const SWIM_LEVEL: i32 = 4;
/// From this water level no step sounds.
const SILENT_LEVEL: i32 = 5;
/// The surface any water sounds as.
pub(crate) const WATER_SURFACE_TYPE: i32 = 20;

/// One frame of the rhythm and its footstep, after the move.
pub(crate) fn footsteps(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    ucmd: &InputCommand,
    scales: StrideScaleArgs,
    feel: Bo2Feel,
    pitch: &mut PitchHistory,
) {
    if avatar.movement_mode >= MOVE_KIND_DEAD {
        return;
    }
    pitch.update(calc_player_pitch(avatar, tick_vars));
    let ground_velocity = libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1]);
    let diving = dive::active(avatar);
    let bob = |avatar: &mut AvatarState| {
        footsteps_bob_cycle(
            avatar,
            tick_vars.frame_ms,
            ucmd.thrust_ahead,
            ucmd.thrust_lateral,
            tick_vars.near_floor_contact != 0,
            ucmd.authority_clock_ms,
            scales,
            feel,
        );
    };
    // A dive keeps its own rhythm, in the air too (and makes no steps).
    if diving {
        bob(avatar);
    }
    if avatar.e_flags & TURRET_MASK != 0 {
        return;
    }
    let mantling = avatar.movement_state_bits & movement_state_bits::MANTLE != 0;
    if is_in_air(avatar, tick_vars) && avatar.waterlevel == 0 && !diving && !mantling {
        ladder_footsteps(avatar, tick_vars.frame_ms, ucmd.authority_clock_ms);
        return;
    }
    let swimming = avatar.waterlevel >= SWIM_LEVEL;
    let moving = (ground_velocity >= PLAYER_MOVE_THRESHHOLD && avatar.movement_mode != 1) || mantling;
    if !moving {
        if ground_velocity < STOP_SPEED && !swimming {
            avatar.sway_phase = 0;
        }
        if swimming {
            swim_bob(avatar, tick_vars.frame_ms, feel);
        }
        return;
    }
    // A dive has its rhythm already.
    if diving {
        return;
    }
    // No keys (coasting) moves only the legs, unless swimming.
    if ucmd.thrust_ahead == 0 && ucmd.thrust_lateral == 0 {
        if swimming {
            swim_bob(avatar, tick_vars.frame_ms, feel);
        }
        return;
    }
    let old = avatar.sway_phase;
    if swimming {
        swim_bob(avatar, tick_vars.frame_ms, feel);
    } else {
        bob(avatar);
    }
    let make = should_make_footsteps(avatar, ground_velocity);
    footstep_event(avatar, tick_vars.floor_probe[4], old, avatar.sway_phase, make, ground_velocity);
}

/// BG_CalcBob in water: the swimming pace, whatever the speed, times the
/// gun's sprint scale (its crouched one when sprinting crouched).
fn swim_bob(avatar: &mut AvatarState, msec: i32, feel: Bo2Feel) {
    let scale = if avatar.movement_state_bits & movement_state_bits::SPRINTING != 0 && avatar.eye_height_goal == 40 {
        feel.ducked_sprint_cycle_scale
    } else {
        feel.sprint_cycle_scale
    };
    avatar.sway_phase = (msec as f32 * SWIM_BOB * scale + avatar.sway_phase as f32) as i32 & 0xff;
}

/// PM_ShouldMakeFootsteps: the rhythm is at a step's moment, the player
/// moves fast enough to be heard, and the water is under the shoulders.
pub(crate) fn should_make_footsteps(avatar: &AvatarState, ground_velocity: f32) -> bool {
    avatar.sway_phase & STEP_HALF >= STEP_QUARTER
        && avatar.waterlevel < SILENT_LEVEL
        && ground_velocity >= PLAYER_FOOTSTEPS_THRESHHOLD
}

/// PM_FootstepEvent: the rhythm crossed a quarter mark. Off the ground only
/// a ladder's rung sounds; on it the step's kind goes by stance and speed,
/// on the ground's surface.
pub(crate) fn footstep_event(
    avatar: &mut AvatarState,
    surf: u32,
    old: i32,
    new: i32,
    make: bool,
    ground_velocity: f32,
) {
    if (old ^ new) & QUARTER_MASK == 0 || !make {
        return;
    }
    if avatar.standing_on == NO_ENTITY_SLOT {
        // A ladder's rungs go foot (a run step), hand, foot, hand.
        if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
            let kind = if avatar.movement_state_bits & PMF_LADDER_HAND == 0 {
                SIGNAL_JOG_STEP
            } else {
                EV_LADDER_HAND_GRAB
            };
            avatar.movement_state_bits ^= PMF_LADDER_HAND;
            queue_forecast_happening(avatar, kind, RUNG_MATERIAL as i32);
        }
        return;
    }
    let kind = footstep_type(avatar, ground_velocity);
    let surface = ground_surface_type(avatar, surf);
    queue_forecast_happening(avatar, kind, surface);
}

/// PM_FootstepType: prone; crouched runs (110 or faster, not walking or
/// leaning) or walks; standing sprints, runs (110 or faster) or walks.
fn footstep_type(avatar: &AvatarState, ground_velocity: f32) -> i32 {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 {
        return SIGNAL_CRAWL_STEP;
    }
    let striding = avatar.movement_state_bits & movement_state_bits::WALKING == 0 && avatar.tilt_sideways == 0.0;
    if avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT != 0 {
        return if striding && ground_velocity >= PLAYER_RUN_THRESHHOLD {
            EV_FOOTSTEP_CROUCH_RUN
        } else {
            EV_FOOTSTEP_CROUCH_WALK
        };
    }
    if striding {
        if avatar.movement_state_bits & movement_state_bits::SPRINTING != 0 {
            return SIGNAL_DASH_STEP;
        }
        if ground_velocity >= PLAYER_RUN_THRESHHOLD {
            return SIGNAL_JOG_STEP;
        }
    }
    SIGNAL_STROLL_STEP
}

/// PM_GroundSurfaceType: the surface under the feet, 0 on a no-steps one,
/// water in any water.
pub(crate) fn ground_surface_type(avatar: &AvatarState, surf: u32) -> i32 {
    if surf & FLOOR_SILENT_FEET != 0 {
        return 0;
    }
    if avatar.waterlevel != 0 {
        return WATER_SURFACE_TYPE;
    }
    ((surf >> 20) & 0x3f) as i32
}

/// PM_Footstep_LadderMove: off the ground on a ladder, the rhythm follows the
/// climbing speed (down runs it backwards) and each rung can sound. Not on
/// a ladder: nothing (the legs only).
fn ladder_footsteps(avatar: &mut AvatarState, msec: i32, authority_clock_ms: i32) {
    if avatar.movement_state_bits & movement_state_bits::LADDER == 0 {
        return;
    }
    if authority_clock_ms.wrapping_sub(avatar.hop_start_ms) <= LADDER_JUMP_QUIET_MS {
        return;
    }
    let climb = avatar.velocity[2];
    let bobmove = if avatar.movement_state_bits & movement_state_bits::WALKING == 0 && avatar.tilt_sideways == 0.0 {
        climb * LADDER_CLIMB_PER_UNIT * RUNG_SWAY
    } else {
        climb * LADDER_WALK_PER_UNIT * LADDER_WALK_BOB
    };
    let old = avatar.sway_phase;
    avatar.sway_phase = (msec as f32 * bobmove + old as f32) as i32 & 0xff;
    let make = avatar.sway_phase & STEP_HALF >= STEP_QUARTER && avatar.waterlevel < SILENT_LEVEL;
    footstep_event(avatar, 0, old, avatar.sway_phase, make, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const STONE: u32 = 18 << 20;

    const SCALES: StrideScaleArgs = StrideScaleArgs {
        retreat_pace_ratio: 0.7,
        sidestep_pace_ratio: 0.8,
        dash_pace_ratio: 1.5,
        downed_crawl_pace_ratio: 0.15,
        gun_stride_ratio: 1.0,
        scoped_carry_pace: 1.0,
        shellshock_movement_scale: 1.0,
        prone_lerp_ms: 0,
    };

    fn feel() -> Bo2Feel {
        Bo2Feel {
            ..Bo2Feel::BO2_MP
        }
    }

    fn tick_vars(msec: i32) -> TickLocals {
        let mut floor_probe = [0; 11];
        floor_probe[4] = STONE;
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed: msec as f32 / 1000.0,
            frame_ms: msec,
            walking: 1,
            standing_on_floor: 1,
            near_floor_contact: 1,
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    fn run(avatar: &mut AvatarState, ucmd: &InputCommand) {
        let mut tick_vars = tick_vars(50);
        if avatar.standing_on == NO_ENTITY_SLOT {
            tick_vars.walking = 0;
            tick_vars.standing_on_floor = 0;
            tick_vars.near_floor_contact = 0;
        }
        footsteps(avatar, &tick_vars, ucmd, SCALES, feel(), &mut PitchHistory::default());
    }

    fn player(speed: f32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 1022;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.eye_height_goal = 60;
        avatar.velocity = [speed, 0.0, 0.0];
        avatar
    }

    fn forward() -> InputCommand {
        InputCommand {
            thrust_ahead: 127,
            authority_clock_ms: 10_000,
            ..InputCommand::default()
        }
    }

    fn event(avatar: &AvatarState) -> Option<(i32, i32)> {
        (avatar.predictable_events[0] != 0).then_some((avatar.predictable_events[0], avatar.predictable_event_parms[0] as i32))
    }

    /// Runs frames until a step sounds, or gives up after a second.
    fn first_step(avatar: &mut AvatarState) -> Option<(i32, i32)> {
        for _ in 0..20 {
            run(avatar, &forward());
            if let Some(step) = event(avatar) {
                return Some(step);
            }
        }
        None
    }

    #[test]
    fn the_step_kind_follows_the_speed() {
        assert_eq!(first_step(&mut player(190.0)).map(|e| e.0), Some(SIGNAL_JOG_STEP));
        assert_eq!(first_step(&mut player(80.0)).map(|e| e.0), Some(SIGNAL_STROLL_STEP));
        let mut avatar = player(280.0);
        avatar.movement_state_bits = movement_state_bits::SPRINTING;
        assert_eq!(first_step(&mut avatar).map(|e| e.0), Some(SIGNAL_DASH_STEP));
    }

    #[test]
    fn crouched_and_lying_down_make_their_own_steps() {
        let mut avatar = player(120.0);
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        avatar.eye_height_goal = 40;
        assert_eq!(first_step(&mut avatar).map(|e| e.0), Some(EV_FOOTSTEP_CROUCH_RUN));
        let mut avatar = player(60.0);
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        avatar.eye_height_goal = 40;
        assert_eq!(first_step(&mut avatar).map(|e| e.0), Some(EV_FOOTSTEP_CROUCH_WALK));
        let mut avatar = player(30.0);
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        avatar.eye_height_goal = 11;
        assert_eq!(first_step(&mut avatar).map(|e| e.0), Some(SIGNAL_CRAWL_STEP));
    }

    #[test]
    fn too_slow_to_hear_makes_no_step() {
        assert_eq!(first_step(&mut player(8.0)), None);
    }

    #[test]
    fn the_step_sounds_on_the_ground_surface() {
        let step = first_step(&mut player(190.0)).expect("a step");
        assert_eq!(step.1, 18);
    }

    #[test]
    fn a_no_steps_surface_still_sounds_as_surface_zero() {
        let mut avatar = player(190.0);
        avatar.sway_phase = 0x3f;
        footstep_event(&mut avatar, STONE | FLOOR_SILENT_FEET, 0x3f, 0x41, true, 190.0);
        assert_eq!(event(&avatar), Some((SIGNAL_JOG_STEP, 0)));
    }

    #[test]
    fn standing_still_puts_the_rhythm_back() {
        let mut avatar = player(0.5);
        avatar.sway_phase = 0x55;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0);
        // Between 1 and 2 units a second the rhythm holds where it is.
        let mut avatar = player(1.5);
        avatar.sway_phase = 0x55;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x55);
    }

    #[test]
    fn coasting_without_keys_holds_the_rhythm() {
        let mut avatar = player(150.0);
        avatar.sway_phase = 0x30;
        run(&mut avatar, &InputCommand::default());
        assert_eq!(avatar.sway_phase, 0x30);
    }

    #[test]
    fn a_ladder_sounds_only_off_the_ground() {
        // At the foot of the ladder the walk rhythm runs instead.
        let mut avatar = player(0.0);
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.velocity = [0.0_f32, 0.0, 100.0];
        avatar.sway_phase = 0x30;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0, "standing at the foot");
        // Off the ground the climb runs the rhythm and a rung sounds.
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.sway_phase = 0x30;
        run(&mut avatar, &forward());
        let expected = (50.0 * (100.0 * LADDER_CLIMB_PER_UNIT * RUNG_SWAY) + 48.0) as i32;
        assert_eq!(avatar.sway_phase, expected);
        assert_eq!(event(&avatar), Some((SIGNAL_JOG_STEP, RUNG_MATERIAL as i32)));
    }

    #[test]
    fn a_ladder_goes_foot_hand_foot() {
        let mut avatar = player(0.0);
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        let rung = |avatar: &mut AvatarState| {
            avatar.predictable_events[0] = 0;
            avatar.predictable_event_sequence = 0;
            footstep_event(avatar, 0, 0x3f, 0x41, true, 0.0);
            event(avatar)
        };
        let metal = RUNG_MATERIAL as i32;
        assert_eq!(rung(&mut avatar), Some((SIGNAL_JOG_STEP, metal)));
        assert_ne!(avatar.movement_state_bits & PMF_LADDER_HAND, 0);
        assert_eq!(rung(&mut avatar), Some((EV_LADDER_HAND_GRAB, metal)));
        assert_eq!(avatar.movement_state_bits & PMF_LADDER_HAND, 0);
        assert_eq!(rung(&mut avatar), Some((SIGNAL_JOG_STEP, metal)));
        // Not on a ladder in the air: no step, and the turn holds.
        avatar.movement_state_bits = PMF_LADDER_HAND;
        assert_eq!(rung(&mut avatar), None);
        assert_eq!(avatar.movement_state_bits, PMF_LADDER_HAND);
    }

    #[test]
    fn climbing_down_runs_the_ladder_rhythm_backwards() {
        let mut avatar = player(0.0);
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.velocity = [0.0_f32, 0.0, -100.0];
        avatar.sway_phase = 0x05;
        run(&mut avatar, &forward());
        // 5 - 23.6 truncates to -18, which wraps to 238: a rung sounds.
        assert_eq!(avatar.sway_phase, 238);
        assert_eq!(event(&avatar).map(|e| e.0), Some(SIGNAL_JOG_STEP));
    }

    #[test]
    fn a_jump_off_a_ladder_is_quiet_for_a_moment() {
        let mut avatar = player(0.0);
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.velocity = [0.0_f32, 0.0, 100.0];
        avatar.hop_start_ms = 9_800;
        avatar.sway_phase = 0x30;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x30);
    }

    #[test]
    fn in_the_air_off_a_ladder_the_rhythm_holds() {
        let mut avatar = player(250.0);
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.sway_phase = 0x30;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x30);
        assert_eq!(event(&avatar), None);
    }

    #[test]
    fn a_step_sounds_only_into_the_second_or_fourth_quarter() {
        let mut avatar = player(190.0);
        // Into the third quarter: the mark is crossed, but not a step's moment.
        avatar.sway_phase = 0x82;
        let make = should_make_footsteps(&avatar, 190.0);
        footstep_event(&mut avatar, STONE, 0x7e, 0x82, make, 190.0);
        assert_eq!(event(&avatar), None);
        avatar.sway_phase = 0xc1;
        let make = should_make_footsteps(&avatar, 190.0);
        footstep_event(&mut avatar, STONE, 0xbe, 0xc1, make, 190.0);
        assert_eq!(event(&avatar).map(|e| e.0), Some(SIGNAL_JOG_STEP));
    }

    /// 50 ms at the swimming pace, the gun's sprint scale 1.
    const SWIM_STEP: i32 = 5;

    #[test]
    fn swimming_keeps_its_own_rhythm() {
        // Keys, no keys, and standing still all run the swimming pace.
        let mut avatar = player(150.0);
        avatar.waterlevel = 4;
        avatar.sway_phase = 0x10;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x10 + SWIM_STEP);
        run(&mut avatar, &InputCommand::default());
        assert_eq!(avatar.sway_phase, 0x10 + 2 * SWIM_STEP);
        avatar.velocity = Default::default();
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x10 + 3 * SWIM_STEP, "still: no reset");
        // Waist deep the walk rules still hold: still puts it back.
        avatar.waterlevel = 3;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0);
    }

    #[test]
    fn the_gun_paces_the_swim() {
        let mut avatar = player(150.0);
        avatar.waterlevel = 5;
        avatar.sway_phase = 0x10;
        let feel = Bo2Feel {
            sprint_cycle_scale: 0.5,
            ..feel()
        };
        footsteps(&mut avatar, &tick_vars(100), &forward(), SCALES, feel, &mut PitchHistory::default());
        assert_eq!(avatar.sway_phase, 0x10 + 5);
    }

    #[test]
    fn swimming_in_the_air_runs_the_swim() {
        // Off the ground in water the ladder rules wait for dry feet.
        let mut avatar = player(0.0);
        avatar.standing_on = NO_ENTITY_SLOT;
        avatar.waterlevel = 6;
        avatar.sway_phase = 0x30;
        run(&mut avatar, &forward());
        assert_eq!(avatar.sway_phase, 0x30 + SWIM_STEP);
        assert_eq!(event(&avatar), None);
    }

    #[test]
    fn deep_water_makes_no_step() {
        let mut avatar = player(190.0);
        avatar.sway_phase = 0x41;
        avatar.waterlevel = 4;
        assert!(should_make_footsteps(&avatar, 190.0));
        avatar.waterlevel = 5;
        assert!(!should_make_footsteps(&avatar, 190.0));
    }

    #[test]
    fn steps_in_water_sound_as_water() {
        let mut avatar = player(190.0);
        avatar.waterlevel = 1;
        let step = first_step(&mut avatar).expect("a step");
        assert_eq!(step.1, WATER_SURFACE_TYPE);
        // A no-steps floor still sounds as surface 0, water or not.
        assert_eq!(ground_surface_type(&avatar, STONE | FLOOR_SILENT_FEET), 0);
        avatar.waterlevel = 0;
        assert_eq!(ground_surface_type(&avatar, STONE), 18);
    }
}
