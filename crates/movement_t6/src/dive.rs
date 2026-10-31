//! bo2zm: Black Ops II's dive to prone (the "dolphin dive"). Not part of
//! the upstream engine. bo2mp: copied from the legacy movement's dive for the BO2
//! frame (movement_t6), plus BO2's Dtp_Interrupt (`interrupt`).
//!
//! Built step for step on BO2's own dive: while sprinting on the ground,
//! pressing prone (or holding a pad's stance button a moment) throws the
//! player up at twice a jump's launch speed, keeping the run's speed and
//! direction. From the next frame the view drops to prone (standing to
//! crouched, then crouched to lying, the usual two steps) and the body
//! shrinks with it: standing until the view is below crouch height, then
//! crouched, then prone, so a dive goes through a window. At a jump's height
//! the dive stops rising, and for its first 0.4 s it hangs there. In the air
//! the player steers with the usual air control. On touch-down they slide on
//! their front: no friction at first, the ground's full friction by 0.3 s,
//! when the slide ends (no friction over a ledge top they could climb). A
//! slide that comes to a stop (into a wall) ends at once. The player then
//! lies still for 0.1 s and stays prone until they press jump, crouch, prone
//! or sprint. Getting up from a dive takes as long as from an ordinary prone,
//! as in BO2 (bo2mp 10-10; its quick get-up is sprinting out of it). A get-up pressed during the dive is kept, so the player rises the moment
//! it is over.
//!
//! The numbers are BO2's dive dvar defaults: `dtp_startup_delay` 250 ms of
//! sprint before a dive, `dtp_min_speed` 3.16, `dtp_new_trajectory_multiplier`
//! 2, `dtp_max_apex_duration` 400 ms, `dtp_max_slide_duration` 300 ms,
//! `dtp_post_move_pause` 100 ms and `dtp_exhaustion_window` 1.5 s from one
//! dive's end to the next dive.
//!
//! State rides in the replicated player state, so the host and the local
//! prediction agree: `PMF_DIVE` (BO2's dive bit) in the air and sliding,
//! `PMF_DIVE_PRONE` held prone after the slide (the still moment included),
//! `PMF_DIVE_GETUP` from a get-up press until the player is up; `hop_start_ms` =
//! take-off, then minus the touch-down (below 0 = sliding, as BO2's
//! Dtp_IsSliding); `takeoff_height` = the take-off height;
//! `dive_end_time` = when the slide ended.

use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, InputCommand, buttons, entity_flag_bits, movement_state_bits};

use crate::bridge::{SQUAT_HULL_TOP, BELLY_HULL_TOP, UPRIGHT_HULL_TOP};
use crate::bridge::{
    AirborneStepArgs, SOLID_CLIMBABLE, SolidProbe, SweepAsk, HullExtents, TickLocals,
};
use crate::bridge::queue_forecast_happening;
use crate::bridge::end_sprint;
use crate::bridge::eye_level;

/// Black Ops II's dive bit: from take-off until the slide ends.
pub const PMF_DIVE: u32 = crate::bridge::PMF_DIVE;
/// After a dive's slide: the player lies still a moment, then stays prone
/// (as BO2 leaves them) until they press jump, crouch, prone or sprint,
/// whatever the keys' hold or toggle style.
pub const PMF_DIVE_PRONE: u32 = 0x0400_0000;
/// Getting up from a dive (or waiting to, once it ends): a held prone key
/// does not drop the player
/// back down until it is let go.
pub const PMF_DIVE_GETUP: u32 = 0x0800_0000;

/// A dive starts only after more than this long sprinting
/// (`dtp_startup_delay`).
const STARTUP_MS: i32 = 250;
/// A dive starts only above this ground speed, and a slide slower than this
/// is over (`dtp_min_speed`).
const MIN_SPEED: f32 = 3.16;
/// Another dive waits until more than this long after the last one ended
/// (`dtp_exhaustion_window`).
const EXHAUSTION_MS: i32 = 1500;
/// The take-off's upward speed is this many times a jump's
/// (`dtp_new_trajectory_multiplier`).
const POP_SCALE: f32 = 2.0;
/// For this long after take-off a dive at a jump's height hangs there
/// (`dtp_max_apex_duration`).
const GLIDE_MS: i32 = 400;
/// The slide's friction comes in over this long, then the slide ends
/// (`dtp_max_slide_duration`).
const SLIDE_MS: i32 = 300;
/// The still moment after the slide (`dtp_post_move_pause`).
const PAUSE_MS: i32 = 100;
/// Pushback flag: no ground friction while it is on (cleared by a dive).
const PMF_TIME_KNOCKBACK: u32 = 0x100;

/// A dive is under way: in the air or sliding.
pub fn active(avatar: &AvatarState) -> bool {
    avatar.movement_state_bits & PMF_DIVE != 0
}

/// BO2's Dtp_IsSliding: the landed dive slides.
pub fn sliding(avatar: &AvatarState) -> bool {
    active(avatar) && avatar.hop_start_ms < 0
}

/// In the air, diving.
pub fn in_air(avatar: &AvatarState) -> bool {
    active(avatar) && avatar.hop_start_ms >= 0
}

/// BO2's `divetoprone` for scripts: from take-off until the slide ends.
/// PhD Flopper's blast and the dive notifies read this.
pub fn dive_to_prone(avatar: &AvatarState) -> bool {
    active(avatar)
}

/// Any part of a dive, up to the player being back on their feet.
pub fn any(avatar: &AvatarState) -> bool {
    avatar.movement_state_bits & (PMF_DIVE | PMF_DIVE_PRONE | PMF_DIVE_GETUP) != 0
}

/// The still moment after a dive's slide: the player's push is ignored.
pub fn pausing(avatar: &AvatarState, now: i32) -> bool {
    !active(avatar)
        && avatar.dive_end_time != 0
        && (0..PAUSE_MS).contains(&now.wrapping_sub(avatar.dive_end_time))
}

/// The dive owns the stance, from the frame after take-off to the end of the
/// still moment after the slide.
pub fn owns_stance(avatar: &AvatarState, now: i32) -> bool {
    active(avatar) || pausing(avatar, now)
}

/// BO2 aims the view at prone through the whole dive; it gets there by the
/// usual steps, standing to crouched, then crouched to lying.
pub fn force_prone(avatar: &mut AvatarState) {
    avatar.e_flags &= !entity_flag_bits::CROUCH_BIT;
    avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::DUCK_BUTTON_BIT) | movement_state_bits::LYING_FLAT;
    avatar.eye_height_goal = eye_level::LYING_FLAT;
}

/// The body's height through a dive, from where the view has dropped to:
/// standing until the view is below crouch height, crouched until it is
/// below prone height, then prone.
pub fn hull_height(avatar: &AvatarState) -> f32 {
    let view = avatar.eye_height_now;
    if view < BELLY_HULL_TOP {
        BELLY_HULL_TOP
    } else if view < SQUAT_HULL_TOP {
        SQUAT_HULL_TOP
    } else {
        UPRIGHT_HULL_TOP
    }
}

/// Whether this command starts a dive: sprinting on the ground long enough
/// and fast enough, and prone newly pressed.
pub fn wants(avatar: &AvatarState, ucmd: &InputCommand, last_tick_buttons: u32) -> bool {
    if active(avatar)
        || avatar.movement_state_bits & movement_state_bits::SPRINTING == 0
        || avatar.movement_state_bits & (movement_state_bits::LADDER | movement_state_bits::MANTLE) != 0
        || avatar.standing_on == NO_ENTITY_SLOT
        || (avatar.movement_mode != 0 && avatar.movement_mode != 1)
    {
        return false;
    }
    // Prone newly pressed. A pad's stance button sends prone once held
    // `cl_dtpHoldTime` while sprinting (the legacy input); a crouch only crouches.
    if (ucmd.buttons & !last_tick_buttons) & buttons::LYING_FLAT == 0 {
        return false;
    }
    if ucmd.authority_clock_ms.wrapping_sub(avatar.sprint_state.sprint_begin_ms) <= STARTUP_MS {
        return false;
    }
    if avatar.dive_end_time != 0
        && (0..=EXHAUSTION_MS).contains(&ucmd.authority_clock_ms.wrapping_sub(avatar.dive_end_time))
    {
        return false;
    }
    ground_speed(avatar) > MIN_SPEED
}

fn ground_speed(avatar: &AvatarState) -> f32 {
    libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1])
}

/// The take-off's upward speed: `POP_SCALE` times a jump's.
fn pop_speed(avatar: &AvatarState, jump_height: f32) -> f32 {
    POP_SCALE * libm::sqrtf(2.0 * jump_height * avatar.gravity as f32)
}

/// Launch the dive: sprint ends and the player leaves the ground at their
/// running speed and direction, with `pop_speed` upward. The stance is left
/// as it is this frame; the drop to prone starts on the next.
pub fn start(avatar: &mut AvatarState, tick_vars: &mut TickLocals, ucmd: &InputCommand, jump_height: f32) {
    end_sprint(avatar, ucmd);
    avatar.velocity[2] = pop_speed(avatar, jump_height);
    avatar.movement_state_bits = (avatar.movement_state_bits
        & !(movement_state_bits::TIME_HARDLANDING
            | PMF_TIME_KNOCKBACK
            | movement_state_bits::JUMPING
            | PMF_DIVE_PRONE
            | PMF_DIVE_GETUP))
        | PMF_DIVE;
    avatar.move_timer = 0;
    avatar.standing_on = NO_ENTITY_SLOT;
    avatar.hop_start_ms = ucmd.authority_clock_ms;
    avatar.takeoff_height = avatar.origin[2];
    tick_vars.walking = 0;
    tick_vars.standing_on_floor = 0;
    tick_vars.near_floor_contact = 0;
}

/// A key that gets the player up from a dive: jump, crouch, prone or sprint
/// newly pressed, or a stance toggle switched off (a pad, or toggle-style
/// keys).
fn getup_pressed(ucmd: &InputCommand, last_tick_buttons: u32) -> bool {
    let new = ucmd.buttons & !last_tick_buttons;
    if new & (buttons::LEAP | buttons::DUCK_BUTTON_BIT | buttons::LYING_FLAT | buttons::SPRINT) != 0 {
        return true;
    }
    last_tick_buttons & buttons::STANCE_LOCK_BIT == 0
        && last_tick_buttons & !ucmd.buttons & (buttons::LYING_FLAT | buttons::DUCK_BUTTON_BIT) != 0
}

/// While the dive owns the stance, a get-up press is kept for when it ends.
pub fn queue_getup(avatar: &mut AvatarState, ucmd: &InputCommand, last_tick_buttons: u32) {
    if getup_pressed(ucmd, last_tick_buttons) {
        avatar.movement_state_bits |= PMF_DIVE_GETUP;
    }
}

/// Getting up: a prone toggle still on would drop the player straight back
/// down, so the client is told to let it go (as when a sprint stands a
/// prone player up).
fn begin_getup(avatar: &mut AvatarState, ucmd: &InputCommand) {
    avatar.movement_state_bits = (avatar.movement_state_bits & !PMF_DIVE_PRONE) | PMF_DIVE_GETUP;
    if ucmd.buttons & buttons::LYING_FLAT != 0 && ucmd.buttons & buttons::STANCE_LOCK_BIT == 0 {
        queue_forecast_happening(avatar, STAND_FORCED, 0);
    }
}

const STAND_FORCED: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;

/// A slide that has come to a stop (into a wall) is over at once. Run
/// before the stance update.
pub fn end_stopped_slide(avatar: &mut AvatarState, now: i32) {
    if sliding(avatar) && speed_3d(avatar) < MIN_SPEED {
        end_slide(avatar, now);
    }
}

/// The slide is over: the player stops, lies still for `PAUSE_MS`, then
/// holds prone.
fn end_slide(avatar: &mut AvatarState, now: i32) {
    avatar.movement_state_bits = (avatar.movement_state_bits & !PMF_DIVE) | PMF_DIVE_PRONE;
    avatar.velocity = Default::default();
    avatar.hop_start_ms = 0;
    avatar.dive_end_time = now;
    avatar.sprint_state.sprint_needs_release = 0;
}

/// Move one frame of a dive: touch-down starts the slide, the dive does not
/// rise above a jump's height (and hangs there early on), then the ordinary
/// air move runs, its ground friction scaled through the slide.
pub fn advance<C: crate::gjk::Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    ucmd: &InputCommand,
    bounds: HullExtents,
    solids: &C,
    jump_height: f32,
    air: AirborneStepArgs,
) -> f32 {
    let now = ucmd.authority_clock_ms;
    if in_air(avatar) && tick_vars.walking != 0 {
        // BO2 marks the slide with jumpTime below 0: minus the touch-down.
        avatar.hop_start_ms = now.wrapping_neg();
    }
    if avatar.origin[2] >= avatar.takeoff_height + jump_height {
        if in_air(avatar) && now.wrapping_sub(avatar.hop_start_ms) < GLIDE_MS {
            avatar.velocity[2] = 0.0;
        }
        avatar.velocity[2] = avatar.velocity[2].min(0.0);
    }
    // A dive never happens in last stand, the only time Zombies' prone check
    // differs.
    // A dive is never a knife lunge.
    crate::air::air_move_with(avatar, tick_vars, ucmd, air, bounds, solids, false, None, |avatar| {
        slide_friction(avatar, now, solids, bounds)
    })
}

/// The slide's share of the ground's friction: none at touch-down, rising
/// to all of it at `SLIDE_MS`, when the slide ends; none over a ledge top
/// the player could climb (mantle ground). Asked only when ground friction
/// applies.
fn slide_friction<C: SolidProbe>(
    avatar: &mut AvatarState,
    now: i32,
    solids: &C,
    bounds: HullExtents,
) -> f32 {
    if !sliding(avatar) {
        return 1.0;
    }
    let elapsed = now.wrapping_add(avatar.hop_start_ms);
    if elapsed < SLIDE_MS {
        if on_mantle_ground(avatar, solids, bounds) {
            return 0.0;
        }
        return elapsed.max(0) as f32 / SLIDE_MS as f32;
    }
    end_slide(avatar, now);
    1.0
}

fn speed_3d(avatar: &AvatarState) -> f32 {
    let v = avatar.velocity;
    libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

/// Ground the player could mantle (a ledge top) under them.
fn on_mantle_ground<C: SolidProbe>(
    avatar: &AvatarState,
    solids: &C,
    bounds: HullExtents,
) -> bool {
    let o = avatar.origin;
    let HullExtents { lo_corner: hull_lo, hi_corner: hull_hi, .. } = bounds;
    solids
        .trace(SweepAsk {
            start: [o[0], o[1], o[2] + 1.0],
            end: [o[0], o[1], o[2] - 1.0],
            lo_corner: hull_lo,
            hi_corner: hull_hi,
            collide_filter: SOLID_CLIMBABLE,
        })
        .fraction
        < 1.0
}

/// After a dive the player holds prone until a get-up key (or one pressed
/// during the dive, or something else stood them up); `true` while it
/// holds, so the stance update is skipped.
pub fn holds_prone(avatar: &mut AvatarState, ucmd: &InputCommand, last_tick_buttons: u32) -> bool {
    if avatar.movement_state_bits & PMF_DIVE_PRONE == 0 {
        return false;
    }
    if avatar.movement_state_bits & PMF_DIVE_GETUP != 0
        || getup_pressed(ucmd, last_tick_buttons)
        || avatar.movement_state_bits & movement_state_bits::SPRINTING != 0
    {
        begin_getup(avatar, ucmd);
        return false;
    }
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0 {
        avatar.movement_state_bits &= !PMF_DIVE_PRONE;
        return false;
    }
    true
}

/// Getting up from a dive, as opposed to a get-up only queued while the dive
/// still owns the stance.
pub fn getting_up(avatar: &AvatarState) -> bool {
    avatar.movement_state_bits & PMF_DIVE_GETUP != 0
        && avatar.movement_state_bits & (PMF_DIVE | PMF_DIVE_PRONE) == 0
}

/// The command the stance and sprint updates see: while getting up from a
/// dive, a held prone key is left out, so it does not drop the player back
/// down (a hold-style key pressed to dive, or a prone toggle the client has
/// not yet let go).
pub fn stance_cmd(avatar: &AvatarState, ucmd: &InputCommand) -> InputCommand {
    let mut out = *ucmd;
    if avatar.movement_state_bits & PMF_DIVE_GETUP != 0 {
        out.buttons &= !buttons::LYING_FLAT;
    }
    out
}

/// The command the aim update sees: the sprint key, held through a dive,
/// does not drop the aim, and the player's own push (it only steers a dive)
/// does not count as crawling.
pub fn ads_cmd(avatar: &AvatarState, ucmd: &InputCommand, diving: bool) -> InputCommand {
    let mut out = *ucmd;
    if diving || avatar.movement_state_bits & PMF_DIVE_PRONE != 0 {
        out.buttons &= !buttons::SPRINT;
    }
    if diving {
        out.thrust_ahead = 0;
        out.thrust_lateral = 0;
    }
    out
}

/// The get-up is over once the view has come to rest out of prone (both
/// steps, lying to crouched and crouched to standing, are done) and the
/// prone key is let go. Run before the view update, so a step that has just
/// ended and the next one not yet begun does not end it early.
pub fn finish_getup(avatar: &mut AvatarState, ucmd: &InputCommand) {
    if !getting_up(avatar) {
        return;
    }
    if avatar.eye_blend_clock == 0
        && avatar.eye_height_goal != eye_level::LYING_FLAT
        && avatar.eye_height_now == avatar.eye_height_goal as f32
        && ucmd.buttons & buttons::LYING_FLAT == 0
    {
        avatar.movement_state_bits &= !PMF_DIVE_GETUP;
    }
}

/// BO2's Dtp_Interrupt: something else (a link to an entity) takes the
/// player out of a dive at once. The dive is over, they stop, and the next
/// dive's rest window counts from just before now.
pub fn interrupt(avatar: &mut AvatarState, now: i32) {
    avatar.movement_state_bits &= !(PMF_DIVE | PMF_DIVE_PRONE | PMF_DIVE_GETUP);
    avatar.hop_start_ms = 0;
    avatar.velocity = Default::default();
    avatar.dive_end_time = now.wrapping_sub(INTERRUPT_BACKDATE_MS).max(0);
    avatar.sprint_state.sprint_needs_release = 0;
}

/// How far before now an interrupted dive counts as ended.
const INTERRUPT_BACKDATE_MS: i32 = 101;
