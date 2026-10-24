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
    Bo2Feel, CmdScaleWalkContext, LADDER_SURFACE_TYPE, Pml, add_predictable_event,
    footsteps_bob_cycle, is_in_air,
};
use playerstate_t6::game::{ENTITYNUM_NONE, PM_TYPE_DEAD, PlayerState, UserCmd, pm_flags};

use crate::dive;
use crate::feel::EF_TURRET;
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
const LADDER_CLIMB_BOB: f32 = 0.45;
/// Walking (or leaning) on a ladder: 1 / 38.1 per unit, at its own pace.
const LADDER_WALK_PER_UNIT: f32 = 0.026_246_719;
const LADDER_WALK_BOB: f32 = 0.35;

/// The rhythm's quarter marks: a step can sound when it crosses one.
const QUARTER_MASK: i32 = !0x3f;
/// The rhythm is in its second or fourth quarter: a step's moment.
const STEP_HALF: i32 = 0x7f;
const STEP_QUARTER: i32 = 0x40;

/// A surface marked to make no footsteps (it still sounds, as surface 0).
const SURF_NOSTEPS: u32 = 0x2000;

const EV_FOOTSTEP_SPRINT: i32 = entity_t6::event::FOOTSTEP_SPRINT as i32;
const EV_FOOTSTEP_RUN: i32 = entity_t6::event::FOOTSTEP_RUN as i32;
const EV_FOOTSTEP_WALK: i32 = entity_t6::event::FOOTSTEP_WALK as i32;
const EV_FOOTSTEP_PRONE: i32 = entity_t6::event::FOOTSTEP_PRONE as i32;
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
    ps: &mut PlayerState,
    pml: &Pml,
    cmd: &UserCmd,
    scales: CmdScaleWalkContext,
    feel: Bo2Feel,
    pitch: &mut PitchHistory,
) {
    if ps.pm_type >= PM_TYPE_DEAD {
        return;
    }
    pitch.update(calc_player_pitch(ps, pml));
    let xyspeed = libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1]);
    let diving = dive::active(ps);
    let bob = |ps: &mut PlayerState| {
        footsteps_bob_cycle(
            ps,
            pml.msec,
            cmd.forwardmove,
            cmd.rightmove,
            pml.almost_ground_plane != 0,
            cmd.server_time,
            scales,
            feel,
        );
    };
    // A dive keeps its own rhythm, in the air too (and makes no steps).
    if diving {
        bob(ps);
    }
    if ps.e_flags & EF_TURRET != 0 {
        return;
    }
    let mantling = ps.pm_flags & pm_flags::MANTLE != 0;
    if is_in_air(ps, pml) && ps.waterlevel == 0 && !diving && !mantling {
        ladder_footsteps(ps, pml.msec, cmd.server_time);
        return;
    }
    let swimming = ps.waterlevel >= SWIM_LEVEL;
    let moving = (xyspeed >= PLAYER_MOVE_THRESHHOLD && ps.pm_type != 1) || mantling;
    if !moving {
        if xyspeed < STOP_SPEED && !swimming {
            ps.bob_cycle = 0;
        }
        if swimming {
            swim_bob(ps, pml.msec, feel);
        }
        return;
    }
    // A dive has its rhythm already.
    if diving {
        return;
    }
    // No keys (coasting) moves only the legs, unless swimming.
    if cmd.forwardmove == 0 && cmd.rightmove == 0 {
        if swimming {
            swim_bob(ps, pml.msec, feel);
        }
        return;
    }
    let old = ps.bob_cycle;
    if swimming {
        swim_bob(ps, pml.msec, feel);
    } else {
        bob(ps);
    }
    let make = should_make_footsteps(ps, xyspeed);
    footstep_event(ps, pml.ground_trace[4], old, ps.bob_cycle, make, xyspeed);
}

/// BG_CalcBob in water: the swimming pace, whatever the speed, times the
/// gun's sprint scale (its crouched one when sprinting crouched).
fn swim_bob(ps: &mut PlayerState, msec: i32, feel: Bo2Feel) {
    let scale = if ps.pm_flags & pm_flags::SPRINTING != 0 && ps.view_height_target == 40 {
        feel.ducked_sprint_cycle_scale
    } else {
        feel.sprint_cycle_scale
    };
    ps.bob_cycle = (msec as f32 * SWIM_BOB * scale + ps.bob_cycle as f32) as i32 & 0xff;
}

/// PM_ShouldMakeFootsteps: the rhythm is at a step's moment, the player
/// moves fast enough to be heard, and the water is under the shoulders.
pub(crate) fn should_make_footsteps(ps: &PlayerState, xyspeed: f32) -> bool {
    ps.bob_cycle & STEP_HALF >= STEP_QUARTER
        && ps.waterlevel < SILENT_LEVEL
        && xyspeed >= PLAYER_FOOTSTEPS_THRESHHOLD
}

/// PM_FootstepEvent: the rhythm crossed a quarter mark. Off the ground only
/// a ladder's rung sounds; on it the step's kind goes by stance and speed,
/// on the ground's surface.
pub(crate) fn footstep_event(
    ps: &mut PlayerState,
    surface_flags: u32,
    old: i32,
    new: i32,
    make: bool,
    xyspeed: f32,
) {
    if (old ^ new) & QUARTER_MASK == 0 || !make {
        return;
    }
    if ps.ground_entity_num == ENTITYNUM_NONE {
        // A ladder's rungs go foot (a run step), hand, foot, hand.
        if ps.pm_flags & pm_flags::LADDER != 0 {
            let kind = if ps.pm_flags & PMF_LADDER_HAND == 0 {
                EV_FOOTSTEP_RUN
            } else {
                EV_LADDER_HAND_GRAB
            };
            ps.pm_flags ^= PMF_LADDER_HAND;
            add_predictable_event(ps, kind, LADDER_SURFACE_TYPE as i32);
        }
        return;
    }
    let kind = footstep_type(ps, xyspeed);
    let surface = ground_surface_type(ps, surface_flags);
    add_predictable_event(ps, kind, surface);
}

/// PM_FootstepType: prone; crouched runs (110 or faster, not walking or
/// leaning) or walks; standing sprints, runs (110 or faster) or walks.
fn footstep_type(ps: &PlayerState, xyspeed: f32) -> i32 {
    if ps.pm_flags & pm_flags::PRONE != 0 {
        return EV_FOOTSTEP_PRONE;
    }
    let striding = ps.pm_flags & pm_flags::WALKING == 0 && ps.leanf == 0.0;
    if ps.pm_flags & pm_flags::CROUCH != 0 {
        return if striding && xyspeed >= PLAYER_RUN_THRESHHOLD {
            EV_FOOTSTEP_CROUCH_RUN
        } else {
            EV_FOOTSTEP_CROUCH_WALK
        };
    }
    if striding {
        if ps.pm_flags & pm_flags::SPRINTING != 0 {
            return EV_FOOTSTEP_SPRINT;
        }
        if xyspeed >= PLAYER_RUN_THRESHHOLD {
            return EV_FOOTSTEP_RUN;
        }
    }
    EV_FOOTSTEP_WALK
}

/// PM_GroundSurfaceType: the surface under the feet, 0 on a no-steps one,
/// water in any water.
pub(crate) fn ground_surface_type(ps: &PlayerState, surface_flags: u32) -> i32 {
    if surface_flags & SURF_NOSTEPS != 0 {
        return 0;
    }
    if ps.waterlevel != 0 {
        return WATER_SURFACE_TYPE;
    }
    ((surface_flags >> 20) & 0x3f) as i32
}

/// PM_Footstep_LadderMove: off the ground on a ladder, the rhythm follows the
/// climbing speed (down runs it backwards) and each rung can sound. Not on
/// a ladder: nothing (the legs only).
fn ladder_footsteps(ps: &mut PlayerState, msec: i32, server_time: i32) {
    if ps.pm_flags & pm_flags::LADDER == 0 {
        return;
    }
    if server_time.wrapping_sub(ps.jump_time) <= LADDER_JUMP_QUIET_MS {
        return;
    }
    let climb = ps.velocity[2];
    let bobmove = if ps.pm_flags & pm_flags::WALKING == 0 && ps.leanf == 0.0 {
        climb * LADDER_CLIMB_PER_UNIT * LADDER_CLIMB_BOB
    } else {
        climb * LADDER_WALK_PER_UNIT * LADDER_WALK_BOB
    };
    let old = ps.bob_cycle;
    ps.bob_cycle = (msec as f32 * bobmove + old as f32) as i32 & 0xff;
    let make = ps.bob_cycle & STEP_HALF >= STEP_QUARTER && ps.waterlevel < SILENT_LEVEL;
    footstep_event(ps, 0, old, ps.bob_cycle, make, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const STONE: u32 = 18 << 20;

    const SCALES: CmdScaleWalkContext = CmdScaleWalkContext {
        player_back_speed_scale: 0.7,
        player_strafe_speed_scale: 0.8,
        player_sprint_speed_scale: 1.5,
        player_last_stand_crawl_speed_scale: 0.15,
        weapon_move_speed_scale: 1.0,
        weapon_ads_move_speed_scale: 1.0,
        shellshock_movement_scale: 1.0,
        prone_lerp_ms: 0,
    };

    fn feel() -> Bo2Feel {
        Bo2Feel {
            ..Bo2Feel::BO2_MP
        }
    }

    fn pml(msec: i32) -> Pml {
        let mut ground_trace = [0; 11];
        ground_trace[4] = STONE;
        Pml {
            forward: [0.0; 3],
            right: [0.0; 3],
            up: [0.0; 3],
            frametime: msec as f32 / 1000.0,
            msec,
            walking: 1,
            ground_plane: 1,
            almost_ground_plane: 1,
            ground_trace,
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        }
    }

    fn run(ps: &mut PlayerState, cmd: &UserCmd) {
        let mut pml = pml(50);
        if ps.ground_entity_num == ENTITYNUM_NONE {
            pml.walking = 0;
            pml.ground_plane = 0;
            pml.almost_ground_plane = 0;
        }
        footsteps(ps, &pml, cmd, SCALES, feel(), &mut PitchHistory::default());
    }

    fn player(speed: f32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 1022;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.view_height_target = 60;
        ps.velocity = [speed, 0.0, 0.0];
        ps
    }

    fn forward() -> UserCmd {
        UserCmd {
            forwardmove: 127,
            server_time: 10_000,
            ..UserCmd::default()
        }
    }

    fn event(ps: &PlayerState) -> Option<(i32, i32)> {
        (ps.predictable_events[0] != 0).then_some((ps.predictable_events[0], ps.predictable_event_parms[0] as i32))
    }

    /// Runs frames until a step sounds, or gives up after a second.
    fn first_step(ps: &mut PlayerState) -> Option<(i32, i32)> {
        for _ in 0..20 {
            run(ps, &forward());
            if let Some(step) = event(ps) {
                return Some(step);
            }
        }
        None
    }

    #[test]
    fn the_step_kind_follows_the_speed() {
        assert_eq!(first_step(&mut player(190.0)).map(|e| e.0), Some(EV_FOOTSTEP_RUN));
        assert_eq!(first_step(&mut player(80.0)).map(|e| e.0), Some(EV_FOOTSTEP_WALK));
        let mut ps = player(280.0);
        ps.pm_flags = pm_flags::SPRINTING;
        assert_eq!(first_step(&mut ps).map(|e| e.0), Some(EV_FOOTSTEP_SPRINT));
    }

    #[test]
    fn crouched_and_lying_down_make_their_own_steps() {
        let mut ps = player(120.0);
        ps.pm_flags = pm_flags::CROUCH;
        ps.view_height_target = 40;
        assert_eq!(first_step(&mut ps).map(|e| e.0), Some(EV_FOOTSTEP_CROUCH_RUN));
        let mut ps = player(60.0);
        ps.pm_flags = pm_flags::CROUCH;
        ps.view_height_target = 40;
        assert_eq!(first_step(&mut ps).map(|e| e.0), Some(EV_FOOTSTEP_CROUCH_WALK));
        let mut ps = player(30.0);
        ps.pm_flags = pm_flags::PRONE;
        ps.view_height_target = 11;
        assert_eq!(first_step(&mut ps).map(|e| e.0), Some(EV_FOOTSTEP_PRONE));
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
        let mut ps = player(190.0);
        ps.bob_cycle = 0x3f;
        footstep_event(&mut ps, STONE | SURF_NOSTEPS, 0x3f, 0x41, true, 190.0);
        assert_eq!(event(&ps), Some((EV_FOOTSTEP_RUN, 0)));
    }

    #[test]
    fn standing_still_puts_the_rhythm_back() {
        let mut ps = player(0.5);
        ps.bob_cycle = 0x55;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0);
        // Between 1 and 2 units a second the rhythm holds where it is.
        let mut ps = player(1.5);
        ps.bob_cycle = 0x55;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x55);
    }

    #[test]
    fn coasting_without_keys_holds_the_rhythm() {
        let mut ps = player(150.0);
        ps.bob_cycle = 0x30;
        run(&mut ps, &UserCmd::default());
        assert_eq!(ps.bob_cycle, 0x30);
    }

    #[test]
    fn a_ladder_sounds_only_off_the_ground() {
        // At the foot of the ladder the walk rhythm runs instead.
        let mut ps = player(0.0);
        ps.pm_flags = pm_flags::LADDER;
        ps.velocity = [0.0, 0.0, 100.0];
        ps.bob_cycle = 0x30;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0, "standing at the foot");
        // Off the ground the climb runs the rhythm and a rung sounds.
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.bob_cycle = 0x30;
        run(&mut ps, &forward());
        let expected = (50.0 * (100.0 * LADDER_CLIMB_PER_UNIT * LADDER_CLIMB_BOB) + 48.0) as i32;
        assert_eq!(ps.bob_cycle, expected);
        assert_eq!(event(&ps), Some((EV_FOOTSTEP_RUN, LADDER_SURFACE_TYPE as i32)));
    }

    #[test]
    fn a_ladder_goes_foot_hand_foot() {
        let mut ps = player(0.0);
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        let rung = |ps: &mut PlayerState| {
            ps.predictable_events[0] = 0;
            ps.predictable_event_sequence = 0;
            footstep_event(ps, 0, 0x3f, 0x41, true, 0.0);
            event(ps)
        };
        let metal = LADDER_SURFACE_TYPE as i32;
        assert_eq!(rung(&mut ps), Some((EV_FOOTSTEP_RUN, metal)));
        assert_ne!(ps.pm_flags & PMF_LADDER_HAND, 0);
        assert_eq!(rung(&mut ps), Some((EV_LADDER_HAND_GRAB, metal)));
        assert_eq!(ps.pm_flags & PMF_LADDER_HAND, 0);
        assert_eq!(rung(&mut ps), Some((EV_FOOTSTEP_RUN, metal)));
        // Not on a ladder in the air: no step, and the turn holds.
        ps.pm_flags = PMF_LADDER_HAND;
        assert_eq!(rung(&mut ps), None);
        assert_eq!(ps.pm_flags, PMF_LADDER_HAND);
    }

    #[test]
    fn climbing_down_runs_the_ladder_rhythm_backwards() {
        let mut ps = player(0.0);
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.velocity = [0.0, 0.0, -100.0];
        ps.bob_cycle = 0x05;
        run(&mut ps, &forward());
        // 5 - 23.6 truncates to -18, which wraps to 238: a rung sounds.
        assert_eq!(ps.bob_cycle, 238);
        assert_eq!(event(&ps).map(|e| e.0), Some(EV_FOOTSTEP_RUN));
    }

    #[test]
    fn a_jump_off_a_ladder_is_quiet_for_a_moment() {
        let mut ps = player(0.0);
        ps.pm_flags = pm_flags::LADDER;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.velocity = [0.0, 0.0, 100.0];
        ps.jump_time = 9_800;
        ps.bob_cycle = 0x30;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x30);
    }

    #[test]
    fn in_the_air_off_a_ladder_the_rhythm_holds() {
        let mut ps = player(250.0);
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.bob_cycle = 0x30;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x30);
        assert_eq!(event(&ps), None);
    }

    #[test]
    fn a_step_sounds_only_into_the_second_or_fourth_quarter() {
        let mut ps = player(190.0);
        // Into the third quarter: the mark is crossed, but not a step's moment.
        ps.bob_cycle = 0x82;
        let make = should_make_footsteps(&ps, 190.0);
        footstep_event(&mut ps, STONE, 0x7e, 0x82, make, 190.0);
        assert_eq!(event(&ps), None);
        ps.bob_cycle = 0xc1;
        let make = should_make_footsteps(&ps, 190.0);
        footstep_event(&mut ps, STONE, 0xbe, 0xc1, make, 190.0);
        assert_eq!(event(&ps).map(|e| e.0), Some(EV_FOOTSTEP_RUN));
    }

    /// 50 ms at the swimming pace, the gun's sprint scale 1.
    const SWIM_STEP: i32 = 5;

    #[test]
    fn swimming_keeps_its_own_rhythm() {
        // Keys, no keys, and standing still all run the swimming pace.
        let mut ps = player(150.0);
        ps.waterlevel = 4;
        ps.bob_cycle = 0x10;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x10 + SWIM_STEP);
        run(&mut ps, &UserCmd::default());
        assert_eq!(ps.bob_cycle, 0x10 + 2 * SWIM_STEP);
        ps.velocity = [0.0; 3];
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x10 + 3 * SWIM_STEP, "still: no reset");
        // Waist deep the walk rules still hold: still puts it back.
        ps.waterlevel = 3;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0);
    }

    #[test]
    fn the_gun_paces_the_swim() {
        let mut ps = player(150.0);
        ps.waterlevel = 5;
        ps.bob_cycle = 0x10;
        let feel = Bo2Feel {
            sprint_cycle_scale: 0.5,
            ..feel()
        };
        footsteps(&mut ps, &pml(100), &forward(), SCALES, feel, &mut PitchHistory::default());
        assert_eq!(ps.bob_cycle, 0x10 + 5);
    }

    #[test]
    fn swimming_in_the_air_runs_the_swim() {
        // Off the ground in water the ladder rules wait for dry feet.
        let mut ps = player(0.0);
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.waterlevel = 6;
        ps.bob_cycle = 0x30;
        run(&mut ps, &forward());
        assert_eq!(ps.bob_cycle, 0x30 + SWIM_STEP);
        assert_eq!(event(&ps), None);
    }

    #[test]
    fn deep_water_makes_no_step() {
        let mut ps = player(190.0);
        ps.bob_cycle = 0x41;
        ps.waterlevel = 4;
        assert!(should_make_footsteps(&ps, 190.0));
        ps.waterlevel = 5;
        assert!(!should_make_footsteps(&ps, 190.0));
    }

    #[test]
    fn steps_in_water_sound_as_water() {
        let mut ps = player(190.0);
        ps.waterlevel = 1;
        let step = first_step(&mut ps).expect("a step");
        assert_eq!(step.1, WATER_SURFACE_TYPE);
        // A no-steps floor still sounds as surface 0, water or not.
        assert_eq!(ground_surface_type(&ps, STONE | SURF_NOSTEPS), 0);
        ps.waterlevel = 0;
        assert_eq!(ground_surface_type(&ps, STONE), 18);
    }
}
