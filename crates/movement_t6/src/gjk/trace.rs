//! Black Ops II's player traces in collision mode 1: a swept GJK test of the
//! player's body against every shape near the move (gjk_trace), the player
//! trace that reports the first hit (gjk_player_trace) and the ground trace
//! that picks the best ground under the feet (PM_gjk_ground_trace).
//!
//! What a trace reports differs from the box traces: it is never all solid,
//! it is in a solid when the body overlaps a shape at the start (its gap is
//! below zero), and only brushes and terrain report contents and surface
//! flags; players and entity boxes report none.
//!
//! Not done: BO2 keeps, per player and per shape, the last search direction
//! and simplex between traces and starts the next search from them. Every
//! trace here starts afresh; the contact found is the same to within the
//! search's 0.1% tolerance.

use alloc::vec::Vec;
use trace_t6::{Trace, TraceHitType};

use super::hull::{Mover, brush_face_walkable, is_walkable, player_hull, triangles_face_walkable};
use super::solver::{Pair, Sweep, do_gjk_collide};
use super::Bo2Collision;
use super::Geom;
use super::Shape;
use super::add;
use super::dot;
use super::sub;
use super::to_local;

/// The trace margin: the body's trace box is this much inside its real box,
/// a shape counts as touching this far out, and a hit's gap is measured from
/// this far out.
pub(crate) const AC_EPS: f32 = 0.125;
const HALF_EPS: f32 = 0.0625;

/// phys_player_collision_adjust_height (0 by default).
const ADJUST_HEIGHT: f32 = 0.0;

/// phys_player_step_on_actors (on by default): the ground trace also hits
/// other players.
const STEP_ON_ACTORS: bool = true;

/// The ground trace weighs hits down to this far below the first one.
const GROUND_BAND: f32 = 0.5;
/// A terrain hit counts this much earlier when the ground trace picks.
const TERRAIN_HEAD_START: f32 = 0.001;

/// The world is asked for shapes this far round the swept trace box; the
/// box test below drops the ones the move cannot reach.
const QUERY_PAD: f32 = 1.0;

/// One contact a trace found.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit<'a> {
    pub shape_data: Geom<'a>,
    /// The contact on the player (with the skin), from its origin at the
    /// hit.
    pub arm: [f32; 3],
    /// The contact on the shape, in the world.
    pub hit_point: [f32; 3],
    /// From the shape toward the player.
    pub facing: [f32; 3],
    /// How far along the move (0..1).
    pub hit_time: f32,
    /// The gap at the hit, plus the margin: below zero the body overlaps.
    pub hit_dist: f32,
    /// The contact is at the feet.
    pub foot: bool,
}

/// BO2's gjk_trace_input_t: the body, its trace box and the move.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TraceInput {
    /// What sweeps: the player's body or a vehicle's brush.
    pub mover: Mover,
    /// The trace box, from the player's origin.
    pub cg_mins: [f32; 3],
    pub cg_maxs: [f32; 3],
    pub from_pt: [f32; 3],
    pub translation: [f32; 3],
    pub solid_mask: u32,
    /// Keep hits the player is moving away from.
    pub keep_all: bool,
    /// Stop at the first hit that starts inside a shape.
    pub exit_on_penetration: bool,
    /// Pass through other players.
    pub skip_sentients: bool,
    /// After a hit, keep looking this much further along the move.
    pub extra_time: f32,
}

/// BO2's setup_trace_info: the body for the player's box (BO2 rebuilds it
/// only when the box changes; it is cheap to build), its trace box (the
/// body's box less the margin), and the move from `start` to `end`.
pub(crate) fn setup_trace_info(box_lo: [f32; 3], box_hi: [f32; 3], from_pt: [f32; 3], to_pt: [f32; 3], mask: u32) -> TraceInput {
    mover_trace_info(player_mover(box_lo, box_hi), from_pt, to_pt, mask)
}

/// The player's body for its box.
pub(crate) fn player_mover(box_lo: [f32; 3], box_hi: [f32; 3]) -> Mover {
    Mover::Body(player_hull(box_lo, box_hi, ADJUST_HEIGHT))
}

/// setup_trace_info for any moving shape: its trace box is its box less the
/// margin.
pub(crate) fn mover_trace_info(mover: Mover, src_pt: [f32; 3], dst_pt: [f32; 3], filter: u32) -> TraceInput {
    let (lo, hi) = mover.calc_aabb();
    TraceInput {
        mover,
        cg_mins: lo.map(|v| v + AC_EPS),
        cg_maxs: hi.map(|v| v - AC_EPS),
        from_pt: src_pt,
        translation: sub(dst_pt, src_pt),
        solid_mask: filter,
        keep_all: false,
        exit_on_penetration: true,
        skip_sentients: false,
        extra_time: 0.0,
    }
}

/// BO2's gjk_trace_aabb: whether the trace box, swept along the move, comes
/// within the margin of the shape's box, and from when (0..1).
fn trace_aabb(input: &TraceInput, shape_data: &Geom) -> Option<f32> {
    // The shape's box grown by the trace box: the origin must reach it.
    let lo = sub(shape_data.lo_corner, input.cg_maxs);
    let hi = sub(shape_data.hi_corner, input.cg_mins);
    let mut t_min = 0.0f32;
    let mut t_max = 1.0f32;
    for a in 0..3 {
        let pos = input.from_pt[a];
        let t = input.translation[a];
        // Each side: how far out the origin starts and ends (above zero:
        // outside).
        for (d, d_end) in [(pos - hi[a], pos - hi[a] + t), (lo[a] - pos, lo[a] - pos - t)] {
            if d <= 0.0 {
                if d_end > 0.0 {
                    // Leaving through this side. As BO2 has it, this only
                    // ever moves the leaving time later, never earlier.
                    let delta = d - d_end;
                    if d - HALF_EPS < delta * t_max {
                        t_max = (d - HALF_EPS) / delta;
                    }
                }
            } else {
                let reach = if d < AC_EPS { d } else { AC_EPS };
                if d_end < reach {
                    // Coming in through this side.
                    let delta = d - d_end;
                    if delta * t_min < d - AC_EPS {
                        t_min = (d - AC_EPS) / delta;
                    }
                    if t_min >= t_max {
                        return None;
                    }
                } else if d > HALF_EPS {
                    return None;
                }
            }
        }
    }
    Some(t_min)
}

/// BO2's gjk_collide: the swept test of the body against one shape, from
/// `start_time` to `finish_stamp`, finding contacts up to `sep_thresh` apart
/// (a trace: the margin). The body meets brushes as its 12-sided self and
/// everything else as the capsule; a vehicle's brush is always itself.
pub(super) fn gjk_collide<'a>(
    input: &TraceInput,
    shape_data: &Geom<'a>,
    start_time: f32,
    finish_stamp: f32,
    sep_thresh: f32,
) -> Option<Hit<'a>> {
    let mover = input.mover.against(shape_data.shape.is_brush());
    let pair = Pair { cg1: &mover, cg2: &shape_data.shape, m: shape_data.axis, w: sub(shape_data.origin, input.from_pt) };
    let sweep = Sweep { translation: input.translation, start_time, finish_stamp };
    let c = do_gjk_collide(&pair, mover.skin(), shape_data.shape.skin(), sep_thresh, Some(sweep))?;
    let closing = dot(input.translation, c.facing);
    let hit_dist = dot(c.facing, sub(c.p1, c.p2)) + c.lambda * closing + AC_EPS;
    let mut hit_time = c.lambda;
    if closing >= 0.0 && HALF_EPS < hit_dist && !input.keep_all {
        // Clear of it and not moving toward it: no hit on this move.
        hit_time = 1.0;
    }
    if !(hit_time < finish_stamp || hit_dist < 0.0) {
        return None;
    }
    Some(Hit {
        shape_data: *shape_data,
        arm: c.p1,
        hit_point: add(input.from_pt, c.p2),
        facing: c.facing,
        hit_time,
        hit_dist,
        foot: mover.is_foot(c.p1[2]),
    })
}

/// The hits of one trace, in the order they were tested, and which one
/// came first.
pub(crate) struct Hits<'a> {
    pub list: Vec<Hit<'a>>,
    pub first: Option<usize>,
}

/// BO2's gjk_trace: every shape near the move but those in `skip` (keys),
/// in the order the box test says they are reached, each tested from then
/// until the first hit so far (plus `extra_time`).
pub(crate) fn gjk_trace<'a, C: Bo2Collision>(solids: &'a C, input: &TraceInput, skip: &[u64]) -> Hits<'a> {
    let end = add(input.from_pt, input.translation);
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for a in 0..3 {
        lo[a] = input.from_pt[a].min(end[a]) + input.cg_mins[a] - QUERY_PAD;
        hi[a] = input.from_pt[a].max(end[a]) + input.cg_maxs[a] + QUERY_PAD;
    }
    let mut geoms = Vec::new();
    solids.gjk_query(lo, hi, input.solid_mask, &mut geoms);
    let mut reached: Vec<(f32, Geom<'a>)> = geoms
        .into_iter()
        .filter(|g| (!input.skip_sentients || !g.sentient) && !skip.contains(&g.key))
        .filter_map(|g| trace_aabb(input, &g).map(|t| (t, g)))
        .collect();
    reached.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(core::cmp::Ordering::Equal));

    let mut hits = Hits { list: Vec::new(), first: None };
    let mut finish_stamp = 1.0f32;
    for (start_time, shape_data) in reached {
        if finish_stamp < start_time {
            break;
        }
        let Some(strike) = gjk_collide(input, &shape_data, start_time, finish_stamp, AC_EPS) else {
            continue;
        };
        if strike.hit_time < finish_stamp {
            let t = input.extra_time + strike.hit_time;
            finish_stamp = if t < 1.0 { t } else { 1.0 };
        }
        let first = match hits.first {
            None => true,
            Some(i) => {
                let f = &hits.list[i];
                strike.hit_time < f.hit_time || (strike.hit_time == 0.0 && strike.hit_dist < f.hit_dist)
            }
        };
        hits.list.push(strike);
        if first {
            hits.first = Some(hits.list.len() - 1);
        }
        if input.exit_on_penetration && hits.first.is_some_and(|i| hits.list[i].hit_dist < 0.0) {
            break;
        }
    }
    hits
}

/// BO2's gjk_player_trace: the body from `start` to `end`, stopped at the
/// first hit.
pub(crate) fn player_trace<C: Bo2Collision>(
    solids: &C,
    from_pt: [f32; 3],
    end: [f32; 3],
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    mask: u32,
) -> Trace {
    let input = setup_trace_info(box_lo, box_hi, from_pt, end, mask);
    let hits = gjk_trace(solids, &input, &[]);
    match hits.first {
        Some(i) => {
            let strike = &hits.list[i];
            fill_results(strike, hit_walkable(strike))
        }
        None => no_hit(),
    }
}

/// BO2's PM_gjk_ground_trace: the body from `start` down to `end`, looking
/// a little past the first hit (half a unit, or the whole way when shorter)
/// for the best ground: among the hits no more than half a unit below the
/// first, ground you can stand on beats ground you cannot, then the earlier
/// one wins, terrain by a hair.
pub(crate) fn floor_probe<C: Bo2Collision>(
    solids: &C,
    from_pt: [f32; 3],
    end: [f32; 3],
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    mask: u32,
) -> Trace {
    let mut input = setup_trace_info(box_lo, box_hi, from_pt, end, mask);
    let depth = from_pt[2] - end[2];
    input.exit_on_penetration = false;
    input.extra_time = if depth >= GROUND_BAND { GROUND_BAND / depth } else { 1.0 };
    input.skip_sentients = !STEP_ON_ACTORS;
    let hits = gjk_trace(solids, &input, &[]);
    let Some(first) = hits.first else {
        return no_hit();
    };
    let z_at = |t: f32| from_pt[2] + (end[2] - from_pt[2]) * t;
    let first_z = z_at(hits.list[first].hit_time);
    let mut best = 0;
    let mut best_time = hits.list[0].hit_time;
    let mut best_walk = hit_walkable(&hits.list[0]);
    for (i, h) in hits.list.iter().enumerate().skip(1) {
        let t = if matches!(h.shape_data.shape, Shape::Partition { .. }) {
            h.hit_time - TERRAIN_HEAD_START
        } else {
            h.hit_time
        };
        if first_z - z_at(h.hit_time) > GROUND_BAND {
            continue;
        }
        if !best_walk {
            let walk = hit_walkable(h);
            if t < best_time || walk {
                (best, best_time, best_walk) = (i, t, walk);
            }
        } else if t < best_time && hit_walkable(h) {
            (best, best_time) = (i, t);
        }
    }
    fill_results(&hits.list[best], best_walk)
}

/// BO2's is_walkable for a hit: at the feet, and either facing up enough or
/// on a face of the shape that does (asked in the shape's own frame).
pub(crate) fn hit_walkable(strike: &Hit) -> bool {
    is_walkable(strike.foot, strike.facing[2], || {
        let g = &strike.shape_data;
        let point = to_local(&g.axis, sub(strike.hit_point, g.origin));
        let up = to_local(&g.axis, [0.0, 0.0, 1.0]);
        match g.shape {
            Shape::Box { center, half, .. } => {
                brush_face_walkable(sub(center, half), add(center, half), &[], point, up)
            }
            Shape::Brush { lo_corner: box_lo, hi_corner: box_hi, sides, .. } => brush_face_walkable(box_lo, box_hi, sides, point, up),
            Shape::Partition { vertex_points, indices } => triangles_face_walkable(
                indices
                    .chunks_exact(3)
                    .map(|t| [vertex_points[usize::from(t[0])], vertex_points[usize::from(t[1])], vertex_points[usize::from(t[2])]]),
                point,
                up,
            ),
            Shape::Player { .. } => false,
        }
    })
}

/// BO2's fill_results.
fn fill_results(strike: &Hit, walkable: bool) -> Trace {
    let g = &strike.shape_data;
    let reports = matches!(g.shape, Shape::Brush { .. } | Shape::Box { brush: true, .. } | Shape::Partition { .. });
    let (cflags, sflags) = if reports { (g.contents, g.surf_bits) } else { (0, 0) };
    let mut out = Trace::new();
    (out.fraction, out.normal) = (strike.hit_time, strike.facing);
    (out.sflags, out.cflags) = (sflags, cflags);
    out.impact_category = TraceHitType::Entity;
    out.struck_entity_num = g.struck_entity_num;
    out.starts_inside = strike.hit_dist < 0.0;
    out.walkable = walkable;
    out
}

/// BO2's fill_results_no_hit.
fn no_hit() -> Trace {
    let mut empty = Trace::default();
    empty.fraction = 1.0_f32;
    empty
}

/// BO2's get_material_from_brush: the surface flags a brush hit reports.
/// Its top face's, unless that face is caulk (`caulk`: the flags of the
/// material named "caulk"); then the most upward-facing other face that is
/// not, then the first box face that is not (low faces x, y, z, then high
/// faces), else caulk. `axial` is the box faces' flags, low then high;
/// `sides` each other face's normal z and flags.
pub fn brush_surface_flags(axial: [[u32; 3]; 2], sides: impl IntoIterator<Item = (f32, u32)>, caulk: u32) -> u32 {
    let top = axial[1][2];
    if top != caulk {
        return top;
    }
    let mut pick = None;
    let mut best_z = -10.0f32;
    for (facing_z, flags) in sides {
        if flags != caulk && best_z < facing_z {
            pick = Some(flags);
            best_z = facing_z;
        }
    }
    if let Some(flags) = pick {
        return flags;
    }
    axial.iter().flatten().copied().find(|&f| f != caulk).unwrap_or(caulk)
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{MASK, MAXS, MINS, World, block, floor, other_player};
    use super::*;
    use trace_t6::ENTITYNUM_WORLD;

    fn ground(world: &World, z: f32) -> Trace {
        floor_probe(world, [0.0, 0.0, z + 0.25], [0.0, 0.0, z - 0.25], MINS, MAXS, MASK)
    }

    /// The body rests with its skin on the floor, the origin 0.125 up
    /// (the feet's tip 0.375 above the origin, the skin 0.5): the ground
    /// trace's half-unit look meets it halfway.
    #[test]
    fn standing_on_a_floor() {
        let world = World::new(alloc::vec![floor()]);
        let t = ground(&world, 0.125);
        assert!((t.fraction - 0.5).abs() < 0.01, "{t:?}");
        assert_eq!(t.normal, [0.0, 0.0, 1.0]);
        assert_eq!((t.walkable, t.starts_inside, t.fully_embedded), (true, false, false));
        assert_eq!((t.impact_category, t.struck_entity_num), (TraceHitType::Entity, ENTITYNUM_WORLD));
        assert_eq!((t.cflags, t.sflags), (MASK, 7));
    }

    #[test]
    fn nothing_below_is_a_miss() {
        let world = World::new(alloc::vec![floor()]);
        let t = ground(&world, 10.0);
        assert_eq!(t.fraction, 1.0);
        assert_eq!((t.impact_category, t.walkable, t.starts_inside), (TraceHitType::None, false, false));
    }

    /// Sunk 3 into the floor: in a solid, never all solid.
    #[test]
    fn sunk_into_the_floor_starts_solid() {
        let world = World::new(alloc::vec![floor()]);
        let p = [0.0, 0.0, -3.0];
        let t = player_trace(&world, p, p, MINS, MAXS, MASK);
        assert_eq!((t.starts_inside, t.fully_embedded), (true, false), "{t:?}");
        assert_eq!(t.fraction, 0.0);
    }

    /// Walking into a wall 40 ahead: the ring's corner (15.14 out, plus the
    /// 0.5 skin) stops 24.36 along.
    #[test]
    fn a_wall_stops_the_body_at_its_corner() {
        let world = World::new(alloc::vec![block([40.0, -256.0, -64.0], [56.0, 256.0, 256.0])]);
        let t = player_trace(&world, [0.0, 0.0, 10.0], [100.0, 0.0, 10.0], MINS, MAXS, MASK);
        let reach = 14.625 / 0.965_925_8 + 0.5;
        assert!((t.fraction * 100.0 - (40.0 - reach)).abs() < 0.1, "{t:?}");
        assert!((t.normal[0] + 1.0).abs() < 1e-3, "{t:?}");
        assert_eq!((t.walkable, t.starts_inside), (false, false));
    }

    /// Another player is met as a capsule (14.625 + 0.5) against its body
    /// (15.14 + 0.5); it reports no contents and its own number.
    #[test]
    fn another_player_is_met_by_the_capsule() {
        let world = World::new(alloc::vec![other_player([40.0, 0.0, 0.0], 3)]);
        let t = player_trace(&world, [0.0; 3], [100.0, 0.0, 0.0], MINS, MAXS, MASK);
        let reach = 14.625 + 0.5 + 14.625 / 0.965_925_8 + 0.5;
        assert!((t.fraction * 100.0 - (40.0 - reach)).abs() < 0.2, "{t:?}");
        assert_eq!((t.struck_entity_num, t.cflags, t.sflags), (3, 0, 0));
    }

    /// The box test: a shape the move never comes near is not tested; one
    /// it does is tested from when it is reached.
    #[test]
    fn the_box_test_finds_when_a_shape_is_reached() {
        let input = setup_trace_info(MINS, MAXS, [0.0; 3], [100.0, 0.0, 0.0], MASK);
        let far = block([40.0, 200.0, 0.0], [56.0, 256.0, 64.0]);
        assert_eq!(trace_aabb(&input, &far), None);
        let ahead = block([40.0, -256.0, 0.0], [56.0, 256.0, 64.0]);
        let t = trace_aabb(&input, &ahead).expect("reached");
        // The trace box's front is 15.52 out; it comes within 0.125.
        let front = input.cg_maxs[0];
        assert!((t * 100.0 - (40.0 - front - AC_EPS)).abs() < 1e-3, "{t}");
        // Behind the start, moving away: never.
        let behind = block([-56.0, -256.0, 0.0], [-40.0, 256.0, 64.0]);
        assert_eq!(trace_aabb(&input, &behind), None);
    }

    /// Standing on terrain: its own flags, walkable.
    #[test]
    fn standing_on_terrain() {
        let vertex_points: &'static [[f32; 3]] = &[[-64.0, -64.0, 0.0], [64.0, -64.0, 0.0], [0.0, 64.0, 0.0]];
        let indices: &'static [u16] = &[0, 2, 1];
        let terrain = Geom {
            shape: Shape::Partition { vertex_points, indices },
            lo_corner: [-64.0, -64.0, 0.0],
            hi_corner: [64.0, 64.0, 0.0],
            surf_bits: 9,
            key: 2,
            ..floor()
        };
        let world = World::new(alloc::vec![terrain]);
        let t = ground(&world, 0.125);
        assert!((t.fraction - 0.5).abs() < 0.01, "{t:?}");
        assert_eq!((t.walkable, t.sflags), (true, 9), "{t:?}");
    }

    /// Pressed 0.1 into a wall: the wall is hit first (at once, and it is
    /// no ground), but the floor half a unit down wins because you can
    /// stand on it.
    #[test]
    fn the_ground_trace_prefers_ground_you_can_stand_on() {
        let reach = 14.625 / 0.965_925_8 + 0.5;
        let wall = Geom { surf_bits: 3, key: 2, ..block([reach - 0.1, -256.0, -64.0], [reach + 16.0, 256.0, 256.0]) };
        let world = World::new(alloc::vec![wall, floor()]);
        let input = setup_trace_info(MINS, MAXS, [0.0, 0.0, 0.375], [0.0, 0.0, -0.125], MASK);
        let hits = gjk_trace(&world, &TraceInput { exit_on_penetration: false, ..input }, &[]);
        let first = &hits.list[hits.first.expect("a hit")];
        assert_eq!((first.shape_data.key, first.hit_time), (2, 0.0), "{first:?}");
        assert!(!hit_walkable(first));
        let t = ground(&world, 0.125);
        assert!((t.fraction - 0.5).abs() < 0.01, "{t:?}");
        assert_eq!((t.walkable, t.sflags, t.normal), (true, 7, [0.0, 0.0, 1.0]), "{t:?}");
    }

    #[test]
    fn a_brush_reports_its_top_unless_it_is_caulk() {
        const CAULK: u32 = 0x40;
        let axial = [[1, 2, 3], [4, 5, 6]];
        assert_eq!(brush_surface_flags(axial, [], CAULK), 6);
        let capped = [[1, 2, 3], [4, 5, CAULK]];
        // The most upward other face that is not caulk.
        assert_eq!(brush_surface_flags(capped, [(0.2, 11), (0.9, CAULK), (0.5, 12)], CAULK), 12);
        // No other faces: the first box face that is not caulk.
        let low_caulk = [[CAULK, 2, 3], [4, 5, CAULK]];
        assert_eq!(brush_surface_flags(low_caulk, [], CAULK), 2);
        assert_eq!(brush_surface_flags([[CAULK; 3]; 2], [], CAULK), CAULK);
    }
}
