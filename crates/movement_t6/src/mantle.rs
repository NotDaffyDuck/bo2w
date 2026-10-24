//! Black Ops II's ledge climb: Mantle_Check, Mantle_FindMantleSurface,
//! Mantle_CheckLedge, Mantle_CalcEndPos, Mantle_Start and the end of
//! Mantle_Move. The climb's timing, the root track and the view cap are the
//! shared ones (they already match BO2 with the t6 animation tree).
//!
//! Where BO2 differs from the shared climb:
//! - The ledge's forward and drop sweeps ignore player bodies and 0x8000
//!   (`tracemask & 0xfdff_7fff`); the fit at the top keeps the full mask.
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
//! Not here: the hatch climb (Hatch_Check), mounting a gun on a ledge or
//! while prone (Mount_CheckLedge, Mount_CheckProne), and the animation event
//! for a vault.

use crate::bridge::mantle::{
    self, CONTENTS_MANTLE, MANTLE_CLEARANCE_MAXS_Z, MANTLE_FORWARD_DIST, MANTLE_HALF_WIDTH,
    MANTLE_LEDGE_FLOOR_Z, MANTLE_LEDGE_HEIGHTS, MANTLE_OVER_FORWARD, MantleCapsuleTrace,
    MantleResults, PERK_FASTMANTLE, SURF_MANTLE_ON_OR_OVER, SURF_MANTLE_OVER,
};
use crate::bridge::{
    MantleMoveContext, MantleRootDelta, MantleXAnimLength, STAND_MAXS_Z, add_predictable_event,
};
use playerstate_t6::game::mantle_flags::{ACTIVE, FAST_MANTLE, OVER};
use playerstate_t6::game::{PM_TYPE_DEAD, PlayerState, pm_flags};

use trace_t6::{TraceHitType, trace_get_entity_hit_id};

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
const LEDGE_SWEEP_MAXS_Z: f32 = MANTLE_HALF_WIDTH * 2.0;

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
const RAD2DEG: f32 = 57.295_776;

const NORMAL_MIN_LEN: f32 = 0.0001;

/// EV_MANTLE, and EF_MANTLE in our entity flag numbers.
const EV_MANTLE: i32 = entity_t6::event::MANTLE as i32;
const EF_MANTLE: u32 = 0x1_0000;

/// Crouching (4) and prone (8): only a standing player climbs.
const EF_CROUCHING: u32 = 4;
const EF_PRONE: u32 = 8;

const BUTTON_JUMP: u32 = 0x400;

pub(crate) struct CheckContext {
    pub buttons: u32,
    pub facing_xy: [f32; 2],
    pub tracemask: u32,
    /// MP (mantle_adjustment_tu on) or zombies.
    pub mp: bool,
}

/// Mantle_Check: looks for a ledge in front, shows the hint, and starts the
/// climb when jump is held. True when a climb started.
pub(crate) fn check(
    ps: &mut PlayerState,
    context: &CheckContext,
    tracer: &mut impl MantleCapsuleTrace,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) -> bool {
    if ps.pm_type >= PM_TYPE_DEAD
        || ps.pm_flags & pm_flags::MANTLE != 0
        || ps.e_flags & (EF_CROUCHING | EF_PRONE) != 0
    {
        return false;
    }
    let range = if context.mp {
        CHECK_RANGE_MP
    } else {
        CHECK_RANGE_ZOMBIES
    };
    let Some((dir, surface_flags)) = find_surface(ps.origin, context.facing_xy, range, tracer) else {
        return false;
    };
    let mut results = MantleResults {
        dir,
        start_pos: ps.origin,
        flags: surface_bits(surface_flags),
        ..MantleResults::default()
    };
    for height in MANTLE_LEDGE_HEIGHTS {
        if check_ledge(ps, &mut results, height, context, tracer, lengths, root) {
            break;
        }
    }
    ps.pm_flags & pm_flags::MANTLE != 0
}

/// Over wins; otherwise onto.
fn surface_bits(surface_flags: u32) -> i32 {
    if surface_flags & SURF_MANTLE_OVER != 0 {
        OVER
    } else if surface_flags & SURF_MANTLE_ON_OR_OVER != 0 {
        MANTLE_ON
    } else {
        0
    }
}

/// Mantle_FindMantleSurface: a thin box sweeps forward at the feet for a
/// climbable surface the player faces within 60 degrees. Returns the
/// direction into it and its surface flags.
fn find_surface(
    origin: [f32; 3],
    facing_xy: [f32; 2],
    range: f32,
    tracer: &mut impl MantleCapsuleTrace,
) -> Option<([f32; 3], u32)> {
    let inner = MANTLE_HALF_WIDTH - CHECK_RADIUS;
    let [fx, fy] = normalize_xy(facing_xy).0;
    let back = -inner;
    let ahead = range + inner;
    let start = [origin[0] + fx * back, origin[1] + fy * back, origin[2]];
    let end = [origin[0] + fx * ahead, origin[1] + fy * ahead, origin[2]];
    let mins = [-CHECK_RADIUS, -CHECK_RADIUS, 0.0];
    let maxs = [CHECK_RADIUS, CHECK_RADIUS, SURFACE_MAXS_Z];
    let hit = tracer.trace(start, end, mins, maxs, CONTENTS_MANTLE);
    if hit.startsolid || hit.allsolid || hit.fraction == 1.0 {
        return None;
    }
    if hit.sflags & SURF_MANTLE_ON_OR_OVER == 0 {
        return None;
    }
    let ([dx, dy], len) = normalize_xy([-hit.normal[0], -hit.normal[1]]);
    if len < NORMAL_MIN_LEN {
        return None;
    }
    // BO2 takes the arccosine without clamping, so a dot a hair past 1 fails.
    let dot = fx * dx + fy * dy;
    let angle = (libm::acos(f64::from(dot)) * f64::from(RAD2DEG)) as f32;
    (angle <= CHECK_ANGLE).then_some(([dx, dy, 0.0], hit.sflags))
}

/// Vec2Normalize as BO2 does it: a zero vector stays zero; the length comes
/// back too.
fn normalize_xy(v: [f32; 2]) -> ([f32; 2], f32) {
    let mut len = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    let found = len;
    if -len >= 0.0 {
        len = 1.0;
    }
    let inv = 1.0 / len;
    ([v[0] * inv, v[1] * inv], found)
}

/// Mantle_CheckLedge at one height: the box sweeps forward over the ledge,
/// drops onto it, and must fit there. A ledge that is climbable shows the
/// hint; with jump held the climb starts. True when the ledge counts (no
/// lower height is tried then).
fn check_ledge(
    ps: &mut PlayerState,
    results: &mut MantleResults,
    height: f32,
    context: &CheckContext,
    tracer: &mut impl MantleCapsuleTrace,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) -> bool {
    let mins = [-MANTLE_HALF_WIDTH, -MANTLE_HALF_WIDTH, 0.0];
    let sweep_maxs = [MANTLE_HALF_WIDTH, MANTLE_HALF_WIDTH, LEDGE_SWEEP_MAXS_Z];
    let ledge_mask = context.tracemask & LEDGE_MASK;
    let from = results.start_pos;
    let dir = results.dir;
    let start = [from[0], from[1], from[2] + height];
    let end = [
        dir[0] * MANTLE_FORWARD_DIST + start[0],
        dir[1] * MANTLE_FORWARD_DIST + start[1],
        dir[2] * MANTLE_FORWARD_DIST + start[2],
    ];
    let forward = tracer.trace(start, end, mins, sweep_maxs, ledge_mask);
    if forward.startsolid || forward.fraction < 1.0 {
        return false;
    }
    let drop_end = [end[0], end[1], from[2] + MANTLE_LEDGE_FLOOR_Z];
    let down = tracer.trace(end, drop_end, mins, sweep_maxs, ledge_mask);
    if down.startsolid || down.fraction == 1.0 || !down.walkable {
        return false;
    }
    // What the drop landed on: a moving thing carries the climb.
    let mover = if down.hit_type == TraceHitType::Entity {
        i32::from(trace_get_entity_hit_id(&down))
    } else {
        WORLD
    };
    let ledge_z = (drop_end[2] - end[2]) * down.fraction + end[2];
    results.ledge_pos = [drop_end[0], drop_end[1], ledge_z];
    if ledge_z - from[2] <= 0.0 {
        return false;
    }
    let fit_maxs = [MANTLE_HALF_WIDTH, MANTLE_HALF_WIDTH, MANTLE_CLEARANCE_MAXS_Z];
    let ledge = results.ledge_pos;
    if tracer.trace(ledge, ledge, mins, fit_maxs, context.tracemask).startsolid {
        return false;
    }
    // A window just past the ledge: the forward sweep again, reaching further.
    let reach = MANTLE_FORWARD_DIST + GLASS_EXTRA_RANGE;
    let glass_end = [
        dir[0] * reach + start[0],
        dir[1] * reach + start[1],
        dir[2] * reach + start[2],
    ];
    let glass = tracer.trace(start, glass_end, mins, sweep_maxs, ledge_mask);
    if glass.startsolid || (glass.fraction < 1.0 && glass.hit_type == TraceHitType::Glass) {
        return false;
    }
    if results.flags & (OVER | MANTLE_ON) == 0 {
        return false;
    }
    ps.mantle_state.flags |= ACTIVE;
    results.flags |= ACTIVE;
    calc_end_pos(results, context.tracemask, tracer);
    if context.buttons & BUTTON_JUMP != 0 {
        if ps.e_flags & EF_CROUCHING == 0 {
            start_clearance(results, context.tracemask, tracer);
        }
        start_climb(ps, results, mover, lengths, root);
    }
    true
}

/// Mantle_CalcEndPos: a vault carries on 31 units past the ledge and drops
/// 18. Without room for that it becomes a climb onto the ledge, and the
/// surface's over and onto bits both go.
fn calc_end_pos(results: &mut MantleResults, tracemask: u32, tracer: &mut impl MantleCapsuleTrace) {
    if results.flags & OVER != 0 {
        let mins = [-MANTLE_HALF_WIDTH, -MANTLE_HALF_WIDTH, 0.0];
        let maxs = [MANTLE_HALF_WIDTH, MANTLE_HALF_WIDTH, MANTLE_CLEARANCE_MAXS_Z];
        let start = results.ledge_pos;
        let end = [
            results.dir[0] * MANTLE_OVER_FORWARD + start[0],
            results.dir[1] * MANTLE_OVER_FORWARD + start[1],
            results.dir[2] * MANTLE_OVER_FORWARD + start[2],
        ];
        let forward = tracer.trace(start, end, mins, maxs, tracemask);
        if !forward.startsolid && forward.fraction >= 1.0 {
            let drop_end = [end[0], end[1], end[2] - MANTLE_LEDGE_FLOOR_Z];
            let down = tracer.trace(end, drop_end, mins, maxs, tracemask);
            if !down.startsolid && down.fraction >= 1.0 {
                let z = (drop_end[2] - end[2]) * down.fraction + end[2];
                results.end_pos = [drop_end[0], drop_end[1], z];
                return;
            }
        }
        results.flags &= !(OVER | MANTLE_ON);
    }
    results.end_pos = results.ledge_pos;
}

/// The start's stance checks, with the standing player's box: no room to
/// stand at the ledge crouches the climb; room at the end stands it up.
fn start_clearance(results: &mut MantleResults, tracemask: u32, tracer: &mut impl MantleCapsuleTrace) {
    let mins = [-MANTLE_HALF_WIDTH, -MANTLE_HALF_WIDTH, 0.0];
    let maxs = [MANTLE_HALF_WIDTH, MANTLE_HALF_WIDTH, STAND_MAXS_Z];
    let ledge = results.ledge_pos;
    if tracer.trace(ledge, ledge, mins, maxs, tracemask).startsolid {
        results.flags |= MANTLE_FORCE_CROUCH;
    }
    let end = results.end_pos;
    if !tracer.trace(end, end, mins, maxs, tracemask).startsolid {
        results.flags |= MANTLE_FORCE_STAND;
    }
}

/// Mantle_Start: faces the climb along the wall, picks the animation for the
/// ledge's height, and puts the player where the climb's root track ends at
/// the end position. A ledge on a moving thing (`mover`) becomes the
/// platform the climber rides.
fn start_climb(
    ps: &mut PlayerState,
    results: &MantleResults,
    mover: i32,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) {
    ps.mantle_state.yaw = bo2_vectoyaw(results.dir[0], results.dir[1]);
    ps.mantle_state.timer = 0;
    ps.mantle_state.trans_index = mantle::find_transition(results.start_pos[2], results.ledge_pos[2]);
    ps.mantle_state.flags = results.flags;
    if ps.perks[0] & PERK_FASTMANTLE != 0 {
        ps.mantle_state.flags |= FAST_MANTLE;
    }
    let duration = mantle::duration(ps, lengths);
    let delta = mantle::sample_root_track(ps, duration, lengths, root);
    ps.origin = [
        results.end_pos[0] - delta[0],
        results.end_pos[1] - delta[1],
        results.end_pos[2] - delta[2],
    ];
    ps.pm_flags |= pm_flags::MANTLE;
    if mover > 0 && mover != WORLD {
        ps.mover_entity_num = mover;
    }
    add_predictable_event(ps, EV_MANTLE, 0);
    ps.e_flags |= EF_MANTLE;
}

/// Mantle_Move: the shared climb moves the player along the root track and
/// sends the crouch and stand events. At the end, in MP, a climb onto a
/// ledge stops dead. Climbing keeps the ledge's mover from timing out.
/// True when a vault over ends this move: the legs play the jump.
pub(crate) fn advance(
    ps: &mut PlayerState,
    msec: i32,
    server_time: i32,
    mp: bool,
    lengths: &impl MantleXAnimLength,
    root: &impl MantleRootDelta,
) -> bool {
    ps.mover_timestamp = server_time;
    let context = MantleMoveContext {
        stop_on_ledge: false,
        ..MantleMoveContext::default()
    };
    mantle::advance(ps, msec, context, lengths, root);
    let ended = ps.pm_flags & pm_flags::MANTLE == 0;
    if ended && mp && ps.mantle_state.flags & MANTLE_ON != 0 {
        ps.velocity = [0.0; 3];
    }
    ended && ps.mantle_state.flags & OVER != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use crate::bridge::mantle::{CreateAnimsMantleRootDelta, FlatMantleAnimLength};
    use trace_t6::Trace;
    use playerstate_t6::game::mantle_flags::QUICK;

    const MASK: u32 = 0x0281_0011;

    /// One sweep as the climb asked for it.
    #[derive(Clone, Copy, Debug)]
    struct Sweep {
        start: [f32; 3],
        end: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
    }

    /// A ledge 40 units up, 10 units ahead along +x, with nothing above it.
    /// `solid_at` lists sweeps (by order) that start solid.
    struct Ledge {
        sweeps: Vec<Sweep>,
        surface_flags: u32,
        solid_at: Vec<usize>,
        vault_blocked: bool,
        /// The entity the ledge's top belongs to.
        top: Option<u16>,
        /// What stands a little past the ledge's edge, out of the forward
        /// sweep's reach (its hit type).
        window: Option<TraceHitType>,
    }

    impl Ledge {
        fn new(surface_flags: u32) -> Self {
            Self {
                sweeps: Vec::new(),
                surface_flags,
                solid_at: Vec::new(),
                vault_blocked: false,
                top: None,
                window: None,
            }
        }
    }

    const LEDGE_TOP: f32 = 40.0;
    const WALL_X: f32 = 10.0;

    impl MantleCapsuleTrace for Ledge {
        fn trace(&mut self, start: [f32; 3], end: [f32; 3], _mins: [f32; 3], maxs: [f32; 3], mask: u32) -> Trace {
            let index = self.sweeps.len();
            self.sweeps.push(Sweep { start, end, maxs, mask });
            let mut t = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            if self.solid_at.contains(&index) {
                t.startsolid = true;
                return t;
            }
            if mask == CONTENTS_MANTLE {
                // The surface sweep meets the wall's face.
                t.fraction = 0.5;
                t.normal = [-1.0, 0.0, 0.0];
                t.sflags = self.surface_flags;
                return t;
            }
            let going_down = end[2] < start[2];
            let reach = libm::sqrtf((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2));
            if let Some(kind) = self.window
                && !going_down
                && start[2] > LEDGE_TOP
                && reach > MANTLE_FORWARD_DIST + 1.0
            {
                // The window check's longer sweep meets it.
                t.fraction = 0.8;
                t.hit_type = kind;
                return t;
            }
            if self.vault_blocked && start != end && start[2] == LEDGE_TOP {
                // Something on the ledge blocks the way past it.
                t.fraction = 0.5;
                return t;
            }
            if going_down && start[2] > LEDGE_TOP && end[2] < LEDGE_TOP && start[0] > WALL_X {
                // The drop lands on the ledge's top.
                t.fraction = (start[2] - LEDGE_TOP) / (start[2] - end[2]);
                t.walkable = true;
                if let Some(entity) = self.top {
                    t.hit_type = TraceHitType::Entity;
                    t.hit_id = entity;
                }
                return t;
            }
            if !going_down && start[2] < LEDGE_TOP {
                // Below the top, the forward sweep hits the wall.
                t.fraction = 0.2;
            }
            t
        }
    }

    fn standing() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.origin = [0.0, 0.0, 0.0];
        ps
    }

    fn context(jump: bool) -> CheckContext {
        CheckContext {
            buttons: if jump { BUTTON_JUMP } else { 0 },
            facing_xy: [1.0, 0.0],
            tracemask: MASK,
            mp: true,
        }
    }

    fn run(ps: &mut PlayerState, ledge: &mut Ledge, jump: bool) -> bool {
        check(
            ps,
            &context(jump),
            ledge,
            &FlatMantleAnimLength::default(),
            &CreateAnimsMantleRootDelta,
        )
    }

    #[test]
    fn the_ledge_sweeps_ignore_bodies_and_the_fit_does_not() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(!run(&mut ps, &mut ledge, false));
        // The surface sweep, then from 60 up: forward, drop onto the top, fit,
        // and the window check.
        let masks: Vec<u32> = ledge.sweeps.iter().map(|s| s.mask).collect();
        assert_eq!(
            masks,
            [CONTENTS_MANTLE, MASK & LEDGE_MASK, MASK & LEDGE_MASK, MASK, MASK & LEDGE_MASK],
            "{:?}",
            ledge.sweeps
        );
        assert_eq!(MASK & LEDGE_MASK & 0x0200_0000, 0);
        // The hint shows without jump; no climb.
        assert_ne!(ps.mantle_state.flags & ACTIVE, 0);
        assert_eq!(ps.pm_flags & pm_flags::MANTLE, 0);
    }

    #[test]
    fn a_window_just_past_the_ledge_refuses_the_climb() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.window = Some(TraceHitType::Glass);
        assert!(!run(&mut ps, &mut ledge, true));
        assert_eq!(ps.mantle_state.flags & ACTIVE, 0);
        // The window check: the forward sweep's start and box, 26 units on.
        let forward = ledge.sweeps[1];
        let glass = ledge.sweeps[4];
        assert_eq!(glass.start, forward.start);
        assert_eq!(glass.maxs, forward.maxs);
        assert_eq!(glass.mask, MASK & LEDGE_MASK);
        assert!((glass.end[0] - glass.start[0] - 26.0).abs() < 1e-4, "{glass:?}");
        // Anything else there is no window: the climb goes ahead.
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.window = Some(TraceHitType::Entity);
        assert!(run(&mut ps, &mut ledge, true));
    }

    #[test]
    fn an_onto_ledge_climbs_and_stops_dead_at_the_top_in_mp() {
        let mut ps = standing();
        ps.velocity = [200.0, 0.0, 0.0];
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(run(&mut ps, &mut ledge, true));
        assert_ne!(ps.mantle_state.flags & MANTLE_ON, 0);
        assert_eq!(ps.mantle_state.flags & OVER, 0);
        let lengths = FlatMantleAnimLength::default();
        for _ in 0..20 {
            if ps.pm_flags & pm_flags::MANTLE == 0 {
                break;
            }
            advance(&mut ps, 50, 1000, true, &lengths, &CreateAnimsMantleRootDelta);
        }
        assert_eq!(ps.pm_flags & pm_flags::MANTLE, 0);
        assert_eq!(ps.velocity, [0.0; 3]);
        assert_ne!(ps.mantle_state.flags & QUICK, 0);
        assert!((ps.origin[2] - LEDGE_TOP).abs() < 1e-3, "{:?}", ps.origin);
    }

    #[test]
    fn a_vault_with_no_room_past_the_ledge_keeps_its_speed() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0400_0000);
        ledge.vault_blocked = true;
        assert!(run(&mut ps, &mut ledge, true));
        // The vault became a climb onto the ledge, and both bits went.
        assert_eq!(ps.mantle_state.flags & (OVER | MANTLE_ON), 0);
        let lengths = FlatMantleAnimLength::default();
        while ps.pm_flags & pm_flags::MANTLE != 0 {
            advance(&mut ps, 50, 1000, true, &lengths, &CreateAnimsMantleRootDelta);
        }
        assert_ne!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn a_vault_goes_over_and_keeps_its_speed() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0400_0000);
        assert!(run(&mut ps, &mut ledge, true));
        assert_ne!(ps.mantle_state.flags & OVER, 0);
        let lengths = FlatMantleAnimLength::default();
        while ps.pm_flags & pm_flags::MANTLE != 0 {
            advance(&mut ps, 50, 1000, true, &lengths, &CreateAnimsMantleRootDelta);
        }
        assert_ne!(ps.velocity, [0.0; 3]);
        assert!((ps.origin[2] - (LEDGE_TOP - MANTLE_LEDGE_FLOOR_Z)).abs() < 1e-3, "{:?}", ps.origin);
    }

    #[test]
    fn zombies_keep_the_speed_onto_a_ledge() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        assert!(run(&mut ps, &mut ledge, true));
        let lengths = FlatMantleAnimLength::default();
        while ps.pm_flags & pm_flags::MANTLE != 0 {
            advance(&mut ps, 50, 1000, false, &lengths, &CreateAnimsMantleRootDelta);
        }
        assert_ne!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn the_stance_checks_use_the_standing_box() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        // Sweeps: surface, forward, drop, fit, window, then the two stance
        // checks (5 and 6). No room to stand at the ledge.
        ledge.solid_at = vec![5];
        assert!(run(&mut ps, &mut ledge, true));
        let checks = &ledge.sweeps[5..];
        assert_eq!(checks.len(), 2, "{:?}", ledge.sweeps);
        for sweep in checks {
            assert_eq!(sweep.maxs, [MANTLE_HALF_WIDTH, MANTLE_HALF_WIDTH, STAND_MAXS_Z]);
            assert_eq!(sweep.mask, MASK);
            assert_eq!(sweep.start, sweep.end);
        }
        assert_ne!(ps.mantle_state.flags & MANTLE_FORCE_CROUCH, 0);
        assert_ne!(ps.mantle_state.flags & MANTLE_FORCE_STAND, 0);
    }

    #[test]
    fn crouched_or_prone_players_never_climb() {
        for e_flags in [EF_CROUCHING, EF_PRONE] {
            let mut ps = standing();
            ps.e_flags = e_flags;
            let mut ledge = Ledge::new(0x0200_0000);
            assert!(!run(&mut ps, &mut ledge, true));
            assert!(ledge.sweeps.is_empty());
        }
    }

    #[test]
    fn the_climb_faces_the_wall_from_0_to_360() {
        let mut ps = standing();
        let mut results = MantleResults {
            dir: [0.0, -1.0, 0.0],
            ledge_pos: [0.0, -26.0, 40.0],
            end_pos: [0.0, -26.0, 40.0],
            flags: MANTLE_ON,
            ..MantleResults::default()
        };
        start_climb(&mut ps, &results, WORLD, &FlatMantleAnimLength::default(), &CreateAnimsMantleRootDelta);
        assert!((ps.mantle_state.yaw - 270.0).abs() < 1e-3, "{}", ps.mantle_state.yaw);
        assert_ne!(ps.e_flags & EF_MANTLE, 0);
        // The fast-climb perk picks the fast tree.
        let mut fast = standing();
        fast.perks[0] = PERK_FASTMANTLE;
        results.dir = [1.0, 0.0, 0.0];
        start_climb(&mut fast, &results, WORLD, &FlatMantleAnimLength::default(), &CreateAnimsMantleRootDelta);
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
        assert!((sweep.start[0] + 14.9 * libm::cosf(59f32.to_radians())).abs() < 1e-4);
        assert_eq!(sweep.maxs, [CHECK_RADIUS, CHECK_RADIUS, SURFACE_MAXS_Z]);
        // A surface that is neither over nor onto is not climbed.
        let mut plain = Ledge::new(0x0080_0000);
        assert!(find_surface([0.0; 3], [1.0, 0.0], CHECK_RANGE_MP, &mut plain).is_none());
    }

    #[test]
    fn a_ledge_on_a_moving_thing_is_ridden() {
        let mut ps = standing();
        let mut ledge = Ledge::new(0x0200_0000);
        ledge.top = Some(40);
        assert!(run(&mut ps, &mut ledge, true));
        assert_eq!(ps.mover_entity_num, 40);
        let lengths = FlatMantleAnimLength::default();
        advance(&mut ps, 50, 1234, true, &lengths, &CreateAnimsMantleRootDelta);
        assert_eq!(ps.mover_timestamp, 1234);
    }

    #[test]
    fn a_ledge_on_the_world_or_nothing_tagged_rides_nothing() {
        for top in [Some(trace_t6::ENTITYNUM_WORLD), None] {
            let mut ps = standing();
            let mut ledge = Ledge::new(0x0200_0000);
            ledge.top = top;
            assert!(run(&mut ps, &mut ledge, true));
            assert_eq!(ps.mover_entity_num, playerstate_t6::game::ENTITYNUM_NONE, "{top:?}");
        }
    }

    #[test]
    fn surface_bits_prefer_over() {
        assert_eq!(surface_bits(0x0600_0000), OVER);
        assert_eq!(surface_bits(0x0200_0000), MANTLE_ON);
        assert_eq!(surface_bits(0), 0);
    }
}
