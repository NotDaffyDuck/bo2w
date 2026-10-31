//! bo2zm: Black Ops II's limits while lying down (audit D4). bo2mp: copied
//! from the legacy movement's prone turn for the BO2 frame (movement_t6).
//!
//! The body has its own yaw (`prone_direction`). Looking more than 40 off it,
//! or moving while it is off at all, swings it toward the view at 55 degrees
//! a second; the view itself stays within 45 of it, so a fast turn lying down
//! is held back to the body's swing. Where the body cannot turn (a wall at
//! the feet), the view is pulled back toward it. The view's pitch stays
//! within 45 of the torso's slope pitch. Each tick the body's and torso's
//! slope pitches ease toward the ground under them at 70 degrees a second,
//! and a body that no longer fits is told to crouch.
//!
//! Any change the code makes to the view is made to `view_offset_angles` too, so
//! the next command's angles land on it.

use game_math::wrap_degrees;
use game_math::degree_gap;
use game_math::slope_pitch;
use playerstate_t6::game::{
    AvatarState, InputCommand, movement_state_bits,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::MOVE_MODE_INTERLUDE;
use playerstate_t6::game::MOVE_MODE_DOWNED;
use playerstate_t6::game::MOVE_MODE_WALK_ATTACHED;

use crate::bridge::eye_level;
use crate::bridge::{
    SolidProbe, BELLY_CLEARANCE_PROBE, BELLY_FOOT_REACH, TickLocals, ProneRules, queue_forecast_happening,
};

use crate::prone_room::check_prone;

/// BO2's `bg_prone_yawcap`: the view stays this close to the body's yaw.
const PRONE_YAW_CAP: f32 = 45.0;

/// The body swings toward the view once it is this far off (the cap less 5).
const PRONE_SWING_TRIGGER: f32 = PRONE_YAW_CAP - 5.0;

/// How fast the body swings toward the view, degrees a second.
const PRONE_SWING_SPEED: f32 = 55.0;

/// The view's pitch stays this close to the torso's slope pitch.
const PRONE_PITCH_CAP: f32 = 45.0;

/// How fast the slope pitches ease toward the ground, degrees a second.
const PRONE_PITCH_SPEED: f32 = 70.0;

/// The body's half width for the turn checks.
const PRONE_SIZE: f32 = 15.0;

/// BO2's `PRONE_FEET_DIST_TURNED`.
const PRONE_FEET_DIST_TURNED: f32 = 55.0;

/// A turned body is checked part way between the view and the new yaw, by
/// this much per degree of turn.
const PRONE_TURN_BLEND: f32 = 0.004_166_666_7;

/// A blocked turn nudges the view back this share of the last tick's turn.
const PRONE_NUDGE: f32 = 0.98;

/// A blocked turn is walked back, and the view dragged, a degree at a time.
const PRONE_STEP: f32 = 1.0;

/// Black Ops II's force-crouch event.
const CROUCH_FORCED: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;

/// The last-stand body is told to crouch when bent more than this.
const LAST_STAND_BEND_LIMIT: f32 = 50.0;

fn wrap(deg: f32) -> f32 {
    degree_gap(deg, 0.0_f32)
}

fn rules(avatar: &AvatarState, zombies: bool) -> ProneRules {
    ProneRules::for_player(avatar, zombies)
}

/// The body fits lying along `yaw`, the turn's longer feet.
fn fits<C: SolidProbe>(avatar: &AvatarState, solids: &C, zombies: bool, yaw: f32) -> bool {
    check_prone(
        solids,
        avatar.origin,
        PRONE_SIZE,
        BELLY_CLEARANCE_PROBE,
        yaw,
        true,
        avatar.standing_on != NO_ENTITY_SLOT,
        true,
        rules(avatar, zombies),
        PRONE_FEET_DIST_TURNED,
        None,
        None,
    )
}

/// BO2's turned check: the body turned to `yaw` is checked part way back
/// toward the view, with feet a little longer the more it turns; it keeps
/// the body's torso and waist pitches.
fn turned_fits<C: SolidProbe>(
    avatar: &mut AvatarState,
    solids: &C,
    zombies: bool,
    yaw: f32,
) -> bool {
    let turn = degree_gap(yaw, avatar.look_angles[1]);
    let share = 1.0 - turn.abs() * PRONE_TURN_BLEND;
    let check_yaw = wrap_degrees(yaw - share * turn);
    let feet = BELLY_FOOT_REACH * share + PRONE_FEET_DIST_TURNED * turn.abs() * PRONE_TURN_BLEND;
    let (mut torso, mut waist) = (avatar.f_torso_pitch, avatar.f_waist_pitch);
    let ok = check_prone(
        solids,
        avatar.origin,
        PRONE_SIZE,
        BELLY_CLEARANCE_PROBE,
        check_yaw,
        true,
        avatar.standing_on != NO_ENTITY_SLOT,
        true,
        rules(avatar, zombies),
        feet,
        Some(&mut torso),
        Some(&mut waist),
    );
    avatar.f_torso_pitch = torso;
    avatar.f_waist_pitch = waist;
    ok
}

/// BO2's prone view limits, run right after the command's angles are put on
/// the view; `old_yaw` is the view's yaw before them.
pub fn bo2_prone_view<C: SolidProbe>(
    avatar: &mut AvatarState,
    ucmd: &InputCommand,
    msec: i32,
    solids: &C,
    zombies: bool,
    old_yaw: f32,
) {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0
        || avatar.movement_state_bits & movement_state_bits::MANTLE != 0
        || matches!(avatar.movement_mode, MOVE_MODE_WALK_ATTACHED | 2 | 3 | MOVE_MODE_INTERLUDE)
        || avatar.movement_mode >= MOVE_KIND_DEAD
    {
        return;
    }
    let view_yaw0 = avatar.look_angles[1];
    let mut turn_stuck = false;

    // Swing the body toward the view.
    let off = degree_gap(avatar.prone_direction, avatar.look_angles[1]);
    let moving = ucmd.thrust_ahead != 0 || ucmd.thrust_lateral != 0;
    if off.abs() > PRONE_SWING_TRIGGER || (moving && off != 0.0) {
        let swing = msec as f32 * PRONE_SWING_SPEED * 0.001;
        let mut yaw = if swing <= off.abs() {
            if off > 0.0 {
                avatar.prone_direction - swing
            } else {
                avatar.prone_direction + swing
            }
        } else {
            avatar.look_angles[1]
        };
        let mut gave_up = false;
        let mut turned = true;
        while !turned_fits(avatar, solids, zombies, yaw) {
            if gave_up {
                turned = false;
                break;
            }
            let back = degree_gap(avatar.prone_direction, yaw);
            let step = if back.abs() <= PRONE_STEP {
                turn_stuck = true;
                gave_up = true;
                back
            } else if back > 0.0 {
                PRONE_STEP
            } else {
                -PRONE_STEP
            };
            yaw = wrap_degrees(yaw + step);
        }
        if turned {
            if fits(avatar, solids, zombies, avatar.look_angles[1]) && fits(avatar, solids, zombies, yaw) {
                avatar.prone_direction = yaw;
            } else {
                turn_stuck = true;
            }
        }
    }

    // Where the body cannot follow, drag the view back toward it.
    let mut off = degree_gap(avatar.prone_direction, avatar.look_angles[1]);
    if off != 0.0 {
        let mut yaw = avatar.prone_direction;
        let mut gave_up = false;
        loop {
            let ok = fits(avatar, solids, zombies, yaw);
            if ok && turned_fits(avatar, solids, zombies, yaw) {
                avatar.prone_direction = yaw;
                break;
            }
            if gave_up {
                break;
            }
            let step = if off.abs() <= PRONE_STEP {
                gave_up = true;
                off
            } else if off > 0.0 {
                PRONE_STEP
            } else {
                -PRONE_STEP
            };
            avatar.view_offset_angles[1] += step;
            turn_stuck = true;
            avatar.look_angles[1] = wrap(avatar.look_angles[1] + step);
            off = degree_gap(avatar.prone_direction, avatar.look_angles[1]);
            if !ok {
                yaw = wrap_degrees(yaw + off);
            }
        }
    }

    yaw_clamp(avatar, off, turn_stuck, old_yaw, view_yaw0);
    pitch_clamp(avatar);
}

/// The view stays within 45 of the body's yaw; a blocked turn nudges it
/// back by most of a last small turn.
fn yaw_clamp(avatar: &mut AvatarState, off: f32, blocked: bool, old_yaw: f32, view_yaw0: f32) {
    let excess = if off > PRONE_YAW_CAP {
        off - PRONE_YAW_CAP
    } else if off < -PRONE_YAW_CAP {
        off + PRONE_YAW_CAP
    } else {
        0.0
    };
    if excess != 0.0 {
        avatar.view_offset_angles[1] += excess;
        avatar.look_angles[1] = wrap(if excess > 0.0 {
            avatar.prone_direction - PRONE_YAW_CAP
        } else {
            avatar.prone_direction + PRONE_YAW_CAP
        });
    }
    // BO2 also flags "prone blocked" here for the HUD (deferred).
    if blocked {
        let back = degree_gap(old_yaw, avatar.look_angles[1]);
        if back.abs() <= PRONE_STEP && degree_gap(view_yaw0, avatar.look_angles[1]) * back > 0.0 {
            avatar.look_angles[1] = wrap(avatar.look_angles[1] + back * PRONE_NUDGE);
            avatar.view_offset_angles[1] += back * PRONE_NUDGE;
        }
    }
}

/// The view's pitch stays within 45 of the torso's slope pitch.
fn pitch_clamp(avatar: &mut AvatarState) {
    let off = degree_gap(avatar.prone_torso_pitch, avatar.look_angles[0]);
    let excess = if off > PRONE_PITCH_CAP {
        off - PRONE_PITCH_CAP
    } else if off < -PRONE_PITCH_CAP {
        off + PRONE_PITCH_CAP
    } else {
        return;
    };
    avatar.view_offset_angles[0] += excess;
    avatar.look_angles[0] = wrap(if excess > 0.0 {
        avatar.prone_torso_pitch - PRONE_PITCH_CAP
    } else {
        avatar.prone_torso_pitch + PRONE_PITCH_CAP
    });
}

fn ease(now_deg: f32, goal: f32, step_millis_elapsed: f32) -> f32 {
    let off = degree_gap(goal, now_deg);
    if off == 0.0 {
        return now_deg;
    }
    let most = step_millis_elapsed * PRONE_PITCH_SPEED;
    let next = if off.abs() <= most {
        now_deg + off
    } else if off < 0.0 {
        now_deg - most
    } else {
        now_deg + most
    };
    wrap(next)
}

/// BO2's `PM_UpdatePronePitch`, after this tick's ground trace: a lying body
/// that no longer fits in the air, lies on ground too steep, or (last stand)
/// is bent too far is told to crouch; its slope pitches ease toward the
/// ground under it.
pub fn bo2_prone_pitch<C: SolidProbe>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    solids: &C,
    zombies: bool,
) {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0 {
        return;
    }
    let walkable = (tick_vars.floor_probe[10] >> 16) & 0xff != 0;
    let last_stand_view = avatar.eye_height_goal == eye_level::DOWNED_GRACE;
    let crouch = if avatar.standing_on == NO_ENTITY_SLOT {
        if last_stand_view {
            true
        } else {
            let (mut torso, mut waist) = (avatar.f_torso_pitch, avatar.f_waist_pitch);
            let ok = check_prone(
                solids,
                avatar.origin,
                PRONE_SIZE,
                BELLY_CLEARANCE_PROBE,
                avatar.prone_direction,
                true,
                false,
                tick_vars.standing_on_floor == 0 || walkable,
                rules(avatar, zombies),
                BELLY_FOOT_REACH,
                Some(&mut torso),
                Some(&mut waist),
            );
            avatar.f_torso_pitch = torso;
            avatar.f_waist_pitch = waist;
            // BO2 also flags "prone blocked" here for the HUD (deferred).
            // Knee-deep water crouches too.
            !ok || avatar.waterlevel > 1
        }
    } else {
        (tick_vars.standing_on_floor != 0 && !walkable)
            || (last_stand_view
                && (avatar.f_torso_pitch.abs() > LAST_STAND_BEND_LIMIT
                    || avatar.f_waist_pitch.abs() > LAST_STAND_BEND_LIMIT))
    };
    if crouch {
        queue_forecast_happening(avatar, CROUCH_FORCED, 0);
    }

    let floor_n = [
        f32::from_bits(tick_vars.floor_probe[1]),
        f32::from_bits(tick_vars.floor_probe[2]),
        f32::from_bits(tick_vars.floor_probe[3]),
    ];
    let body_yaw = if avatar.movement_mode == MOVE_MODE_DOWNED {
        avatar.look_angles[1]
    } else {
        avatar.prone_direction
    };
    let (body, torso) = if tick_vars.standing_on_floor == 0 {
        (0.0, 0.0)
    } else {
        (
            slope_pitch(body_yaw, floor_n),
            slope_pitch(avatar.look_angles[1], floor_n),
        )
    };
    avatar.prone_direction_pitch = ease(avatar.prone_direction_pitch, body, tick_vars.step_millis_elapsed);
    avatar.prone_torso_pitch = ease(avatar.prone_torso_pitch, torso, tick_vars.step_millis_elapsed);
}

#[cfg(test)]
mod tests {
    use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};
    use trace_t6::Trace;

    use super::*;
    use crate::bridge::SweepAsk;

    /// Nothing anywhere.
    struct Open;

    impl SolidProbe for Open {
        fn trace(&self, _input: SweepAsk) -> Trace {
            Trace::new()
        }
    }

    /// Solid everywhere: every trace stops where it starts.
    struct Stuck;

    impl SolidProbe for Stuck {
        fn trace(&self, _input: SweepAsk) -> Trace {
            let mut met = Trace::new();
            (met.fraction, met.starts_inside) = (0.0, true);
            met
        }
    }

    /// Lying in the air (so the open world fits), body along `dir`.
    fn lying(dir: f32, view: [f32; 3]) -> AvatarState {
        AvatarState {
            movement_state_bits: movement_state_bits::LYING_FLAT,
            standing_on: NO_ENTITY_SLOT,
            look_angles: view,
            prone_direction: dir,
            ..AvatarState::ZERO
        }
    }

    fn turn<C: SolidProbe>(avatar: &mut AvatarState, solids: &C, msec: i32, forward: i8) {
        let old_yaw = avatar.look_angles[1];
        let ucmd = InputCommand {
            thrust_ahead: forward,
            ..InputCommand::default()
        };
        bo2_prone_view(avatar, &ucmd, msec, solids, false, old_yaw);
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Looking within 40 of the body, standing still: nothing moves.
    #[test]
    fn bo2_prone_small_look_leaves_the_body() {
        let mut avatar = lying(0.0, [0.0, 30.0, 0.0]);
        turn(&mut avatar, &Open, 50, 0);
        assert_eq!((avatar.prone_direction, avatar.look_angles[1]), (0.0, 30.0));
        assert_eq!(avatar.view_offset_angles, [0.0; 3]);
    }

    /// Looking 60 off: the body swings 55 a second toward the view and the
    /// view is held 45 from it, the difference kept in `view_offset_angles`.
    #[test]
    fn bo2_prone_turn_is_held_to_the_body_swing() {
        let mut avatar = lying(0.0, [0.0, 60.0, 0.0]);
        turn(&mut avatar, &Open, 50, 0);
        assert!(close(avatar.prone_direction, 2.75), "{}", avatar.prone_direction);
        assert!(close(avatar.look_angles[1], 47.75), "{}", avatar.look_angles[1]);
        assert!(close(avatar.view_offset_angles[1], -12.25), "{}", avatar.view_offset_angles[1]);
    }

    /// Moving: the body swings toward the view even a little off, and lands
    /// on it when the swing would pass it.
    #[test]
    fn bo2_prone_moving_swings_the_body_to_the_view() {
        let mut avatar = lying(0.0, [0.0, 10.0, 0.0]);
        turn(&mut avatar, &Open, 100, 127);
        assert!(close(avatar.prone_direction, 5.5), "{}", avatar.prone_direction);
        let mut avatar = lying(0.0, [0.0, 3.0, 0.0]);
        turn(&mut avatar, &Open, 100, 127);
        assert_eq!(avatar.prone_direction, 3.0);
    }

    /// Where the body cannot turn, the view is dragged back onto it.
    #[test]
    fn bo2_prone_blocked_body_pulls_the_view_back() {
        let mut avatar = lying(90.0, [0.0, 150.0, 0.0]);
        turn(&mut avatar, &Stuck, 50, 0);
        assert_eq!(avatar.prone_direction, 90.0);
        assert!(close(avatar.look_angles[1], 90.0), "{}", avatar.look_angles[1]);
        // Sixty 1-degree steps add up a little float error.
        assert!((avatar.view_offset_angles[1] + 60.0).abs() < 1e-3, "{}", avatar.view_offset_angles[1]);
    }

    /// The view's pitch stays within 45 of the torso's.
    #[test]
    fn bo2_prone_pitch_is_held_to_the_torso() {
        let mut avatar = lying(0.0, [60.0, 0.0, 0.0]);
        turn(&mut avatar, &Open, 50, 0);
        assert_eq!(avatar.look_angles[0], 45.0);
        assert_eq!(avatar.view_offset_angles[0], -15.0);
    }

    /// Not lying, or mantling: the view is left alone.
    #[test]
    fn bo2_prone_view_only_lying() {
        let mut avatar = lying(0.0, [0.0, 60.0, 0.0]);
        avatar.movement_state_bits = 0;
        turn(&mut avatar, &Open, 50, 0);
        assert_eq!(avatar.look_angles[1], 60.0);
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT | movement_state_bits::MANTLE;
        turn(&mut avatar, &Open, 50, 0);
        assert_eq!(avatar.look_angles[1], 60.0);
    }

    fn tick_vars(ground: Option<[f32; 3]>, walkable: bool, step_millis_elapsed: f32) -> TickLocals {
        let mut floor_probe = [0; 11];
        if let Some(normal) = ground {
            floor_probe[1] = normal[0].to_bits();
            floor_probe[2] = normal[1].to_bits();
            floor_probe[3] = normal[2].to_bits();
            floor_probe[10] = u32::from(walkable) << 16;
        }
        TickLocals {
            fwd_axis: Default::default(),
            right_axis: Default::default(),
            up_axis: [0.0; 3],
            step_millis_elapsed,
            frame_ms: (step_millis_elapsed * 1000.0) as i32,
            walking: 0,
            standing_on_floor: u32::from(ground.is_some()),
            near_floor_contact: 0,
            floor_probe,
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        }
    }

    const RAMP_NORMAL: [f32; 3] = [-0.196_116_14, 0.0, 0.980_580_7];

    /// On a ramp the slope pitches ease toward it at 70 a second.
    #[test]
    fn bo2_prone_slope_pitch_eases_at_70_a_second() {
        let mut avatar = lying(0.0, [0.0, 0.0, 0.0]);
        avatar.standing_on = 0;
        let target = slope_pitch(0.0, RAMP_NORMAL);
        assert!(target < -10.0, "{target}");
        bo2_prone_pitch(&mut avatar, &tick_vars(Some(RAMP_NORMAL), true, 0.05), &Open, false);
        assert!(close(avatar.prone_direction_pitch, -3.5), "{}", avatar.prone_direction_pitch);
        assert!(close(avatar.prone_torso_pitch, -3.5), "{}", avatar.prone_torso_pitch);
        assert_eq!(avatar.predictable_event_sequence, 0);
        for _ in 0..10 {
            bo2_prone_pitch(&mut avatar, &tick_vars(Some(RAMP_NORMAL), true, 0.05), &Open, false);
        }
        assert!(close(avatar.prone_direction_pitch, target), "{}", avatar.prone_direction_pitch);
    }

    /// Lying on ground too steep, or in the air where the body no longer
    /// fits: told to crouch.
    #[test]
    fn bo2_prone_no_room_is_told_to_crouch() {
        let mut avatar = lying(0.0, [0.0, 0.0, 0.0]);
        avatar.standing_on = 0;
        bo2_prone_pitch(&mut avatar, &tick_vars(Some([0.0, 0.0, 1.0]), false, 0.05), &Open, false);
        assert_eq!((avatar.predictable_event_sequence, avatar.predictable_events[0]), (1, CROUCH_FORCED));
        let mut avatar = lying(0.0, [0.0, 0.0, 0.0]);
        bo2_prone_pitch(&mut avatar, &tick_vars(None, false, 0.05), &Open, false);
        assert_eq!(avatar.predictable_event_sequence, 0);
        bo2_prone_pitch(&mut avatar, &tick_vars(None, false, 0.05), &Stuck, false);
        assert_eq!((avatar.predictable_event_sequence, avatar.predictable_events[0]), (1, CROUCH_FORCED));
    }
}
