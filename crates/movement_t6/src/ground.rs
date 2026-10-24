//! Black Ops II's PM_GroundTrace with its two helpers: PM_CorrectAllSolid
//! (a player stuck inside something is moved to the first free spot around
//! him) and PM_GroundTraceMissed (nothing under the feet: is the ground
//! still close?).
//!
//! Where BO2 differs from the shared (MW2) ground check:
//! - Stuck inside something, the free spot's drop to the floor looks 1.25
//!   units down (MW2 0.25), and a spot is taken even if that drop starts
//!   inside something.
//! - With no free spot at all, BO2 keeps the player on whatever he is stuck
//!   in (MW2 drops him into the air).
//! - Another player is never ground to stand on (G_IsEntWalkable); it is the
//!   entity that says no, not the body's contents.
//!
//! Landing (PM_CrashLand, with BO2's water rules) is crate::crash.
//!
//! The legs' animations it asks for come back as `BodyAnims`: leaving the
//! ground with nothing under the feet for a 64th of the 64-unit look plays
//! the jump (not in a dive), and a landing from over 12 units the landing.
//!
//! Slide ground (a surface with the slick flag) sets `PMF_SLIDING` while the
//! player's ground look sees it, and keeps the time he first stood on it in
//! `slide_time`. Stepping on asks for crouch, stepping off (the air counts)
//! for stand, both as predicted stance events. The server's move (not a
//! client's prediction) follows each with a slide start or stop event whose
//! parm is the ground's slide sound class.
//!
//! Not done here yet: the ground's surface type for the footstep sounds (no
//! field for it), and the deployed riot shield and the dogs as ground that
//! is not walkable.

use crate::bridge::{
    Bo2Feel, BG_CORRECT_SOLID_DELTAS, GroundTraceInput, MoveBounds, Pml, add_predictable_event, jump,
};
use playerstate_t6::game::{ENTITYNUM_NONE, PLAYER_CORPSE_ENTITY_BASE, PlayerState};
use trace_t6::Trace;

use crate::crash::crash_land;
use crate::dive;
use crate::footstep::ground_surface_type;
use crate::feel::EF_TURRET;
use crate::frame::BodyAnims;
use crate::gjk::Bo2Collision;

/// How far above and below the feet the ground check looks; on a turret
/// from the feet to 1 unit down.
const PROBE: f32 = 0.25;
const TURRET_PROBE: f32 = 1.0;

/// Black Ops II's pm_flags 0x400000: the player is on slide ground. Under
/// the shared (MW2) movement the same bit is the last-stand flag; Black Ops
/// II keeps last stand in the move type alone.
pub const PMF_SLIDING: u32 = 0x40_0000;

/// The surface flag that makes ground slide ground.
const SURF_SLICK: u32 = 0x2;

/// The predicted stance events slide ground asks for.
const EV_STANCE_FORCE_STAND: i32 = entity_t6::event::STANCE_FORCE_STAND as i32;
const EV_STANCE_FORCE_CROUCH: i32 = entity_t6::event::STANCE_FORCE_CROUCH as i32;

/// Black Ops II's slide start and stop events, sent by the server's move only.
const EV_SLIDE_START: i32 = entity_t6::event::SLIDE_START as i32;
const EV_SLIDE_STOP: i32 = entity_t6::event::SLIDE_STOP as i32;

/// Dtp_CalcSurfaceType's table: each surface type's slide sound class
/// (0 default, 1 metal, 2 concrete, 3 wood, 4 snow, 5 dirt, 6 gravel,
/// 7 grass, 8 mud).
const SLIDE_SOUND_CLASS: [i32; 32] = [
    0, 3, 2, 4, 4, 2, 5, 2, 7, 5, 7, 6, 4, 1, 8, 2, 2, 2, 5, 4, 8, 3, 2, 2, 2, 2, 5, 5, 1, 5, 7, 0,
];

/// Stuck in the floor: try again from just below the feet.
const STARTSOLID_RETRY: f32 = 0.001;

/// PM_CorrectAllSolid: from the free spot, the drop to the floor.
const CORRECT_SOLID_DROP: f32 = 1.25;

/// PM_GroundTraceMissed: in the air a 1-unit look down, on leaving the
/// ground a 64-unit one, and the gap that still counts as almost on the
/// ground.
const MISSED_AIR_PROBE: f32 = 1.0;
const MISSED_LEFT_PROBE: f32 = 64.0;
const MISSED_ALMOST_FRACTION: f32 = 1.0 / 64.0;

/// PM_GroundTrace, and the legs' animations it starts.
pub(crate) fn ground_trace<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    bounds: MoveBounds,
    collision: &C,
    feel: Bo2Feel,
    server_time: i32,
    server: bool,
) -> BodyAnims {
    let origin = ps.origin;
    let (above, below) = if ps.e_flags & EF_TURRET != 0 {
        (0.0, TURRET_PROBE)
    } else {
        (PROBE, PROBE)
    };
    let end = [origin[0], origin[1], origin[2] - below];
    // Where the kept trace ran from and to (its end is a lerp along it).
    let mut span = [[origin[0], origin[1], origin[2] + above], end];
    let mut trace = ground_probe(collision, span[0], end, bounds);
    copy_ground_trace(pml, &trace);
    slide_ground(ps, trace.sflags, server_time, server);

    if trace.allsolid && !correct_all_solid(ps, pml, bounds, collision, &mut trace, &mut span) {
        return BodyAnims::default();
    }

    let mut walkable = trace.walkable && ent_walkable(entity_hit(&trace));
    if trace.startsolid {
        let start = [ps.origin[0], ps.origin[1], ps.origin[2] - STARTSOLID_RETRY];
        trace = ground_probe(collision, start, end, bounds);
        span = [start, end];
        if trace.startsolid {
            // Still stuck: he stays on what he is stuck in.
            ps.ground_entity_num = entity_hit(&trace);
            pml.ground_plane = 1;
            pml.almost_ground_plane = 1;
            pml.walking = u32::from(trace.walkable);
            return BodyAnims::default();
        }
        copy_ground_trace(pml, &trace);
        walkable = trace.walkable;
    }

    if trace.fraction == 1.0 {
        let jump = ground_trace_missed(ps, pml, bounds, collision);
        return BodyAnims { jump, ..BodyAnims::default() };
    }

    // BO2's kick-off from a slope only runs in its old collision mode; its
    // players use mode 1, so there is none.

    pml.ground_plane = 1;
    pml.almost_ground_plane = 1;
    if !walkable {
        ps.ground_entity_num = ENTITYNUM_NONE;
        pml.walking = 0;
        jump::clear_state(ps);
        return BodyAnims::default();
    }
    pml.walking = 1;
    let land = ps.ground_entity_num == ENTITYNUM_NONE && crash_land(ps, pml, feel);
    let entity = entity_hit(&trace);
    ps.ground_entity_num = entity;
    collision.touch_entity(&trace, trace.end_pos(span[0], span[1]));
    BodyAnims { land, ..BodyAnims::default() }
}

/// PM_CorrectAllSolid: the player's box is inside something. Try the 26
/// spots one unit around him; at the first free one, drop to the floor
/// (1.25 down) and keep that as the ground. With none free he stays put on
/// what he is stuck in, off the ground plane, and any jump is over.
fn correct_all_solid<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    bounds: MoveBounds,
    collision: &C,
    trace: &mut Trace,
    span: &mut [[f32; 3]; 2],
) -> bool {
    for delta in BG_CORRECT_SOLID_DELTAS {
        let point = [
            ps.origin[0] + delta[0],
            ps.origin[1] + delta[1],
            ps.origin[2] + delta[2],
        ];
        *trace = player_trace(collision, point, point, bounds);
        if !trace.startsolid {
            let end = [point[0], point[1], point[2] - CORRECT_SOLID_DROP];
            *trace = player_trace(collision, point, end, bounds);
            *span = [point, end];
            copy_ground_trace(pml, trace);
            ps.origin = [point[0], point[1], point[2] + (end[2] - point[2]) * trace.fraction];
            return true;
        }
    }
    ps.ground_entity_num = entity_hit(trace);
    pml.walking = 0;
    pml.ground_plane = 0;
    pml.almost_ground_plane = 0;
    jump::clear_state(ps);
    false
}

/// PM_GroundTraceMissed. True when the player just left the ground with
/// the ground not almost under him: the legs play the jump (not in a dive).
fn ground_trace_missed<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    bounds: MoveBounds,
    collision: &C,
) -> bool {
    let origin = ps.origin;
    let airborne = ps.ground_entity_num == ENTITYNUM_NONE;
    let depth = if airborne { MISSED_AIR_PROBE } else { MISSED_LEFT_PROBE };
    let trace = player_trace(collision, origin, [origin[0], origin[1], origin[2] - depth], bounds);
    let almost = if airborne {
        trace.fraction != 1.0
    } else {
        trace.fraction < MISSED_ALMOST_FRACTION
    };
    pml.almost_ground_plane = u32::from(almost);
    ps.ground_entity_num = ENTITYNUM_NONE;
    pml.ground_plane = 0;
    pml.walking = 0;
    !airborne && !almost && ps.pm_flags & dive::PMF_DIVE == 0
}

/// G_IsEntWalkable: players are not ground.
fn ent_walkable(entity: i32) -> bool {
    !is_player(entity)
}

/// Entity numbers below 18 are the players.
pub(crate) fn is_player(entity: i32) -> bool {
    entity != ENTITYNUM_NONE && (0..PLAYER_CORPSE_ENTITY_BASE).contains(&entity)
}

/// PM_playerTrace: with BO2's collision on, a sweep of the body.
fn player_trace<C: Bo2Collision>(collision: &C, start: [f32; 3], end: [f32; 3], bounds: MoveBounds) -> Trace {
    if collision.gjk_on() {
        return crate::gjk::trace::player_trace(collision, start, end, bounds.mins, bounds.maxs, bounds.tracemask);
    }
    collision.trace(GroundTraceInput {
        start,
        end,
        mins: bounds.mins,
        maxs: bounds.maxs,
        tracemask: bounds.tracemask,
    })
}

/// The ground check's own look under the feet: with BO2's collision on,
/// its ground trace (PM_gjk_ground_trace); off, the player trace.
fn ground_probe<C: Bo2Collision>(collision: &C, start: [f32; 3], end: [f32; 3], bounds: MoveBounds) -> Trace {
    if collision.gjk_on() {
        return crate::gjk::trace::ground_trace(collision, start, end, bounds.mins, bounds.maxs, bounds.tracemask);
    }
    player_trace(collision, start, end, bounds)
}

fn entity_hit(trace: &Trace) -> i32 {
    i32::from(trace_t6::trace_get_entity_hit_id(&trace))
}

/// The ground trace kept for the frame, in the shared layout the other
/// moves read.
/// On slide ground the flag is set and, on the first frame, the time and a
/// crouch; off it (or in the air) the flag is cleared and, on the first
/// frame, the time and a stand. The server also sends the slide start or
/// stop, with the ground's slide sound class.
fn slide_ground(ps: &mut PlayerState, surface_flags: u32, server_time: i32, server: bool) {
    if surface_flags & SURF_SLICK != 0 {
        ps.pm_flags |= PMF_SLIDING;
        if ps.slide_time == 0 {
            ps.slide_time = server_time;
            add_predictable_event(ps, EV_STANCE_FORCE_CROUCH, 0);
            if server {
                let class = slide_sound_class(ps, surface_flags);
                add_predictable_event(ps, EV_SLIDE_START, class);
            }
        }
    } else {
        ps.pm_flags &= !PMF_SLIDING;
        if ps.slide_time != 0 {
            ps.slide_time = 0;
            add_predictable_event(ps, EV_STANCE_FORCE_STAND, 0);
            if server {
                let class = slide_sound_class(ps, surface_flags);
                add_predictable_event(ps, EV_SLIDE_STOP, class);
            }
        }
    }
}

/// Dtp_CalcSurfaceType: the ground's surface type as a slide sound class.
fn slide_sound_class(ps: &PlayerState, surface_flags: u32) -> i32 {
    usize::try_from(ground_surface_type(ps, surface_flags))
        .ok()
        .and_then(|surface| SLIDE_SOUND_CLASS.get(surface))
        .copied()
        .unwrap_or(0)
}

fn copy_ground_trace(pml: &mut Pml, trace: &Trace) {
    pml.ground_trace = [
        trace.fraction.to_bits(),
        trace.normal[0].to_bits(),
        trace.normal[1].to_bits(),
        trace.normal[2].to_bits(),
        trace.sflags,
        trace.cflags,
        0,
        trace.hit_type as u32,
        u32::from(trace.hit_id) | (u32::from(trace.model_index) << 16),
        u32::from(trace.part_name) | (u32::from(trace.part_group) << 16),
        u32::from(trace.allsolid) | (u32::from(trace.startsolid) << 8) | (u32::from(trace.walkable) << 16),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::CollisionBackend;
    use playerstate_t6::game::pm_flags;
    use trace_t6::TraceHitType;

    const WORLD: u16 = trace_t6::ENTITYNUM_WORLD;

    /// A floor at z = 0 (solid below), with optional solid regions that trap
    /// a box whose start lies inside them.
    struct Floor {
        /// A player body's entity number standing under the feet, if any.
        body: Option<u16>,
        /// Points inside this box (by the start's origin) are stuck.
        stuck: Option<([f32; 3], [f32; 3])>,
    }

    impl Floor {
        fn inside(&self, p: [f32; 3]) -> bool {
            self.stuck
                .is_some_and(|(lo, hi)| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]))
        }
    }

    impl crate::gjk::Bo2Collision for Floor {}

    impl CollisionBackend for Floor {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let mut t = Trace {
                fraction: 1.0,
                hit_type: TraceHitType::Entity,
                hit_id: self.body.unwrap_or(WORLD),
                ..Trace::default()
            };
            if self.inside(input.start) {
                t.startsolid = true;
                t.allsolid = self.inside(input.end);
                t.fraction = 0.0;
                return t;
            }
            // The floor, or the top of the stuck block under the feet.
            let top = match self.stuck {
                Some((lo, hi)) if (0..2).all(|i| input.start[i] >= lo[i] && input.start[i] <= hi[i]) => hi[2].max(0.0),
                _ => 0.0,
            };
            if input.end[2] < top && input.start[2] >= top {
                t.fraction = (input.start[2] - top) / (input.start[2] - input.end[2]);
                t.normal = [0.0, 0.0, 1.0];
                t.walkable = true;
            }
            t
        }
    }

    fn bounds() -> MoveBounds {
        MoveBounds {
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 70.0],
            tracemask: 1,
        }
    }

    fn run(floor: &Floor, origin: [f32; 3], ground: i32) -> (PlayerState, Pml) {
        let mut ps = PlayerState::ZERO;
        ps.origin = origin;
        ps.ground_entity_num = ground;
        let mut pml = crate::testing::pml(false);
        ground_trace(&mut ps, &mut pml, bounds(), floor, Bo2Feel::default(), 0, false);
        (ps, pml)
    }

    #[test]
    fn standing_on_the_floor_walks() {
        let floor = Floor { body: None, stuck: None };
        let (ps, pml) = run(&floor, [0.0, 0.0, 0.0], i32::from(WORLD));
        assert_eq!(ps.ground_entity_num, i32::from(WORLD));
        assert_eq!((pml.walking, pml.ground_plane, pml.almost_ground_plane), (1, 1, 1));
    }

    #[test]
    fn another_player_is_not_ground() {
        let floor = Floor { body: Some(3), stuck: None };
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.ground_entity_num = 3;
        let mut pml = crate::testing::pml(false);
        ground_trace(&mut ps, &mut pml, bounds(), &floor, Bo2Feel::default(), 0, false);
        assert_eq!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert_eq!((pml.walking, pml.ground_plane), (0, 1));
        assert_eq!(ps.pm_flags & pm_flags::JUMPING, 0, "the jump is over");
        // A corpse or a thing (18 and up) is.
        let floor = Floor { body: Some(18), stuck: None };
        let (ps, pml) = run(&floor, [0.0, 0.0, 0.0], 18);
        assert_eq!(ps.ground_entity_num, 18);
        assert_eq!(pml.walking, 1);
    }

    #[test]
    fn stuck_moves_to_the_first_free_spot_and_drops_to_the_floor() {
        // Stuck in a block whose top is just over the feet; one unit up is
        // free.
        let floor = Floor {
            body: None,
            stuck: Some(([-0.5, -0.5, -1.0], [0.5, 0.5, 0.5])),
        };
        let (ps, pml) = run(&floor, [0.0, 0.0, 0.2], i32::from(WORLD));
        // Up one (1.2), then the 1.25 drop lands on the block's top at 0.5.
        assert!((ps.origin[2] - 0.5).abs() < 1e-5, "{:?}", ps.origin);
        assert_eq!(pml.walking, 1);
    }

    #[test]
    fn no_free_spot_keeps_him_on_what_he_is_stuck_in() {
        let floor = Floor {
            body: None,
            stuck: Some(([-5.0; 3], [5.0; 3])),
        };
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = ENTITYNUM_NONE;
        ps.pm_flags = pm_flags::JUMPING;
        let mut pml = crate::testing::pml(true);
        ground_trace(&mut ps, &mut pml, bounds(), &floor, Bo2Feel::default(), 0, false);
        assert_eq!(ps.ground_entity_num, i32::from(WORLD));
        assert_eq!((pml.walking, pml.ground_plane, pml.almost_ground_plane), (0, 0, 0));
        assert_eq!(ps.pm_flags & pm_flags::JUMPING, 0);
        assert_eq!(ps.origin, [0.0; 3], "he is not moved");
    }

    #[test]
    fn walking_off_a_ledge_looks_64_down() {
        let floor = Floor { body: None, stuck: None };
        // 0.5 up: the 0.25 probe misses, the 64 one finds the floor at a
        // fraction under 1/64, so still almost on the ground.
        let (ps, pml) = run(&floor, [0.0, 0.0, 0.5], i32::from(WORLD));
        assert_eq!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert_eq!((pml.walking, pml.ground_plane, pml.almost_ground_plane), (0, 0, 1));
        // In the air 0.5 up, the 1-unit look finds it too.
        let (_, pml) = run(&floor, [0.0, 0.0, 0.5], ENTITYNUM_NONE);
        assert_eq!(pml.almost_ground_plane, 1);
        // 2 up in the air, it does not.
        let (_, pml) = run(&floor, [0.0, 0.0, 2.0], ENTITYNUM_NONE);
        assert_eq!(pml.almost_ground_plane, 0);
    }

    #[test]
    fn slide_ground_crouches_once_and_stands_on_leaving() {
        let mut ps = PlayerState::ZERO;
        slide_ground(&mut ps, SURF_SLICK, 5000, false);
        assert_ne!(ps.pm_flags & PMF_SLIDING, 0);
        assert_eq!(ps.slide_time, 5000);
        assert_eq!((ps.predictable_event_sequence, ps.predictable_events[0]), (1, EV_STANCE_FORCE_CROUCH));
        // Still on it: the start time and the one crouch stay.
        slide_ground(&mut ps, SURF_SLICK | 0x10, 5050, false);
        assert_eq!((ps.slide_time, ps.predictable_event_sequence), (5000, 1));
        // Off it (or in the air): cleared, and one stand.
        slide_ground(&mut ps, 0, 5100, false);
        assert_eq!(ps.pm_flags & PMF_SLIDING, 0);
        assert_eq!(ps.slide_time, 0);
        assert_eq!((ps.predictable_event_sequence, ps.predictable_events[1]), (2, EV_STANCE_FORCE_STAND));
        slide_ground(&mut ps, 0, 5150, false);
        assert_eq!(ps.predictable_event_sequence, 2);
    }

    #[test]
    fn the_server_also_sends_the_slide_start_and_stop_with_the_ground_class() {
        let mut ps = PlayerState::ZERO;
        // Surface type 8 is the grass class (7).
        slide_ground(&mut ps, SURF_SLICK | (8 << 20), 5000, true);
        assert_eq!(ps.predictable_event_sequence, 2);
        assert_eq!(
            (ps.predictable_events[0], ps.predictable_events[1], ps.predictable_event_parms[1]),
            (EV_STANCE_FORCE_CROUCH, EV_SLIDE_START, 7)
        );
        slide_ground(&mut ps, SURF_SLICK | (8 << 20), 5050, true);
        assert_eq!(ps.predictable_event_sequence, 2);
        // Off into the air: the stand, then the stop on the default class.
        slide_ground(&mut ps, 0, 5100, true);
        assert_eq!(ps.predictable_event_sequence, 4);
        assert_eq!(
            (ps.predictable_events[2], ps.predictable_events[3], ps.predictable_event_parms[3]),
            (EV_STANCE_FORCE_STAND, EV_SLIDE_STOP, 0)
        );
        // In water the class is the water surface's (mud).
        let mut ps = PlayerState::ZERO;
        ps.waterlevel = 1;
        slide_ground(&mut ps, SURF_SLICK, 5000, true);
        assert_eq!(ps.predictable_event_parms[1], 8);
    }

    fn anims(floor: &Floor, origin: [f32; 3], ground: i32, flags: u32) -> BodyAnims {
        let mut ps = PlayerState::ZERO;
        ps.origin = origin;
        ps.ground_entity_num = ground;
        ps.pm_flags = flags;
        ps.gravity = 800;
        ps.velocity = [0.0, 0.0, -300.0];
        let mut pml = crate::testing::pml(false);
        pml.previous_velocity = ps.velocity;
        ground_trace(&mut ps, &mut pml, bounds(), floor, Bo2Feel::default(), 0, false)
    }

    #[test]
    fn walking_off_a_drop_plays_the_jump() {
        let floor = Floor { body: None, stuck: None };
        let world = i32::from(WORLD);
        // Off the ground with the floor 2 units down (past a 64th of 64):
        // the legs jump.
        assert_eq!(anims(&floor, [0.0, 0.0, 2.0], world, 0), BodyAnims { jump: true, ..BodyAnims::default() });
        // Nothing under the 64-unit look: the same.
        assert!(anims(&floor, [0.0, 0.0, 100.0], world, 0).jump);
        // Half a unit down is still almost on the ground: no jump.
        assert!(!anims(&floor, [0.0, 0.0, 0.5], world, 0).jump);
        // Already in the air, or diving: none.
        assert!(!anims(&floor, [0.0, 0.0, 2.0], ENTITYNUM_NONE, 0).jump);
        assert!(!anims(&floor, [0.0, 0.0, 2.0], world, dive::PMF_DIVE).jump);
    }

    #[test]
    fn coming_down_hard_plays_the_landing() {
        let floor = Floor { body: None, stuck: None };
        // Touching down at 300 a second is a 56-unit fall.
        assert_eq!(anims(&floor, [0.0; 3], ENTITYNUM_NONE, 0), BodyAnims { land: true, ..BodyAnims::default() });
        // Already on the ground, or landing a dive: no landing.
        assert!(!anims(&floor, [0.0; 3], i32::from(WORLD), 0).land);
        assert!(!anims(&floor, [0.0; 3], ENTITYNUM_NONE, dive::PMF_DIVE).land);
    }
}
