//! Black Ops II's stance: PM_CheckDuck (the stance keys, the box, the
//! stance the view heads for) and PM_ViewHeightAdjust (the view's rise and
//! fall between them), the frame a player lies down, and its sniper scope
//! check. Written from the facts and numbers in Black Ops II's server;
//! docs/MOVE-T6.md says what is left out and why.

use crate::bridge::{
    ScopeWishArgs, SQUAT_HULL_TOP as CROUCH_TOP, SolidProbe, SweepAsk, HullExtents,
    BELLY_HULL_TOP as PRONE_TOP, ProneRules, UPRIGHT_HULL_TOP as STAND_TOP, PostureShift, add_event,
    queue_forecast_happening, eye_level,
};
use playerstate_t6::game::{
    buttons, entity_flag_bits, movement_state_bits,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::MOVE_MODE_DOWNED;
use playerstate_t6::game::MOVE_MODE_OBSERVER;
use playerstate_t6::game::AvatarState;
use playerstate_t6::game::InputCommand;

use super::prone_room::prone_space_ok;
use playerstate_t6::game::entity_flag_bits::TURRET_MASK;
use crate::gjk::{Bo2Collision, PenetrationChecks};

/// Half the side of the player's square footprint. BO2 sets the box from it
/// every frame; the height comes from the stance.
pub(crate) const BOX_HALF_WIDTH: f32 = 15.0;
const BOX_MINS: [f32; 3] = [-BOX_HALF_WIDTH, -BOX_HALF_WIDTH, 0.0];
const BOX_MAXS: [f32; 3] = [BOX_HALF_WIDTH, BOX_HALF_WIDTH, STAND_TOP];

/// Box of a spectator (such a player has no view height at all).
const SPECTATOR_MINS: [f32; 3] = [-8.0, -8.0, -8.0];
const SPECTATOR_MAXS: [f32; 3] = [8.0, 8.0, 16.0];

/// Eye height given to a dead player.
const DEAD_EYE_HEIGHT: i32 = 8;

/// Both stance bits of `movement_state_bits` together.
const LYING_OR_CROUCHED: u32 = movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT;

/// Flag bits that freeze the stance where it is (BO2's
/// PM_ShouldLeaveStanceAlone).
const STANCE_FROZEN_BITS: u32 = 0xc00;

/// Events that make the client snap the player back into a stance after a
/// stance key could not be honoured.
const SNAP_TO_STAND: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;
const SNAP_TO_CROUCH: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;
const SNAP_TO_PRONE: i32 = entity_t6::event::STANCE_FORCE_PRONE as i32;

/// Animation ids for getting up from prone (PRONE_TO_CROUCH and
/// PRONE_TO_STAND in the anim script). The stand/crouch pair only plays when
/// bg_playStandToCrouchAnims is on, and it ships off: there the view simply
/// blends.
const ANIM_PRONE_TO_CROUCH: u8 = 12;
const ANIM_PRONE_TO_STAND: u8 = 16;

/// For heights outside the three stances (last stand) the eye travels at this
/// many units per second.
const EYE_SPEED: f32 = 180.0;

/// An eye that is sinking marks the aim as fully spread (bg_ads, default on).
const FULL_SPREAD: f32 = 255.0;

const CROUCH_TO_PRONE: &'static [(i32, f32)] = &[
    (0, 1.0),
    (11, 0.931),
    (22, 0.759),
    (34, 0.483),
    (45, 0.172),
    (50, 0.138),
    (55, 0.172),
    (70, 0.241),
    (90, 0.207),
    (100, 0.0),
];
const PRONE_TO_CROUCH: &'static [(i32, f32)] = &[
    (0, 0.0),
    (5, -0.034),
    (30, 0.345),
    (50, 0.483),
    (67, 0.69),
    (83, 0.79),
    (100, 1.0),
];
const STAND_TO_CROUCH: &'static [(i32, f32)] = &[
    (0, 1.0),
    (1, 0.975),
    (4, 0.925),
    (30, 0.8),
    (80, 0.2),
    (90, 0.075),
    (95, 0.025),
    (100, 0.0),
];
const CROUCH_TO_STAND: &'static [(i32, f32)] = &[
    (0, 0.0),
    (5, 0.025),
    (10, 0.075),
    (20, 0.2),
    (70, 0.8),
    (96, 0.925),
    (99, 0.975),
    (100, 1.0),
];

/// Per-frame facts the stance update needs from its caller.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DuckContext {
    pub body: HullExtents,
    pub prone_barred: bool,
    pub zombies: bool,
    /// While a dive runs it decides the stance: stance keys are ignored and
    /// the dive's sprint does not make the player stand.
    pub dive_owns: bool,
}

/// What is left for [`finish_duck`] to do once PM_CheckDuck has run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DuckTail {
    /// A spectator: small box, no eye height, nothing to update.
    Spectator,
    /// A dead player or a sprinter: the box has this height, the eye is
    /// updated, and the flags stay as they are.
    Fixed(f32),
    /// Update the eye, then derive box and flags from the stance it heads for.
    Stance,
}

/// Result of this frame's PM_CheckDuck.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Duck {
    pub tail: DuckTail,
    /// Whether the stance the eye heads for changed (the torso resets, and
    /// lying down also slows the player).
    pub change: PostureShift,
    /// A stance animation to start, if any.
    pub anim: Option<u8>,
}

/// Tell the client about a stance it has to snap to.
fn snap_client(avatar: &mut AvatarState, event: i32, parm: i32) {
    queue_forecast_happening(avatar, event, parm);
}

/// Aim the eye at a stance height.
fn aim_eye_at(avatar: &mut AvatarState, height: i32) {
    avatar.eye_height_goal = height;
}

/// Neither lying down nor crouched in the player flags.
fn clear_stance_flags(avatar: &mut AvatarState) {
    avatar.movement_state_bits &= !LYING_OR_CROUCHED;
}

/// Neither eflag for a ducked or lying body.
fn clear_stance_eflags(avatar: &mut AvatarState) {
    avatar.e_flags &= !(entity_flag_bits::CROUCH_BIT | entity_flag_bits::LYING_FLAT);
}

fn flag_set(avatar: &AvatarState, bit: u32) -> bool {
    avatar.movement_state_bits & bit != 0
}

/// BO2's PM_CheckDuck up to the eye update: the stance keys, the turret's
/// stance, and the stance the eye heads for. Getting up is checked with BO2's
/// own collision; lying down keeps the box traces.
pub(crate) fn check_duck<C: Bo2Collision>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    solids: &C,
    ctx: DuckContext,
) -> Duck {
    let mut duck = Duck {
        tail: DuckTail::Stance,
        change: PostureShift::Unchanged,
        anim: None,
    };
    if avatar.movement_mode == MOVE_MODE_OBSERVER {
        settle_spectator(avatar, ucmd);
        duck.tail = DuckTail::Spectator;
        return duck;
    }
    if avatar.movement_mode == MOVE_KIND_DEAD {
        aim_eye_at(avatar, DEAD_EYE_HEIGHT);
        duck.tail = DuckTail::Fixed(dead_box_top(avatar));
        return duck;
    }
    if force_upright(avatar, &mut duck, ctx) {
        return duck;
    }

    if avatar.e_flags & TURRET_MASK != 0 {
        avatar.movement_state_bits = turret_stance_flags(avatar);
    } else if !flag_set(avatar, STANCE_FROZEN_BITS) {
        if avatar.movement_mode == MOVE_MODE_DOWNED {
            avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::LYING_FLAT) | movement_state_bits::DUCK_BUTTON_BIT;
        } else if !ctx.dive_owns {
            duck.anim = stance_keys(avatar, ucmd, solids, ctx);
        }
    }

    duck.change = stance_target(avatar);
    duck
}

/// A spectator never lies down and has no eye height of its own.
fn settle_spectator(avatar: &mut AvatarState, ucmd: &mut InputCommand) {
    clear_stance_flags(avatar);
    if ucmd.buttons & buttons::LYING_FLAT != 0 {
        ucmd.buttons &= !buttons::LYING_FLAT;
        snap_client(avatar, SNAP_TO_STAND, 0);
    }
    aim_eye_at(avatar, 0);
    avatar.eye_height_now = 0.0;
}

/// Box height of a dead player: whatever stance the body died in.
fn dead_box_top(avatar: &AvatarState) -> f32 {
    if flag_set(avatar, movement_state_bits::DUCK_BUTTON_BIT) {
        CROUCH_TOP
    } else if flag_set(avatar, movement_state_bits::LYING_FLAT) {
        PRONE_TOP
    } else {
        STAND_TOP
    }
}

/// Water and sprinting override the stance keys. Returns true when one of
/// them took over, with the tail (and animation) filled in.
///
/// Lying in any water puts the player on a crouch eye height (the flags just
/// let go of prone); from chest depth up, the player stands completely. A
/// crouched or lying player past the waist stands too, and a sprint stands the
/// player up every frame unless a dive is running. The box stays standing in
/// all of these.
fn force_upright(avatar: &mut AvatarState, duck: &mut Duck, ctx: DuckContext) -> bool {
    if flag_set(avatar, movement_state_bits::LYING_FLAT) && avatar.waterlevel > 0 {
        aim_eye_at(avatar, eye_level::DUCK_BUTTON_BIT);
        snap_client(avatar, SNAP_TO_CROUCH, 0);
        if avatar.movement_mode != MOVE_MODE_DOWNED {
            duck.anim = Some(ANIM_PRONE_TO_CROUCH);
        }
        avatar.movement_state_bits &= !movement_state_bits::LYING_FLAT;
        if avatar.waterlevel > 3 {
            clear_stance_flags(avatar);
            aim_eye_at(avatar, eye_level::UPRIGHT);
            snap_client(avatar, SNAP_TO_STAND, 0);
        }
        duck.tail = DuckTail::Fixed(STAND_TOP);
        return true;
    }
    let wading = flag_set(avatar, LYING_OR_CROUCHED) && avatar.waterlevel > 2;
    let sprinting = flag_set(avatar, movement_state_bits::SPRINTING) && !ctx.dive_owns;
    if !wading && !sprinting {
        return false;
    }
    if !wading {
        clear_stance_eflags(avatar);
    }
    clear_stance_flags(avatar);
    aim_eye_at(avatar, eye_level::UPRIGHT);
    snap_client(avatar, SNAP_TO_STAND, 0);
    duck.tail = DuckTail::Fixed(STAND_TOP);
    true
}

/// A turret keeps the stance it is fired from: crouched or standing (BO2 has
/// no prone turret).
fn turret_stance_flags(avatar: &AvatarState) -> u32 {
    match avatar.e_flags & TURRET_MASK {
        entity_flag_bits::TURRET_CROUCH => (avatar.movement_state_bits & !movement_state_bits::LYING_FLAT) | movement_state_bits::DUCK_BUTTON_BIT,
        _ => avatar.movement_state_bits & !LYING_OR_CROUCHED,
    }
}

/// Does a body of this height fit where the player is? Bodies and the other
/// get-up exceptions are left out of the trace.
fn fits_here<C: Bo2Collision>(avatar: &AvatarState, solids: &C, ctx: DuckContext, top: f32) -> bool {
    let probe = SweepAsk {
        start: avatar.origin,
        end: avatar.origin,
        lo_corner: BOX_MINS,
        hi_corner: [BOX_MAXS[0], BOX_MAXS[1], top],
        collide_filter: ctx.body.collide_filter & !BO2_GETUP_IGNORES,
    };
    !PenetrationChecks(solids).trace(probe).fully_embedded
}

/// The stance keys: on a ladder they are dropped; prone lies down where
/// allowed, crouch crouches, and with neither held the player rises as far as
/// the room permits. A key that cannot be honoured tells the client, unless
/// the stance key is still held.
fn stance_keys<C: Bo2Collision>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    solids: &C,
    ctx: DuckContext,
) -> Option<u8> {
    if flag_set(avatar, movement_state_bits::LADDER) && ucmd.buttons & (buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT) != 0 {
        ucmd.buttons &= !(buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT);
        snap_client(avatar, SNAP_TO_STAND, 0);
    }
    let held = ucmd.buttons & buttons::STANCE_LOCK_BIT != 0;
    if ucmd.buttons & buttons::LYING_FLAT != 0 {
        press_prone(avatar, solids, ctx, held);
        None
    } else if ucmd.buttons & buttons::DUCK_BUTTON_BIT != 0 {
        press_crouch(avatar, solids, ctx, held)
    } else {
        rise_as_far_as_possible(avatar, solids, ctx, held)
    }
}

fn press_prone<C: Bo2Collision>(avatar: &mut AvatarState, solids: &C, ctx: DuckContext, held: bool) {
    let rules = ProneRules::for_player(avatar, ctx.zombies);
    if prone_space_ok(avatar, solids, BOX_MAXS[0], ctx.prone_barred, rules) {
        avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::DUCK_BUTTON_BIT) | movement_state_bits::LYING_FLAT;
    } else if avatar.standing_on != NO_ENTITY_SLOT && !held {
        let was_low = flag_set(avatar, LYING_OR_CROUCHED);
        let snap = if was_low {
            SNAP_TO_CROUCH
        } else {
            SNAP_TO_STAND
        };
        snap_client(avatar, snap, 0);
    }
}

fn press_crouch<C: Bo2Collision>(
    avatar: &mut AvatarState,
    solids: &C,
    ctx: DuckContext,
    held: bool,
) -> Option<u8> {
    if avatar.waterlevel >= 3 {
        // Too deep to crouch: the client is told to stand.
        snap_client(avatar, SNAP_TO_STAND, 0);
        return None;
    }
    if !flag_set(avatar, movement_state_bits::LYING_FLAT) {
        avatar.movement_state_bits |= movement_state_bits::DUCK_BUTTON_BIT;
        return None;
    }
    if fits_here(avatar, solids, ctx, CROUCH_TOP) {
        let grounded = avatar.movement_mode != MOVE_MODE_DOWNED && avatar.standing_on != NO_ENTITY_SLOT;
        avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::LYING_FLAT) | movement_state_bits::DUCK_BUTTON_BIT;
        return grounded.then_some(ANIM_PRONE_TO_CROUCH);
    }
    if !held {
        snap_client(avatar, SNAP_TO_PRONE, 2);
    }
    None
}

fn rise_as_far_as_possible<C: Bo2Collision>(
    avatar: &mut AvatarState,
    solids: &C,
    ctx: DuckContext,
    held: bool,
) -> Option<u8> {
    if flag_set(avatar, movement_state_bits::LYING_FLAT) {
        if fits_here(avatar, solids, ctx, STAND_TOP) {
            clear_stance_flags(avatar);
            return Some(ANIM_PRONE_TO_STAND);
        }
        if fits_here(avatar, solids, ctx, CROUCH_TOP) {
            avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::LYING_FLAT) | movement_state_bits::DUCK_BUTTON_BIT;
            return (avatar.movement_mode != MOVE_MODE_DOWNED).then_some(ANIM_PRONE_TO_CROUCH);
        }
        if !held {
            snap_client(avatar, SNAP_TO_PRONE, 1);
        }
    } else if flag_set(avatar, movement_state_bits::DUCK_BUTTON_BIT) {
        if fits_here(avatar, solids, ctx, STAND_TOP) {
            avatar.movement_state_bits &= !movement_state_bits::DUCK_BUTTON_BIT;
        } else if !held {
            snap_client(avatar, SNAP_TO_CROUCH, 1);
        }
    }
    None
}

/// Picks the stance the eye heads for, once the previous change has finished.
/// The eye never skips a step: standing to prone passes through crouch, and
/// so does prone to standing. In last stand it heads for its own height.
fn stance_target(avatar: &mut AvatarState) -> PostureShift {
    if avatar.eye_blend_clock != 0 {
        return PostureShift::Unchanged;
    }
    let before = avatar.eye_height_goal;
    let (next, change) = if avatar.movement_mode == MOVE_MODE_DOWNED {
        let was_stance = matches!(before, eye_level::UPRIGHT | eye_level::DUCK_BUTTON_BIT | eye_level::LYING_FLAT);
        let change = if was_stance {
            PostureShift::Changed
        } else {
            PostureShift::Unchanged
        };
        (eye_level::DOWNED_GRACE, change)
    } else if !flag_set(avatar, movement_state_bits::LYING_FLAT) {
        if before == eye_level::LYING_FLAT {
            (eye_level::DUCK_BUTTON_BIT, PostureShift::Changed)
        } else if flag_set(avatar, movement_state_bits::DUCK_BUTTON_BIT) {
            (eye_level::DUCK_BUTTON_BIT, PostureShift::Unchanged)
        } else {
            (eye_level::UPRIGHT, PostureShift::Unchanged)
        }
    } else {
        match before {
            eye_level::UPRIGHT => (eye_level::DUCK_BUTTON_BIT, PostureShift::Unchanged),
            eye_level::LYING_FLAT => (eye_level::LYING_FLAT, PostureShift::Unchanged),
            _ => (eye_level::LYING_FLAT, PostureShift::WentFlat),
        }
    };
    aim_eye_at(avatar, next);
    change
}

/// The rest of PM_CheckDuck: the eye moves toward its stance, and the box and
/// stance flags follow wherever it is heading. Returns this frame's box.
pub(crate) fn finish_duck(
    avatar: &mut AvatarState,
    tail: DuckTail,
    bounds: HullExtents,
    clock_millis: i32,
    dt: f32,
) -> HullExtents {
    let mut bounds = HullExtents {
        lo_corner: BOX_MINS,
        hi_corner: BOX_MAXS,
        ..bounds
    };
    match tail {
        DuckTail::Spectator => {
            bounds.lo_corner = SPECTATOR_MINS;
            bounds.hi_corner = SPECTATOR_MAXS;
        }
        DuckTail::Fixed(top) => {
            bounds.hi_corner[2] = top;
            view_height_adjust(avatar, clock_millis, dt);
        }
        DuckTail::Stance => {
            view_height_adjust(avatar, clock_millis, dt);
            bounds.hi_corner[2] = effective_stance(avatar);
        }
    }
    bounds
}

/// BO2's PM_GetEffectiveStance: the stance the eye heads for decides the box
/// and the flags. Last stand lies down on dry ground, crouches in shallow
/// water and stands from the waist up.
fn effective_stance(avatar: &mut AvatarState) -> f32 {
    let settled = match (avatar.eye_height_goal, avatar.waterlevel) {
        (eye_level::DOWNED_GRACE, 3..) => eye_level::UPRIGHT,
        (eye_level::DOWNED_GRACE, 1..) => eye_level::DUCK_BUTTON_BIT,
        (eye_level::DOWNED_GRACE, _) => eye_level::LYING_FLAT,
        (other, _) => other,
    };
    let (top, e_bit, pm_bit) = match settled {
        eye_level::LYING_FLAT => (PRONE_TOP, entity_flag_bits::LYING_FLAT, movement_state_bits::LYING_FLAT),
        eye_level::DUCK_BUTTON_BIT => (CROUCH_TOP, entity_flag_bits::CROUCH_BIT, movement_state_bits::DUCK_BUTTON_BIT),
        _ => (STAND_TOP, 0, 0),
    };
    clear_stance_eflags(avatar);
    avatar.e_flags |= e_bit;
    clear_stance_flags(avatar);
    avatar.movement_state_bits |= pm_bit;
    top
}

/// BO2's PM_ViewHeightAdjust. Between standing, crouched and prone the eye
/// follows a curve over the change's duration, one stance at a time, and a
/// change of mind turns it round from wherever it is; any other height (last
/// stand) is approached at a constant speed. A sinking eye spreads the aim.
pub(crate) fn view_height_adjust(avatar: &mut AvatarState, clock_millis: i32, dt: f32) {
    let goal = avatar.eye_height_goal;
    if goal == 0 || avatar.eye_height_now == 0.0 {
        avatar.eye_height_now = if avatar.movement_mode == MOVE_MODE_OBSERVER {
            0.0
        } else {
            goal as f32
        };
        return;
    }
    if avatar.eye_height_now == goal as f32 && avatar.eye_blend_clock == 0 {
        return;
    }
    if avatar.eye_height_now > goal as f32 {
        avatar.sight_spread_factor = FULL_SPREAD;
    }
    if !matches!(goal, eye_level::LYING_FLAT | eye_level::DUCK_BUTTON_BIT | eye_level::UPRIGHT) {
        glide_to(avatar, goal, dt);
        return;
    }

    let progress = advance_step(avatar, clock_millis);
    if avatar.eye_blend_clock == 0 {
        if avatar.eye_height_now != goal as f32 {
            start_lerp(avatar, goal, clock_millis);
        }
        return;
    }
    turn_round(avatar, goal, progress, clock_millis);
}

/// Move the eye at a constant speed to a height that is not a regular stance.
fn glide_to(avatar: &mut AvatarState, goal: i32, dt: f32) {
    avatar.eye_blend_clock = 0;
    let target = goal as f32;
    let travel = dt * EYE_SPEED;
    let now = avatar.eye_height_now;
    avatar.eye_height_now = if target <= now {
        let next = now - travel;
        if target > next { target } else { next }
    } else {
        let next = now + travel;
        if target < next { target } else { next }
    };
}

/// Progress a running step to the present moment; gives how many hundredths
/// of it have elapsed (0 when none is running).
fn advance_step(avatar: &mut AvatarState, clock_millis: i32) -> i32 {
    if avatar.eye_blend_clock == 0 {
        return 0;
    }
    let end = avatar.eye_blend_goal;
    let duration = lerp_ms(end, avatar.eye_lower_blend);
    let mut percent = clock_millis
        .wrapping_sub(avatar.eye_blend_clock)
        .wrapping_mul(100_i32)
        / duration;
    if percent < 0 {
        percent = 0;
    }
    if percent >= 100 {
        avatar.eye_height_now = end as f32;
        avatar.eye_blend_clock = 0;
    } else {
        avatar.eye_height_now = curve_height(percent, end, avatar.eye_lower_blend);
    }
    percent
}

/// The next stance a reversed step ends at.
fn reversed_end(end: i32, now_down: i32) -> i32 {
    if now_down == 0 {
        match end {
            eye_level::LYING_FLAT => eye_level::DUCK_BUTTON_BIT,
            eye_level::DUCK_BUTTON_BIT => eye_level::UPRIGHT,
            same => same,
        }
    } else {
        match end {
            eye_level::UPRIGHT => eye_level::DUCK_BUTTON_BIT,
            eye_level::DUCK_BUTTON_BIT => eye_level::LYING_FLAT,
            same => same,
        }
    }
}

/// A change of mind: the eye turns back from where it is, and the way back
/// takes as long as the way out has so far.
fn turn_round(avatar: &mut AvatarState, goal: i32, progress: i32, clock_millis: i32) {
    let end = avatar.eye_blend_goal;
    let was_down = avatar.eye_lower_blend;
    let reversed = (goal < end && was_down == 0) || (goal > end && was_down != 0);
    if !reversed {
        return;
    }
    let remaining = 100 - progress;
    let now_down = was_down ^ 1;
    avatar.eye_lower_blend = now_down;
    avatar.eye_blend_goal = reversed_end(end, now_down);
    if remaining == 100 {
        avatar.eye_height_now = avatar.eye_blend_goal as f32;
        avatar.eye_blend_clock = 0;
    } else {
        let duration = lerp_ms(avatar.eye_blend_goal, now_down) as f32;
        avatar.eye_blend_clock =
            clock_millis.wrapping_sub((duration * (remaining as f32 * 0.01)) as i32);
    }
}

fn pick(cond: bool, yes: i32, no: i32) -> i32 {
    if cond { yes } else { no }
}

/// Begin a new step toward `goal` from the eye's present height.
fn start_lerp(avatar: &mut AvatarState, goal: i32, clock_millis: i32) {
    const CROUCH_EYE: f32 = eye_level::DUCK_BUTTON_BIT as f32;
    let here = avatar.eye_height_now;
    avatar.eye_blend_clock = clock_millis;
    let (down, end) = match goal {
        eye_level::LYING_FLAT => (1, pick(here <= CROUCH_EYE, eye_level::LYING_FLAT, eye_level::DUCK_BUTTON_BIT)),
        eye_level::DUCK_BUTTON_BIT => (i32::from(here > CROUCH_EYE), eye_level::DUCK_BUTTON_BIT),
        _ => (0, pick(here >= CROUCH_EYE, eye_level::UPRIGHT, eye_level::DUCK_BUTTON_BIT)),
    };
    avatar.eye_lower_blend = down;
    avatar.eye_blend_goal = end;
}

/// Duration of one step in milliseconds: 600 down to prone, 400 up out of
/// it, 200 for the rest.
fn lerp_ms(end: i32, down: i32) -> i32 {
    match end {
        eye_level::LYING_FLAT => 600,
        eye_level::DUCK_BUTTON_BIT if down == 0 => 400,
        _ => 200,
    }
}

/// Eye height `percent` hundredths of the way through a step.
fn curve_height(percent: i32, end: i32, down: i32) -> f32 {
    let (curve, low, high) = match (end, down) {
        (eye_level::LYING_FLAT, _) => (CROUCH_TO_PRONE, eye_level::LYING_FLAT, eye_level::DUCK_BUTTON_BIT),
        (eye_level::DUCK_BUTTON_BIT, 0) => (PRONE_TO_CROUCH, eye_level::LYING_FLAT, eye_level::DUCK_BUTTON_BIT),
        (eye_level::DUCK_BUTTON_BIT, _) => (STAND_TO_CROUCH, eye_level::DUCK_BUTTON_BIT, eye_level::UPRIGHT),
        _ => (CROUCH_TO_STAND, eye_level::DUCK_BUTTON_BIT, eye_level::UPRIGHT),
    };
    table_lerp(percent, curve, low, high)
}

/// BO2's PM_ViewHeightTableLerp. At 0 it yields the lower height whatever the
/// curve says (a BO2 quirk: a step a few milliseconds old can show the lower
/// height for a frame); beyond the last sample it also yields the lower.
fn table_lerp(frac: i32, curve: &'static [(i32, f32)], low: i32, high: i32) -> f32 {
    if frac == 0 {
        return low as f32;
    }
    let mut part = 0.0;
    for pair in curve.windows(2) {
        let ((prev_frac, prev_part), (at, at_part)) = (pair[0], pair[1]);
        if at == frac {
            part = at_part;
            break;
        }
        if at > frac {
            part = (frac - prev_frac) as f32 / (at - prev_frac) as f32 * (at_part - prev_part)
                + prev_part;
            break;
        }
    }
    part * (high - low) as f32 + low as f32
}

/// Contents (0x0200_0000 for bodies, plus 0x8000) that BO2's get-up and
/// lie-down traces skip (the dead skip them in every trace).
pub(crate) const BO2_GETUP_IGNORES: u32 = 0x0200_0000 | 0x8000;

/// Lying down, Black Ops II raises the body by this much, then lowers it.
const LIE_DOWN_LIFT: f32 = 10.0;

/// Depth below the lying body that Black Ops II searches for ground.
const LIE_DOWN_GROUND_PROBE: f32 = 0.25;

/// The torso pitch may differ from the player's aim by at most this.
const PRONE_TORSO_PITCH_RANGE: f32 = 45.0;

const SIGNAL_SCOPE_RESTART: i32 = 0x11;

/// BO2's BG_UsingSniperScope: a scope overlay that is fully raised.
pub(crate) fn sniper_scope_raised(avatar: &AvatarState, frame_ctx: ScopeWishArgs) -> bool {
    frame_ctx.scoped_weapon && avatar.weapon_raise_fraction >= 1.0
}

/// Point `share` of the way from `near` to `far`, component by component.
fn blend(near: [f32; 3], far: [f32; 3], share: f32) -> [f32; 3] {
    core::array::from_fn(|axis| (far[axis] - near[axis]) * share + near[axis])
}

/// Move a point straight down by `drop` units.
fn dropped(point: [f32; 3], drop: f32) -> [f32; 3] {
    [point[0], point[1], point[2] - drop]
}

/// Frame a player lies down (`was_prone` false): held movement keys drop the
/// aim; the body is raised 10 units, lowered again, turned the way the player
/// looks, and takes its slope pitch from the ground just below it (flat in
/// last stand, over nothing, or when stuck); the torso follows that pitch but
/// stays within 45 of the view. In last stand the body faces the player's
/// look direction on every frame.
pub(crate) fn bo2_lie_down<C: SolidProbe>(
    avatar: &mut AvatarState,
    ucmd: &InputCommand,
    solids: &C,
    bounds: HullExtents,
    was_prone: bool,
) {
    if !flag_set(avatar, movement_state_bits::LYING_FLAT) {
        return;
    }
    if !was_prone {
        if ucmd.thrust_ahead != 0 || ucmd.thrust_lateral != 0 {
            avatar.movement_state_bits &= !movement_state_bits::PRONEMOVE_OVERRIDDEN;
            add_event(avatar, SIGNAL_SCOPE_RESTART);
            avatar.movement_state_bits &= !movement_state_bits::ADS_INTENT;
        }
        let HullExtents { lo_corner: hull_lo, hi_corner: hull_hi, .. } = bounds;
        let sweep = |low: [f32; 3], high: [f32; 3]| {
            solids.trace(SweepAsk {
                start: low,
                end: high,
                lo_corner: hull_lo,
                hi_corner: hull_hi,
                collide_filter: bounds.collide_filter & !BO2_GETUP_IGNORES,
            })
        };
        let floor_point = avatar.origin;
        let ceiling_point = [
            floor_point[0],
            floor_point[1],
            floor_point[2] + LIE_DOWN_LIFT,
        ];
        let raised = blend(
            floor_point,
            ceiling_point,
            sweep(floor_point, ceiling_point).fraction,
        );
        avatar.origin = blend(raised, floor_point, sweep(raised, floor_point).fraction);
        avatar.prone_direction = avatar.look_angles[1];
        let ground = sweep(avatar.origin, dropped(avatar.origin, LIE_DOWN_GROUND_PROBE));
        avatar.prone_direction_pitch =
            if ground.starts_inside || ground.fraction >= 1.0 || avatar.movement_mode == MOVE_MODE_DOWNED {
                0.0
            } else {
                game_math::slope_pitch(avatar.prone_direction, ground.normal)
            };
        let pitch_gap = game_math::degree_gap(avatar.prone_direction_pitch, avatar.look_angles[0]);
        avatar.prone_torso_pitch = if pitch_gap < -PRONE_TORSO_PITCH_RANGE {
            avatar.look_angles[0] - PRONE_TORSO_PITCH_RANGE
        } else if pitch_gap > PRONE_TORSO_PITCH_RANGE {
            avatar.look_angles[0] + PRONE_TORSO_PITCH_RANGE
        } else {
            avatar.prone_direction_pitch
        };
    }
    if avatar.movement_mode == MOVE_MODE_DOWNED {
        avatar.prone_direction = avatar.look_angles[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_t6::Trace;

    /// A floor at z = 0 and a ceiling at `.0` (nothing else).
    struct Room(f32);

    impl Bo2Collision for Room {}

    impl SolidProbe for Room {
        fn trace(&self, input: SweepAsk) -> Trace {
            let solid = |p: [f32; 3]| p[2] + input.lo_corner[2] < 0.0 || p[2] + input.hi_corner[2] > self.0;
            let mut strike = Trace::new();
            if solid(input.start) {
                strike.starts_inside = true;
                strike.fully_embedded = solid(input.end);
                strike.fraction = 0.0_f32;
                return strike;
            }
            let (start, end) = (input.start[2] + input.lo_corner[2], input.end[2] + input.lo_corner[2]);
            if end < 0.0 {
                let fraction = start / (start - end);
                strike.fraction = fraction;
                strike.normal = [0.0, 0.0, 1.0];
                strike.walkable = true;
            }
            strike
        }
    }

    const OPEN: Room = Room(1000.0);

    fn ctx() -> DuckContext {
        DuckContext {
            body: HullExtents {
                lo_corner: BOX_MINS,
                hi_corner: BOX_MAXS,
                collide_filter: 0x0281_0011,
            },
            prone_barred: false,
            zombies: false,
            dive_owns: false,
        }
    }

    /// A player standing on the floor, the view settled.
    fn player() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 1022;
        avatar.eye_height_goal = eye_level::UPRIGHT;
        avatar.eye_height_now = STAND_TOP - 10.0;
        avatar
    }

    fn held(keys: u32) -> InputCommand {
        InputCommand {
            buttons: keys,
            ..InputCommand::default()
        }
    }

    /// The last event added: (event, parm).
    fn newest_happening(avatar: &AvatarState) -> (i32, i32) {
        match (avatar.predictable_event_sequence - 1) & 3 {
            0 => (avatar.predictable_events[0], avatar.predictable_event_parms[0] as i32),
            1 => (avatar.predictable_events[1], avatar.predictable_event_parms[1] as i32),
            2 => (avatar.predictable_events[2], avatar.predictable_event_parms[2] as i32),
            _ => (avatar.predictable_events[3], avatar.predictable_event_parms[3] as i32),
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_curves_give_the_lower_height_at_their_start() {
        // BO2's quirk: whatever the curve says at 0.
        assert_eq!(table_lerp(0, STAND_TO_CROUCH, 40, 60), 40.0);
        assert_eq!(table_lerp(0, CROUCH_TO_PRONE, 11, 40), 11.0);
        // On a point, and between two.
        assert!(close(table_lerp(20, CROUCH_TO_STAND, 40, 60), 44.0));
        assert!(close(table_lerp(45, CROUCH_TO_STAND, 40, 60), 50.0));
        assert!(close(
            table_lerp(50, CROUCH_TO_PRONE, 11, 40),
            0.138 * 29.0 + 11.0
        ));
        assert!(close(table_lerp(100, PRONE_TO_CROUCH, 11, 40), 40.0));
        // Prone to crouch dips below prone first.
        assert!(table_lerp(5, PRONE_TO_CROUCH, 11, 40) < 11.0);
    }

    #[test]
    fn the_steps_take_their_time() {
        assert_eq!(lerp_ms(eye_level::LYING_FLAT, 1), 600);
        assert_eq!(lerp_ms(eye_level::DUCK_BUTTON_BIT, 0), 400);
        assert_eq!(lerp_ms(eye_level::DUCK_BUTTON_BIT, 1), 200);
        assert_eq!(lerp_ms(eye_level::UPRIGHT, 0), 200);
    }

    #[test]
    fn sprinting_stands_the_player_up_every_frame() {
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::SPRINTING | movement_state_bits::DUCK_BUTTON_BIT;
        avatar.e_flags = entity_flag_bits::CROUCH_BIT;
        for frame in 1..=2 {
            let duck = check_duck(&mut avatar, &mut held(0), &OPEN, ctx());
            assert_eq!(duck.tail, DuckTail::Fixed(STAND_TOP));
            assert_eq!(avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT), 0);
            assert_eq!(avatar.e_flags & entity_flag_bits::CROUCH_BIT, 0);
            assert_eq!(avatar.eye_height_goal, eye_level::UPRIGHT);
            assert_eq!(avatar.predictable_event_sequence, frame);
            assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
        }
    }

    #[test]
    fn a_dive_keeps_its_stance_through_its_sprint() {
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::SPRINTING | movement_state_bits::LYING_FLAT;
        avatar.eye_height_goal = eye_level::LYING_FLAT;
        let duck = check_duck(
            &mut avatar,
            &mut held(0),
            &OPEN,
            DuckContext {
                dive_owns: true,
                ..ctx()
            },
        );
        assert_eq!(duck.tail, DuckTail::Stance);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::LYING_FLAT, 0);
        assert_eq!(avatar.predictable_event_sequence, 0);
    }

    #[test]
    fn the_dead_keep_the_box_of_their_stance() {
        for (flags, maxs_z) in [
            (movement_state_bits::DUCK_BUTTON_BIT, CROUCH_TOP),
            (movement_state_bits::LYING_FLAT, PRONE_TOP),
            (0, STAND_TOP),
        ] {
            let mut avatar = player();
            avatar.movement_mode = MOVE_KIND_DEAD;
            avatar.movement_state_bits = flags;
            let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
            assert_eq!(duck.tail, DuckTail::Fixed(maxs_z));
            assert_eq!(avatar.eye_height_goal, DEAD_EYE_HEIGHT);
            let bounds = finish_duck(&mut avatar, duck.tail, ctx().body, 1000, 0.05);
            assert_eq!(bounds.hi_corner[2], maxs_z);
            assert_eq!(avatar.movement_state_bits, flags);
        }
    }

    #[test]
    fn last_stand_lies_prone_without_stance_animations() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_DOWNED;
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(duck.change, PostureShift::Changed);
        assert_eq!(duck.anim, None);
        assert_eq!(avatar.eye_height_goal, eye_level::DOWNED_GRACE);
        let bounds = finish_duck(&mut avatar, duck.tail, ctx().body, 1000, 0.05);
        assert_eq!(bounds.hi_corner[2], PRONE_TOP);
        assert_eq!(
            avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT),
            movement_state_bits::LYING_FLAT
        );
        assert_eq!(avatar.e_flags & (entity_flag_bits::CROUCH_BIT | entity_flag_bits::LYING_FLAT), entity_flag_bits::LYING_FLAT);
        // Prone now, and no keys: still no get-up animation.
        let duck = check_duck(&mut avatar, &mut held(0), &OPEN, ctx());
        assert_eq!(duck.change, PostureShift::Unchanged);
        assert_eq!(duck.anim, None);
        assert_eq!(avatar.predictable_event_sequence, 0);
    }

    #[test]
    fn the_view_drops_to_last_stand_at_a_steady_speed() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_DOWNED;
        avatar.eye_height_goal = eye_level::DOWNED_GRACE;
        avatar.eye_height_now = 60.0_f32;
        view_height_adjust(&mut avatar, 1000, 0.05);
        assert!(close(avatar.eye_height_now, 51.0));
        assert_eq!(avatar.sight_spread_factor, FULL_SPREAD);
        avatar.eye_height_now = 23.0;
        view_height_adjust(&mut avatar, 1050, 0.05);
        assert_eq!(avatar.eye_height_now, 22.0);
    }

    #[test]
    fn only_a_falling_view_spreads_the_aim() {
        let mut avatar = player();
        avatar.eye_height_goal = eye_level::UPRIGHT;
        avatar.eye_height_now = 40.0;
        view_height_adjust(&mut avatar, 1000, 0.05);
        assert_eq!(avatar.sight_spread_factor, 0.0);
        assert_eq!(avatar.eye_blend_clock, 1000);
        assert_eq!(avatar.eye_blend_goal, eye_level::UPRIGHT);
        assert_eq!(avatar.eye_lower_blend, 0);
    }

    #[test]
    fn a_change_of_mind_turns_the_view_back_from_where_it_is() {
        let mut avatar = player();
        avatar.eye_height_goal = eye_level::LYING_FLAT;
        avatar.eye_height_now = 40.0;
        view_height_adjust(&mut avatar, 1000, 0.05);
        assert_eq!(
            (avatar.eye_blend_clock, avatar.eye_blend_goal),
            (1000, eye_level::LYING_FLAT)
        );
        assert_eq!(avatar.eye_height_now, 40.0);
        // Halfway down, back up to crouch: the way back takes as long as the
        // way down has, and the view stays where it is this frame.
        avatar.eye_height_goal = eye_level::DUCK_BUTTON_BIT;
        view_height_adjust(&mut avatar, 1300, 0.05);
        let halfway = 0.138 * 29.0 + 11.0;
        assert!(close(avatar.eye_height_now, halfway));
        assert_eq!(avatar.eye_blend_goal, eye_level::DUCK_BUTTON_BIT);
        assert_eq!(avatar.eye_lower_blend, 0);
        assert_eq!(avatar.eye_blend_clock, 1300 - 200);
        // The rest of the way back.
        view_height_adjust(&mut avatar, 1500, 0.05);
        assert_eq!(avatar.eye_height_now, 40.0);
        assert_eq!(avatar.eye_blend_clock, 0);
    }

    #[test]
    fn a_spectator_is_a_small_box_with_no_view() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_OBSERVER;
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        let mut ucmd = held(buttons::LYING_FLAT);
        let duck = check_duck(&mut avatar, &mut ucmd, &OPEN, ctx());
        assert_eq!(ucmd.buttons & buttons::LYING_FLAT, 0);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
        assert_eq!(avatar.movement_state_bits, 0);
        let bounds = finish_duck(&mut avatar, duck.tail, ctx().body, 1000, 0.05);
        let HullExtents { lo_corner: mins, hi_corner: maxs, .. } = bounds;
        assert_eq!((mins, maxs), (SPECTATOR_MINS, SPECTATOR_MAXS));
        assert_eq!((avatar.eye_height_goal, avatar.eye_height_now), (0, 0.0));
    }

    #[test]
    fn crouching_plays_no_animation() {
        let mut avatar = player();
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(duck.change, PostureShift::Unchanged);
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
        let bounds = finish_duck(&mut avatar, duck.tail, ctx().body, 1000, 0.05);
        assert_eq!(bounds.hi_corner[2], CROUCH_TOP);
        assert_eq!(avatar.e_flags & entity_flag_bits::CROUCH_BIT, entity_flag_bits::CROUCH_BIT);
    }

    fn lying_down() -> AvatarState {
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        avatar.e_flags = entity_flag_bits::LYING_FLAT;
        avatar.eye_height_goal = eye_level::LYING_FLAT;
        avatar.eye_height_now = 11.0;
        avatar
    }

    #[test]
    fn getting_up_from_prone_stands_by_way_of_crouch() {
        let mut avatar = lying_down();
        let duck = check_duck(&mut avatar, &mut held(0), &OPEN, ctx());
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_STAND));
        assert_eq!(duck.change, PostureShift::Changed);
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
    }

    #[test]
    fn under_a_low_roof_prone_gets_up_only_to_crouch() {
        let mut avatar = lying_down();
        let duck = check_duck(&mut avatar, &mut held(0), &Room(60.0), ctx());
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_CROUCH));
        assert_eq!(
            avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT),
            movement_state_bits::DUCK_BUTTON_BIT
        );
        // Lower still, it stays down and says so.
        let mut avatar = lying_down();
        let duck = check_duck(&mut avatar, &mut held(0), &Room(40.0), ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::LYING_FLAT, movement_state_bits::LYING_FLAT);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_PRONE, 1));
        // Not with the stance key held.
        let mut avatar = lying_down();
        check_duck(&mut avatar, &mut held(buttons::STANCE_LOCK_BIT), &Room(40.0), ctx());
        assert_eq!(avatar.predictable_event_sequence, 0);
    }

    #[test]
    fn crouching_from_prone_in_the_air_plays_no_animation() {
        let mut avatar = lying_down();
        avatar.standing_on = NO_ENTITY_SLOT;
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(
            avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT),
            movement_state_bits::DUCK_BUTTON_BIT
        );
    }

    #[test]
    fn a_blocked_prone_key_puts_the_player_back() {
        let blocked = DuckContext {
            prone_barred: true,
            ..ctx()
        };
        let mut avatar = player();
        check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, blocked);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, blocked);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_CROUCH, 0));
        // In the air nothing is said.
        let mut avatar = player();
        avatar.standing_on = NO_ENTITY_SLOT;
        check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, blocked);
        assert_eq!(avatar.predictable_event_sequence, 0);
    }

    #[test]
    fn lying_down_from_standing_goes_by_crouch() {
        let mut avatar = player();
        let duck = check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, ctx());
        assert_eq!(avatar.movement_state_bits & movement_state_bits::LYING_FLAT, movement_state_bits::LYING_FLAT);
        assert_eq!(duck.change, PostureShift::Unchanged);
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
        // Once at crouch height, prone.
        avatar.eye_height_now = 40.0;
        let duck = check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, ctx());
        assert_eq!(duck.change, PostureShift::WentFlat);
        assert_eq!(avatar.eye_height_goal, eye_level::LYING_FLAT);
    }

    #[test]
    fn a_turret_holds_the_stance_it_is_fired_from() {
        for (turret, stance) in [
            (entity_flag_bits::TURRET_STAND, 0),
            (entity_flag_bits::TURRET_CROUCH, movement_state_bits::DUCK_BUTTON_BIT),
            (TURRET_MASK, 0),
        ] {
            let mut avatar = player();
            avatar.e_flags = turret;
            avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
            check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, ctx());
            assert_eq!(avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT), stance);
        }
    }

    #[test]
    fn water_gets_a_prone_player_up() {
        let mut avatar = lying_down();
        avatar.waterlevel = 1;
        let duck = check_duck(&mut avatar, &mut held(buttons::LYING_FLAT), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Fixed(STAND_TOP));
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_CROUCH));
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
        assert_eq!(avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT), 0);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_CROUCH, 0));
        // Chest deep it goes all the way up.
        let mut avatar = lying_down();
        avatar.waterlevel = 4;
        check_duck(&mut avatar, &mut held(0), &OPEN, ctx());
        assert_eq!(avatar.eye_height_goal, eye_level::UPRIGHT);
        assert_eq!(avatar.predictable_event_sequence, 2);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
    }

    #[test]
    fn crouching_ends_past_the_waist() {
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::DUCK_BUTTON_BIT;
        avatar.eye_height_goal = eye_level::DUCK_BUTTON_BIT;
        avatar.waterlevel = 2;
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Stance);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT, 0);
        avatar.waterlevel = 3;
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Fixed(STAND_TOP));
        assert_eq!(avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT, 0);
        assert_eq!(avatar.eye_height_goal, eye_level::UPRIGHT);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
    }

    #[test]
    fn the_crouch_key_does_nothing_waist_deep() {
        let mut avatar = player();
        avatar.waterlevel = 3;
        let duck = check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_eq!(avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT, 0);
        assert_eq!(duck.change, PostureShift::Unchanged);
        assert_eq!(newest_happening(&avatar), (SNAP_TO_STAND, 0));
        avatar.waterlevel = 2;
        check_duck(&mut avatar, &mut held(buttons::DUCK_BUTTON_BIT), &OPEN, ctx());
        assert_ne!(avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT, 0);
    }

    #[test]
    fn last_stand_rises_with_the_water() {
        for (level, maxs_z, stance) in [
            (0, PRONE_TOP, movement_state_bits::LYING_FLAT),
            (1, CROUCH_TOP, movement_state_bits::DUCK_BUTTON_BIT),
            (2, CROUCH_TOP, movement_state_bits::DUCK_BUTTON_BIT),
            (3, STAND_TOP, 0),
        ] {
            let mut avatar = player();
            avatar.movement_mode = MOVE_MODE_DOWNED;
            avatar.eye_height_goal = eye_level::DOWNED_GRACE;
            avatar.waterlevel = level;
            assert_eq!(effective_stance(&mut avatar), maxs_z, "level {level}");
            assert_eq!(avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT), stance);
        }
    }
}
