//! Moving the player through the world: sliding along what they hit, and
//! stepping up and down stairs.

use playerstate_t6::consts::pm_type;
use playerstate_t6::game::{ENTITYNUM_NONE, PLAYER_CORPSE_ENTITY_BASE, PlayerState, pm_flags};
use trace_t6::Trace;

use crate::bridge::{CollisionBackend, GroundTraceInput, Pml, jump};

/// The slide flag of slick ground.
const PMF_SLIDING: u32 = 0x40_0000;

/// A clipped velocity is pushed this much further off the surface.
const OVERCLIP: f32 = 1.001;

/// Surfaces this flat to the player's heading are left out of the projection.
const PROJECT_NORMAL_Z_EPSILON: f32 = 0.001;

/// The most surfaces one move keeps track of, and the most bumps it takes.
const MAX_PLANES: usize = 8;
const MAX_BUMPS: usize = 4;

/// Two surfaces whose normals agree this much count as the same.
const SAME_PLANE_DOT: f32 = 0.999;

/// A velocity this far into a surface is stopped by it.
const PLANE_INTO: f32 = 0.1;

/// The tallest step, standing and crawling.
const STEP_SIZE: f32 = 18.0;
const PRONE_STEP_SIZE: f32 = 10.0;

/// The sweep up is this much taller than the step it looks for.
const STEP_UP_EXTRA: f32 = 1.0;

/// Ground steeper than this (the normal's z) can be stepped off.
const STEP_GROUND_NORMAL_Z: f32 = 0.9;

/// A walking player looks this much further down for the ground.
const WALKING_STEP_DOWN_EXTRA: f32 = 9.0;

/// A step that gains no more than this along the heading is taken back.
const STEP_PROGRESS_EPSILON: f32 = 0.001;

/// The width a lying player is checked to still fit with.
const PRONE_VERIFY_SIZE: f32 = 15.0;

/// Entity numbers below this are players (corpses start here).
const MAX_CLIENTS: u16 = PLAYER_CORPSE_ENTITY_BASE as u16;

/// A stair of less rise than this costs nothing.
const STEP_SLOW_MIN_RISE: f32 = 0.5;

/// Added before rounding a rise to whole units: 2^-30.
const STEP_ROUND_BIAS: f64 = 1.0 / 1_073_741_824.0;

/// A full step leaves this much speed; a smaller one slows by its height
/// over the step size across this range above it.
const STEP_SLOW_FLOOR: f32 = f32::from_bits(0x3e4c_cccc);
const STEP_SLOW_RANGE: f32 = 0.8;

/// A step of more than three units moves the walk rhythm on, by this much
/// per beat (a beat is two units, at most four) plus a base.
const STEP_BOB_PER_UNIT: f32 = 1.25;
const STEP_BOB_BASE: f32 = 7.0;

/// The box the player moves with, and what it collides with.
#[derive(Clone, Copy)]
struct Hull {
    mins: [f32; 3],
    maxs: [f32; 3],
    tracemask: u32,
}

impl Hull {
    fn sweep<C: CollisionBackend>(self, world: &C, start: [f32; 3], end: [f32; 3]) -> Trace {
        world.trace(GroundTraceInput {
            start,
            end,
            mins: self.mins,
            maxs: self.maxs,
            tracemask: self.tracemask,
        })
    }
}

/// Where a move had got to before a step was tried.
#[derive(Clone, Copy)]
struct Snapshot {
    origin: [f32; 3],
    velocity: [f32; 3],
}

impl Snapshot {
    fn of(ps: &PlayerState) -> Self {
        Self {
            origin: ps.origin,
            velocity: ps.velocity,
        }
    }

    fn restore(self, ps: &mut PlayerState) {
        ps.origin = self.origin;
        ps.velocity = self.velocity;
    }
}

/// The surfaces a move has met, at most `MAX_PLANES`.
struct Planes {
    normals: [[f32; 3]; MAX_PLANES],
    len: usize,
}

impl Planes {
    fn new() -> Self {
        Self {
            normals: [[0.0; 3]; MAX_PLANES],
            len: 0,
        }
    }

    fn is_full(&self) -> bool {
        self.len >= MAX_PLANES
    }

    fn push(&mut self, normal: [f32; 3]) {
        self.normals[self.len] = normal;
        self.len += 1;
    }

    fn as_slice(&self) -> &[[f32; 3]] {
        &self.normals[..self.len]
    }

    /// Is a surface with this normal already among them?
    fn has_same_as(&self, normal: [f32; 3]) -> bool {
        self.as_slice()
            .iter()
            .any(|known| dot(normal, *known) > SAME_PLANE_DOT)
    }
}

/// The normal of the ground the last ground trace found.
fn ground_normal(pml: &Pml) -> [f32; 3] {
    [
        f32::from_bits(pml.ground_trace[1]),
        f32::from_bits(pml.ground_trace[2]),
        f32::from_bits(pml.ground_trace[3]),
    ]
}

/// Move for the rest of the frame, bumping along up to four surfaces. With
/// `gravity` the velocity falls through the frame and ends the frame lower.
/// Returns whether anything was hit.
fn slide_move<C: CollisionBackend>(
    ps: &mut PlayerState,
    pml: &Pml,
    world: &C,
    hull: Hull,
    gravity: Option<f32>,
) -> bool {
    let mut end_velocity = ps.velocity;
    let mut entry_velocity = ps.velocity;
    if let Some(g) = gravity {
        end_velocity[2] -= g * pml.frametime;
        entry_velocity[2] = end_velocity[2];
        ps.velocity[2] = (ps.velocity[2] + end_velocity[2]) * 0.5;
        if pml.ground_plane != 0 {
            ps.velocity = clip_velocity(ps.velocity, ground_normal(pml));
        }
    }

    let mut planes = Planes::new();
    if pml.ground_plane != 0 {
        planes.push(ground_normal(pml));
    }
    let mut heading = ps.velocity;
    normalize(&mut heading);
    planes.push(heading);

    let mut time_left = pml.frametime;
    let mut bumped = false;
    for _ in 0..MAX_BUMPS {
        if time_left <= 0.0 {
            break;
        }
        let end = [
            ps.origin[0] + ps.velocity[0] * time_left,
            ps.origin[1] + ps.velocity[1] * time_left,
            ps.origin[2] + ps.velocity[2] * time_left,
        ];
        let trace = hull.sweep(world, ps.origin, end);
        if trace.fraction < 1.0 {
            world.touch_entity(&trace, trace.end_pos(ps.origin, end));
        }
        if trace.allsolid {
            ps.velocity[2] = 0.0;
            return true;
        }
        if trace.fraction > 0.0 {
            ps.origin = if trace.fraction >= 1.0 {
                end
            } else {
                trace.end_pos(ps.origin, end)
            };
        }
        if trace.fraction >= 1.0 {
            break;
        }

        bumped = true;
        time_left *= 1.0 - trace.fraction;
        if planes.is_full() {
            ps.velocity = [0.0; 3];
            return true;
        }
        if planes.has_same_as(trace.normal) {
            // The same surface again: push off it a little and go on.
            ps.velocity = clip_velocity(ps.velocity, trace.normal);
            for axis in 0..3 {
                ps.velocity[axis] += trace.normal[axis];
            }
            continue;
        }
        planes.push(trace.normal);

        let mut order = [0usize; MAX_PLANES];
        let into = rank_planes(ps.velocity, planes.as_slice(), &mut order);
        if into < PLANE_INTO {
            match follow_creases(ps.velocity, end_velocity, planes.as_slice(), &order) {
                Creased::Slide { velocity, end } => {
                    ps.velocity = velocity;
                    end_velocity = end;
                }
                Creased::Stuck => {
                    ps.velocity = [0.0; 3];
                    return true;
                }
            }
        }
    }

    if gravity.is_some() {
        ps.velocity = end_velocity;
    }
    // Just after a landing a wall moves the player without slowing them.
    if ps.pm_time != 0 {
        ps.velocity = entry_velocity;
    }
    bumped
}

/// What the surfaces the move is wedged between leave of its velocity.
enum Creased {
    /// It slides along them, this fast now and this fast at the frame's end.
    Slide { velocity: [f32; 3], end: [f32; 3] },
    /// Nowhere is open.
    Stuck,
}

/// Slide along the most restrictive surface; where that carries the player
/// into another, slide along the crease the two make instead, and give up
/// if a third closes that too. `order` lists the planes most restrictive
/// first.
fn follow_creases(
    velocity: [f32; 3],
    end_velocity: [f32; 3],
    planes: &[[f32; 3]],
    order: &[usize],
) -> Creased {
    let first = planes[order[0]];
    let mut now = clip_velocity(velocity, first);
    let mut end = clip_velocity(end_velocity, first);
    for j in 1..planes.len() {
        let second = planes[order[j]];
        if dot(now, second) >= PLANE_INTO {
            continue;
        }
        now = clip_velocity(now, second);
        end = clip_velocity(end, second);
        if dot(now, first) >= 0.0 {
            continue;
        }

        let mut crease = cross(first, second);
        normalize(&mut crease);
        now = scaled(crease, dot(velocity, crease));
        end = scaled(crease, dot(end_velocity, crease));
        let closed = (1..planes.len())
            .filter(|&k| k != j)
            .any(|k| dot(now, planes[order[k]]) < PLANE_INTO);
        if closed {
            return Creased::Stuck;
        }
    }
    Creased::Slide { velocity: now, end }
}

/// Fill `order` with the plane numbers, the one the velocity runs into
/// hardest first (a tie goes to the later plane). Returns how far into that
/// plane the velocity runs: the smallest dot product.
fn rank_planes(velocity: [f32; 3], planes: &[[f32; 3]], order: &mut [usize]) -> f32 {
    let mut into = [0.0_f32; MAX_PLANES];
    for (index, plane) in planes.iter().enumerate() {
        into[index] = dot(velocity, *plane);
        let mut at = index;
        while at > 0 && into[index] <= into[order[at - 1]] {
            order[at] = order[at - 1];
            at -= 1;
        }
        order[at] = index;
    }
    into[order[0]]
}

/// Move one frame and climb the stairs in the way. Returns how far a stair
/// raised (or lowered) the player, for the camera to ease over; 0 when no
/// step counted.
#[expect(
    clippy::too_many_arguments,
    reason = "one call boundary for a player move"
)]
pub fn step_slide_move<C: CollisionBackend>(
    ps: &mut PlayerState,
    pml: &Pml,
    world: &C,
    mins: [f32; 3],
    maxs: [f32; 3],
    tracemask: u32,
    gravity: Option<f32>,
    zombies: bool,
) -> f32 {
    let hull = Hull {
        mins,
        maxs,
        tracemask,
    };
    let had_ground = started_on_ground(ps, pml);
    let start = Snapshot::of(ps);

    let bumped = slide_move(ps, pml, world, hull, gravity);
    let flat = Snapshot::of(ps);

    // A last-stand player slides without stepping when nothing was hit.
    if !bumped && ps.pm_type == pm_type::LASTSTAND && ps.pm_flags & PMF_SLIDING != 0 {
        return 0.0;
    }

    let mut step_size = if ps.pm_flags & pm_flags::PRONE != 0 {
        PRONE_STEP_SIZE
    } else {
        STEP_SIZE
    };
    let mut jump_step = false;
    if ps.ground_entity_num == ENTITYNUM_NONE {
        match airborne_step(ps, bumped, start.origin, step_size) {
            Some((size, jumping)) => (step_size, jump_step) = (size, jumping),
            None => return 0.0,
        }
    }

    let mut step_height = 0.0;
    let sloped_ground =
        pml.ground_plane != 0 && f32::from_bits(pml.ground_trace[3]) < STEP_GROUND_NORMAL_Z;
    if bumped || sloped_ground {
        step_height = try_step_up(ps, pml, world, hull, gravity, start, step_size);
    }

    if had_ground || step_height != 0.0 {
        let mut descent = step_height;
        if had_ground {
            descent += WALKING_STEP_DOWN_EXTRA;
        }
        let target = [ps.origin[0], ps.origin[1], ps.origin[2] - descent];
        let down = hull.sweep(world, ps.origin, target);
        // The step comes back if it would land on another player, or on a
        // face too steep to stand on.
        if trace_t6::trace_get_entity_hit_id(&down) < MAX_CLIENTS {
            flat.restore(ps);
            return 0.0;
        }
        if down.fraction >= 1.0 {
            if step_height != 0.0 {
                ps.origin[2] -= step_height;
            }
        } else if down.walkable {
            ps.origin = down.end_pos(ps.origin, target);
            project_velocity(&mut ps.velocity, &down.normal);
        } else {
            flat.restore(ps);
            return 0.0;
        }
    }

    // A step that got no further along the heading than the flat move, or a
    // jump's step that ended at the top of the jump, comes back.
    let along = |origin: [f32; 3]| {
        start.velocity[0] * (origin[0] - start.origin[0])
            + start.velocity[1] * (origin[1] - start.origin[1])
    };
    let stepped = along(ps.origin);
    if stepped <= along(flat.origin) + STEP_PROGRESS_EPSILON || (jump_step && jump::above_max(ps)) {
        flat.restore(ps);
        if had_ground {
            let target = [
                ps.origin[0],
                ps.origin[1],
                ps.origin[2] - WALKING_STEP_DOWN_EXTRA,
            ];
            let down = hull.sweep(world, ps.origin, target);
            if down.fraction < 1.0 {
                ps.origin = down.end_pos(ps.origin, target);
                // The speed is kept along the slope.
                project_velocity(&mut ps.velocity, &down.normal);
            }
        }
    }

    if jump_step {
        jump::clamp_velocity(ps, flat.origin);
    }
    if had_ground && ps.pm_type < pm_type::DEAD {
        if !prone_still_fits(ps, world, start, zombies) {
            return 0.0;
        }
        return stair_step(ps, pml, start.origin, flat.origin, step_size);
    }
    0.0
}

/// Was the player on the ground (and not on a ladder) when the move began?
/// Leaving a ladder, or a jump that has run its time, forgets the jump.
fn started_on_ground(ps: &mut PlayerState, pml: &Pml) -> bool {
    if ps.pm_flags & pm_flags::LADDER != 0 {
        jump::clear_state(ps);
        return false;
    }
    if pml.ground_plane != 0 {
        return true;
    }
    if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time != 0 {
        jump::clear_state(ps);
    }
    false
}

/// Whether a player in the air may step, and with what: a jump that bumped
/// may step up to the height the jump allows, and a player climbing a ladder
/// may step as usual. Returns the step size and whether it is a jump's step.
fn airborne_step(
    ps: &mut PlayerState,
    bumped: bool,
    start_origin: [f32; 3],
    step_size: f32,
) -> Option<(f32, bool)> {
    if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time != 0 {
        jump::clear_state(ps);
    }
    let jumping = ps.pm_flags & pm_flags::JUMPING != 0;
    let climbing = ps.pm_flags & pm_flags::LADDER != 0 && ps.velocity[2] > 0.0;
    if bumped && jumping {
        return match jump::get_step_height(ps, start_origin) {
            Some(height) if height < 1.0 => None,
            Some(height) => Some((height, true)),
            None if climbing => Some((step_size, false)),
            None => None,
        };
    }
    climbing.then_some((step_size, false))
}

/// Raise the player over what the flat move hit and slide again from there.
/// Returns the rise, 0 when there was no room for one.
fn try_step_up<C: CollisionBackend>(
    ps: &mut PlayerState,
    pml: &Pml,
    world: &C,
    hull: Hull,
    gravity: Option<f32>,
    start: Snapshot,
    step_size: f32,
) -> f32 {
    let top = [
        start.origin[0],
        start.origin[1],
        start.origin[2] + step_size + STEP_UP_EXTRA,
    ];
    let up = hull.sweep(world, start.origin, top);
    let rise = (step_size + STEP_UP_EXTRA) * up.fraction - STEP_UP_EXTRA;
    if rise < 1.0 {
        return 0.0;
    }
    ps.origin = [start.origin[0], start.origin[1], start.origin[2] + rise];
    ps.velocity = start.velocity;
    slide_move(ps, pml, world, hull, gravity);
    rise
}

/// After a move on the ground a lying player must still have room to lie. If
/// not, they go back to where they started, unless that is no better either,
/// and then the move stands. False when the move was taken back.
fn prone_still_fits<C: CollisionBackend>(
    ps: &mut PlayerState,
    world: &C,
    start: Snapshot,
    zombies: bool,
) -> bool {
    if ps.pm_flags & pm_flags::PRONE == 0 || prone_fits(ps, world, zombies) {
        return true;
    }
    let moved = Snapshot::of(ps);
    start.restore(ps);
    if prone_fits(ps, world, zombies) {
        return false;
    }
    moved.restore(ps);
    true
}

/// Does the lying body fit where the player is? The pitches it finds are kept.
fn prone_fits<C: CollisionBackend>(ps: &mut PlayerState, world: &C, zombies: bool) -> bool {
    let rules = crate::bridge::ProneRules::for_player(ps, zombies);
    crate::bridge::check_prone(
        world,
        ps.origin,
        PRONE_VERIFY_SIZE,
        crate::bridge::PRONE_CHECK_HEIGHT,
        ps.prone_direction,
        true,
        true,
        true,
        rules,
        crate::bridge::PRONE_FEET_DIST,
        Some(&mut ps.f_torso_pitch),
        Some(&mut ps.f_waist_pitch),
    )
}

/// A stair on the ground slows the player by its height (a full step leaves
/// a fifth of the speed), and one of more than three units moves the walk
/// rhythm on, which can sound a footstep. Returns the rise, 0 when it does
/// not count.
fn stair_step(
    ps: &mut PlayerState,
    pml: &Pml,
    start_origin: [f32; 3],
    flat_origin: [f32; 3],
    step_size: f32,
) -> f32 {
    let rise = ps.origin[2] - flat_origin[2];
    if libm::fabsf(rise) <= STEP_SLOW_MIN_RISE {
        return 0.0;
    }
    let units = libm::rint(f64::from(rise) + STEP_ROUND_BIAS) as i32;
    if units == 0 {
        return 0.0;
    }

    let slow = (1.0 - libm::fabsf(ps.origin[2] - start_origin[2]) / step_size) * STEP_SLOW_RANGE
        + STEP_SLOW_FLOOR;
    for component in &mut ps.velocity {
        *component *= slow;
    }

    let units = units.abs();
    if units > 3 && ps.ground_entity_num != ENTITYNUM_NONE {
        let beats = (units / 2).min(4);
        let before = ps.bob_cycle;
        ps.bob_cycle =
            ((beats as f32 * STEP_BOB_PER_UNIT + STEP_BOB_BASE + before as f32) as i32) & 0xff;
        let audible = crate::bridge::should_make_footsteps(ps);
        let _ = crate::bridge::footstep_event(
            ps,
            before as u8,
            ps.bob_cycle as u8,
            pml.ground_trace[4],
            audible,
        );
    }
    rise
}

/// Turn a velocity to run along a surface, keeping its speed, unless that
/// would only speed up a fall onto an upward slope.
pub(crate) fn project_velocity(velocity: &mut [f32; 3], normal: &[f32; 3]) {
    let flat_sq = velocity[0] * velocity[0] + velocity[1] * velocity[1];
    if libm::fabsf(normal[2]) < PROJECT_NORMAL_Z_EPSILON || flat_sq == 0.0 {
        return;
    }
    let z = -(normal[1] * velocity[1] + normal[0] * velocity[0]) / normal[2];
    let scale = libm::sqrtf((velocity[2] * velocity[2] + flat_sq) / (z * z + flat_sq));
    if scale >= 1.0 && z >= 0.0 && velocity[2] <= 0.0 {
        return;
    }
    velocity[0] *= scale;
    velocity[1] *= scale;
    velocity[2] = z * scale;
}

/// The velocity with what runs into the surface taken out (and a little more,
/// so it does not stay on it).
fn clip_velocity(velocity: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let into = dot(velocity, normal);
    let backoff = -(into - libm::fabsf(into) * (OVERCLIP - 1.0));
    [
        velocity[0] + backoff * normal[0],
        velocity[1] + backoff * normal[1],
        velocity[2] + backoff * normal[2],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scaled(v: [f32; 3], by: f32) -> [f32; 3] {
    [v[0] * by, v[1] * by, v[2] * by]
}

/// Scale to length 1; a zero vector stays zero.
fn normalize(v: &mut [f32; 3]) {
    let len = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let by = if len <= 0.0 { 1.0 } else { 1.0 / len };
    v[0] *= by;
    v[1] *= by;
    v[2] *= by;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pml;
    use trace_t6::{ENTITYNUM_WORLD, TraceHitType};

    const HULL: Hull = Hull {
        mins: [-15.0, -15.0, 0.0],
        maxs: [15.0, 15.0, 70.0],
        tracemask: 1,
    };

    /// Open space over a floor at z = 0; the floor reports `hit_id` and is
    /// tilted to `normal_z` (walkable from 0.7).
    struct Floor {
        hit_id: u16,
        normal_z: f32,
    }

    impl Floor {
        fn level() -> Self {
            Self {
                hit_id: ENTITYNUM_WORLD,
                normal_z: 1.0,
            }
        }
    }

    impl CollisionBackend for Floor {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            let from = input.start[2] + input.mins[2];
            let to = input.end[2] + input.mins[2];
            if from >= 0.0 && to < 0.0 {
                hit.fraction = from / (from - to);
                hit.normal = [
                    libm::sqrtf(1.0 - self.normal_z * self.normal_z),
                    0.0,
                    self.normal_z,
                ];
                hit.walkable = self.normal_z >= 0.7;
                hit.hit_type = TraceHitType::Entity;
                hit.hit_id = self.hit_id;
            }
            hit
        }
    }

    /// A wall across x = `x` facing -x, and nothing else.
    struct Wall {
        x: f32,
    }

    impl CollisionBackend for Wall {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            if input.start[0] < self.x && input.end[0] > self.x {
                hit.fraction = (self.x - input.start[0]) / (input.end[0] - input.start[0]);
                hit.normal = [-1.0, 0.0, 0.0];
            }
            hit
        }
    }

    /// Nothing in the way anywhere.
    struct OpenAir;

    impl CollisionBackend for OpenAir {
        fn trace(&self, _input: GroundTraceInput) -> Trace {
            Trace {
                fraction: 1.0,
                ..Trace::default()
            }
        }
    }

    fn running(origin: [f32; 3], velocity: [f32; 3]) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.origin = origin;
        ps.velocity = velocity;
        ps
    }

    /// Walk off a 5-unit drop: the height the player ends at.
    fn walk_off(floor: &Floor, last_stand: bool) -> f32 {
        let mut ps = running([0.0, 0.0, 5.0], [100.0, 0.0, 0.0]);
        ps.ground_entity_num = i32::from(ENTITYNUM_WORLD);
        if last_stand {
            ps.pm_type = pm_type::LASTSTAND;
            ps.pm_flags |= PMF_SLIDING;
        }
        step_slide_move(
            &mut ps,
            &pml(true),
            floor,
            HULL.mins,
            HULL.maxs,
            1,
            None,
            false,
        );
        ps.origin[2]
    }

    #[test]
    fn a_step_down_onto_another_player_is_taken_back() {
        assert_eq!(walk_off(&Floor::level(), false), 0.0);
        assert_eq!(
            walk_off(
                &Floor {
                    hit_id: 3,
                    normal_z: 1.0
                },
                false
            ),
            5.0
        );
        // A zombie (entity 26 and up) is no player.
        assert_eq!(
            walk_off(
                &Floor {
                    hit_id: 26,
                    normal_z: 1.0
                },
                false
            ),
            0.0
        );
    }

    #[test]
    fn a_clear_move_in_last_stand_never_steps() {
        assert_eq!(walk_off(&Floor::level(), true), 5.0);
    }

    #[test]
    fn a_step_down_onto_a_face_too_steep_to_stand_on_is_taken_back() {
        assert_eq!(
            walk_off(
                &Floor {
                    hit_id: ENTITYNUM_WORLD,
                    normal_z: 0.5
                },
                false
            ),
            5.0
        );
    }

    #[test]
    fn a_wall_stops_the_run_into_it_and_the_rest_slides_along() {
        let mut ps = running([0.0; 3], [100.0, 50.0, 0.0]);
        let bumped = slide_move(&mut ps, &pml(false), &Wall { x: 3.0 }, HULL, None);
        assert!(bumped);
        // The trace stops a hair short of the wall.
        assert!((ps.origin[0] - 3.0).abs() < 5e-3, "{:?}", ps.origin);
        // 50 units a second for the whole 0.05 s frame, in y.
        assert!((ps.origin[1] - 2.5).abs() < 1e-3, "{:?}", ps.origin);
        assert!(
            ps.velocity[0] <= 0.0 && ps.velocity[0] > -1.0,
            "{:?}",
            ps.velocity
        );
    }

    #[test]
    fn right_after_a_landing_a_wall_does_not_slow_the_player() {
        let mut ps = running([0.0; 3], [100.0, 50.0, 0.0]);
        ps.pm_time = 100;
        slide_move(&mut ps, &pml(false), &Wall { x: 3.0 }, HULL, None);
        assert_eq!(ps.velocity, [100.0, 50.0, 0.0]);
    }

    #[test]
    fn nothing_in_the_way_reports_no_bump() {
        let mut ps = running([0.0; 3], [100.0, 0.0, 0.0]);
        assert!(!slide_move(&mut ps, &pml(false), &OpenAir, HULL, None));
        assert!((ps.origin[0] - 5.0).abs() < 1e-3);
    }

    #[test]
    fn gravity_pulls_through_the_frame_at_the_mean_speed() {
        let mut ps = running([0.0; 3], [0.0; 3]);
        slide_move(&mut ps, &pml(false), &OpenAir, HULL, Some(800.0));
        // Falls at half of the frame's end speed on average: 20 a second for
        // 0.05 s, and ends the frame at 40 a second.
        assert!((ps.origin[2] + 1.0).abs() < 1e-3, "{:?}", ps.origin);
        assert!((ps.velocity[2] + 40.0).abs() < 1e-3, "{:?}", ps.velocity);
    }

    #[test]
    fn a_velocity_into_a_surface_loses_it_and_is_pushed_slightly_off() {
        let out = clip_velocity([100.0, 50.0, 0.0], [-1.0, 0.0, 0.0]);
        assert!(out[0] < 0.0 && out[0] > -0.2, "{out:?}");
        assert_eq!(out[1], 50.0);
    }

    #[test]
    fn a_run_up_a_slope_is_turned_along_it_at_the_same_speed() {
        let mut velocity = [100.0, 0.0, 0.0];
        project_velocity(&mut velocity, &[-0.5, 0.0, 0.866_025_4]);
        assert!(velocity[2] > 0.0);
        let speed = libm::sqrtf(velocity[0] * velocity[0] + velocity[2] * velocity[2]);
        assert!((speed - 100.0).abs() < 1e-2, "{speed}");
    }

    #[test]
    fn flat_ground_leaves_the_velocity_alone() {
        let mut velocity = [100.0, 20.0, 0.0];
        project_velocity(&mut velocity, &[0.0, 0.0, 1.0]);
        assert_eq!(velocity, [100.0, 20.0, 0.0]);
    }

    #[test]
    fn a_fall_onto_an_upward_slope_is_not_sped_up() {
        let mut velocity = [100.0, 0.0, -200.0];
        project_velocity(&mut velocity, &[-0.5, 0.0, 0.866_025_4]);
        assert_eq!(velocity, [100.0, 0.0, -200.0]);
    }

    #[test]
    fn the_planes_the_velocity_runs_into_hardest_come_first_and_a_tie_goes_to_the_later() {
        let planes = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        ];
        let mut order = [0usize; MAX_PLANES];
        // Running at +x: the second plane is into it, the others are not.
        let into = rank_planes([10.0, 0.0, 0.0], &planes, &mut order);
        assert_eq!(into, -10.0);
        assert_eq!(order[0], 1);
        // The two planes it does not touch tie at 0, and the later one is first.
        assert_eq!(&order[1..3], &[3, 2]);
    }
}
