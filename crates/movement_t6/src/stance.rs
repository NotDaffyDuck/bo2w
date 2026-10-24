//! Black Ops II's stance: PM_CheckDuck (the stance keys, the box, the
//! stance the view heads for) and PM_ViewHeightAdjust (the view's rise and
//! fall between them), the frame a player lies down, and its sniper scope
//! check. Written from the facts and numbers in Black Ops II's server;
//! docs/MOVE-T6.md says what is left out and why.

use crate::bridge::{
    AdsIntentContext, CROUCH_MAXS_Z, CollisionBackend, GroundTraceInput, MoveBounds, PRONE_MAXS_Z,
    ProneRules, STAND_MAXS_Z, StanceChange, add_event, add_predictable_event, view_height,
};
use playerstate_t6::game::{
    ENTITYNUM_NONE, PM_TYPE_DEAD, PM_TYPE_LAST_STAND, PM_TYPE_SPECTATOR, PlayerState, UserCmd,
    buttons, eflags, pm_flags,
};

use crate::check_prone::player_prone_allowed;
use crate::feel::EF_TURRET;
use crate::gjk::{Bo2Collision, PenetrationChecks};

/// The player's box (BO2 sets it every frame); its top is the stance's.
const BOX_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
const BOX_MAXS: [f32; 3] = [15.0, 15.0, STAND_MAXS_Z];

/// A spectator's box, and its view height (none).
const SPECTATOR_MINS: [f32; 3] = [-8.0, -8.0, -8.0];
const SPECTATOR_MAXS: [f32; 3] = [8.0, 8.0, 16.0];

/// The view height of the dead.
const VIEW_DEAD: i32 = 8;

/// The turret bits of eFlags: a turret fired lying down, or crouched.
const EF_TURRET_PRONE: u32 = 0x400;
const EF_TURRET_CROUCH: u32 = 0x800;

/// The stance is held as it is (BO2's PM_ShouldLeaveStanceAlone).
const PMF_STANCE_LOCKED: u32 = 0xc00;

/// The stance keys could not be obeyed: the client puts the player back
/// standing, crouched or prone.
const EV_STANCE_FORCE_STAND: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;
const EV_STANCE_FORCE_CROUCH: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;
const EV_STANCE_FORCE_PRONE: i32 = entity_t6::event::STANCE_FORCE_PRONE as i32;

/// The player's stance animations (the anim script's PRONE_TO_CROUCH and
/// PRONE_TO_STAND). BO2's stand-to-crouch and crouch-to-stand play only with
/// bg_playStandToCrouchAnims, off by default: the view just blends.
const ANIM_PRONE_TO_CROUCH: u8 = 12;
const ANIM_PRONE_TO_STAND: u8 = 16;

/// Off the usual heights (last stand), the view moves this many units a
/// second.
const VIEW_HEIGHT_SPEED: f32 = 180.0;

/// The view dropping marks the aim fully spread (bg_ads, on by default).
const AIM_SPREAD_MAX: f32 = 255.0;

/// Where the view is through a stance change, as a part of the way from the
/// lower height to the higher (per hundred of the change's time).
type Curve = &'static [(i32, f32)];
const CROUCH_TO_PRONE: Curve = &[
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
const PRONE_TO_CROUCH: Curve = &[
    (0, 0.0),
    (5, -0.034),
    (30, 0.345),
    (50, 0.483),
    (67, 0.69),
    (83, 0.79),
    (100, 1.0),
];
const STAND_TO_CROUCH: Curve = &[
    (0, 1.0),
    (1, 0.975),
    (4, 0.925),
    (30, 0.8),
    (80, 0.2),
    (90, 0.075),
    (95, 0.025),
    (100, 0.0),
];
const CROUCH_TO_STAND: Curve = &[
    (0, 0.0),
    (5, 0.025),
    (10, 0.075),
    (20, 0.2),
    (70, 0.8),
    (96, 0.925),
    (99, 0.975),
    (100, 1.0),
];

/// What the stance update is told about the frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DuckContext {
    pub bounds: MoveBounds,
    pub weapon_blocks_prone: bool,
    pub zombies: bool,
    /// A dive owns the stance: no stance keys, and its sprint does not stand
    /// the player up.
    pub dive_owns: bool,
}

/// How PM_CheckDuck ends, for [`finish_duck`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DuckTail {
    /// A spectator: its small box, no view height, no view update.
    Spectator,
    /// The dead, or a sprint: a box this tall, the view updated, the flags
    /// as they are.
    Fixed(f32),
    /// The view updated, then the box and flags from the stance it heads for.
    Stance,
}

/// What PM_CheckDuck did this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Duck {
    pub tail: DuckTail,
    /// The stance the view heads for changed (the torso resets; lying down
    /// also slows the player).
    pub change: StanceChange,
    /// A stance animation to play.
    pub anim: Option<u8>,
}

/// BO2's PM_CheckDuck up to the view update: the stance keys, the turret's
/// stance, and the stance the view heads for. The get-up checks put the
/// body in place with BO2's own collision; the lie-down check keeps the box
/// traces.
pub(crate) fn check_duck<C: Bo2Collision>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    collision: &C,
    ctx: DuckContext,
) -> Duck {
    let mut duck = Duck {
        tail: DuckTail::Stance,
        change: StanceChange::Unchanged,
        anim: None,
    };
    if ps.pm_type == PM_TYPE_SPECTATOR {
        ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
        if cmd.buttons & buttons::PRONE != 0 {
            cmd.buttons &= !buttons::PRONE;
            add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
        }
        ps.view_height_target = 0;
        ps.view_height_current = 0.0;
        duck.tail = DuckTail::Spectator;
        return duck;
    }
    if ps.pm_type == PM_TYPE_DEAD {
        ps.view_height_target = VIEW_DEAD;
        duck.tail = DuckTail::Fixed(if ps.pm_flags & pm_flags::CROUCH != 0 {
            CROUCH_MAXS_Z
        } else if ps.pm_flags & pm_flags::PRONE != 0 {
            PRONE_MAXS_Z
        } else {
            STAND_MAXS_Z
        });
        return duck;
    }
    // Water: lying in any of it gets the player up to a crouch view (the
    // flags let go of prone and set nothing), and from chest deep up, or
    // crouched past the waist, all the way up. The box stays standing.
    if ps.pm_flags & pm_flags::PRONE != 0 && ps.waterlevel > 0 {
        ps.view_height_target = view_height::CROUCH;
        add_predictable_event(ps, EV_STANCE_FORCE_CROUCH, 0);
        if ps.pm_type != PM_TYPE_LAST_STAND {
            duck.anim = Some(ANIM_PRONE_TO_CROUCH);
        }
        ps.pm_flags &= !pm_flags::PRONE;
        if ps.waterlevel > 3 {
            ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
            ps.view_height_target = view_height::STAND;
            add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
        }
        duck.tail = DuckTail::Fixed(STAND_MAXS_Z);
        return duck;
    }
    if ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH) != 0 && ps.waterlevel > 2 {
        ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
        ps.view_height_target = view_height::STAND;
        add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
        duck.tail = DuckTail::Fixed(STAND_MAXS_Z);
        return duck;
    }
    // Sprinting stands the player up, every frame (not through a dive).
    if ps.pm_flags & pm_flags::SPRINTING != 0 && !ctx.dive_owns {
        ps.e_flags &= !(eflags::DUCK | eflags::PRONE);
        ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
        ps.view_height_target = view_height::STAND;
        add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
        duck.tail = DuckTail::Fixed(STAND_MAXS_Z);
        return duck;
    }

    if ps.e_flags & EF_TURRET != 0 {
        // A turret holds the stance it is fired from.
        ps.pm_flags = match ps.e_flags & EF_TURRET {
            EF_TURRET_PRONE => (ps.pm_flags & !pm_flags::CROUCH) | pm_flags::PRONE,
            EF_TURRET_CROUCH => (ps.pm_flags & !pm_flags::PRONE) | pm_flags::CROUCH,
            _ => ps.pm_flags & !(pm_flags::PRONE | pm_flags::CROUCH),
        };
    } else if ps.pm_flags & PMF_STANCE_LOCKED != 0 {
    } else if ps.pm_type == PM_TYPE_LAST_STAND {
        ps.pm_flags = (ps.pm_flags & !pm_flags::PRONE) | pm_flags::CROUCH;
    } else if !ctx.dive_owns {
        duck.anim = stance_keys(ps, cmd, collision, ctx);
    }

    duck.change = stance_target(ps);
    duck
}

/// The stance keys: off a ladder they let go; prone lies down where it can,
/// crouch crouches, and neither stands up as far as the space allows. A key
/// that cannot be obeyed tells the client (unless the stance key is held).
fn stance_keys<C: Bo2Collision>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    collision: &C,
    ctx: DuckContext,
) -> Option<u8> {
    if ps.pm_flags & pm_flags::LADDER != 0 && cmd.buttons & (buttons::PRONE | buttons::CROUCH) != 0
    {
        cmd.buttons &= !(buttons::PRONE | buttons::CROUCH);
        add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
    }
    let stance_held = cmd.buttons & buttons::STANCE_HELD != 0;
    let clear = |maxs_z: f32| {
        !PenetrationChecks(collision)
            .trace(GroundTraceInput {
                start: ps.origin,
                end: ps.origin,
                mins: BOX_MINS,
                maxs: [BOX_MAXS[0], BOX_MAXS[1], maxs_z],
                tracemask: ctx.bounds.tracemask & !BO2_GETUP_IGNORES,
            })
            .allsolid
    };
    let prone = ps.pm_flags & pm_flags::PRONE != 0;
    let crouched = ps.pm_flags & pm_flags::CROUCH != 0;

    if cmd.buttons & buttons::PRONE != 0 {
        let rules = ProneRules::for_player(ps, ctx.zombies);
        if player_prone_allowed(ps, collision, BOX_MAXS[0], ctx.weapon_blocks_prone, rules) {
            ps.pm_flags = (ps.pm_flags & !pm_flags::CROUCH) | pm_flags::PRONE;
        } else if ps.ground_entity_num != ENTITYNUM_NONE && !stance_held {
            let event = if prone || crouched {
                EV_STANCE_FORCE_CROUCH
            } else {
                EV_STANCE_FORCE_STAND
            };
            add_predictable_event(ps, event, 0);
        }
        return None;
    }
    if cmd.buttons & buttons::CROUCH != 0 {
        if ps.waterlevel >= 3 {
            // Too deep to crouch: the client is told to stand.
            add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
            return None;
        }
        if !prone {
            ps.pm_flags |= pm_flags::CROUCH;
        } else if clear(CROUCH_MAXS_Z) {
            let anim = (ps.pm_type != PM_TYPE_LAST_STAND && ps.ground_entity_num != ENTITYNUM_NONE)
                .then_some(ANIM_PRONE_TO_CROUCH);
            ps.pm_flags = (ps.pm_flags & !pm_flags::PRONE) | pm_flags::CROUCH;
            return anim;
        } else if !stance_held {
            add_predictable_event(ps, EV_STANCE_FORCE_PRONE, 2);
        }
        return None;
    }
    if prone {
        if clear(STAND_MAXS_Z) {
            ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
            return Some(ANIM_PRONE_TO_STAND);
        }
        if clear(CROUCH_MAXS_Z) {
            ps.pm_flags = (ps.pm_flags & !pm_flags::PRONE) | pm_flags::CROUCH;
            return (ps.pm_type != PM_TYPE_LAST_STAND).then_some(ANIM_PRONE_TO_CROUCH);
        }
        if !stance_held {
            add_predictable_event(ps, EV_STANCE_FORCE_PRONE, 1);
        }
    } else if crouched {
        if clear(STAND_MAXS_Z) {
            ps.pm_flags &= !pm_flags::CROUCH;
        } else if !stance_held {
            add_predictable_event(ps, EV_STANCE_FORCE_CROUCH, 1);
        }
    }
    None
}

/// The stance the view heads for, picked once the last change is done. The
/// view never skips a step: standing to prone goes by crouch, and so does
/// prone to standing. In last stand it heads for the last-stand height.
fn stance_target(ps: &mut PlayerState) -> StanceChange {
    if ps.view_height_lerp_time != 0 {
        return StanceChange::Unchanged;
    }
    let previous = ps.view_height_target;
    if ps.pm_type == PM_TYPE_LAST_STAND {
        ps.view_height_target = view_height::LAST_STAND;
        return if matches!(
            previous,
            view_height::STAND | view_height::CROUCH | view_height::PRONE
        ) {
            StanceChange::Changed
        } else {
            StanceChange::Unchanged
        };
    }
    if ps.pm_flags & pm_flags::PRONE == 0 {
        if previous == view_height::PRONE {
            ps.view_height_target = view_height::CROUCH;
            return StanceChange::Changed;
        }
        ps.view_height_target = if ps.pm_flags & pm_flags::CROUCH != 0 {
            view_height::CROUCH
        } else {
            view_height::STAND
        };
        return StanceChange::Unchanged;
    }
    if previous == view_height::STAND {
        ps.view_height_target = view_height::CROUCH;
        return StanceChange::Unchanged;
    }
    if previous == view_height::PRONE {
        return StanceChange::Unchanged;
    }
    ps.view_height_target = view_height::PRONE;
    StanceChange::EnteredProne
}

/// The rest of PM_CheckDuck: the view moves toward its stance, and the box
/// and stance flags follow where it is heading. Returns the frame's box.
pub(crate) fn finish_duck(
    ps: &mut PlayerState,
    tail: DuckTail,
    bounds: MoveBounds,
    server_time: i32,
    frametime: f32,
) -> MoveBounds {
    let mut bounds = MoveBounds {
        mins: BOX_MINS,
        maxs: BOX_MAXS,
        ..bounds
    };
    match tail {
        DuckTail::Spectator => {
            bounds.mins = SPECTATOR_MINS;
            bounds.maxs = SPECTATOR_MAXS;
        }
        DuckTail::Fixed(maxs_z) => {
            bounds.maxs[2] = maxs_z;
            view_height_adjust(ps, server_time, frametime);
        }
        DuckTail::Stance => {
            view_height_adjust(ps, server_time, frametime);
            bounds.maxs[2] = effective_stance(ps);
        }
    }
    bounds
}

/// BO2's PM_GetEffectiveStance: the stance the view heads for sets the box
/// and the flags. Last stand lies prone on dry ground, crouches in water and
/// stands past the waist.
fn effective_stance(ps: &mut PlayerState) -> f32 {
    let target = match (ps.view_height_target, ps.waterlevel) {
        (view_height::LAST_STAND, 3..) => view_height::STAND,
        (view_height::LAST_STAND, 1..) => view_height::CROUCH,
        (view_height::LAST_STAND, _) => view_height::PRONE,
        (target, _) => target,
    };
    match target {
        view_height::PRONE => {
            ps.e_flags = (ps.e_flags & !eflags::DUCK) | eflags::PRONE;
            ps.pm_flags = (ps.pm_flags & !pm_flags::CROUCH) | pm_flags::PRONE;
            PRONE_MAXS_Z
        }
        view_height::CROUCH => {
            ps.e_flags = (ps.e_flags & !eflags::PRONE) | eflags::DUCK;
            ps.pm_flags = (ps.pm_flags & !pm_flags::PRONE) | pm_flags::CROUCH;
            CROUCH_MAXS_Z
        }
        _ => {
            ps.e_flags &= !(eflags::DUCK | eflags::PRONE);
            ps.pm_flags &= !(pm_flags::PRONE | pm_flags::CROUCH);
            STAND_MAXS_Z
        }
    }
}

/// BO2's PM_ViewHeightAdjust. Between standing, crouched and prone the view
/// follows a curve over the change's time, one step at a time, and a change
/// of mind turns it back from where it is; any other height (last stand) it
/// moves to at a steady speed. A view on its way down spreads the aim.
pub(crate) fn view_height_adjust(ps: &mut PlayerState, server_time: i32, frametime: f32) {
    let target = ps.view_height_target;
    if target == 0 || ps.view_height_current == 0.0 {
        ps.view_height_current = if ps.pm_type == PM_TYPE_SPECTATOR {
            0.0
        } else {
            target as f32
        };
        return;
    }
    if ps.view_height_current == target as f32 && ps.view_height_lerp_time == 0 {
        return;
    }
    if ps.view_height_current > target as f32 {
        ps.aim_spread_scale = AIM_SPREAD_MAX;
    }
    if !matches!(
        target,
        view_height::PRONE | view_height::CROUCH | view_height::STAND
    ) {
        ps.view_height_lerp_time = 0;
        let goal = target as f32;
        let step = frametime * VIEW_HEIGHT_SPEED;
        ps.view_height_current = if goal <= ps.view_height_current {
            let lowered = ps.view_height_current - step;
            if lowered < goal { goal } else { lowered }
        } else {
            let raised = ps.view_height_current + step;
            if raised > goal { goal } else { raised }
        };
        return;
    }

    let mut frac = 0;
    if ps.view_height_lerp_time != 0 {
        let lerp_target = ps.view_height_lerp_target;
        let ms = lerp_ms(lerp_target, ps.view_height_lerp_down);
        frac = server_time
            .wrapping_sub(ps.view_height_lerp_time)
            .wrapping_mul(100)
            / ms;
        if frac < 0 {
            frac = 0;
        }
        if frac >= 100 {
            ps.view_height_current = lerp_target as f32;
            ps.view_height_lerp_time = 0;
        } else {
            ps.view_height_current = curve_height(frac, lerp_target, ps.view_height_lerp_down);
        }
    }

    if ps.view_height_lerp_time == 0 {
        if ps.view_height_current != target as f32 {
            start_lerp(ps, target, server_time);
        }
        return;
    }
    // A change of mind: the view turns back from where it is, the way back
    // taking as long as the way there has so far.
    let lerp_target = ps.view_height_lerp_target;
    let down = ps.view_height_lerp_down;
    if target == lerp_target
        || !((target < lerp_target && down == 0) || (target > lerp_target && down != 0))
    {
        return;
    }
    let rewind = 100 - frac;
    let down = down ^ 1;
    ps.view_height_lerp_down = down;
    ps.view_height_lerp_target = match (down, lerp_target) {
        (0, view_height::PRONE) => view_height::CROUCH,
        (0, view_height::CROUCH) => view_height::STAND,
        (0, other) => other,
        (_, view_height::STAND) => view_height::CROUCH,
        (_, view_height::CROUCH) => view_height::PRONE,
        (_, other) => other,
    };
    if rewind == 100 {
        ps.view_height_current = ps.view_height_lerp_target as f32;
        ps.view_height_lerp_time = 0;
    } else {
        let ms = lerp_ms(ps.view_height_lerp_target, down) as f32;
        ps.view_height_lerp_time = server_time.wrapping_sub((ms * (rewind as f32 * 0.01)) as i32);
    }
}

/// A new step toward `target`, from where the view is now.
fn start_lerp(ps: &mut PlayerState, target: i32, server_time: i32) {
    const CROUCH: f32 = view_height::CROUCH as f32;
    let current = ps.view_height_current;
    ps.view_height_lerp_time = server_time;
    match target {
        view_height::PRONE => {
            ps.view_height_lerp_down = 1;
            ps.view_height_lerp_target = if current <= CROUCH {
                view_height::PRONE
            } else {
                view_height::CROUCH
            };
        }
        view_height::CROUCH => {
            ps.view_height_lerp_down = i32::from(current > CROUCH);
            ps.view_height_lerp_target = view_height::CROUCH;
        }
        _ => {
            ps.view_height_lerp_down = 0;
            ps.view_height_lerp_target = if current >= CROUCH {
                view_height::STAND
            } else {
                view_height::CROUCH
            };
        }
    }
}

/// How long a step takes: down to prone 600 ms, up from prone 400 ms, the
/// others 200 ms.
fn lerp_ms(lerp_target: i32, down: i32) -> i32 {
    match lerp_target {
        view_height::PRONE => 600,
        view_height::CROUCH if down == 0 => 400,
        _ => 200,
    }
}

/// The view's height `frac` hundredths of the way through a step.
fn curve_height(frac: i32, lerp_target: i32, down: i32) -> f32 {
    let (curve, low, high) = match (lerp_target, down) {
        (view_height::PRONE, _) => (CROUCH_TO_PRONE, view_height::PRONE, view_height::CROUCH),
        (view_height::CROUCH, 0) => (PRONE_TO_CROUCH, view_height::PRONE, view_height::CROUCH),
        (view_height::CROUCH, _) => (STAND_TO_CROUCH, view_height::CROUCH, view_height::STAND),
        _ => (CROUCH_TO_STAND, view_height::CROUCH, view_height::STAND),
    };
    table_lerp(frac, curve, low, high)
}

/// BO2's PM_ViewHeightTableLerp. At 0 it gives the lower height whatever the
/// curve says (BO2's own quirk: a step only a few milliseconds old can show
/// the lower height for a frame); past the curve's end, also the lower.
fn table_lerp(frac: i32, curve: Curve, low: i32, high: i32) -> f32 {
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

/// Bodies (0x0200_0000) and 0x8000, left out of BO2's get-up and lie-down
/// traces (and, for the dead, out of every trace).
pub(crate) const BO2_GETUP_IGNORES: u32 = 0x0200_0000 | 0x8000;

/// Lying down, Black Ops II lifts the body this far and drops it back.
const LIE_DOWN_LIFT: f32 = 10.0;

/// How far below a body lying down Black Ops II looks for the ground.
const LIE_DOWN_GROUND_PROBE: f32 = 0.25;

/// The torso's slope pitch stays within this of where the player looks.
const PRONE_TORSO_PITCH_RANGE: f32 = 45.0;

const EV_RESET_ADS: i32 = 0x11;

/// BO2's BG_UsingSniperScope: a scope overlay, fully raised.
pub(crate) fn sniper_scope_raised(ps: &PlayerState, context: AdsIntentContext) -> bool {
    context.weapon_def_scope && ps.f_weapon_pos_frac >= 1.0
}

/// On the frame a player lies down (`was_prone` false): moving keys held
/// drop the aim; the body is lifted 10 units and dropped back, points where
/// the player looks, and takes its slope pitch from the ground just under it
/// (flat in last stand, over nothing or stuck); the torso's pitch follows it
/// but stays within 45 of the view. In last stand the body points where the
/// player looks every frame.
pub(crate) fn bo2_lie_down<C: CollisionBackend>(
    ps: &mut PlayerState,
    cmd: &UserCmd,
    collision: &C,
    bounds: MoveBounds,
    was_prone: bool,
) {
    if (ps.pm_flags & pm_flags::PRONE) == 0 {
        return;
    }
    if !was_prone {
        if cmd.forwardmove != 0 || cmd.rightmove != 0 {
            ps.pm_flags &= !pm_flags::PRONEMOVE_OVERRIDDEN;
            add_event(ps, EV_RESET_ADS);
            ps.pm_flags &= !pm_flags::ADS_INTENT;
        }
        let trace = |start: [f32; 3], end: [f32; 3]| {
            collision.trace(GroundTraceInput {
                start,
                end,
                mins: bounds.mins,
                maxs: bounds.maxs,
                tracemask: bounds.tracemask & !BO2_GETUP_IGNORES,
            })
        };
        let lerp = |from: [f32; 3], to: [f32; 3], frac: f32| {
            [
                (to[0] - from[0]) * frac + from[0],
                (to[1] - from[1]) * frac + from[1],
                (to[2] - from[2]) * frac + from[2],
            ]
        };
        let origin = ps.origin;
        let up = [origin[0], origin[1], origin[2] + LIE_DOWN_LIFT];
        let lifted = lerp(origin, up, trace(origin, up).fraction);
        ps.origin = lerp(lifted, origin, trace(lifted, origin).fraction);
        ps.prone_direction = ps.viewangles[1];
        let below = [
            ps.origin[0],
            ps.origin[1],
            ps.origin[2] - LIE_DOWN_GROUND_PROBE,
        ];
        let ground = trace(ps.origin, below);
        ps.prone_direction_pitch =
            if ground.startsolid || ground.fraction >= 1.0 || ps.pm_type == PM_TYPE_LAST_STAND
            {
                0.0
            } else {
                game_math::pitch_for_yaw_on_normal(ps.prone_direction, ground.normal)
            };
        let off = game_math::angle_subtract(ps.prone_direction_pitch, ps.viewangles[0]);
        ps.prone_torso_pitch = if off < -PRONE_TORSO_PITCH_RANGE {
            ps.viewangles[0] - PRONE_TORSO_PITCH_RANGE
        } else if off > PRONE_TORSO_PITCH_RANGE {
            ps.viewangles[0] + PRONE_TORSO_PITCH_RANGE
        } else {
            ps.prone_direction_pitch
        };
    }
    if ps.pm_type == PM_TYPE_LAST_STAND {
        ps.prone_direction = ps.viewangles[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_t6::Trace;

    /// A floor at z = 0 and a ceiling at `.0` (nothing else).
    struct Room(f32);

    impl Bo2Collision for Room {}

    impl CollisionBackend for Room {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let solid = |p: [f32; 3]| p[2] + input.mins[2] < 0.0 || p[2] + input.maxs[2] > self.0;
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            if solid(input.start) {
                hit.startsolid = true;
                hit.allsolid = solid(input.end);
                hit.fraction = 0.0;
                return hit;
            }
            let (start, end) = (input.start[2] + input.mins[2], input.end[2] + input.mins[2]);
            if end < 0.0 {
                let fraction = start / (start - end);
                hit.fraction = fraction;
                hit.normal = [0.0, 0.0, 1.0];
                hit.walkable = true;
            }
            hit
        }
    }

    const OPEN: Room = Room(1000.0);

    fn ctx() -> DuckContext {
        DuckContext {
            bounds: MoveBounds {
                mins: BOX_MINS,
                maxs: BOX_MAXS,
                tracemask: 0x0281_0011,
            },
            weapon_blocks_prone: false,
            zombies: false,
            dive_owns: false,
        }
    }

    /// A player standing on the floor, the view settled.
    fn player() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 1022;
        ps.view_height_target = view_height::STAND;
        ps.view_height_current = STAND_MAXS_Z - 10.0;
        ps
    }

    fn held(keys: u32) -> UserCmd {
        UserCmd {
            buttons: keys,
            ..UserCmd::default()
        }
    }

    /// The last event added: (event, parm).
    fn last_event(ps: &PlayerState) -> (i32, i32) {
        match (ps.predictable_event_sequence - 1) & 3 {
            0 => (ps.predictable_events[0], ps.predictable_event_parms[0] as i32),
            1 => (ps.predictable_events[1], ps.predictable_event_parms[1] as i32),
            2 => (ps.predictable_events[2], ps.predictable_event_parms[2] as i32),
            _ => (ps.predictable_events[3], ps.predictable_event_parms[3] as i32),
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
        assert_eq!(lerp_ms(view_height::PRONE, 1), 600);
        assert_eq!(lerp_ms(view_height::CROUCH, 0), 400);
        assert_eq!(lerp_ms(view_height::CROUCH, 1), 200);
        assert_eq!(lerp_ms(view_height::STAND, 0), 200);
    }

    #[test]
    fn sprinting_stands_the_player_up_every_frame() {
        let mut ps = player();
        ps.pm_flags = pm_flags::SPRINTING | pm_flags::CROUCH;
        ps.e_flags = eflags::DUCK;
        for frame in 1..=2 {
            let duck = check_duck(&mut ps, &mut held(0), &OPEN, ctx());
            assert_eq!(duck.tail, DuckTail::Fixed(STAND_MAXS_Z));
            assert_eq!(ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH), 0);
            assert_eq!(ps.e_flags & eflags::DUCK, 0);
            assert_eq!(ps.view_height_target, view_height::STAND);
            assert_eq!(ps.predictable_event_sequence, frame);
            assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
        }
    }

    #[test]
    fn a_dive_keeps_its_stance_through_its_sprint() {
        let mut ps = player();
        ps.pm_flags = pm_flags::SPRINTING | pm_flags::PRONE;
        ps.view_height_target = view_height::PRONE;
        let duck = check_duck(
            &mut ps,
            &mut held(0),
            &OPEN,
            DuckContext {
                dive_owns: true,
                ..ctx()
            },
        );
        assert_eq!(duck.tail, DuckTail::Stance);
        assert_ne!(ps.pm_flags & pm_flags::PRONE, 0);
        assert_eq!(ps.predictable_event_sequence, 0);
    }

    #[test]
    fn the_dead_keep_the_box_of_their_stance() {
        for (flags, maxs_z) in [
            (pm_flags::CROUCH, CROUCH_MAXS_Z),
            (pm_flags::PRONE, PRONE_MAXS_Z),
            (0, STAND_MAXS_Z),
        ] {
            let mut ps = player();
            ps.pm_type = PM_TYPE_DEAD;
            ps.pm_flags = flags;
            let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
            assert_eq!(duck.tail, DuckTail::Fixed(maxs_z));
            assert_eq!(ps.view_height_target, VIEW_DEAD);
            let bounds = finish_duck(&mut ps, duck.tail, ctx().bounds, 1000, 0.05);
            assert_eq!(bounds.maxs[2], maxs_z);
            assert_eq!(ps.pm_flags, flags);
        }
    }

    #[test]
    fn last_stand_lies_prone_without_stance_animations() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_LAST_STAND;
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(duck.change, StanceChange::Changed);
        assert_eq!(duck.anim, None);
        assert_eq!(ps.view_height_target, view_height::LAST_STAND);
        let bounds = finish_duck(&mut ps, duck.tail, ctx().bounds, 1000, 0.05);
        assert_eq!(bounds.maxs[2], PRONE_MAXS_Z);
        assert_eq!(
            ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH),
            pm_flags::PRONE
        );
        assert_eq!(ps.e_flags & (eflags::DUCK | eflags::PRONE), eflags::PRONE);
        // Prone now, and no keys: still no get-up animation.
        let duck = check_duck(&mut ps, &mut held(0), &OPEN, ctx());
        assert_eq!(duck.change, StanceChange::Unchanged);
        assert_eq!(duck.anim, None);
        assert_eq!(ps.predictable_event_sequence, 0);
    }

    #[test]
    fn the_view_drops_to_last_stand_at_a_steady_speed() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_LAST_STAND;
        ps.view_height_target = view_height::LAST_STAND;
        ps.view_height_current = 60.0;
        view_height_adjust(&mut ps, 1000, 0.05);
        assert!(close(ps.view_height_current, 51.0));
        assert_eq!(ps.aim_spread_scale, AIM_SPREAD_MAX);
        ps.view_height_current = 23.0;
        view_height_adjust(&mut ps, 1050, 0.05);
        assert_eq!(ps.view_height_current, 22.0);
    }

    #[test]
    fn only_a_falling_view_spreads_the_aim() {
        let mut ps = player();
        ps.view_height_target = view_height::STAND;
        ps.view_height_current = 40.0;
        view_height_adjust(&mut ps, 1000, 0.05);
        assert_eq!(ps.aim_spread_scale, 0.0);
        assert_eq!(ps.view_height_lerp_time, 1000);
        assert_eq!(ps.view_height_lerp_target, view_height::STAND);
        assert_eq!(ps.view_height_lerp_down, 0);
    }

    #[test]
    fn a_change_of_mind_turns_the_view_back_from_where_it_is() {
        let mut ps = player();
        ps.view_height_target = view_height::PRONE;
        ps.view_height_current = 40.0;
        view_height_adjust(&mut ps, 1000, 0.05);
        assert_eq!(
            (ps.view_height_lerp_time, ps.view_height_lerp_target),
            (1000, view_height::PRONE)
        );
        assert_eq!(ps.view_height_current, 40.0);
        // Halfway down, back up to crouch: the way back takes as long as the
        // way down has, and the view stays where it is this frame.
        ps.view_height_target = view_height::CROUCH;
        view_height_adjust(&mut ps, 1300, 0.05);
        let halfway = 0.138 * 29.0 + 11.0;
        assert!(close(ps.view_height_current, halfway));
        assert_eq!(ps.view_height_lerp_target, view_height::CROUCH);
        assert_eq!(ps.view_height_lerp_down, 0);
        assert_eq!(ps.view_height_lerp_time, 1300 - 200);
        // The rest of the way back.
        view_height_adjust(&mut ps, 1500, 0.05);
        assert_eq!(ps.view_height_current, 40.0);
        assert_eq!(ps.view_height_lerp_time, 0);
    }

    #[test]
    fn a_spectator_is_a_small_box_with_no_view() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_SPECTATOR;
        ps.pm_flags = pm_flags::CROUCH;
        let mut cmd = held(buttons::PRONE);
        let duck = check_duck(&mut ps, &mut cmd, &OPEN, ctx());
        assert_eq!(cmd.buttons & buttons::PRONE, 0);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
        assert_eq!(ps.pm_flags, 0);
        let bounds = finish_duck(&mut ps, duck.tail, ctx().bounds, 1000, 0.05);
        assert_eq!((bounds.mins, bounds.maxs), (SPECTATOR_MINS, SPECTATOR_MAXS));
        assert_eq!((ps.view_height_target, ps.view_height_current), (0, 0.0));
    }

    #[test]
    fn crouching_plays_no_animation() {
        let mut ps = player();
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(duck.change, StanceChange::Unchanged);
        assert_eq!(ps.view_height_target, view_height::CROUCH);
        let bounds = finish_duck(&mut ps, duck.tail, ctx().bounds, 1000, 0.05);
        assert_eq!(bounds.maxs[2], CROUCH_MAXS_Z);
        assert_eq!(ps.e_flags & eflags::DUCK, eflags::DUCK);
    }

    fn lying_down() -> PlayerState {
        let mut ps = player();
        ps.pm_flags = pm_flags::PRONE;
        ps.e_flags = eflags::PRONE;
        ps.view_height_target = view_height::PRONE;
        ps.view_height_current = 11.0;
        ps
    }

    #[test]
    fn getting_up_from_prone_stands_by_way_of_crouch() {
        let mut ps = lying_down();
        let duck = check_duck(&mut ps, &mut held(0), &OPEN, ctx());
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_STAND));
        assert_eq!(duck.change, StanceChange::Changed);
        assert_eq!(ps.view_height_target, view_height::CROUCH);
    }

    #[test]
    fn under_a_low_roof_prone_gets_up_only_to_crouch() {
        let mut ps = lying_down();
        let duck = check_duck(&mut ps, &mut held(0), &Room(60.0), ctx());
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_CROUCH));
        assert_eq!(
            ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH),
            pm_flags::CROUCH
        );
        // Lower still, it stays down and says so.
        let mut ps = lying_down();
        let duck = check_duck(&mut ps, &mut held(0), &Room(40.0), ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(ps.pm_flags & pm_flags::PRONE, pm_flags::PRONE);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_PRONE, 1));
        // Not with the stance key held.
        let mut ps = lying_down();
        check_duck(&mut ps, &mut held(buttons::STANCE_HELD), &Room(40.0), ctx());
        assert_eq!(ps.predictable_event_sequence, 0);
    }

    #[test]
    fn crouching_from_prone_in_the_air_plays_no_animation() {
        let mut ps = lying_down();
        ps.ground_entity_num = ENTITYNUM_NONE;
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(duck.anim, None);
        assert_eq!(
            ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH),
            pm_flags::CROUCH
        );
    }

    #[test]
    fn a_blocked_prone_key_puts_the_player_back() {
        let blocked = DuckContext {
            weapon_blocks_prone: true,
            ..ctx()
        };
        let mut ps = player();
        check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, blocked);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
        let mut ps = player();
        ps.pm_flags = pm_flags::CROUCH;
        check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, blocked);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_CROUCH, 0));
        // In the air nothing is said.
        let mut ps = player();
        ps.ground_entity_num = ENTITYNUM_NONE;
        check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, blocked);
        assert_eq!(ps.predictable_event_sequence, 0);
    }

    #[test]
    fn lying_down_from_standing_goes_by_crouch() {
        let mut ps = player();
        let duck = check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, ctx());
        assert_eq!(ps.pm_flags & pm_flags::PRONE, pm_flags::PRONE);
        assert_eq!(duck.change, StanceChange::Unchanged);
        assert_eq!(ps.view_height_target, view_height::CROUCH);
        // Once at crouch height, prone.
        ps.view_height_current = 40.0;
        let duck = check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, ctx());
        assert_eq!(duck.change, StanceChange::EnteredProne);
        assert_eq!(ps.view_height_target, view_height::PRONE);
    }

    #[test]
    fn a_turret_holds_the_stance_it_is_fired_from() {
        for (turret, stance) in [
            (EF_TURRET_PRONE, pm_flags::PRONE),
            (EF_TURRET_CROUCH, pm_flags::CROUCH),
            (EF_TURRET, 0),
        ] {
            let mut ps = player();
            ps.e_flags = turret;
            ps.pm_flags = pm_flags::CROUCH;
            check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, ctx());
            assert_eq!(ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH), stance);
        }
    }

    #[test]
    fn water_gets_a_prone_player_up() {
        let mut ps = lying_down();
        ps.waterlevel = 1;
        let duck = check_duck(&mut ps, &mut held(buttons::PRONE), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Fixed(STAND_MAXS_Z));
        assert_eq!(duck.anim, Some(ANIM_PRONE_TO_CROUCH));
        assert_eq!(ps.view_height_target, view_height::CROUCH);
        assert_eq!(ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH), 0);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_CROUCH, 0));
        // Chest deep it goes all the way up.
        let mut ps = lying_down();
        ps.waterlevel = 4;
        check_duck(&mut ps, &mut held(0), &OPEN, ctx());
        assert_eq!(ps.view_height_target, view_height::STAND);
        assert_eq!(ps.predictable_event_sequence, 2);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
    }

    #[test]
    fn crouching_ends_past_the_waist() {
        let mut ps = player();
        ps.pm_flags = pm_flags::CROUCH;
        ps.view_height_target = view_height::CROUCH;
        ps.waterlevel = 2;
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Stance);
        assert_ne!(ps.pm_flags & pm_flags::CROUCH, 0);
        ps.waterlevel = 3;
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(duck.tail, DuckTail::Fixed(STAND_MAXS_Z));
        assert_eq!(ps.pm_flags & pm_flags::CROUCH, 0);
        assert_eq!(ps.view_height_target, view_height::STAND);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
    }

    #[test]
    fn the_crouch_key_does_nothing_waist_deep() {
        let mut ps = player();
        ps.waterlevel = 3;
        let duck = check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_eq!(ps.pm_flags & pm_flags::CROUCH, 0);
        assert_eq!(duck.change, StanceChange::Unchanged);
        assert_eq!(last_event(&ps), (EV_STANCE_FORCE_STAND, 0));
        ps.waterlevel = 2;
        check_duck(&mut ps, &mut held(buttons::CROUCH), &OPEN, ctx());
        assert_ne!(ps.pm_flags & pm_flags::CROUCH, 0);
    }

    #[test]
    fn last_stand_rises_with_the_water() {
        for (level, maxs_z, stance) in [
            (0, PRONE_MAXS_Z, pm_flags::PRONE),
            (1, CROUCH_MAXS_Z, pm_flags::CROUCH),
            (2, CROUCH_MAXS_Z, pm_flags::CROUCH),
            (3, STAND_MAXS_Z, 0),
        ] {
            let mut ps = player();
            ps.pm_type = PM_TYPE_LAST_STAND;
            ps.view_height_target = view_height::LAST_STAND;
            ps.waterlevel = level;
            assert_eq!(effective_stance(&mut ps), maxs_z, "level {level}");
            assert_eq!(ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH), stance);
        }
    }
}
