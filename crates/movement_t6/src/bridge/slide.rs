//! Player movement through the world: gliding along whatever is hit, and
//! climbing or descending stairs.

use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::{
    AvatarState, BODY_ENTITY_FIRST_SLOT, NO_ENTITY_SLOT, movement_state_bits,
};
use trace_t6::Trace;

use crate::bridge::{SweepAsk, SolidProbe, TickLocals, leap};

const STILL: [f32; 3] = [0.0; 3];

/// Player flag set on slick ground.
const PMF_SLIDING: u32 = 0x40_0000;

/// After clipping, the velocity is nudged a bit beyond the surface.
const BACKOFF_SCALE: f32 = 1.001;

/// A surface whose normal has less z than this is not used for projecting.
const LEVEL_PLANE_Z_TOLERANCE: f32 = 0.001;

/// Capacity of the surface list kept by one move, and the number of bumps it
/// may take.
const MAX_PLANES: usize = 8;
const MAX_BUMPS: usize = 4;

/// Normals with a dot product above this are taken to be the same surface.
const SAME_PLANE_DOT: f32 = 0.999;

/// Moving into a surface by at least this much (negative dot) halts it.
const INTO_PLANE_LIMIT: f32 = 0.1;

/// Highest stair for a standing player and for a crawling one.
const STAIR_HEIGHT: f32 = 18.0;
const CRAWL_STAIR_HEIGHT: f32 = 10.0;

/// The upward sweep reaches this much beyond the stair it hunts for.
const LIFT_MARGIN: f32 = 1.0;

/// Ground whose normal z is below this counts as steep enough to step from.
const STEEP_GROUND_Z: f32 = 0.9;

/// A player on foot searches this far further down for the floor.
const STICK_TO_GROUND_REACH: f32 = 9.0;

/// A stair that gains no more than this in the heading's direction is undone.
const MIN_FORWARD_GAIN: f32 = 0.001;

/// Width used when checking that a lying player still fits.
const PRONE_RECHECK_WIDTH: f32 = 15.0;

/// Entities numbered below this are players (corpses begin here).
const MAX_CLIENTS: u16 = BODY_ENTITY_FIRST_SLOT as u16;

/// A rise no bigger than this is ignored entirely.
const MIN_COUNTED_RISE: f32 = 0.5;

/// Added before a rise is rounded to whole units: 2^-30.
const RISE_ROUND_BIAS: f64 = 1.0 / 1_073_741_824.0;

/// Speed left after a full-height stair; a lower one slows the player by its
/// height over the stair height, spread over this range above the floor value.
const SLOW_FLOOR: f32 = f32::from_bits(0x3e4c_cccc);
const SLOW_SPAN: f32 = 0.8;

/// A rise over three units advances the walk cycle: per beat (two units, up
/// to four beats) this much, plus a fixed amount.
const BOB_PER_BEAT: f32 = 1.25;
const BOB_FIXED: f32 = 7.0;

/// The player's collision box.
#[derive(Clone, Copy)]
struct Hull {
    lower: [f32; 3],
    upper: [f32; 3],
    collide_filter: u32,
}

impl Hull {
    fn sweep<C: SolidProbe>(self, world: &C, src: [f32; 3], dst: [f32; 3]) -> Trace {
        world.trace(SweepAsk {
            start: src,
            end: dst,
            lo_corner: self.lower,
            hi_corner: self.upper,
            collide_filter: self.collide_filter,
        })
    }
}

/// Position and motion saved before a stair was tried.
#[derive(Clone, Copy)]
struct Shot {
    place: [f32; 3],
    motion: [f32; 3],
}

impl Shot {
    fn of(avatar: &AvatarState) -> Self {
        Self {
            place: avatar.origin,
            motion: avatar.velocity,
        }
    }

    fn restore(self, avatar: &mut AvatarState) {
        avatar.origin = self.place;
        avatar.velocity = self.motion;
    }
}

/// Surface surface_directions met during a move, capped at `MAX_PLANES`.
struct Planes {
    surface_directions: [[f32; 3]; MAX_PLANES],
    count: usize,
}

impl Planes {
    fn empty() -> Self {
        Self {
            surface_directions: [STILL; MAX_PLANES],
            count: 0,
        }
    }

    fn is_full(&self) -> bool {
        self.count >= MAX_PLANES
    }

    fn stack_plane(&mut self, toward: [f32; 3]) {
        self.surface_directions[self.count] = toward;
        self.count += 1;
    }

    fn as_slice(&self) -> &[[f32; 3]] {
        &self.surface_directions[..self.count]
    }

    /// Has a surface facing this way been recorded already?
    fn has_same_as(&self, toward: [f32; 3]) -> bool {
        self.as_slice()
            .iter()
            .any(|known| inner(toward, *known) > SAME_PLANE_DOT)
    }
}

/// Normal of the floor found by the latest ground trace.
fn ground_normal(tick_vars: &TickLocals) -> [f32; 3] {
    core::array::from_fn(|axis| f32::from_bits(tick_vars.floor_probe[axis + 1]))
}

/// Gravity for the frame: the velocity used for moving is the mean of start
/// and end, and the end value is kept aside. Returns (end velocity, velocity
/// as it is before any bump).
fn fall_through_frame(avatar: &mut AvatarState, tick_vars: &TickLocals, pull: Option<f32>) -> ([f32; 3], [f32; 3]) {
    let mut at_end = avatar.velocity;
    let mut at_entry = avatar.velocity;
    if let Some(strength) = pull {
        at_end[2] -= strength * tick_vars.step_millis_elapsed;
        at_entry[2] = at_end[2];
        avatar.velocity[2] = (avatar.velocity[2] + at_end[2]) * 0.5;
        if tick_vars.standing_on_floor != 0 {
            avatar.velocity = clip_velocity(avatar.velocity, ground_normal(tick_vars));
        }
    }
    (at_end, at_entry)
}

/// What a single leg of the move ran into.
enum Leg {
    /// Started inside something solid.
    Wedged,
    /// Got all the way.
    Free,
    /// Stopped short by this.
    Hit(Trace),
}

/// Travel along the velocity for `countdown_rest`, moving the origin as far as it
/// goes.
fn travel<C: SolidProbe>(
    avatar: &mut AvatarState,
    world: &C,
    hull: Hull,
    countdown_rest: f32,
) -> Leg {
    let from = avatar.origin;
    let goal = add_scaled(from, avatar.velocity, countdown_rest);
    let swept = hull.sweep(world, from, goal);
    let whole_way = swept.fraction >= 1.0;
    if swept.fraction < 1.0 {
        world.brush_against(&swept, swept.finish_location(from, goal));
    }
    if swept.fully_embedded {
        let [vx, vy, _] = avatar.velocity;
        avatar.velocity = [vx, vy, 0.0];
        return Leg::Wedged;
    }
    let moved_at_all = swept.fraction > 0.0;
    match (whole_way, moved_at_all) {
        (true, _) => {
            avatar.origin = goal;
            Leg::Free
        }
        (false, true) => {
            avatar.origin = swept.finish_location(from, goal);
            Leg::Hit(swept)
        }
        (false, false) => Leg::Hit(swept),
    }
}

/// Move for the rest of the frame, bumping against up to four surfaces. With
/// `pull` set, gravity acts through the frame and the velocity finishes lower.
/// Returns whether anything was hit.
fn slide_move<C: SolidProbe>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    world: &C,
    hull: Hull,
    pull: Option<f32>,
) -> bool {
    let falls = pull.is_some();
    let (mut final_velocity, entry_velocity) = fall_through_frame(avatar, tick_vars, pull);

    let mut planes = Planes::empty();
    if tick_vars.standing_on_floor != 0 {
        planes.stack_plane(ground_normal(tick_vars));
    }
    let mut heading = avatar.velocity;
    to_unit(&mut heading);
    planes.stack_plane(heading);

    let mut countdown_rest = tick_vars.step_millis_elapsed;
    let mut met_surface = false;
    for _ in 0..MAX_BUMPS {
        if countdown_rest <= 0.0 {
            break;
        }
        let trace = match travel(avatar, world, hull, countdown_rest) {
            Leg::Wedged => return true,
            Leg::Free => break,
            Leg::Hit(trace) => trace,
        };

        met_surface = true;
        countdown_rest *= 1.0 - trace.fraction;
        if planes.is_full() {
            avatar.velocity = STILL;
            return true;
        }
        if planes.has_same_as(trace.normal) {
            // Hit the same surface again: shove away from it slightly, go on.
            avatar.velocity = clip_velocity(avatar.velocity, trace.normal);
            avatar.velocity = add(avatar.velocity, trace.normal);
            continue;
        }
        planes.stack_plane(trace.normal);

        let mut ranking: [usize; MAX_PLANES] = [0; MAX_PLANES];
        let depth = rank_planes(avatar.velocity, planes.as_slice(), &mut ranking);
        if depth < INTO_PLANE_LIMIT {
            match follow_creases(avatar.velocity, final_velocity, planes.as_slice(), &ranking) {
                Creased::Slide { speed_now: velocity, speed_at_end: end } => {
                    avatar.velocity = velocity;
                    final_velocity = end;
                }
                Creased::Stuck => {
                    avatar.velocity = STILL;
                    return true;
                }
            }
        }
    }

    if falls {
        avatar.velocity = final_velocity;
    }
    // Right after landing, a wall redirects the player without costing speed.
    let just_landed = avatar.move_timer != 0;
    if just_landed {
        avatar.velocity = entry_velocity;
    }
    met_surface
}

/// What is left of the velocity between the surfaces the move is wedged in.
enum Creased {
    /// It slides along them: this fast now, this fast when the frame ends.
    Slide { speed_now: [f32; 3], speed_at_end: [f32; 3] },
    /// There is no way out.
    Stuck,
}

/// Slide along the most restrictive surface; when that pushes the player
/// into another one, slide along the crease between the two, and give up if
/// a third closes that off as well. `order` lists the planes with the most
/// restrictive first.
fn follow_creases(speed: [f32; 3], speed_at_end: [f32; 3], surfaces: &[[f32; 3]], ranking: &[usize]) -> Creased {
    let primary = surfaces[ranking[0]];
    let mut now = clip_velocity(speed, primary);
    let mut end = clip_velocity(speed_at_end, primary);
    for j in 1..surfaces.len() {
        let other_plane = surfaces[ranking[j]];
        if inner(now, other_plane) >= INTO_PLANE_LIMIT {
            continue;
        }
        now = clip_velocity(now, other_plane);
        end = clip_velocity(end, other_plane);
        if inner(now, primary) >= 0.0 {
            continue;
        }

        let mut seam = cross_product(primary, other_plane);
        to_unit(&mut seam);
        now = scaled(seam, inner(speed, seam));
        end = scaled(seam, inner(speed_at_end, seam));
        let sealed = (1..surfaces.len())
            .filter(|&k| k != j)
            .any(|k| inner(now, surfaces[ranking[k]]) < INTO_PLANE_LIMIT);
        if sealed {
            return Creased::Stuck;
        }
    }
    Creased::Slide { speed_now: now, speed_at_end: end }
}

/// Fill `order` with plane indices, the plane the velocity runs hardest into
/// first (on a tie the later plane wins). Returns the smallest dot product,
/// which is how deep the velocity runs into that plane.
fn rank_planes(speed: [f32; 3], surfaces: &[[f32; 3]], ranking: &mut [usize]) -> f32 {
    let mut depth = [0.0_f32; MAX_PLANES];
    for (index, surface) in surfaces.iter().enumerate() {
        depth[index] = inner(speed, *surface);
        let mut slot = index;
        while slot > 0 && depth[index] <= depth[ranking[slot - 1]] {
            ranking[slot] = ranking[slot - 1];
            slot -= 1;
        }
        ranking[slot] = index;
    }
    depth[ranking[0]]
}

/// Standing height limit for a stair, lower while crawling.
fn stair_limit(avatar: &AvatarState) -> f32 {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 {
        CRAWL_STAIR_HEIGHT
    } else {
        STAIR_HEIGHT
    }
}

/// Move for one frame, climbing any stairs in the way. Returns how much a
/// stair raised (or lowered) the player, for the camera to ease over; 0 when
/// no stair counted.
#[expect(clippy::too_many_arguments, reason = "single entry for a whole move")]
pub fn step_slide_move<C: SolidProbe>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    world: &C,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    collide_filter: u32,
    pull: Option<f32>,
    zombies: bool,
) -> f32 {
    let hull = Hull {
        lower: box_lo,
        upper: box_hi,
        collide_filter,
    };
    let was_footed = started_on_ground(avatar, tick_vars);
    let before_move = Shot::of(avatar);

    let bumped = slide_move(avatar, tick_vars, world, hull, pull);
    let flat = Shot::of(avatar);

    // A last-stand player glides, without stepping, as long as nothing is hit.
    if !bumped
        && avatar.movement_mode == movement_mode::LASTSTAND
        && avatar.movement_state_bits & PMF_SLIDING != 0
    {
        return 0.0;
    }

    let usual_size = stair_limit(avatar);
    let (step_size, jump_step) = if avatar.standing_on == NO_ENTITY_SLOT {
        match airborne_step(avatar, bumped, before_move.place, usual_size) {
            Some(pair) => pair,
            None => return 0.0,
        }
    } else {
        (usual_size, false)
    };

    let steep_floor =
        tick_vars.standing_on_floor != 0 && f32::from_bits(tick_vars.floor_probe[3]) < STEEP_GROUND_Z;
    let rise = if bumped || steep_floor {
        try_step_up(avatar, tick_vars, world, hull, pull, before_move, step_size)
    } else {
        0.0
    };

    if (was_footed || rise != 0.0) && !settle_down(avatar, world, hull, flat, was_footed, rise) {
        return 0.0;
    }

    // A stair that got no further along the heading than the flat move did,
    // or a jump's stair that ended at the jump's top, is undone.
    let stepped = forward_gain(before_move, avatar.origin);
    if stepped <= forward_gain(before_move, flat.place) + MIN_FORWARD_GAIN
        || (jump_step && leap::above_max(avatar))
    {
        undo_stair(avatar, world, hull, flat, was_footed);
    }

    if jump_step {
        leap::clamp_velocity(avatar, flat.place);
    }
    if was_footed
        && avatar.movement_mode < movement_mode::DEAD
        && prone_still_fits(avatar, world, before_move, zombies)
    {
        stair_step(avatar, tick_vars, before_move.place, flat.place, step_size)
    } else {
        0.0
    }
}

/// How far the move got along the starting heading (x and y only).
fn forward_gain(before_move: Shot, at: [f32; 3]) -> f32 {
    before_move.motion[0] * (at[0] - before_move.place[0]) + before_move.motion[1] * (at[1] - before_move.place[1])
}

/// After a stair attempt, put the player back on the floor beneath. The stair
/// is taken back (false) if it lands on another player or on a face that is
/// too steep to stand on.
fn settle_down<C: SolidProbe>(
    avatar: &mut AvatarState,
    world: &C,
    hull: Hull,
    flat: Shot,
    was_footed: bool,
    rise: f32,
) -> bool {
    let mut descent = rise;
    if was_footed {
        descent += STICK_TO_GROUND_REACH;
    }
    let goal = lowered(avatar.origin, descent);
    let below = hull.sweep(world, avatar.origin, goal);
    if trace_t6::trace_get_entity_hit_id(&below) < MAX_CLIENTS {
        flat.restore(avatar);
        return false;
    }
    if below.fraction >= 1.0 {
        if rise != 0.0 {
            avatar.origin[2] -= rise;
        }
    } else if below.walkable {
        avatar.origin = below.finish_location(avatar.origin, goal);
        project_velocity(&mut avatar.velocity, &below.normal);
    } else {
        flat.restore(avatar);
        return false;
    }
    true
}

/// Go back to the flat move's result, and for a walking player drop to the
/// floor under it.
fn undo_stair<C: SolidProbe>(
    avatar: &mut AvatarState,
    world: &C,
    hull: Hull,
    flat: Shot,
    was_footed: bool,
) {
    flat.restore(avatar);
    if !was_footed {
        return;
    }
    let goal = lowered(avatar.origin, STICK_TO_GROUND_REACH);
    let below = hull.sweep(world, avatar.origin, goal);
    if below.fraction < 1.0 {
        avatar.origin = below.finish_location(avatar.origin, goal);
        // Speed carries along the slope.
        project_velocity(&mut avatar.velocity, &below.normal);
    }
}

/// Did the player begin the move on the ground (and not on a ladder)? Getting
/// off a ladder, or a jump whose time ran out, clears the jump state.
fn started_on_ground(avatar: &mut AvatarState, tick_vars: &TickLocals) -> bool {
    if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        leap::clear_state(avatar);
        return false;
    }
    let on_floor = tick_vars.standing_on_floor != 0;
    if !on_floor {
        forget_timed_jump(avatar);
    }
    on_floor
}

/// A jump with time still on its clock is forgotten.
fn forget_timed_jump(avatar: &mut AvatarState) {
    if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer != 0 {
        leap::clear_state(avatar);
    }
}

/// May a player in the air step, and by how much? A jump that bumped steps up
/// to the height the jump allows; a player climbing a ladder steps as usual.
/// Returns the stair size and whether it is a jump's stair.
fn airborne_step(
    avatar: &mut AvatarState,
    bumped: bool,
    from: [f32; 3],
    usual_size: f32,
) -> Option<(f32, bool)> {
    forget_timed_jump(avatar);
    let jumping = avatar.movement_state_bits & movement_state_bits::JUMPING != 0;
    let climbing =
        avatar.movement_state_bits & movement_state_bits::LADDER != 0 && avatar.velocity[2] > 0.0;
    if !(bumped && jumping) {
        return climbing.then_some((usual_size, false));
    }
    match leap::step_room(avatar, from[2]) {
        Some(height) if height < 1.0 => None,
        Some(height) => Some((height, true)),
        None if climbing => Some((usual_size, false)),
        None => None,
    }
}

/// Lift the player over whatever the flat move hit and slide on from there.
/// Returns the rise, or 0 when there was no room.
fn try_step_up<C: SolidProbe>(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    world: &C,
    hull: Hull,
    pull: Option<f32>,
    before_move: Shot,
    step_size: f32,
) -> f32 {
    let ceiling = raised(raised(before_move.place, step_size), LIFT_MARGIN);
    let lift = hull.sweep(world, before_move.place, ceiling);
    let rise = (step_size + LIFT_MARGIN) * lift.fraction - LIFT_MARGIN;
    if rise < 1.0 {
        return 0.0;
    }
    avatar.origin = raised(before_move.place, rise);
    avatar.velocity = before_move.motion;
    slide_move(avatar, tick_vars, world, hull, pull);
    rise
}

/// After walking, a lying player needs room to lie. Without it the player
/// goes back to where the move began, unless that fails too, in which case
/// the move stands. False when the move was taken back.
fn prone_still_fits<C: SolidProbe>(
    avatar: &mut AvatarState,
    world: &C,
    before_move: Shot,
    zombies: bool,
) -> bool {
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT == 0
        || prone_fits(avatar, world, zombies)
    {
        return true;
    }
    let arrived = Shot::of(avatar);
    before_move.restore(avatar);
    if prone_fits(avatar, world, zombies) {
        return false;
    }
    arrived.restore(avatar);
    true
}

/// Does the lying body fit at the player's position? Pitches found on the way
/// are stored.
fn prone_fits<C: SolidProbe>(avatar: &mut AvatarState, world: &C, zombies: bool) -> bool {
    let rules = crate::bridge::ProneRules::for_player(avatar, zombies);
    crate::bridge::check_prone(
        world,
        avatar.origin,
        PRONE_RECHECK_WIDTH,
        crate::bridge::BELLY_CLEARANCE_PROBE,
        avatar.prone_direction,
        true,
        true,
        true,
        rules,
        crate::bridge::BELLY_FOOT_REACH,
        Some(&mut avatar.f_torso_pitch),
        Some(&mut avatar.f_waist_pitch),
    )
}

/// A stair on the ground slows the player by its height (a full stair leaves
/// a fifth of the speed); more than three units also advances the walk cycle,
/// which may produce a footstep. Returns the rise, 0 when it does not count.
fn stair_step(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    from: [f32; 3],
    flat_at: [f32; 3],
    step_size: f32,
) -> f32 {
    let rise = avatar.origin[2] - flat_at[2];
    if libm::fabsf(rise) <= MIN_COUNTED_RISE {
        return 0.0;
    }
    let whole = libm::rint(f64::from(rise) + RISE_ROUND_BIAS) as i32;
    if whole == 0 {
        return 0.0;
    }

    let slow = (1.0 - libm::fabsf(avatar.origin[2] - from[2]) / step_size) * SLOW_SPAN + SLOW_FLOOR;
    for part in &mut avatar.velocity {
        *part *= slow;
    }

    let size = whole.abs();
    if size > 3 && avatar.standing_on != NO_ENTITY_SLOT {
        let beats = (size / 2).min(4);
        let cycle_before = avatar.sway_phase;
        avatar.sway_phase =
            ((beats as f32 * BOB_PER_BEAT + BOB_FIXED + cycle_before as f32) as i32) & 0xff;
        let cycle_after = avatar.sway_phase;
        let audible = crate::bridge::should_make_footsteps(avatar);
        let _ = crate::bridge::footstep_event(
            avatar,
            cycle_before as u8,
            cycle_after as u8,
            tick_vars.floor_probe[4],
            audible,
        );
    }
    rise
}

/// Turn a velocity so it runs along a surface at the same speed, except when
/// that would only speed up a fall onto an upward slope.
pub(crate) fn project_velocity(velocity: &mut [f32; 3], surface: &[f32; 3]) {
    let flat_sq = velocity[0] * velocity[0] + velocity[1] * velocity[1];
    if libm::fabsf(surface[2]) < LEVEL_PLANE_Z_TOLERANCE || flat_sq == 0.0 {
        return;
    }
    let z = -(surface[1] * velocity[1] + surface[0] * velocity[0]) / surface[2];
    let scale = libm::sqrtf((velocity[2] * velocity[2] + flat_sq) / (z * z + flat_sq));
    if scale >= 1.0 && z >= 0.0 && velocity[2] <= 0.0 {
        return;
    }
    velocity[0] *= scale;
    velocity[1] *= scale;
    velocity[2] = z * scale;
}

/// A velocity with the part running into the surface removed (and slightly
/// more, so it does not rest on it).
fn clip_velocity(speed: [f32; 3], surface: [f32; 3]) -> [f32; 3] {
    let depth = inner(speed, surface);
    let backoff = -(depth - libm::fabsf(depth) * (BACKOFF_SCALE - 1.0));
    add_scaled(speed, surface, backoff)
}

/// `base + step * amount`, per component.
fn add_scaled(base: [f32; 3], step: [f32; 3], amount: f32) -> [f32; 3] {
    core::array::from_fn(|axis| base[axis] + step[axis] * amount)
}

fn add(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|axis| lhs[axis] + rhs[axis])
}

/// The point moved up by `by`.
fn raised(point: [f32; 3], by: f32) -> [f32; 3] {
    [point[0], point[1], point[2] + by]
}

/// The point moved down by `by`.
fn lowered(point: [f32; 3], by: f32) -> [f32; 3] {
    [point[0], point[1], point[2] - by]
}

fn inner(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

fn cross_product(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scaled(v: [f32; 3], by: f32) -> [f32; 3] {
    core::array::from_fn(|axis| v[axis] * by)
}

/// Rescale to length 1; the zero vector stays as it is.
fn to_unit(v: &mut [f32; 3]) {
    let length = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let factor = if length <= 0.0 { 1.0 } else { 1.0 / length };
    for part in v.iter_mut() {
        *part *= factor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;
    use trace_t6::ENTITYNUM_WORLD;
    use trace_t6::TraceHitType;

    const HULL: Hull = Hull {
        lower: [-15.0, -15.0, 0.0],
        upper: [15.0, 15.0, 70.0],
        collide_filter: 1,
    };

    /// Open space over a floor at z = 0; the floor reports `struck_entity_num` and is
    /// tilted to `normal_z` (walkable from 0.7).
    struct Floor {
        struck_entity_num: u16,
        normal_z: f32,
    }

    impl Floor {
        fn level() -> Self {
            Self {
                struck_entity_num: ENTITYNUM_WORLD,
                normal_z: 1.0,
            }
        }
    }

    impl SolidProbe for Floor {
        fn trace(&self, input: SweepAsk) -> Trace {
            let mut met = Trace::new();
            let from = input.start[2] + input.lo_corner[2];
            let to = input.end[2] + input.lo_corner[2];
            if from >= 0.0 && to < 0.0 {
                met.fraction = from / (from - to);
                met.normal = [
                    libm::sqrtf(1.0 - self.normal_z * self.normal_z),
                    0.0,
                    self.normal_z,
                ];
                met.walkable = self.normal_z >= 0.7;
                met.impact_category = TraceHitType::Entity;
                met.struck_entity_num = self.struck_entity_num;
            }
            met
        }
    }

    /// A wall across x = `x` facing -x, and nothing else.
    struct Wall {
        x: f32,
    }

    impl SolidProbe for Wall {
        fn trace(&self, input: SweepAsk) -> Trace {
            let mut met = Trace::new();
            if input.start[0] < self.x && input.end[0] > self.x {
                met.fraction = (self.x - input.start[0]) / (input.end[0] - input.start[0]);
                met.normal = [-1.0, 0.0, 0.0];
            }
            met
        }
    }

    /// Nothing in the way anywhere.
    struct OpenAir;

    impl SolidProbe for OpenAir {
        fn trace(&self, _input: SweepAsk) -> Trace {
            Trace::new()
        }
    }

    fn running(at: [f32; 3], motion: [f32; 3]) -> AvatarState {
        AvatarState {
            origin: at,
            velocity: motion,
            ..AvatarState::ZERO
        }
    }

    /// Walk off a 5-unit drop: the height the player ends at.
    fn walk_off(floor: &Floor, last_stand: bool) -> f32 {
        let mut avatar = running([0.0, 0.0, 5.0], [100.0, 0.0, 0.0]);
        avatar.standing_on = i32::from(ENTITYNUM_WORLD);
        if last_stand {
            avatar.movement_mode = movement_mode::LASTSTAND;
            avatar.movement_state_bits |= PMF_SLIDING;
        }
        step_slide_move(
            &mut avatar,
            &tick_vars(true),
            floor,
            HULL.lower,
            HULL.upper,
            1,
            None,
            false,
        );
        avatar.origin[2]
    }

    #[test]
    fn a_step_down_onto_another_player_is_taken_back() {
        assert_eq!(walk_off(&Floor::level(), false), 0.0);
        assert_eq!(
            walk_off(
                &Floor {
                    struck_entity_num: 3,
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
                    struck_entity_num: 26,
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
                    struck_entity_num: ENTITYNUM_WORLD,
                    normal_z: 0.5
                },
                false
            ),
            5.0
        );
    }

    #[test]
    fn a_wall_stops_the_run_into_it_and_the_rest_slides_along() {
        let mut avatar = running([0.0; 3], [100.0, 50.0, 0.0]);
        let bumped = slide_move(&mut avatar, &tick_vars(false), &Wall { x: 3.0 }, HULL, None);
        assert!(bumped);
        // The trace stops a hair short of the wall.
        assert!((avatar.origin[0] - 3.0).abs() < 5e-3, "{:?}", avatar.origin);
        // 50 units a second for the whole 0.05 s frame, in y.
        assert!((avatar.origin[1] - 2.5).abs() < 1e-3, "{:?}", avatar.origin);
        assert!(
            avatar.velocity[0] <= 0.0 && avatar.velocity[0] > -1.0,
            "{:?}",
            avatar.velocity
        );
    }

    #[test]
    fn right_after_a_landing_a_wall_does_not_slow_the_player() {
        let mut avatar = running([0.0; 3], [100.0, 50.0, 0.0]);
        avatar.move_timer = 100;
        slide_move(&mut avatar, &tick_vars(false), &Wall { x: 3.0 }, HULL, None);
        assert_eq!(avatar.velocity, [100.0, 50.0, 0.0]);
    }

    #[test]
    fn nothing_in_the_way_reports_no_bump() {
        let mut avatar = running([0.0; 3], [100.0, 0.0, 0.0]);
        assert!(!slide_move(&mut avatar, &tick_vars(false), &OpenAir, HULL, None));
        assert!((avatar.origin[0] - 5.0).abs() < 1e-3);
    }

    #[test]
    fn gravity_pulls_through_the_frame_at_the_mean_speed() {
        let mut avatar = running([0.0; 3], [0.0; 3]);
        slide_move(&mut avatar, &tick_vars(false), &OpenAir, HULL, Some(800.0));
        // Falls at half of the frame's end speed on average: 20 a second for
        // 0.05 s, and ends the frame at 40 a second.
        assert!((avatar.origin[2] + 1.0).abs() < 1e-3, "{:?}", avatar.origin);
        assert!((avatar.velocity[2] + 40.0).abs() < 1e-3, "{:?}", avatar.velocity);
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
        let (east, west, up, north) = (
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        );
        let planes = [east, west, up, north];
        let mut ranking: [usize; MAX_PLANES] = [0; MAX_PLANES];
        // Running at +x: the second plane is into it, the others are not.
        let into = rank_planes([10.0, 0.0, 0.0], &planes, &mut ranking);
        assert_eq!(into, -10.0);
        assert_eq!(ranking[0], 1);
        // The two planes it does not touch tie at 0, and the later one is first.
        assert_eq!(&ranking[1..3], &[3, 2]);
    }
}
