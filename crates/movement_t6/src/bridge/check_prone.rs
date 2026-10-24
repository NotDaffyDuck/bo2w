//! Whether a prone body fits where the player stands, and how it bends.
//!
//! The body is three joints in a line: the torso (the player's origin), the
//! waist and the feet. The waist and the feet are found by sweeping a small
//! sphere along the body and down onto the ground. The two segments between
//! the joints give the pitches the animation needs; a bend that is too sharp,
//! or a segment with something in it, means the player cannot lie down here.

use game_math::{angle_subtract, yaw_vectors_2d};
use playerstate_t6::consts::pm_type;
use playerstate_t6::game::PlayerState;
use trace_t6::Trace;

use crate::bridge::{CollisionBackend, GroundTraceInput};

/// The height of the hull checked for room to lie down.
pub const PRONE_CHECK_HEIGHT: f32 = 30.0;

/// How far from the player the feet are looked for.
pub const PRONE_FEET_DIST: f32 = 50.0;

/// What a lying body cannot pass through.
const BODY_MASK: u32 = 0x0081_0011;

/// Radius of the sphere swept along the body.
const LIMB_RADIUS: f32 = 6.0;

/// How far up the hull is moved to see whether a standing player has room.
const ROOM_LIFT: f32 = 10.0;

/// The body lies behind the way the player faces.
const FACING_FLIP: f32 = 180.0;

/// The waist is this far along the body.
const WAIST_ALONG: f32 = 12.0;

/// The body is this long in last stand (it lies in front), and in Zombies.
const LAST_STAND_REACH: f32 = -0.75;
const LAST_STAND_REACH_ZOMBIES: f32 = -0.5;

/// A sweep that stops the feet closer than this much past the player's own
/// size is refused.
const FEET_CLEARANCE_PAD: f32 = 2.0;

/// A blocked sweep shorter than this fraction of the sweep height (plus the
/// waist's distance) is raised and tried again.
const FEET_SHORT_FRACTION: f32 = 0.7;
const FEET_SHORT_RAISE: f32 = 22.0;

/// How far below the sweep the waist looks for ground: this many player
/// sizes on the ground, one in the air.
const GROUNDED_WAIST_DEPTH: f32 = 2.5;

/// The waist and the blocked feet must not be this much out of order.
const WAIST_FEET_REJECT: f32 = -0.75;

/// A retry of a blocked far sweep goes this much higher.
const FAR_RETRY_LIFT: f32 = 18.0;

/// The bend between torso and waist may be this much (degrees), in Zombies
/// last stand this much further the other way.
const BEND_MIN: f32 = -50.0;
const BEND_MIN_ZOMBIES_LAST_STAND: f32 = -65.0;
const BEND_MAX: f32 = 70.0;

/// The lines from joint to joint are checked this far above the joints.
const LINE_LIFT: f32 = 5.0;

/// Legs running downhill by less than this may touch the ground.
const DOWNHILL_FORGIVEN: f32 = -20.0;

/// Radians to degrees, pitched the way the engine reads it (up is negative).
const RAD_TO_PITCH: f32 = -180.0 / core::f32::consts::PI;

/// What decides how the body lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProneRules {
    /// In last stand the body lies in front, its pitches flipped.
    pub last_stand: bool,

    /// Zombies' last-stand numbers.
    pub zombies: bool,

    /// Diving through the air: the body reads flat.
    pub diving: bool,
}

impl ProneRules {
    /// The rules for this player.
    #[must_use]
    pub fn for_player(ps: &PlayerState, zombies: bool) -> Self {
        Self {
            last_stand: ps.pm_type == pm_type::LASTSTAND,
            zombies,
            diving: crate::bridge::dive::in_air(ps),
        }
    }
}

/// The player's own measurements, as the check receives them.
struct Body {
    origin: [f32; 3],
    size: f32,
    height: f32,
    yaw: f32,
    on_ground: bool,
}

/// The body's measurements along its own line.
struct Shape {
    origin: [f32; 3],
    size: f32,
    on_ground: bool,
    /// Signed distance from the player to the waist.
    waist_along: f32,
    /// Signed distance from the player to the feet.
    feet_dist: f32,
    /// How high above the origin the body is swept.
    sweep_height: f32,
}

impl Shape {
    /// How far the feet sweep runs: the feet distance less the sphere.
    fn along(&self) -> f32 {
        self.feet_dist - LIMB_RADIUS
    }
}

/// Why the body could not be laid out.
enum Setback {
    /// The body cannot lie here and nothing is reported.
    Refused,
    /// The shape did not work out: a player on the ground is refused, one in
    /// the air is let down flat.
    Unfit,
}

/// How the body bends, and whether it fits.
struct Bend {
    torso: f32,
    waist: f32,
    fits: bool,
}

/// The sweep from the shoulders toward the feet.
struct FeetSweep {
    start: [f32; 3],
    end: [f32; 3],
    fraction: f32,
    /// The direction of the body (the slanted one after a raised retry).
    forward: [f32; 3],
    /// How far along the body the sweep got, when something stopped it.
    reached: Option<f32>,
}

/// Where the waist found the ground.
struct Waist {
    start: [f32; 3],
    point: [f32; 3],
    reach: f32,
}

/// Can a body lie down at `origin` facing `yaw`? A lying body (`already_prone`)
/// skips the check for room above. The pitches of the torso and the waist go
/// to the slots given; a player in the air whose body does not fit is let down
/// flat (both pitches 0) and allowed.
#[expect(
    clippy::too_many_arguments,
    reason = "one call boundary for the prone check"
)]
#[must_use]
pub fn check_prone<C: CollisionBackend>(
    collision: &C,
    origin: [f32; 3],
    f_size: f32,
    f_height: f32,
    f_yaw: f32,
    already_prone: bool,
    on_ground: bool,
    ground_walkable: bool,
    rules: ProneRules,
    prone_feet_dist: f32,
    mut torso_pitch: Option<&mut f32>,
    mut waist_pitch: Option<&mut f32>,
) -> bool {
    if !already_prone {
        let room = collision.trace(GroundTraceInput {
            start: origin,
            end: [origin[0], origin[1], origin[2] + ROOM_LIFT],
            mins: [-f_size, -f_size, 0.0],
            maxs: [f_size, f_size, f_height],
            tracemask: BODY_MASK,
        });
        if room.allsolid {
            return false;
        }
    }
    if on_ground && !ground_walkable {
        return false;
    }

    let body = Body {
        origin,
        size: f_size,
        height: f_height,
        yaw: f_yaw,
        on_ground,
    };
    match lay_body(collision, &body, rules, prone_feet_dist) {
        Err(Setback::Refused) => false,
        Err(Setback::Unfit) => let_down_flat(on_ground, torso_pitch, waist_pitch),
        Ok(bend) => {
            if let Some(slot) = torso_pitch.as_deref_mut() {
                *slot = bend.torso;
            }
            if let Some(slot) = waist_pitch.as_deref_mut() {
                *slot = bend.waist;
            }
            bend.fits || let_down_flat(on_ground, torso_pitch, waist_pitch)
        }
    }
}

/// A body that does not fit: refused on the ground, laid flat in the air.
fn let_down_flat(
    on_ground: bool,
    torso_pitch: Option<&mut f32>,
    waist_pitch: Option<&mut f32>,
) -> bool {
    if on_ground {
        return false;
    }
    if let Some(slot) = torso_pitch {
        *slot = 0.0;
    }
    if let Some(slot) = waist_pitch {
        *slot = 0.0;
    }
    true
}

/// Find the waist and the feet, then measure how the body bends over them.
fn lay_body<C: CollisionBackend>(
    world: &C,
    body: &Body,
    rules: ProneRules,
    feet_dist: f32,
) -> Result<Bend, Setback> {
    let ProneRules {
        last_stand,
        zombies,
        diving,
    } = rules;
    let reach = match (last_stand, zombies) {
        (false, _) => 1.0,
        (true, false) => LAST_STAND_REACH,
        (true, true) => LAST_STAND_REACH_ZOMBIES,
    };
    let bend_min = if last_stand && zombies {
        BEND_MIN_ZOMBIES_LAST_STAND
    } else {
        BEND_MIN
    };
    let (facing, _) = yaw_vectors_2d(body.yaw - FACING_FLIP);
    let shape = Shape {
        origin: body.origin,
        size: body.size,
        on_ground: body.on_ground,
        waist_along: reach * WAIST_ALONG,
        feet_dist: reach * feet_dist,
        sweep_height: body.height - LIMB_RADIUS,
    };

    let sweep = sweep_feet(world, &shape, [facing[0], facing[1], 0.0])?;
    let waist = find_waist(world, &shape, sweep.forward)?;
    let feet = aim_feet(world, &shape, &sweep, &waist)?;
    let feet = ground_feet(world, &shape, feet, waist.point)?;
    Ok(measure_bend(
        world,
        &shape,
        waist.point,
        feet,
        bend_min,
        last_stand,
        diving,
    ))
}

/// Sweep the sphere from the shoulders along the body. Something in the way
/// close to the player refuses the body; something a little further is
/// retried along a line raised at the far end.
fn sweep_feet<C: CollisionBackend>(
    world: &C,
    shape: &Shape,
    mut forward: [f32; 3],
) -> Result<FeetSweep, Setback> {
    let along = shape.along();
    let start = [
        shape.origin[0],
        shape.origin[1],
        shape.origin[2] + shape.sweep_height,
    ];
    let mut end = [
        start[0] + along * forward[0],
        start[1] + along * forward[1],
        start[2] + along * forward[2],
    ];
    let mut trace = limb_sweep(world, start, end);
    if trace.fraction >= 1.0 {
        return Ok(FeetSweep {
            start,
            end,
            fraction: trace.fraction,
            forward,
            reached: None,
        });
    }
    if !shape.on_ground {
        return Err(Setback::Refused);
    }

    let reached = along * trace.fraction + LIMB_RADIUS;
    if reached < shape.size + FEET_CLEARANCE_PAD {
        return Err(Setback::Refused);
    }
    let short = shape.sweep_height * FEET_SHORT_FRACTION + shape.waist_along;
    if reached < short {
        end[2] += FEET_SHORT_RAISE;
        forward = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let slanted_len = normalize(&mut forward);
        trace = limb_sweep(world, start, end);
        if trace.fraction >= 1.0 {
            return Ok(FeetSweep {
                start,
                end,
                fraction: trace.fraction,
                forward,
                reached: None,
            });
        }
        let retry = trace.fraction * slanted_len + LIMB_RADIUS;
        if retry < short {
            return Err(Setback::Refused);
        }
        return Ok(FeetSweep {
            start,
            end,
            fraction: trace.fraction,
            forward,
            reached: Some(retry),
        });
    }
    Ok(FeetSweep {
        start,
        end,
        fraction: trace.fraction,
        forward,
        reached: Some(reached),
    })
}

/// Drop the sphere onto the ground where the waist is.
fn find_waist<C: CollisionBackend>(
    world: &C,
    shape: &Shape,
    forward: [f32; 3],
) -> Result<Waist, Setback> {
    let depth = if shape.on_ground {
        GROUNDED_WAIST_DEPTH
    } else {
        1.0
    };
    let start = [
        shape.origin[0] + shape.waist_along * forward[0],
        shape.origin[1] + shape.waist_along * forward[1],
        shape.origin[2] + shape.sweep_height + shape.waist_along * forward[2],
    ];
    let down = depth * shape.size + shape.sweep_height - LIMB_RADIUS;
    let end = [start[0], start[1], start[2] - down];
    let trace = limb_sweep(world, start, end);
    if trace.fraction == 1.0 {
        return Err(Setback::Unfit);
    }
    if !trace.walkable {
        return Err(Setback::Refused);
    }
    let mut point = lerp(start, end, trace.fraction);
    point[2] -= LIMB_RADIUS;
    Ok(Waist {
        start,
        point,
        reach: down * trace.fraction + LIMB_RADIUS,
    })
}

/// Where the feet are. A sweep nothing stopped ends at its far end; a stopped
/// one is aimed again from the waist toward where it got, and that second
/// sweep (raised once if it too is stopped) decides.
fn aim_feet<C: CollisionBackend>(
    world: &C,
    shape: &Shape,
    sweep: &FeetSweep,
    waist: &Waist,
) -> Result<[f32; 3], Setback> {
    let stopped_feet = lerp(sweep.start, sweep.end, sweep.fraction);
    let Some(reached) = sweep.reached else {
        return Ok(stopped_feet);
    };
    if reached - waist.reach < waist.reach * WAIST_FEET_REJECT {
        return Err(Setback::Unfit);
    }

    let forward = sweep.forward;
    let mut toward = [
        stopped_feet[0] - waist.point[0],
        stopped_feet[1] - waist.point[1],
        stopped_feet[2] - waist.point[2],
    ];
    toward[0] += LIMB_RADIUS * forward[0];
    toward[1] += LIMB_RADIUS * forward[1];
    toward[2] += LIMB_RADIUS * forward[2] + LIMB_RADIUS;
    normalize(&mut toward);

    let along = shape.along();
    let length = along - shape.waist_along;
    let mut from = waist.start;
    let mut to = [
        from[0] + length * toward[0],
        from[1] + length * toward[1],
        from[2] + length * toward[2],
    ];
    to[0] = (along * forward[0] + shape.origin[0] + to[0]) * 0.5;
    to[1] = (along * forward[1] + shape.origin[1] + to[1]) * 0.5;
    let mut trace = limb_sweep(world, from, to);
    if trace.fraction < 1.0 {
        from = lerp(from, to, trace.fraction);
        from[2] += FAR_RETRY_LIFT;
        to[2] += FAR_RETRY_LIFT;
        trace = limb_sweep(world, from, to);
        if trace.fraction < 1.0 {
            return Err(Setback::Unfit);
        }
    }
    Ok(lerp(from, to, trace.fraction))
}

/// Drop the sphere onto the ground under the feet.
fn ground_feet<C: CollisionBackend>(
    world: &C,
    shape: &Shape,
    feet: [f32; 3],
    waist: [f32; 3],
) -> Result<[f32; 3], Setback> {
    let drop = (feet[2] - waist[2]) + (feet[2] - waist[2]) + shape.size;
    let to = [feet[0], feet[1], feet[2] - drop];
    let trace = limb_sweep(world, feet, to);
    if trace.fraction == 1.0 {
        return Err(Setback::Unfit);
    }
    if !trace.walkable {
        return Err(Setback::Refused);
    }
    let mut settled = lerp(feet, to, trace.fraction);
    settled[2] -= LIMB_RADIUS;
    Ok(settled)
}

/// Pitch the two segments, check the bend between them and look along them
/// for anything in the way.
fn measure_bend<C: CollisionBackend>(
    world: &C,
    shape: &Shape,
    waist: [f32; 3],
    feet: [f32; 3],
    bend_min: f32,
    last_stand: bool,
    diving: bool,
) -> Bend {
    let torso = shape.origin;
    let to_torso = [
        torso[0] - waist[0],
        torso[1] - waist[1],
        torso[2] - waist[2],
    ];
    let to_waist = [waist[0] - feet[0], waist[1] - feet[1], waist[2] - feet[2]];
    let torso_pitch = level(pitch_of(to_torso));
    let mut waist_pitch = level(pitch_of(to_waist));

    let bend = angle_subtract(torso_pitch, waist_pitch);
    let mut fits = (bend_min..=BEND_MAX).contains(&bend);

    if line_blocked(world, torso, waist) {
        fits = false;
    }
    if line_blocked(world, waist, feet) {
        // Legs a little downhill may touch the ground; the waist reads flat.
        if !(DOWNHILL_FORGIVEN < waist_pitch && waist_pitch < 0.0) {
            fits = false;
        }
        waist_pitch = 0.0;
    }

    let (mut torso, mut waist) = (torso_pitch, waist_pitch);
    if last_stand {
        torso = -torso;
        waist = -waist;
    }
    if diving {
        torso = 0.0;
        waist = 0.0;
    }
    Bend { torso, waist, fits }
}

/// Is anything on the thin line between two joints, a little above them?
fn line_blocked<C: CollisionBackend>(world: &C, from: [f32; 3], to: [f32; 3]) -> bool {
    let lifted = |p: [f32; 3]| [p[0], p[1], p[2] + LINE_LIFT];
    let nothing = [0.0; 3];
    let trace = world.trace(GroundTraceInput {
        start: lifted(from),
        end: lifted(to),
        mins: nothing,
        maxs: nothing,
        tracemask: BODY_MASK,
    });
    trace.fraction < 1.0
}

/// Sweep the limb sphere between two points.
fn limb_sweep<C: CollisionBackend>(world: &C, start: [f32; 3], end: [f32; 3]) -> Trace {
    world.trace(GroundTraceInput {
        start,
        end,
        mins: [-LIMB_RADIUS; 3],
        maxs: [LIMB_RADIUS; 3],
        tracemask: BODY_MASK,
    })
}

fn lerp(from: [f32; 3], to: [f32; 3], fraction: f32) -> [f32; 3] {
    [
        from[0] + fraction * (to[0] - from[0]),
        from[1] + fraction * (to[1] - from[1]),
        from[2] + fraction * (to[2] - from[2]),
    ]
}

/// Scale to length 1 and return the length it had; a zero vector stays zero.
fn normalize(v: &mut [f32; 3]) -> f32 {
    let len = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let inverse = 1.0 / if len <= 0.0 { 1.0 } else { len };
    for component in v.iter_mut() {
        *component *= inverse;
    }
    len
}

/// The pitch (degrees, 0 to 360, up is past 180) of a direction.
fn pitch_of(v: [f32; 3]) -> f32 {
    if v[0] == 0.0 && v[1] == 0.0 {
        return if v[2] > 0.0 { 270.0 } else { 90.0 };
    }
    let flat = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    let pitch = libm::atan2f(v[2], flat) * RAD_TO_PITCH;
    if pitch < 0.0 { pitch + 360.0 } else { pitch }
}

/// Fold a pitch into -180..180.
fn level(pitch: f32) -> f32 {
    angle_subtract(pitch, 0.0)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::cell::RefCell;
    use std::vec::Vec;

    use super::*;

    /// Ground at `floor_z + slope * x`. Straight-down sweeps land on it; a
    /// wall stops sweeps heading toward -x; legs and the ceiling can be made
    /// to be in the way.
    struct Hill {
        floor_z: f32,
        slope: f32,
        legs_blocked: bool,
        low_ceiling: bool,
        wall_x: f32,
        sweeps: RefCell<Vec<GroundTraceInput>>,
    }

    impl Hill {
        fn flat() -> Self {
            Self {
                floor_z: 0.0,
                slope: 0.0,
                legs_blocked: false,
                low_ceiling: false,
                wall_x: -1.0e9,
                sweeps: RefCell::new(Vec::new()),
            }
        }

        fn sloped(slope: f32) -> Self {
            Self {
                slope,
                ..Self::flat()
            }
        }
    }

    impl CollisionBackend for Hill {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            self.sweeps.borrow_mut().push(input);
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            if input.mins == [0.0; 3] && input.maxs == [0.0; 3] {
                // Only the line from the waist to the feet starts behind the
                // player.
                if self.legs_blocked && input.start[0] < -1.0 {
                    hit.fraction = 0.5;
                }
                return hit;
            }
            if self.low_ceiling && input.mins[0] == -15.0 {
                hit.allsolid = true;
                return hit;
            }
            let straight_down = input.start[0] == input.end[0] && input.start[1] == input.end[1];
            if !straight_down {
                if input.start[0] > self.wall_x && input.end[0] < self.wall_x {
                    hit.fraction = (input.start[0] - self.wall_x) / (input.start[0] - input.end[0]);
                }
                return hit;
            }
            let ground = self.floor_z + self.slope * input.start[0];
            let bottom_start = input.start[2] + input.mins[2];
            let bottom_end = input.end[2] + input.mins[2];
            if bottom_start >= ground && bottom_end < ground {
                hit.fraction = (bottom_start - ground) / (bottom_start - bottom_end);
                hit.normal = [0.0, 0.0, 1.0];
                hit.walkable = true;
            }
            hit
        }
    }

    const PLAIN: ProneRules = ProneRules {
        last_stand: false,
        zombies: false,
        diving: false,
    };

    /// Lie down at the origin, facing +x: (fits, torso pitch, waist pitch).
    /// A pitch the check never wrote stays NaN.
    fn lie(world: &Hill, rules: ProneRules, on_ground: bool) -> (bool, f32, f32) {
        let (mut torso, mut waist) = (f32::NAN, f32::NAN);
        let fits = check_prone(
            world,
            [0.0; 3],
            15.0,
            PRONE_CHECK_HEIGHT,
            0.0,
            false,
            on_ground,
            true,
            rules,
            PRONE_FEET_DIST,
            Some(&mut torso),
            Some(&mut waist),
        );
        (fits, torso, waist)
    }

    /// The x the waist sweep dropped at.
    fn waist_x(world: &Hill) -> f32 {
        let sweeps = world.sweeps.borrow();
        sweeps
            .iter()
            .find(|s| s.mins[0] == -LIMB_RADIUS && s.end[2] < s.start[2])
            .expect("a sweep down for the waist")
            .start[0]
    }

    /// The x the sweep along the body ended at.
    fn feet_x(world: &Hill) -> f32 {
        let sweeps = world.sweeps.borrow();
        sweeps
            .iter()
            .find(|s| s.mins[0] == -LIMB_RADIUS && s.end[2] == s.start[2])
            .expect("a sweep along the body")
            .end[0]
    }

    #[test]
    fn a_body_on_flat_ground_lies_level() {
        let (fits, torso, waist) = lie(&Hill::flat(), PLAIN, true);
        assert!(fits);
        assert!(torso.abs() < 1e-3 && waist.abs() < 1e-3, "{torso} {waist}");
    }

    #[test]
    fn the_waist_sits_twelve_units_behind() {
        let world = Hill::sloped(0.2);
        let (fits, torso, waist) = lie(&world, PLAIN, true);
        assert!(fits);
        assert!((waist_x(&world) + 12.0).abs() < 1e-4);
        // Up the slope behind, so both pitches read the slope's angle.
        assert!((torso + 11.309_932).abs() < 1e-3, "{torso}");
        assert!((waist + 11.309_932).abs() < 1e-3, "{waist}");
    }

    #[test]
    fn legs_a_little_downhill_may_touch_the_ground_and_read_flat() {
        let (fits, torso, waist) = lie(
            &Hill {
                legs_blocked: true,
                ..Hill::sloped(0.2)
            },
            PLAIN,
            true,
        );
        assert!(fits);
        assert!((torso + 11.309_932).abs() < 1e-3, "{torso}");
        assert_eq!(waist, 0.0);

        let (fits, _, _) = lie(
            &Hill {
                legs_blocked: true,
                ..Hill::sloped(-0.2)
            },
            PLAIN,
            true,
        );
        assert!(!fits, "legs running uphill into the ground do not fit");
    }

    #[test]
    fn last_stand_lies_in_front_three_quarters_as_long() {
        for (zombies, reach) in [(false, -0.75_f32), (true, -0.5)] {
            let world = Hill::sloped(0.2);
            let rules = ProneRules {
                last_stand: true,
                zombies,
                diving: false,
            };
            let (fits, torso, waist) = lie(&world, rules, true);
            assert!(fits);
            assert!((waist_x(&world) + reach * 12.0).abs() < 1e-4);
            assert!((feet_x(&world) + (reach * PRONE_FEET_DIST - 6.0)).abs() < 1e-4);
            // Seen from the other side, the pitches come back flipped.
            assert!((torso + 11.309_932).abs() < 1e-3, "{torso}");
            assert!((waist + 11.309_932).abs() < 1e-3, "{waist}");
        }
    }

    #[test]
    fn a_dive_reads_flat() {
        let rules = ProneRules {
            diving: true,
            ..PLAIN
        };
        assert_eq!(lie(&Hill::sloped(0.2), rules, true), (true, 0.0, 0.0));
    }

    #[test]
    fn a_wall_right_behind_the_player_refuses_and_reports_nothing() {
        let world = Hill {
            wall_x: -10.0,
            ..Hill::flat()
        };
        let (fits, torso, waist) = lie(&world, PLAIN, true);
        assert!(!fits);
        assert!(torso.is_nan() && waist.is_nan());
    }

    #[test]
    fn nothing_under_the_waist_refuses_on_the_ground_and_lets_a_falling_player_down_flat() {
        let world = Hill {
            floor_z: -1000.0,
            ..Hill::flat()
        };
        let (fits, torso, _) = lie(&world, PLAIN, true);
        assert!(!fits);
        assert!(torso.is_nan());
        assert_eq!(lie(&world, PLAIN, false), (true, 0.0, 0.0));
    }

    #[test]
    fn a_standing_player_needs_room_above_but_a_lying_one_does_not() {
        let world = Hill {
            low_ceiling: true,
            ..Hill::flat()
        };
        assert!(!lie(&world, PLAIN, true).0);

        let already_prone = check_prone(
            &world,
            [0.0; 3],
            15.0,
            PRONE_CHECK_HEIGHT,
            0.0,
            true,
            true,
            true,
            PLAIN,
            PRONE_FEET_DIST,
            None,
            None,
        );
        assert!(already_prone);
    }

    #[test]
    fn ground_that_is_too_steep_to_stand_on_refuses_before_any_sweep() {
        let world = Hill::flat();
        let fits = check_prone(
            &world,
            [0.0; 3],
            15.0,
            PRONE_CHECK_HEIGHT,
            0.0,
            true,
            true,
            false,
            PLAIN,
            PRONE_FEET_DIST,
            None,
            None,
        );
        assert!(!fits);
        assert!(world.sweeps.borrow().is_empty());
    }

    #[test]
    fn the_rules_come_from_the_player() {
        let mut ps = PlayerState::ZERO;
        assert!(!ProneRules::for_player(&ps, true).last_stand);
        ps.pm_type = pm_type::LASTSTAND;
        let rules = ProneRules::for_player(&ps, true);
        assert!(rules.last_stand && rules.zombies);
    }
}
