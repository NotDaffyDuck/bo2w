//! Black Ops II's slide move in physics mode 1 (gjk_slide_move). The body
//! sweeps along the frame's move; each shape it meets becomes a plane it
//! may not cross, and the plane solver bends what is left of the move, and
//! the velocity, along all of them, round after round. Walking on the
//! ground, it then drops up to 30 units onto ground below (the step down).
//!
//! A move that comes out much farther than it went in means the body
//! started inside something. Then it is pushed out instead (gjk_push_out);
//! failing that, it slides without being pushed out of anything.
//!
//! Where ours differs, by choice:
//! - Past 128 planes BO2 stops the game; ours leaves the extra shapes out of
//!   the solve (they stay solid: they are not skipped).
//! - In the push-out, a plane whose shape cannot be measured again (the
//!   cores overlap) keeps its last plane; BO2 reads whatever its search left.
//! - When even the slide without push-out goes too far, ours stays put with
//!   the velocity it had; BO2 leaves its output unset.
//! - Nothing is glass here, so glass is never touched.

use crate::bridge::TickLocals;
use alloc::vec::Vec;
use playerstate_t6::game::{AvatarState, movement_state_bits};
use trace_t6::ENTITYNUM_WORLD;
use trace_t6::Trace;
use trace_t6::TraceHitType;

use super::hull::{MAX_PLANES, SolvePlane, project};
use super::hull::{Mover, MoverBrush};
use super::trace::{
    Hit, TraceInput, gjk_collide, gjk_trace, hit_walkable, mover_trace_info, player_mover,
};
use super::Bo2Collision;
use super::Geom;
use super::add;
use super::dot;
use super::magnitude_squared;
use super::scale;
use super::sub;
use crate::project::project_velocity;

/// A sixtieth of a second: hits this close after the first are taken with
/// it, and a hit this soon does not move the body first.
const CONTACT_TIME: f32 = 0.016_666_668;

/// What is left of the move is done when it is this short (squared).
const MIN_MOVE_SQ: f32 = 0.0001;

/// The most rounds of one slide.
const MAX_ROUNDS: u32 = 128;

/// The slide gave up (the body started inside something) when it comes out
/// farther from the start than four times the move, or 20 units (squared).
const FAR_SCALE_SQ: f32 = 16.0;
const FAR_MIN_SQ: f32 = 400.0;

/// The step down: on, and how far down it looks.
const DO_STEP_DOWN: bool = true;
const STEP_DOWN: [f32; 3] = [0.0, 0.0, -30.0];
/// Ground within this of the body counts as under it already.
const WALKABLE_DIST: f32 = 0.1;
/// No walkable plane at all.
const NO_GROUND: f32 = 10000.0;
/// A plane this side-on to the step down is never reached by it.
const SWEEP_PARALLEL: f32 = 0.001;

/// The push-out fails when a plane is still this far in after the solve.
const PUSH_OUT_FAIL: f32 = -5.0;
/// A push shorter than this is no push.
const PUSH_OUT_DONE: f32 = 0.001;
/// The push-out measures shapes at any distance.
const PUSH_OUT_SEP: f32 = 1.0e9;
/// The push-out's planes are measured again until the push moves less than
/// this (squared), at most this many times, and the whole thing is tried
/// this many times more.
const SETTLE_SQ: f32 = 0.010_000_001;
const SETTLE_ROUNDS: u32 = 5;
const PUSH_OUT_BUMPS: u32 = 4;

/// BO2's gjk_slide_move_input_t.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlideInput {
    pub position: [f32; 3],
    pub vel: [f32; 3],
    pub fall_accel: f32,
    /// Gravity pulls during the move (in the air).
    pub has_gravity: bool,
    /// What slides: the player's body or a vehicle's brush.
    pub mover: Mover,
    pub solid_mask: u32,
    pub step_millis_elapsed: f32,
    /// Drop onto ground below at the end (walking).
    pub do_step_down: bool,
}

/// Where the slide put the player, and whether it met anything.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlideOutput {
    pub position: [f32; 3],
    pub vel: [f32; 3],
    pub bumped: bool,
}

/// BO2's geom_plane: a shape the body met, as a plane through the contact.
#[derive(Clone, Copy, Debug)]
struct Plane<'a> {
    shape_data: Geom<'a>,
    plane_dir: [f32; 3],
    /// The contact on the body, from its origin.
    arm: [f32; 3],
    /// The plane's distance (through the contact on the shape).
    d: f32,
    walkable: bool,
    no_push_out: bool,
}

impl<'a> Plane<'a> {
    fn from_hit(strike: &Hit<'a>) -> Self {
        let Hit {
            shape_data,
            arm,
            hit_point,
            facing: normal,
            ..
        } = *strike;
        Plane {
            d: dot(normal, hit_point),
            walkable: hit_walkable(strike),
            no_push_out: shape_data.no_push_out,
            shape_data,
            plane_dir: normal,
            arm,
        }
    }

    /// How far the body's contact is out of the plane with its origin at
    /// `pos` (below zero: in it).
    fn gap(&self, pos: [f32; 3]) -> f32 {
        dot(self.plane_dir, add(self.arm, pos)) - self.d
    }
}

/// The planes of one slide or push-out, and the shapes already met (the
/// traces skip them).
#[derive(Default)]
struct Planes<'a> {
    list: Vec<Plane<'a>>,
    solve: Vec<SolvePlane>,
    skip: Vec<u64>,
}

impl<'a> Planes<'a> {
    /// BO2's add_hit_info.
    fn note_hit(&mut self, strike: &Hit<'a>) {
        if self.list.len() >= MAX_PLANES {
            return;
        }
        self.list.push(Plane::from_hit(strike));
        self.solve.push(SolvePlane {
            facing: strike.facing,
            rs: 0.0,
            engaged: true,
        });
        self.skip.push(strike.shape_data.key);
    }

    /// Each plane's push for the body at `pos`: how far in it is. A plane
    /// `held(plane)` says so for is never pushed out of (its push is at
    /// most zero).
    fn set_rs(&mut self, pos: [f32; 3], held: impl Fn(&Plane) -> bool) {
        for (s, p) in self.solve.iter_mut().zip(&self.list) {
            s.rs = -p.gap(pos);
            if held(p) && s.rs > 0.0 {
                s.rs = 0.0;
            }
        }
    }

    fn hold_all(&mut self) {
        for s in &mut self.solve {
            if s.rs > 0.0 {
                s.rs = 0.0;
            }
        }
    }

    fn project(&self, v: [f32; 3]) -> [f32; 3] {
        project(v, &self.solve)
    }

    /// The push-out's failure: a plane still more than 5 in after the
    /// solve.
    fn stuck(&self, step: [f32; 3]) -> bool {
        self.solve
            .iter()
            .any(|s| dot(s.facing, step) - s.rs < PUSH_OUT_FAIL)
    }
}

/// PM_AddTouchEnt for a hit (the world is never touched).
fn touch<C: Bo2Collision>(solids: &C, strike: &Hit) {
    let entity = strike.shape_data.struck_entity_num;
    if entity == ENTITYNUM_WORLD {
        return;
    }
    // Hand the contact over as an entity trace stopped at the hit.
    let Hit {
        hit_time,
        facing: normal,
        hit_point,
        ..
    } = *strike;
    let mut as_trace = Trace::new();
    as_trace.impact_category = TraceHitType::Entity;
    as_trace.struck_entity_num = entity;
    (as_trace.fraction, as_trace.normal) = (hit_time, normal);
    solids.brush_against(&as_trace, hit_point);
}

/// BO2's gjk_slide_move1. `no_push`: never push out of anything. None: the
/// slide went too far.
fn slide_move1<C: Bo2Collision>(
    solids: &C,
    input: &SlideInput,
    no_push: bool,
) -> Option<SlideOutput> {
    let ft = input.step_millis_elapsed;
    let p0 = input.position;
    let v = input.vel;
    let v_end_z = if input.has_gravity {
        v[2] - input.fall_accel * ft
    } else {
        v[2]
    };
    let half = ft * 0.5;
    let end = [
        p0[0] + (v[0] + v[0]) * half,
        p0[1] + (v[1] + v[1]) * half,
        p0[2] + (v[2] + v_end_z) * half,
    ];
    let mut trace = mover_trace_info(input.mover, p0, end, input.solid_mask);
    trace.exit_on_penetration = false;

    let mut trans_left = sub(end, p0);
    let mut vel_want = [v[0] * ft, v[1] * ft, v_end_z * ft];
    let far_sq = (FAR_SCALE_SQ * magnitude_squared(trans_left)).max(FAR_MIN_SQ);

    let mut planes = Planes::default();
    let mut pos = p0;
    let mut time_used = 0.0f32;
    let mut vel_out = vel_want;
    let mut cur = trans_left;
    let mut rounds = 0;
    loop {
        trace.from_pt = pos;
        trace.translation = cur;
        let remaining = ft - time_used;
        trace.extra_time = if CONTACT_TIME <= remaining {
            CONTACT_TIME / remaining
        } else {
            1.0
        };
        trace.skip_sentients = false;
        let hits = gjk_trace(solids, &trace, &planes.skip);
        let Some(first) = hits.first else {
            pos = add(pos, cur);
            break;
        };
        let hit_time = hits.list[first].hit_time;
        let t_hit = hit_time * remaining;
        let mut advanced = 0.0;
        if CONTACT_TIME < t_hit {
            // Far enough off: move up to it first.
            pos = add(scale(cur, hit_time), pos);
            trans_left = scale(cur, 1.0 - hit_time);
            vel_want = vel_out;
            time_used += t_hit;
            advanced = t_hit;
        }
        for strike in &hits.list {
            if strike.hit_time * remaining - advanced > CONTACT_TIME {
                continue;
            }
            let mut strike = *strike;
            if no_push && strike.hit_dist < 0.0 {
                // Take the body as it is: the plane through where it sits.
                strike.hit_point = add(scale(strike.facing, strike.hit_dist), strike.hit_point);
                strike.hit_dist = 0.0;
            }
            planes.note_hit(&strike);
            touch(solids, &strike);
        }
        planes.set_rs(pos, |p| no_push || p.no_push_out);
        cur = planes.project(trans_left);
        if magnitude_squared(sub(add(cur, pos), p0)) > far_sq {
            return None;
        }
        // The velocity is only ever slid, never pushed.
        planes.hold_all();
        vel_out = planes.project(vel_want);
        if magnitude_squared(cur) < MIN_MOVE_SQ {
            break;
        }
        rounds += 1;
        if rounds >= MAX_ROUNDS {
            break;
        }
    }
    let mut velocity = scale(vel_out, 1.0 / ft);

    if DO_STEP_DOWN && input.do_step_down {
        step_down(&mut planes, &mut pos, &mut velocity);
    }
    Some(SlideOutput {
        position: pos,
        vel: velocity,
        bumped: !planes.list.is_empty(),
    })
}

/// The end of gjk_slide_move1 when walking: if the slide left the body
/// above the ground it met (more than 0.1 off the nearest walkable plane),
/// sweep it down up to 30 along the planes until it lands on one, and lay
/// the velocity along that ground.
fn step_down(planes: &mut Planes, pos: &mut [f32; 3], vel: &mut [f32; 3]) {
    for slot in &mut planes.solve {
        slot.engaged = false;
    }
    let nearest = planes
        .list
        .iter()
        .filter(|p| p.walkable)
        .map(|p| p.gap(*pos))
        .fold(NO_GROUND, |m, g| if g < m { g } else { m });
    if !(WALKABLE_DIST < nearest && nearest != NO_GROUND) {
        return;
    }
    let n = planes.list.len();
    let mut sweep = STEP_DOWN;
    let mut tries = 0;
    loop {
        // The first plane the sweep reaches.
        let reachable = planes
            .solve
            .iter()
            .zip(&planes.list)
            .enumerate()
            .filter_map(|(k, (slot, p))| {
                let facing = dot(sweep, p.plane_dir);
                (!slot.engaged && libm::fabsf(facing) > SWEEP_PARALLEL)
                    .then(|| (k, p.gap(*pos) * (-1.0 / facing)))
            });
        // Earliest wins; a tie keeps the lower index.
        let (first_met, best_t) = reachable.fold((None, 1.0f32), |(kept, kept_t), (i, t)| {
            if t < kept_t {
                (Some(i), t)
            } else {
                (kept, kept_t)
            }
        });
        let Some(best) = first_met else {
            return;
        };
        planes.solve[best].engaged = true;
        let cand = add(scale(sweep, best_t), *pos);
        let landing = if planes.list[best].walkable {
            Some(best)
        } else {
            // Walkable ground within 0.1 of where the sweep stops.
            let mut landing = None;
            let mut closest = WALKABLE_DIST;
            for (i, p) in planes.list.iter().enumerate() {
                if p.walkable {
                    let g = p.gap(cand);
                    if g <= closest {
                        landing = Some(i);
                        closest = g;
                    }
                }
            }
            landing
        };
        if let Some(l) = landing {
            if 0.0 < best_t {
                *pos = cand;
                project_velocity(vel, &planes.list[l].plane_dir);
            }
            return;
        }
        planes.set_rs(*pos, |_| true);
        sweep = planes.project(STEP_DOWN);
        tries += 1;
        if tries >= n {
            return;
        }
    }
}

/// BO2's gjk_push_out: from inside something, the shortest way out of
/// everything the body overlaps, measured again as it moves, a few times.
/// `skip_sentients`: pass through other players. None: no way out.
fn push_out<C: Bo2Collision>(
    solids: &C,
    input: &SlideInput,
    skip_sentients: bool,
) -> Option<[f32; 3]> {
    let base = input.position;
    let mut cur = base;
    let mut trace = mover_trace_info(input.mover, base, base, input.solid_mask);
    trace.extra_time = 1.0;
    trace.skip_sentients = skip_sentients;
    trace.keep_all = true;
    trace.exit_on_penetration = false;
    let mut planes = Planes::default();
    let mut bumps = 0;
    loop {
        trace.from_pt = base;
        trace.translation = sub(cur, base);
        let hits = gjk_trace(solids, &trace, &planes.skip);
        if hits.first.is_none() {
            return Some(cur);
        }
        for strike in &hits.list {
            planes.note_hit(strike);
            touch(solids, strike);
        }
        planes.set_rs(base, |p| p.no_push_out);
        let mut delta = planes.project([0.0; 3]);
        if planes.stuck(delta) {
            return None;
        }
        let len = libm::sqrtf(magnitude_squared(delta));
        if len <= PUSH_OUT_DONE {
            return Some(cur);
        }
        cur = add(base, delta);

        // Measure each shape again from where the push puts the body.
        let sep = len + PUSH_OUT_SEP;
        let mut rounds = 0;
        loop {
            let at = TraceInput {
                from_pt: cur,
                translation: [0.0; 3],
                keep_all: true,
                ..trace
            };
            for (slot, p) in planes.solve.iter_mut().zip(planes.list.iter_mut()) {
                if let Some(strike) = gjk_collide(&at, &p.shape_data, 0.0, 1.0, sep) {
                    *p = Plane::from_hit(&strike);
                    slot.facing = strike.facing;
                    slot.engaged = true;
                }
            }
            planes.set_rs(base, |p| p.no_push_out);
            let prev = delta;
            delta = planes.project([0.0; 3]);
            if planes.stuck(delta) {
                return None;
            }
            cur = add(base, delta);
            if magnitude_squared(sub(delta, prev)) <= SETTLE_SQ {
                break;
            }
            rounds += 1;
            if rounds >= SETTLE_ROUNDS {
                break;
            }
        }
        bumps += 1;
        if bumps > PUSH_OUT_BUMPS {
            return Some(cur);
        }
    }
}

/// BO2's gjk_slide_move: the slide, or when it went too far (the body
/// started inside something) the push-out, through other players if need
/// be, and failing both the slide that pushes out of nothing.
pub(crate) fn gjk_slide_move<C: Bo2Collision>(solids: &C, input: &SlideInput) -> SlideOutput {
    if let Some(out) = slide_move1(solids, input, false) {
        return out;
    }
    let stay = SlideOutput {
        position: input.position,
        vel: input.vel,
        bumped: true,
    };
    if let Some(position) =
        push_out(solids, input, false).or_else(|| push_out(solids, input, true))
    {
        return SlideOutput { position, ..stay };
    }
    match slide_move1(solids, input, true) {
        Some(out) => SlideOutput {
            bumped: true,
            ..out
        },
        None => stay,
    }
}

/// BO2's PM_SlideMove in physics mode 1: the slide from the player's origin
/// and velocity, gravity pulling when `gravity` (the air), stepping down
/// onto ground below when not (walking on ground, off a ladder). True when
/// it met anything.
pub(crate) fn slide_move<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    solids: &C,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    mask: u32,
    gravity: bool,
) -> bool {
    let on_ladder = avatar.movement_state_bits & movement_state_bits::LADDER != 0;
    let input = SlideInput {
        do_step_down: !gravity && tick_vars.standing_on_floor != 0 && !on_ladder,
        has_gravity: gravity,
        fall_accel: avatar.gravity as f32,
        step_millis_elapsed: tick_vars.step_millis_elapsed,
        mover: player_mover(box_lo, box_hi),
        position: avatar.origin,
        vel: avatar.velocity,
        solid_mask: mask,
    };
    let out = gjk_slide_move(solids, &input);
    avatar.origin = out.position;
    avatar.velocity = out.vel;
    out.bumped
}

/// BO2's vehicle slide with a collision map (setup_gjkcc_input, then
/// gjk_slide_move): the vehicle's brush, never turned, slid from `start`
/// along `moving` for `step_millis_elapsed` through what `hull_hits` holds, never
/// stepping down; `pull` is gravity when set. Where it ends up, its velocity,
/// and whether it met anything.
pub fn vehicle_slide_move<C: Bo2Collision>(
    solids: &C,
    brush: &MoverBrush,
    from_pt: [f32; 3],
    moving: [f32; 3],
    mask: u32,
    step_millis_elapsed: f32,
    pull: Option<f32>,
) -> ([f32; 3], [f32; 3], bool) {
    let input = SlideInput {
        position: from_pt,
        vel: moving,
        fall_accel: pull.unwrap_or(0.0),
        has_gravity: pull.is_some(),
        mover: Mover::Brush(*brush),
        solid_mask: mask,
        step_millis_elapsed,
        do_step_down: false,
    };
    let out = gjk_slide_move(solids, &input);
    (out.position, out.vel, out.bumped)
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{MASK, MAXS, MINS, World, block, floor, other_player};
    use super::super::trace::{floor_probe, player_trace};
    use super::super::{IDENTITY, Shape};
    use super::*;

    /// Where the body rests on the floor: its skin on it, the origin 0.125
    /// up.
    const REST_Z: f32 = 0.125;
    /// The ring's corner out in front, with the skin.
    const REACH: f32 = 14.625 / 0.965_925_8 + 0.5;

    fn input(spot: [f32; 3], speed_vec: [f32; 3], in_air: bool) -> SlideInput {
        SlideInput {
            position: spot,
            vel: speed_vec,
            fall_accel: 800.0,
            has_gravity: in_air,
            mover: player_mover(MINS, MAXS),
            solid_mask: MASK,
            step_millis_elapsed: 0.05,
            do_step_down: !in_air,
        }
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn nothing_in_the_way() {
        let world = World::new(alloc::vec![]);
        let out = gjk_slide_move(&world, &input([0.0, 0.0, 50.0], [100.0, -40.0, 0.0], false));
        assert_eq!(out.position, [5.0, -2.0, 50.0]);
        assert_eq!(out.vel, [100.0, -40.0, 0.0]);
        assert!(!out.bumped);
    }

    /// Gravity over the frame moves the body by the average speed.
    #[test]
    fn falling_in_the_open() {
        let world = World::new(alloc::vec![]);
        let out = gjk_slide_move(&world, &input([0.0, 0.0, 100.0], [0.0, 0.0, 0.0], true));
        // 800 over 50 ms: ends at -40, moved -20 * 0.05.
        assert!(near(out.position[2], 99.0, 1e-4), "{out:?}");
        assert!(near(out.vel[2], -40.0, 1e-3), "{out:?}");
    }

    /// Running diagonally into a wall: stopped at it across, slid the whole
    /// way along it, the speed into it gone.
    #[test]
    fn sliding_along_a_wall() {
        let world = World::new(alloc::vec![block(
            [20.0, -256.0, -64.0],
            [36.0, 256.0, 256.0]
        )]);
        let out = gjk_slide_move(
            &world,
            &input([-10.0, 0.0, 10.0], [400.0, 400.0, 0.0], false),
        );
        assert!(out.bumped);
        assert!(near(out.position[0], 20.0 - REACH, 0.05), "{out:?}");
        assert!(near(out.position[1], 20.0, 0.05), "{out:?}");
        assert!(near(out.position[2], 10.0, 1e-4), "{out:?}");
        assert!(near(out.vel[0], 0.0, 0.01), "{out:?}");
        assert!(near(out.vel[1], 400.0, 0.01), "{out:?}");
    }

    /// Falling onto a floor: on it, the fall stopped.
    #[test]
    fn landing_on_a_floor() {
        let world = World::new(alloc::vec![floor()]);
        let out = gjk_slide_move(&world, &input([0.0, 0.0, 30.0], [50.0, 0.0, -600.0], true));
        assert!(out.bumped);
        assert!(near(out.position[2], REST_Z, 0.01), "{out:?}");
        assert!(near(out.vel[2], 0.0, 0.01), "{out:?}");
        assert!(near(out.vel[0], 50.0, 0.01), "{out:?}");
    }

    /// A floor met in the frame's last sixtieth of a second: the body ends
    /// on it, still falling just fast enough to have reached it (BO2 only
    /// takes the speed into a plane away when it has time left to move).
    #[test]
    fn landing_at_the_end_of_the_frame() {
        let world = World::new(alloc::vec![floor()]);
        let out = gjk_slide_move(&world, &input([0.0, 0.0, 5.0], [50.0, 0.0, -300.0], true));
        assert!(near(out.position[2], REST_Z, 0.01), "{out:?}");
        assert!(
            near(out.vel[2], (REST_Z - 5.0) / 0.05, 0.01),
            "{out:?}"
        );
    }

    /// A slope down toward +x, 20 degrees, through the origin.
    fn ramp() -> Geom<'static> {
        let (s, c) = (
            libm::sinf(20f32.to_radians()),
            libm::cosf(20f32.to_radians()),
        );
        let mut g = block([-1.0; 3], [1.0; 3]);
        g.shape = Shape::Box {
            center: [0.0, 0.0, -16.0],
            half: [256.0, 256.0, 16.0],
            brush: true,
        };
        g.axis = [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]];
        g.lo_corner = [-300.0, -300.0, -200.0];
        g.hi_corner = [300.0, 300.0, 200.0];
        g
    }

    /// How far the body at `pos` is above what is under it (to 4 units).
    fn height_above(world: &World, pos: [f32; 3]) -> f32 {
        let t = floor_probe(world, pos, [pos[0], pos[1], pos[2] - 4.0], MINS, MAXS, MASK);
        4.0 * t.fraction
    }

    /// Walking down a slope from just inside it: the move goes straight
    /// out, then the step down takes the body back onto the slope, the
    /// velocity laid along it. Without the step down it is left in the air.
    #[test]
    fn the_step_down_keeps_the_body_on_a_slope() {
        let world = World::new(alloc::vec![ramp()]);
        let (high, low) = ([0.0, 0.0, 20.0], [0.0, 0.0, -20.0]);
        let top = floor_probe(&world, high, low, MINS, MAXS, MASK).finish_location(high, low);
        let start = [top[0], top[1], top[2] - 0.3];
        let walk = input(start, [300.0, 0.0, 0.0], false);
        let out = gjk_slide_move(&world, &walk);
        assert!(out.bumped);
        assert!(height_above(&world, out.position) < 0.2, "{out:?}");
        assert!(out.position[2] < start[2] - 4.0, "{out:?}");
        assert!(out.vel[2] < -50.0, "{out:?}");

        let floating = gjk_slide_move(
            &world,
            &SlideInput {
                do_step_down: false,
                ..walk
            },
        );
        assert!(
            height_above(&world, floating.position) > 3.0,
            "{floating:?}"
        );
        assert!(near(floating.vel[2], 0.0, 1e-3), "{floating:?}");
    }

    /// Standing a little into a wall: the slide pushes the body straight
    /// out.
    #[test]
    fn a_small_overlap_is_pushed_out() {
        let world = World::new(alloc::vec![block(
            [10.0, -256.0, -64.0],
            [50.0, 256.0, 256.0]
        )]);
        let x = 10.0 - REACH + 1.5;
        let out = gjk_slide_move(&world, &input([x, 0.0, 10.0], [0.0; 3], false));
        assert!(out.bumped);
        assert!(near(out.position[0], 10.0 - REACH, 0.05), "{out:?}");
        assert_eq!(out.vel, [0.0; 3]);
    }

    /// Deep in a thin wall (more than 20 units from either face): too far
    /// for the slide, so the push-out takes the body out the nearer side,
    /// the velocity kept, the frame's move not made.
    #[test]
    fn deep_in_a_wall_is_pushed_out() {
        let world = World::new(alloc::vec![block(
            [-6.0, -256.0, -64.0],
            [8.0, 256.0, 256.0]
        )]);
        let out = gjk_slide_move(&world, &input([0.0, 0.0, 10.0], [0.0, 20.0, 0.0], false));
        assert!(out.bumped);
        assert!(near(out.position[0], -6.0 - REACH, 0.1), "{out:?}");
        assert!(near(out.position[1], 0.0, 0.01), "{out:?}");
        assert_eq!(out.vel, [0.0, 20.0, 0.0]);
        let t = player_trace(&world, out.position, out.position, MINS, MAXS, MASK);
        assert!(!t.starts_inside, "{t:?}");
    }

    /// Running into another player: stopped at them (never pushed out of
    /// them), and they are touched.
    #[test]
    fn running_into_another_player() {
        let world = World::new(alloc::vec![floor(), other_player([40.0, 0.0, REST_Z], 3)]);
        let out = gjk_slide_move(
            &world,
            &input([-20.0, 0.0, REST_Z], [600.0, 0.0, 0.0], false),
        );
        assert!(out.bumped);
        let gap = 40.0 - (14.625 + 0.5) - REACH;
        assert!(near(out.position[0], gap, 0.1), "{out:?}");
        assert!(near(out.vel[0], 0.0, 0.01), "{out:?}");
        assert_eq!(*world.touched.borrow(), alloc::vec![3]);
    }

    /// The Dragonfire's collision brush: a waist of 16 x 16 corners, a
    /// narrow top and a narrow bottom, each corner listed more than once as
    /// the model has them.
    fn drone_brush() -> MoverBrush {
        let mut points = alloc::vec::Vec::new();
        for _ in 0..2 {
            for (r, z) in [(2.5, -15.5), (3.0, 9.5), (16.0, 3.0), (16.0, -2.0)] {
                for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
                    points.push([r * sx, r * sy, z]);
                }
            }
        }
        MoverBrush::new(&points).expect("16 corners")
    }

    #[test]
    fn a_vehicle_brush_keeps_each_corner_once() {
        let brush = drone_brush();
        assert_eq!(brush.vertex_points().len(), 16);
        assert_eq!(brush.bounds(), ([-16.0, -16.0, -15.5], [16.0, 16.0, 9.5]));
        assert!(MoverBrush::new(&[]).is_none());
        let too_many: alloc::vec::Vec<[f32; 3]> = (0..40).map(|i| [i as f32, 0.0, 0.0]).collect();
        assert!(MoverBrush::new(&too_many).is_none());
    }

    /// A vehicle coming down onto a floor stops with its brush's bottom on
    /// it, no gravity and no step down. The brush has no skin, so the frame
    /// it arrives, its sweep starts with it already touching the floor: as
    /// in BO2, that frame's speed down is only what it covered, and from the
    /// next frame it rests on the floor and goes on along it.
    #[test]
    fn a_vehicle_brush_stops_on_a_floor() {
        let world = World::new(alloc::vec![floor()]);
        let (pos, motion_vec, bumped) = vehicle_slide_move(
            &world,
            &drone_brush(),
            [0.0, 0.0, 30.0],
            [50.0, 0.0, -600.0],
            MASK,
            0.05,
            None,
        );
        assert!(bumped);
        assert!(near(pos[2], 15.5, 1e-3), "{pos:?}");
        assert!(near(motion_vec[2], -14.5 / 0.05, 0.01), "{motion_vec:?}");
        assert!(near(motion_vec[0], 50.0, 0.01), "{motion_vec:?}");
        assert!(near(pos[0], 2.5, 0.01), "{pos:?}");
        let mut pos = pos;
        for frame in 2..6 {
            let (p, motion_vec, bumped) = vehicle_slide_move(
                &world,
                &drone_brush(),
                pos,
                [50.0, 0.0, -600.0],
                MASK,
                0.05,
                None,
            );
            assert!(bumped);
            assert!(near(p[2], 15.5, 1e-3), "{frame} {p:?}");
            assert!(near(motion_vec[2], 0.0, 0.01), "{frame} {motion_vec:?}");
            assert!(near(p[0], pos[0] + 2.5, 0.01), "{frame} {p:?}");
            pos = p;
        }
    }

    /// Flying diagonally into a wall: the brush's waist stops at it (that
    /// frame's speed into it only what it covered), and from the next frame
    /// it has no speed into the wall and the move along it goes on.
    #[test]
    fn a_vehicle_brush_slides_along_a_wall() {
        let world = World::new(alloc::vec![block(
            [30.0, -256.0, -64.0],
            [46.0, 256.0, 256.0]
        )]);
        let (pos, motion_vec, bumped) = vehicle_slide_move(
            &world,
            &drone_brush(),
            [0.0, 0.0, 50.0],
            [400.0, 400.0, 0.0],
            MASK,
            0.05,
            None,
        );
        assert!(bumped);
        assert!(near(pos[0], 14.0, 1e-3), "{pos:?}");
        assert!(near(pos[1], 20.0, 1e-3), "{pos:?}");
        assert!(near(pos[2], 50.0, 1e-4), "{pos:?}");
        assert!(near(motion_vec[0], 14.0 / 0.05, 0.01), "{motion_vec:?}");
        assert!(near(motion_vec[1], 400.0, 0.01), "{motion_vec:?}");
        let mut pos = pos;
        for frame in 2..6 {
            let (p, motion_vec, bumped) = vehicle_slide_move(
                &world,
                &drone_brush(),
                pos,
                [400.0, 400.0, 0.0],
                MASK,
                0.05,
                None,
            );
            assert!(bumped);
            assert!(near(p[0], 14.0, 1e-3), "{frame} {p:?}");
            assert!(near(p[1], pos[1] + 20.0, 1e-3), "{frame} {p:?}");
            assert!(near(motion_vec[0], 0.0, 0.01), "{frame} {motion_vec:?}");
            assert!(near(motion_vec[1], 400.0, 0.01), "{frame} {motion_vec:?}");
            pos = p;
        }
    }

    /// Gravity pulls only when asked for.
    #[test]
    fn a_vehicle_brush_falls_only_with_gravity() {
        let world = World::new(alloc::vec::Vec::new());
        let (pos, motion_vec, bumped) = vehicle_slide_move(
            &world,
            &drone_brush(),
            [0.0, 0.0, 100.0],
            [0.0; 3],
            MASK,
            0.05,
            None,
        );
        assert!(!bumped);
        assert_eq!((pos, motion_vec), ([0.0, 0.0, 100.0], [0.0; 3]));
        let (pos, motion_vec, _) = vehicle_slide_move(
            &world,
            &drone_brush(),
            [0.0, 0.0, 100.0],
            [0.0; 3],
            MASK,
            0.05,
            Some(800.0),
        );
        assert!(near(pos[2], 99.0, 1e-4), "{pos:?}");
        assert!(near(motion_vec[2], -40.0, 1e-3), "{motion_vec:?}");
    }

    /// The world is never touched.
    #[test]
    fn the_world_is_not_touched() {
        let world = World::new(alloc::vec![floor()]);
        gjk_slide_move(&world, &input([0.0, 0.0, 5.0], [0.0, 0.0, -300.0], true));
        assert!(world.touched.borrow().is_empty());
    }

    #[test]
    fn plane_gap() {
        let p = Plane {
            shape_data: block([0.0; 3], [1.0; 3]),
            plane_dir: [0.0_f32, 0.0, 1.0],
            arm: [0.0, 0.0, -0.125],
            d: 0.0,
            walkable: true,
            no_push_out: false,
        };
        assert_eq!(p.gap([0.0, 0.0, 0.125]), 0.0);
        assert_eq!(p.gap([5.0, 0.0, 2.125]), 2.0);
        let _ = IDENTITY;
    }
}
