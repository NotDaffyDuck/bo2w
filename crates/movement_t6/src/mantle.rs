//! Black Ops II's ledge climb: Mantle_Check, Mantle_FindMantleSurface,
//! Mantle_CheckLedge, Mantle_CalcEndPos, Mantle_Start and the end of
//! Mantle_Move. The climb's timing, the root track and the view cap are the
//! shared ones (they already match BO2 with the t6 animation tree).
//!
//! Where BO2 differs from the shared climb:
//! - The ledge's forward and drop sweeps ignore player bodies and 0x8000
//!   (`collide_filter & 0xfdff_7fff`); the fit at the top keeps the full mask.
//! - A surface marked to climb onto sets its own bit (`MANTLE_ON`), and only
//!   a surface marked over or onto shows the hint and can start a climb.
//! - The start's crouch and stand checks use the standing player's box, not
//!   a point.
//! - The climb's yaw is BO2's vectoyaw (0 to 360, double-precision arctan).
//! - In MP a climb onto a ledge stops dead at the top; a vault keeps its
//!   speed, and so does a vault that had no room past the ledge.
//! - A ledge on a moving thing makes that thing the platform the climber
//!   rides (mover.rs); every frame of the climb renews it.
//! - An unbroken window up to 26 units ahead (10 past the forward sweep)
//!   refuses the climb.
//!
//! - Zombies only: a roof hatch over the player (Hatch_Check) comes before any
//!   ledge: jump, let go and jump again climbs up through it.
//!
//! Not here: mounting a gun on a ledge or while prone (Mount_CheckLedge,
//! Mount_CheckProne), and the animation event for a vault.

use crate::bridge::mantle::{
    self, SOLID_CLIMBABLE, CLIMB_HEADROOM_TOP, CLIMB_AHEAD_REACH, CLIMB_SIDE_REACH,
    CLIMB_LIP_LOWEST_RISE, CLIMB_LIP_RISES, VAULT_AHEAD_REACH, ClimbPillSweep,
    ClimbOutcomes, PERK_FASTMANTLE, FLOOR_CLIMB_ONTO_OR_VAULT, FLOOR_VAULT_ONLY,
};
use crate::bridge::{
    ClimbStepArgs, ClimbBoneOffsetSource, ClimbClipDuration, UPRIGHT_HULL_TOP, queue_forecast_happening,
};
use playerstate_t6::game::ledge_climb_bits::{ACTIVE, FAST_MANTLE, OVER};
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::{AvatarState, entity_flag_bits, movement_state_bits};

use trace_t6::trace_get_entity_hit_id;
use trace_t6::TraceHitType;

use crate::view::bo2_vectoyaw;

/// The world as an entity number: a ledge on it rides nothing.
const WORLD: i32 = trace_t6::ENTITYNUM_WORLD as i32;

/// BO2's mantle flag 2: the surface is climbed onto, not over. Ours sits on a
/// bit the client and the gun never read.
pub(crate) const MANTLE_ON: i32 = 1 << 11;

/// BO2's mantle flag 4: no room to stand at the ledge, so the climb crouches
/// (event 7 every frame). The shared climb reads this bit, and so do the
/// bots (they hold crouch through it).
pub const MANTLE_FORCE_CROUCH: i32 = 1 << 1;

/// BO2's mantle flag 8: room to stand at the end, so the climb stands up
/// (event 6 at the end). The shared climb reads this bit.
const MANTLE_FORCE_STAND: i32 = 1 << 2;

/// The ledge's forward and drop sweeps leave out player bodies (0x0200_0000)
/// and 0x8000.
const LEDGE_MASK: u32 = 0xfdff_7fff;

/// The ledge sweeps' box is 30 units tall.
const LEDGE_SWEEP_MAXS_Z: f32 = CLIMB_SIDE_REACH * 2.0;

/// BO2's mantle_check_glass_extra_range: the window check reaches this much
/// past the forward sweep.
const GLASS_EXTRA_RANGE: f32 = 10.0;

/// BO2's mantle_check_radius and mantle_check_angle.
const CHECK_RADIUS: f32 = 0.1;
const CHECK_ANGLE: f32 = 60.0;

/// mantle_check_range_tu (MP) and mantle_check_range (zombies).
const CHECK_RANGE_MP: f32 = 10.0;
const CHECK_RANGE_ZOMBIES: f32 = 20.0;

/// The surface sweep's box is as tall as a standing player.
const SURFACE_MAXS_Z: f32 = 70.0;

/// The constant BO2 turns the angle to the wall into degrees with.
const DEGREES_PER_RADIAN: f32 = 57.295_776;

const NORMAL_MIN_LEN: f32 = 0.0001;

/// EV_MANTLE, and BODY_CLIMBING_FLAG in our entity flag numbers.
const EV_MANTLE: i32 = entity_t6::event::MANTLE as i32;
const BODY_CLIMBING_FLAG: u32 = entity_flag_bits::MANTLE;

/// Crouching (4) and prone (8): only a standing player climbs.
const EF_CROUCHING: u32 = 4;
const EF_PRONE: u32 = 8;

const INTENT_LEAP_BIT: u32 = 0x400;

/// BO2's hatch_mantle_up and hatch_mantle_fwd.
const HATCH_UP: f32 = 10.0;
const HATCH_FWD: f32 = 44.0;

/// Mantle flags for the hatch climb, on bits the climb and the gun never
/// read: a hatch climb, and "jump was let go under the hatch" (the next
/// press climbs).
pub(crate) const MANTLE_HATCH: i32 = 1 << 12;
pub(crate) const MANTLE_HATCH_READY: i32 = 1 << 13;

pub(crate) struct CheckContext {
    pub held_buttons: u32,
    pub heading_planar: [f32; 2],
    pub collide_filter: u32,
    /// MP (mantle_adjustment_tu on) or zombies.
    pub mp: bool,
}

/// Mantle_Check: looks for a ledge in front, shows the hint, and starts the
/// climb when jump is held. True when a climb started.
pub(crate) fn check(
    avatar: &mut AvatarState,
    frame_ctx: &CheckContext,
    tracer: &mut impl ClimbPillSweep,
    lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) -> bool {
    if avatar.movement_mode >= MOVE_KIND_DEAD
        || avatar.movement_state_bits & movement_state_bits::MANTLE != 0
        || avatar.e_flags & (EF_CROUCHING | EF_PRONE) != 0
    {
        return false;
    }
    let range = if frame_ctx.mp {
        CHECK_RANGE_MP
    } else {
        CHECK_RANGE_ZOMBIES
    };
    let Some((dir, surface_flags)) = find_surface(avatar.origin, frame_ctx.heading_planar, range, tracer) else {
        return false;
    };
    let mut ledge_out = ClimbOutcomes {
        heading: dir,
        begin_at: avatar.origin,
        flag_word: surface_bits(surface_flags),
        ..ClimbOutcomes::default()
    };
    for height in CLIMB_LIP_RISES {
        if check_ledge(avatar, &mut ledge_out, height, frame_ctx, tracer, lengths, root) {
            break;
        }
    }
    avatar.movement_state_bits & movement_state_bits::MANTLE != 0
}

/// Hatch_FindHatchSurface: a thin column 20 to 70 above the feet, swept from
/// 10 below to 10 above along the view's up, meets a climbable mantle brush
/// (a roof hatch). Where the column stopped.
fn hatch_find_surface(
    at_pos: [f32; 3],
    up: [f32; 3],
    tracer: &mut impl ClimbPillSweep,
) -> Option<[f32; 3]> {
    let box_lo = [-CHECK_RADIUS, -CHECK_RADIUS, HATCH_UP * 2.0];
    let box_hi = [CHECK_RADIUS, CHECK_RADIUS, SURFACE_MAXS_Z];
    let len = libm::sqrtf(up[0] * up[0] + up[1] * up[1] + up[2] * up[2]);
    let stow_kind = 1.0 / if len <= 0.0 { 1.0 } else { len };
    let start: [f32; 3] = core::array::from_fn(|i| at_pos[i] - up[i] * stow_kind * 10.0);
    let end: [f32; 3] = core::array::from_fn(|i| at_pos[i] + up[i] * stow_kind * HATCH_UP);
    let strike = tracer.trace(start, end, box_lo, box_hi, SOLID_CLIMBABLE);
    if strike.starts_inside || strike.fully_embedded || strike.fraction == 1.0 {
        return None;
    }
    // BO2 asks for its mantle-on, mantle-over or hatch surface bits; the map
    // loader keeps the first two, which the hatches carry.
    if strike.sflags & FLOOR_CLIMB_ONTO_OR_VAULT == 0 {
        return None;
    }
    Some(core::array::from_fn(|i| start[i] + (end[i] - start[i]) * strike.fraction))
}

/// Hatch_Check and Hatch_CheckHatch (Zombies only, before the ledge search):
/// under a hatch, jump, let go and jump again climbs up through it, the 57
/// climb ending 44 ahead and 87 above where the column met it. True while a
/// hatch is over the player (the ledge search is skipped).
pub(crate) fn hatch_check(
    avatar: &mut AvatarState,
    held_buttons: u32,
    fwd_axis: [f32; 3],
    up: [f32; 3],
    tracer: &mut impl ClimbPillSweep,
    lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) -> bool {
    let Some(strike) = hatch_find_surface(avatar.origin, up, tracer) else {
        avatar.mantle_state.flags &= !MANTLE_HATCH_READY;
        return false;
    };
    let ([fx, fy], _) = normalize_xy([fwd_axis[0], fwd_axis[1]]);
    let dir = [fx, fy, 0.0];
    let end = [
        strike[0] + dir[0] * HATCH_FWD,
        strike[1] + dir[1] * HATCH_FWD,
        strike[2] + HATCH_UP + 77.0,
    ];
    let mut ledge_out = ClimbOutcomes {
        heading: dir,
        begin_at: avatar.origin,
        lip_point: end,
        finish_location: end,
        flag_word: MANTLE_HATCH,
    };
    let jump = held_buttons & INTENT_LEAP_BIT != 0;
    if !jump {
        avatar.mantle_state.flags |= MANTLE_HATCH | MANTLE_HATCH_READY;
        ledge_out.flag_word |= MANTLE_HATCH_READY;
    }
    if avatar.mantle_state.flags & MANTLE_HATCH_READY != 0 && jump {
        // BO2 adds its "stand up at the end" bit and not its "stop at the
        // end" one (mantle_adjustment_tu is off in Zombies).
        ledge_out.flag_word |= MANTLE_FORCE_STAND;
        start_climb(avatar, &ledge_out, 0, lengths, root);
    }
    true
}

/// Over wins; otherwise onto.
fn surface_bits(surface_flags: u32) -> i32 {
    if surface_flags & FLOOR_VAULT_ONLY != 0 {
        OVER
    } else if surface_flags & FLOOR_CLIMB_ONTO_OR_VAULT != 0 {
        MANTLE_ON
    } else {
        0
    }
}

/// Mantle_FindMantleSurface: a thin box sweeps forward at the feet for a
/// climbable surface the player faces within 60 degrees. Returns the
/// direction into it and its surface flags.
fn find_surface(
    at_pos: [f32; 3],
    heading_planar: [f32; 2],
    range: f32,
    tracer: &mut impl ClimbPillSweep,
) -> Option<([f32; 3], u32)> {
    let inner = CLIMB_SIDE_REACH - CHECK_RADIUS;
    let [fx, fy] = normalize_xy(heading_planar).0;
    let back = -inner;
    let ahead = range + inner;
    let foot_z = at_pos[2];
    let start = [at_pos[0] + fx * back, at_pos[1] + fy * back, foot_z];
    let end = [at_pos[0] + fx * ahead, at_pos[1] + fy * ahead, foot_z];
    let box_lo = [-CHECK_RADIUS, -CHECK_RADIUS, 0.0];
    let box_hi = [CHECK_RADIUS, CHECK_RADIUS, SURFACE_MAXS_Z];
    let strike = tracer.trace(start, end, box_lo, box_hi, SOLID_CLIMBABLE);
    if strike.starts_inside || strike.fully_embedded || strike.fraction == 1.0 {
        return None;
    }
    if strike.sflags & FLOOR_CLIMB_ONTO_OR_VAULT == 0 {
        return None;
    }
    let ([step_along_x, dy], len) = normalize_xy([-strike.normal[0], -strike.normal[1]]);
    if len < NORMAL_MIN_LEN {
        return None;
    }
    // BO2 takes the arccosine without clamping, so a dot a hair past 1 fails.
    let dot = fx * step_along_x + fy * dy;
    let angle = (libm::acos(f64::from(dot)) * f64::from(DEGREES_PER_RADIAN)) as f32;
    (angle <= CHECK_ANGLE).then_some(([step_along_x, dy, 0.0], strike.sflags))
}

/// Vec2Normalize as BO2 does it: a zero vector stays zero; the length comes
/// back too.
fn normalize_xy(v: [f32; 2]) -> ([f32; 2], f32) {
    let length = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    // Zero (or -0.0) divides by one instead; NaN passes through as BO2 lets it.
    let divisor = if -length >= 0.0 { 1.0 } else { length };
    let inv = 1.0 / divisor;
    ([v[0] * inv, v[1] * inv], length)
}

/// Mantle_CheckLedge at one height: the box sweeps forward over the ledge,
/// drops onto it, and must fit there. A ledge that is climbable shows the
/// hint; with jump held the climb starts. True when the ledge counts (no
/// lower height is tried then).
fn check_ledge(
    avatar: &mut AvatarState,
    ledge_out: &mut ClimbOutcomes,
    height: f32,
    frame_ctx: &CheckContext,
    tracer: &mut impl ClimbPillSweep,
    lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) -> bool {
    let box_lo = [-CLIMB_SIDE_REACH, -CLIMB_SIDE_REACH, 0.0];
    let sweep_maxs = [CLIMB_SIDE_REACH, CLIMB_SIDE_REACH, LEDGE_SWEEP_MAXS_Z];
    let ledge_mask = frame_ctx.collide_filter & LEDGE_MASK;
    let from = ledge_out.begin_at;
    let dir = ledge_out.heading;
    let start = [from[0], from[1], from[2] + height];
    let end = [
        dir[0] * CLIMB_AHEAD_REACH + start[0],
        dir[1] * CLIMB_AHEAD_REACH + start[1],
        dir[2] * CLIMB_AHEAD_REACH + start[2],
    ];
    let forward = tracer.trace(start, end, box_lo, sweep_maxs, ledge_mask);
    if forward.starts_inside || forward.fraction < 1.0 {
        return false;
    }
    let lip_probe = [end[0], end[1], from[2] + CLIMB_LIP_LOWEST_RISE];
    let down = tracer.trace(end, lip_probe, box_lo, sweep_maxs, ledge_mask);
    if down.starts_inside || down.fraction == 1.0 || !down.walkable {
        return false;
    }
    // What the drop landed on: a moving thing carries the climb.
    let mover = if down.impact_category == TraceHitType::Entity {
        i32::from(trace_get_entity_hit_id(&down))
    } else {
        WORLD
    };
    let ledge_z = (lip_probe[2] - end[2]) * down.fraction + end[2];
    ledge_out.lip_point = [lip_probe[0], lip_probe[1], ledge_z];
    if ledge_z - from[2] <= 0.0 {
        return false;
    }
    let fit_maxs = [CLIMB_SIDE_REACH, CLIMB_SIDE_REACH, CLIMB_HEADROOM_TOP];
    let ledge = ledge_out.lip_point;
    if tracer.trace(ledge, ledge, box_lo, fit_maxs, frame_ctx.collide_filter).starts_inside {
        return false;
    }
    // A window just past the ledge: the forward sweep again, reaching further.
    let reach = CLIMB_AHEAD_REACH + GLASS_EXTRA_RANGE;
    let glass_end = [
        dir[0] * reach + start[0],
        dir[1] * reach + start[1],
        dir[2] * reach + start[2],
    ];
    let glass = tracer.trace(start, glass_end, box_lo, sweep_maxs, ledge_mask);
    if glass.starts_inside || (glass.fraction < 1.0 && glass.impact_category == TraceHitType::Glass) {
        return false;
    }
    if ledge_out.flag_word & (OVER | MANTLE_ON) == 0 {
        return false;
    }
    avatar.mantle_state.flags |= ACTIVE;
    ledge_out.flag_word |= ACTIVE;
    calc_end_pos(ledge_out, frame_ctx.collide_filter, tracer);
    if frame_ctx.held_buttons & INTENT_LEAP_BIT != 0 {
        if avatar.e_flags & EF_CROUCHING == 0 {
            start_clearance(ledge_out, frame_ctx.collide_filter, tracer);
        }
        start_climb(avatar, ledge_out, mover, lengths, root);
    }
    true
}

/// Mantle_CalcEndPos: a vault carries on 31 units past the ledge and drops
/// 18. Without room for that it becomes a climb onto the ledge, and the
/// surface's over and onto bits both go.
fn calc_end_pos(ledge_out: &mut ClimbOutcomes, collide_filter: u32, tracer: &mut impl ClimbPillSweep) {
    if ledge_out.flag_word & OVER != 0 {
        let box_lo = [-CLIMB_SIDE_REACH, -CLIMB_SIDE_REACH, 0.0];
        let box_hi = [CLIMB_SIDE_REACH, CLIMB_SIDE_REACH, CLIMB_HEADROOM_TOP];
        let start = ledge_out.lip_point;
        let end = [
            ledge_out.heading[0] * VAULT_AHEAD_REACH + start[0],
            ledge_out.heading[1] * VAULT_AHEAD_REACH + start[1],
            ledge_out.heading[2] * VAULT_AHEAD_REACH + start[2],
        ];
        let forward = tracer.trace(start, end, box_lo, box_hi, collide_filter);
        if !forward.starts_inside && forward.fraction >= 1.0 {
            let lip_probe = [end[0], end[1], end[2] - CLIMB_LIP_LOWEST_RISE];
            let down = tracer.trace(end, lip_probe, box_lo, box_hi, collide_filter);
            if !down.starts_inside && down.fraction >= 1.0 {
                let z = (lip_probe[2] - end[2]) * down.fraction + end[2];
                ledge_out.finish_location = [lip_probe[0], lip_probe[1], z];
                return;
            }
        }
        ledge_out.flag_word &= !(OVER | MANTLE_ON);
    }
    ledge_out.finish_location = ledge_out.lip_point;
}

/// The start's stance checks, with the standing player's box: no room to
/// stand at the ledge crouches the climb; room at the end stands it up.
fn start_clearance(ledge_out: &mut ClimbOutcomes, collide_filter: u32, tracer: &mut impl ClimbPillSweep) {
    let box_lo = [-CLIMB_SIDE_REACH, -CLIMB_SIDE_REACH, 0.0];
    let box_hi = [CLIMB_SIDE_REACH, CLIMB_SIDE_REACH, UPRIGHT_HULL_TOP];
    let ledge = ledge_out.lip_point;
    if tracer.trace(ledge, ledge, box_lo, box_hi, collide_filter).starts_inside {
        ledge_out.flag_word |= MANTLE_FORCE_CROUCH;
    }
    let end = ledge_out.finish_location;
    if !tracer.trace(end, end, box_lo, box_hi, collide_filter).starts_inside {
        ledge_out.flag_word |= MANTLE_FORCE_STAND;
    }
}

/// Mantle_Start: faces the climb along the wall, picks the animation for the
/// ledge's height, and puts the player where the climb's root track ends at
/// the end position. A ledge on a moving thing (`mover`) becomes the
/// platform the climber rides.
fn start_climb(
    avatar: &mut AvatarState,
    ledge_out: &ClimbOutcomes,
    mover: i32,
    lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) {
    avatar.mantle_state.yaw = bo2_vectoyaw(ledge_out.heading[0], ledge_out.heading[1]);
    avatar.mantle_state.timer = 0;
    // A hatch climb is always the highest (57) climb.
    avatar.mantle_state.trans_index = if ledge_out.flag_word & MANTLE_HATCH != 0 {
        0
    } else {
        mantle::find_transition(ledge_out.begin_at[2], ledge_out.lip_point[2])
    };
    avatar.mantle_state.flags = ledge_out.flag_word;
    if avatar.has_perk_bit(PERK_FASTMANTLE) {
        avatar.mantle_state.flags |= FAST_MANTLE;
    }
    let duration = mantle::duration(avatar, lengths);
    let delta = mantle::sample_root_track(avatar, duration, lengths, root);
    avatar.origin = [
        ledge_out.finish_location[0] - delta[0],
        ledge_out.finish_location[1] - delta[1],
        ledge_out.finish_location[2] - delta[2],
    ];
    avatar.movement_state_bits |= movement_state_bits::MANTLE;
    if mover > 0 && mover != WORLD {
        avatar.mover_entity_num = mover;
    }
    queue_forecast_happening(avatar, EV_MANTLE, 0);
    avatar.e_flags |= BODY_CLIMBING_FLAG;
}

/// Mantle_Move: the shared climb moves the player along the root track and
/// sends the crouch and stand events. At the end, in MP, a climb onto a
/// ledge stops dead. Climbing keeps the ledge's mover from timing out.
/// True when a vault over ends this move: the legs play the jump.
pub(crate) fn tick_climb(
    avatar: &mut AvatarState,
    msec: i32,
    authority_clock_ms: i32,
    mp: bool,
    lengths: &impl ClimbClipDuration,
    root: &impl ClimbBoneOffsetSource,
) -> bool {
    avatar.mover_timestamp = authority_clock_ms;
    let frame_ctx = ClimbStepArgs {
        stop_on_ledge: false,
        ..ClimbStepArgs::default()
    };
    mantle::run_climb(avatar, msec, frame_ctx, lengths, root);
    let ended = avatar.movement_state_bits & movement_state_bits::MANTLE == 0;
    if ended && mp && avatar.mantle_state.flags & MANTLE_ON != 0 {
        avatar.velocity = Default::default();
    }
    ended && avatar.mantle_state.flags & OVER != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use crate::bridge::mantle::{ClipDrivenClimbOffset, FixedClimbClipDuration};
    use trace_t6::Trace;
    use playerstate_t6::game::ledge_climb_bits::QUICK;

    const MASK: u32 = 0x0281_0011;

    /// One sweep as the climb asked for it.
    #[derive(Clone, Copy, Debug)]
    struct Sweep {
        from_pt: [f32; 3],
        to_pt: [f32; 3],
        box_hi: [f32; 3],
        mask: u32,
    }

    /// A ledge 40 units up, 10 units ahead along +x, with nothing above it.
    /// `solid_at` lists sweeps (by order) that start solid.
    struct Ledge {
        sweeps: Vec<Sweep>,
        floor_bits: u32,
        solid_at: Vec<usize>,
        vault_blocked: bool,
        /// The entity the ledge's top belongs to.
        top: Option<u16>,
        /// What stands a little past the ledge's edge, out of the forward
        /// sweep's reach (its hit type).
        window: Option<TraceHitType>,
    }

    impl Ledge {
        fn new(flags_in: u32) -> Self {
            Self {
                sweeps: Vec::new(),
                floor_bits: flags_in,
                solid_at: Vec::new(),
                vault_blocked: false,
                top: None,
                window: None,
            }
        }
    }

    const LEDGE_TOP: f32 = 40.0;
    const WALL_X: f32 = 10.0;

    impl ClimbPillSweep for Ledge {
        fn trace(&mut self, from_pt: [f32; 3], to_pt: [f32; 3], _mins: [f32; 3], box_hi: [f32; 3], mask: u32) -> Trace {
            let index = self.sweeps.len();
            self.sweeps.push(Sweep { from_pt: from_pt, to_pt: to_pt, box_hi, mask });
            let mut t = Trace::new();
            if self.solid_at.contains(&index) {
                t.starts_inside = true;
                return t;
            }
            if mask == SOLID_CLIMBABLE {
                // The surface sweep meets the wall's face.
                t.fraction = 0.5;
                t.normal = [-1.0, 0.0, 0.0];
                t.sflags = self.floor_bits;
                return t;
            }
            let going_down = to_pt[2] < from_pt[2];
            let reach = libm::sqrtf((to_pt[0] - from_pt[0]).powi(2) + (to_pt[1] - from_pt[1]).powi(2));
            if let Some(kind) = self.window
                && !going_down
                && from_pt[2] > LEDGE_TOP
                && reach > CLIMB_AHEAD_REACH + 1.0
            {
                // The window check's longer sweep meets it.
                t.fraction = 0.8;
                t.impact_category = kind;
                return t;
            }
            if self.vault_blocked && from_pt != to_pt && from_pt[2] == LEDGE_TOP {
                // Something on the ledge blocks the way past it.
                t.fraction = 0.5;
                return t;
            }
            if going_down && from_pt[2] > LEDGE_TOP && to_pt[2] < LEDGE_TOP && from_pt[0] > WALL_X {
                // The drop lands on the ledge's top.
                t.fraction = (from_pt[2] - LEDGE_TOP) / (from_pt[2] - to_pt[2]);
                t.walkable = true;
                if let Some(entity) = self.top {
                    t.impact_category = TraceHitType::Entity;
                    t.struck_entity_num = entity;
                }
                return t;
            }
            if !going_down && from_pt[2] < LEDGE_TOP {
                // Below the top, the forward sweep hits the wall.
                t.fraction = 0.2;
            }
            t
        }
    }

    fn standing() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.origin = [0.0, 0.0, 0.0];
        avatar
    }

    fn frame_ctx(jump: bool) -> CheckContext {
        CheckContext {
            held_buttons: if jump { INTENT_LEAP_BIT } else { 0 },
            heading_planar: [1.0, 0.0],
            collide_filter: MASK,
            mp: true,
        }
    }

    fn run(avatar: &mut AvatarState, ledge: &mut Ledge, jump: bool) -> bool {
        check(
            avatar,
            &frame_ctx(jump),
            ledge,
            &FixedClimbClipDuration::default(),
            &ClipDrivenClimbOffset,
        )
    }

    #[test]
    fn the_ledge_sweeps_ignore_bodies_and_the_fit_does_not() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(!run(&mut avatar, &mut ledge, false));
        // The surface sweep, then from 60 up: forward, drop onto the top, fit,
        // and the window check.
        let masks: Vec<u32> = ledge.sweeps.iter().map(|s| s.mask).collect();
        assert_eq!(
            masks,
            [SOLID_CLIMBABLE, MASK & LEDGE_MASK, MASK & LEDGE_MASK, MASK, MASK & LEDGE_MASK],
            "{:?}",
            ledge.sweeps
        );
        assert_eq!(MASK & LEDGE_MASK & 0x0200_0000, 0);
        // The hint shows without jump; no climb.
        assert_ne!(avatar.mantle_state.flags & ACTIVE, 0);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
    }

    #[test]
    fn a_window_just_past_the_ledge_refuses_the_climb() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.window = Some(TraceHitType::Glass);
        assert!(!run(&mut avatar, &mut ledge, true));
        assert_eq!(avatar.mantle_state.flags & ACTIVE, 0);
        // The window check: the forward sweep's start and box, 26 units on.
        let forward = ledge.sweeps[1];
        let glass = ledge.sweeps[4];
        assert_eq!(glass.from_pt, forward.from_pt);
        assert_eq!(glass.box_hi, forward.box_hi);
        assert_eq!(glass.mask, MASK & LEDGE_MASK);
        assert!((glass.to_pt[0] - glass.from_pt[0] - 26.0).abs() < 1e-4, "{glass:?}");
        // Anything else there is no window: the climb goes ahead.
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.window = Some(TraceHitType::Entity);
        assert!(run(&mut avatar, &mut ledge, true));
    }

    #[test]
    fn an_onto_ledge_climbs_and_stops_dead_at_the_top_in_mp() {
        let mut avatar = standing();
        avatar.velocity = [200.0, 0.0, 0.0];
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(run(&mut avatar, &mut ledge, true));
        assert_ne!(avatar.mantle_state.flags & MANTLE_ON, 0);
        assert_eq!(avatar.mantle_state.flags & OVER, 0);
        let lengths = FixedClimbClipDuration::default();
        for _ in 0..20 {
            if avatar.movement_state_bits & movement_state_bits::MANTLE == 0 {
                break;
            }
            tick_climb(&mut avatar, 50, 1000, true, &lengths, &ClipDrivenClimbOffset);
        }
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
        assert_eq!(avatar.velocity, [0.0; 3]);
        assert_ne!(avatar.mantle_state.flags & QUICK, 0);
        assert!((avatar.origin[2] - LEDGE_TOP).abs() < 1e-3, "{:?}", avatar.origin);
    }

    #[test]
    fn a_vault_with_no_room_past_the_ledge_keeps_its_speed() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0400_0000);
        ledge.vault_blocked = true;
        assert!(run(&mut avatar, &mut ledge, true));
        // The vault became a climb onto the ledge, and both bits went.
        assert_eq!(avatar.mantle_state.flags & (OVER | MANTLE_ON), 0);
        let lengths = FixedClimbClipDuration::default();
        while avatar.movement_state_bits & movement_state_bits::MANTLE != 0 {
            tick_climb(&mut avatar, 50, 1000, true, &lengths, &ClipDrivenClimbOffset);
        }
        assert_ne!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn a_vault_goes_over_and_keeps_its_speed() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0400_0000);
        assert!(run(&mut avatar, &mut ledge, true));
        assert_ne!(avatar.mantle_state.flags & OVER, 0);
        let lengths = FixedClimbClipDuration::default();
        while avatar.movement_state_bits & movement_state_bits::MANTLE != 0 {
            tick_climb(&mut avatar, 50, 1000, true, &lengths, &ClipDrivenClimbOffset);
        }
        assert_ne!(avatar.velocity, [0.0; 3]);
        assert!((avatar.origin[2] - (LEDGE_TOP - CLIMB_LIP_LOWEST_RISE)).abs() < 1e-3, "{:?}", avatar.origin);
    }

    #[test]
    fn zombies_keep_the_speed_onto_a_ledge() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(run(&mut avatar, &mut ledge, true));
        let lengths = FixedClimbClipDuration::default();
        while avatar.movement_state_bits & movement_state_bits::MANTLE != 0 {
            tick_climb(&mut avatar, 50, 1000, false, &lengths, &ClipDrivenClimbOffset);
        }
        assert_ne!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn the_stance_checks_use_the_standing_box() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        // Sweeps: surface, forward, drop, fit, window, then the two stance
        // checks (5 and 6). No room to stand at the ledge.
        ledge.solid_at = vec![5];
        assert!(run(&mut avatar, &mut ledge, true));
        let checks = &ledge.sweeps[5..];
        assert_eq!(checks.len(), 2, "{:?}", ledge.sweeps);
        for sweep in checks {
            assert_eq!(sweep.box_hi, [CLIMB_SIDE_REACH, CLIMB_SIDE_REACH, UPRIGHT_HULL_TOP]);
            assert_eq!(sweep.mask, MASK);
            assert_eq!(sweep.from_pt, sweep.to_pt);
        }
        assert_ne!(avatar.mantle_state.flags & MANTLE_FORCE_CROUCH, 0);
        assert_ne!(avatar.mantle_state.flags & MANTLE_FORCE_STAND, 0);
    }

    #[test]
    fn crouched_or_prone_players_never_climb() {
        for stance_bits in [EF_CROUCHING, EF_PRONE] {
            let mut avatar = standing();
            avatar.e_flags = stance_bits;
            let mut ledge = Ledge::new(0x0200_0000);
            assert!(!run(&mut avatar, &mut ledge, true));
            assert!(ledge.sweeps.is_empty());
        }
    }

    #[test]
    fn the_climb_faces_the_wall_from_0_to_360() {
        let mut avatar = standing();
        let mut ledge_out = ClimbOutcomes {
            heading: [0.0, -1.0, 0.0],
            lip_point: [0.0, -26.0, 40.0],
            finish_location: [0.0, -26.0, 40.0],
            flag_word: MANTLE_ON,
            ..ClimbOutcomes::default()
        };
        start_climb(&mut avatar, &ledge_out, WORLD, &FixedClimbClipDuration::default(), &ClipDrivenClimbOffset);
        assert!((avatar.mantle_state.yaw - 270.0).abs() < 1e-3, "{}", avatar.mantle_state.yaw);
        assert_ne!(avatar.e_flags & BODY_CLIMBING_FLAG, 0);
        // BO2's BODY_CLIMBING_FLAG is 0x8000; 0x10000 is the vehicle gunner.
        assert_eq!(avatar.e_flags & 0x1_8000, 0x8000);
        // The fast-climb perk picks the fast tree.
        let mut fast = standing();
        fast.set_perk(PERK_FASTMANTLE, true);
        ledge_out.heading = [1.0, 0.0, 0.0];
        start_climb(&mut fast, &ledge_out, WORLD, &FixedClimbClipDuration::default(), &ClipDrivenClimbOffset);
        assert_ne!(fast.mantle_state.flags & FAST_MANTLE, 0);
        assert_eq!(fast.mantle_state.yaw, 0.0);
    }

    #[test]
    fn walls_past_60_degrees_are_not_climbed() {
        let mut ledge = Ledge::new(0x0200_0000);
        let face = |deg: f32| {
            let r = deg.to_radians();
            [libm::cosf(r), libm::sinf(r)]
        };
        assert!(find_surface([0.0; 3], face(59.0), CHECK_RANGE_MP, &mut ledge).is_some());
        assert!(find_surface([0.0; 3], face(61.0), CHECK_RANGE_MP, &mut ledge).is_none());
        // The sweep reaches from 14.9 back to 24.9 ahead in MP.
        let sweep = ledge.sweeps[0];
        assert!((sweep.from_pt[0] + 14.9 * libm::cosf(59f32.to_radians())).abs() < 1e-4);
        assert_eq!(sweep.box_hi, [CHECK_RADIUS, CHECK_RADIUS, SURFACE_MAXS_Z]);
        // A surface that is neither over nor onto is not climbed.
        let mut plain = Ledge::new(0x0080_0000);
        assert!(find_surface([0.0; 3], [1.0, 0.0], CHECK_RANGE_MP, &mut plain).is_none());
    }

    #[test]
    fn a_ledge_on_a_moving_thing_is_ridden() {
        let mut avatar = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.top = Some(40);
        assert!(run(&mut avatar, &mut ledge, true));
        assert_eq!(avatar.mover_entity_num, 40);
        let lengths = FixedClimbClipDuration::default();
        tick_climb(&mut avatar, 50, 1234, true, &lengths, &ClipDrivenClimbOffset);
        assert_eq!(avatar.mover_timestamp, 1234);
    }

    #[test]
    fn a_ledge_on_the_world_or_nothing_tagged_rides_nothing() {
        for top in [Some(trace_t6::ENTITYNUM_WORLD), None] {
            let mut avatar = standing();
            let mut ledge = Ledge::new(0x0200_0000);
            ledge.top = top;
            assert!(run(&mut avatar, &mut ledge, true));
            assert_eq!(avatar.mover_entity_num, playerstate_t6::game::NO_ENTITY_SLOT, "{top:?}");
        }
    }

    #[test]
    fn surface_bits_prefer_over() {
        assert_eq!(surface_bits(0x0600_0000), OVER);
        assert_eq!(surface_bits(0x0200_0000), MANTLE_ON);
        assert_eq!(surface_bits(0), 0);
    }
}

#[cfg(test)]
mod hatch_tests {
    use super::*;
    use crate::bridge::mantle::{FixedClimbClipDuration, StillClimbOffset};
    use trace_t6::Trace;

    /// A roof hatch: a climbable slab from 144 to 150 up, over everything.
    struct Hatch;

    impl ClimbPillSweep for Hatch {
        fn trace(
            &mut self,
            from_pt: [f32; 3],
            end: [f32; 3],
            _mins: [f32; 3],
            box_hi: [f32; 3],
            mask: u32,
        ) -> Trace {
            let mut t = Trace::new();
            if mask & SOLID_CLIMBABLE == 0 {
                return t;
            }
            let (top0, top1) = (from_pt[2] + box_hi[2], end[2] + box_hi[2]);
            if top0 > 144.0 {
                t.starts_inside = true;
                return t;
            }
            if top1 < 144.0 {
                return t;
            }
            t.fraction = (144.0 - top0) / (top1 - top0);
            t.sflags = 0x0200_0000;
            t.normal = [0.0, 0.0, -1.0];
            t
        }
    }

    const FORWARD: [f32; 3] = [1.0, 0.0, 0.0];
    const UP: [f32; 3] = [0.0, 0.0, 1.0];

    fn step(avatar: &mut AvatarState, jump: bool) -> bool {
        let buttons = if jump { INTENT_LEAP_BIT } else { 0 };
        let lengths = FixedClimbClipDuration::default();
        hatch_check(avatar, buttons, FORWARD, UP, &mut Hatch, &lengths, &StillClimbOffset)
    }

    fn at(z: f32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.origin = [0.0, 0.0, z];
        avatar
    }

    #[test]
    fn standing_under_it_finds_nothing() {
        // The column reaches 80 above the feet: from the aisle (38) the hatch
        // (144) is out of reach.
        let mut avatar = at(38.0);
        avatar.mantle_state.flags = MANTLE_HATCH_READY;
        assert!(!step(&mut avatar, false));
        assert_eq!(avatar.mantle_state.flags & MANTLE_HATCH_READY, 0);
    }

    #[test]
    fn jump_let_go_jump_again_climbs() {
        // Up in a jump, jump still held from the floor: found, no climb.
        let mut avatar = at(70.0);
        assert!(step(&mut avatar, true));
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
        assert_eq!(avatar.mantle_state.flags & MANTLE_HATCH_READY, 0);
        // Let go: ready.
        assert!(step(&mut avatar, false));
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
        assert_ne!(avatar.mantle_state.flags & MANTLE_HATCH_READY, 0);
        // Press again: the 57 climb to 44 ahead, 87 above where the column
        // met the hatch (its top at 144: 74 up).
        assert!(step(&mut avatar, true));
        assert_ne!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
        assert_eq!(avatar.mantle_state.trans_index, 0);
        assert_eq!(avatar.mantle_state.flags & 0xffff, MANTLE_HATCH | MANTLE_FORCE_STAND);
        assert!((avatar.origin[0] - HATCH_FWD).abs() < 1e-3);
        assert!((avatar.origin[2] - (74.0 + 87.0)).abs() < 1e-3);
    }

    #[test]
    fn leaving_it_forgets_the_let_go() {
        let mut avatar = at(70.0);
        assert!(step(&mut avatar, false));
        assert_ne!(avatar.mantle_state.flags & MANTLE_HATCH_READY, 0);
        avatar.origin[2] = 38.0;
        assert!(!step(&mut avatar, false));
        avatar.origin[2] = 70.0;
        assert!(step(&mut avatar, true));
        assert_eq!(avatar.movement_state_bits & movement_state_bits::MANTLE, 0);
    }

    #[test]
    fn head_already_in_it_is_no_hatch() {
        assert!(!step(&mut at(90.0), false));
    }
}
