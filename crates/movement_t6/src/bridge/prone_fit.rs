//! Can a prone body fit here, and how does it bend? Answered by the BO2 rules.
//!
//! A prone body is modelled as three joints on one line: shoulders (the
//! player's origin), hips and feet. Hips and feet are located with a small
//! sphere swept out along the body and then down to the floor. The two
//! segments between joints supply the pitches the animation uses; a fold that
//! is too sharp, or any obstruction inside a segment, rules the position out.

use game_math::degree_gap;
use game_math::flat_basis;
use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::AvatarState;
use trace_t6::Trace;

use crate::bridge::{SolidProbe, SweepAsk};

/// Hull height used when testing whether there is space to go prone.
pub const BELLY_CLEARANCE_PROBE: f32 = 30.0;

/// Distance from the player at which the feet are searched for.
pub const BELLY_FOOT_REACH: f32 = 50.0;

/// Solid types that block a prone body.
const SOLID_MASK: u32 = 0x0081_0011;

/// Radius of the ball dragged along the body.
const SWEEP_SPHERE: f32 = 6.0;

/// Upward shift of the hull for the headroom test.
const HEADROOM_LIFT: f32 = 10.0;

/// The body trails the view direction, so the heading turns half a circle.
const HALF_TURN: f32 = 180.0;

/// Hips sit this far along the body.
const HIP_DISTANCE: f32 = 12.0;

/// Body-length multiplier in last stand, where the body points forward;
/// the Zombies value is shorter.
const LAST_STAND_LENGTH: f32 = -0.75;
const LAST_STAND_LENGTH_ZOMBIES: f32 = -0.5;

/// A feet sweep halted within the player's size plus this much is rejected.
const NEAR_STOP_PAD: f32 = 2.0;

/// A halted sweep covering less than this share of the sweep height (plus the
/// hip distance) is lifted and run once more.
const SHORT_SHARE: f32 = 0.7;
const SHORT_LIFT: f32 = 22.0;

/// Number of player sizes the hip ball may drop on the ground; one when
/// airborne.
const HIP_DROP_SIZES: f32 = 2.5;

/// Hips and a halted foot sweep may not disagree by more than this share.
const HIP_FOOT_DISAGREE: f32 = -0.75;

/// Lift applied when the far leg sweep is blocked and retried.
const LEG_RETRY_LIFT: f32 = 18.0;

/// Smallest allowed shoulder-to-hip fold in degrees; Zombies last stand may
/// fold further the other way. The most is `FOLD_MAX`.
const FOLD_MIN: f32 = -50.0;
const FOLD_MIN_ZOMBIES_LAST_STAND: f32 = -65.0;
const FOLD_MAX: f32 = 70.0;

/// Joint-to-joint lines are tested this far above the joints.
const SPAN_LIFT: f32 = 5.0;

/// Legs sloping down by less than this are allowed to touch the ground.
const SLOPE_ALLOWANCE: f32 = -20.0;

/// Converts radians to engine pitch degrees (up reads negative).
const RADIANS_TO_PITCH: f32 = -180.0 / core::f32::consts::PI;

/// Inputs that decide how the body lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProneRules {
    /// The body lies out in front, with its pitches flipped, in last stand.
    pub last_stand: bool,

    /// Use the Zombies last-stand figures.
    pub zombies: bool,

    /// Mid-dive: report the body flat.
    pub diving: bool,
}

impl ProneRules {
    /// Rules matching this player.
    #[must_use]
    pub fn for_player(avatar: &AvatarState, zombies: bool) -> Self {
        Self {
            last_stand: avatar.movement_mode == movement_mode::LASTSTAND,
            zombies,
            diving: crate::bridge::dive::dive_in_air(avatar),
        }
    }
}

/// Player measurements handed to the check.
struct Posture {
    at: [f32; 3],
    girth: f32,
    tall: f32,
    heading: f32,
    grounded: bool,
}

/// Distances along the body's own line.
struct Layout {
    at: [f32; 3],
    girth: f32,
    grounded: bool,
    /// Signed distance from the player to the hips.
    hip_dist: f32,
    /// Signed distance from the player to the feet.
    foot_dist: f32,
    /// Height above the origin at which the body is swept.
    sweep_z: f32,
}

impl Layout {
    /// Length of the foot sweep: the foot distance minus the ball radius.
    fn run(&self) -> f32 {
        self.foot_dist - SWEEP_SPHERE
    }
}

/// Reasons the body could not be placed.
enum Failure {
    /// Lying is impossible here; nothing is reported.
    Refuse,
    /// The geometry failed: grounded players are refused, airborne ones are
    /// laid flat.
    FlatDown,
}

/// Resulting fold, and whether it fits.
struct Fold {
    upper: f32,
    lower: f32,
    ok: bool,
}

/// The sweep from the shoulders towards the feet.
struct LegSweep {
    from: [f32; 3],
    to: [f32; 3],
    clear: f32,
    /// Body direction (the slanted one after a lifted retry).
    heading: [f32; 3],
    /// How far along the body the sweep went, if it was stopped.
    got_to: Option<f32>,
}

/// Where the hips found the floor.
struct Hips {
    from: [f32; 3],
    spot: [f32; 3],
    extent: f32,
}

/// Tests whether a body can lie at `origin` facing `yaw`. With `was_lying_flat`
/// the headroom test is skipped. Torso and waist pitches are written to the
/// given slots; an airborne body that fails to fit is laid flat (both pitches
/// zero) and accepted.
#[expect(
    clippy::too_many_arguments,
    reason = "one call boundary for the prone check"
)]
#[must_use]
pub fn check_prone<C: SolidProbe>(
    solids: &C,
    at_pos: [f32; 3],
    size: f32,
    height: f32,
    yaw: f32,
    was_lying_flat: bool,
    grounded: bool,
    walkable: bool,
    rules: ProneRules,
    feet_dist: f32,
    mut torso_out: Option<&mut f32>,
    mut waist_out: Option<&mut f32>,
) -> bool {
    if !was_lying_flat && headroom_missing(solids, at_pos, size, height) {
        return false;
    }
    if grounded && !walkable {
        return false;
    }

    let posture = Posture {
        at: at_pos,
        girth: size,
        tall: height,
        heading: yaw,
        grounded,
    };
    match arrange_body(solids, &posture, rules, feet_dist) {
        Err(Failure::Refuse) => false,
        Err(Failure::FlatDown) => flatten(grounded, torso_out, waist_out),
        Ok(fold) => {
            if let Some(out) = torso_out.as_deref_mut() {
                *out = fold.upper;
            }
            if let Some(out) = waist_out.as_deref_mut() {
                *out = fold.lower;
            }
            fold.ok || flatten(grounded, torso_out, waist_out)
        }
    }
}

/// Whether the hull, lifted a little, starts inside solid.
fn headroom_missing<C: SolidProbe>(
    solids: &C,
    spot: [f32; 3],
    size: f32,
    height: f32,
) -> bool {
    solids
        .trace(SweepAsk {
            start: spot,
            end: raised(spot, HEADROOM_LIFT),
            lo_corner: [-size, -size, 0.0],
            hi_corner: [size, size, height],
            collide_filter: SOLID_MASK,
        })
        .fully_embedded
}

/// For a body that does not fit: grounded players are refused, airborne ones
/// are laid flat.
fn flatten(grounded: bool, torso_out: Option<&mut f32>, waist_out: Option<&mut f32>) -> bool {
    if grounded {
        return false;
    }
    if let Some(out) = torso_out {
        *out = 0.0;
    }
    if let Some(out) = waist_out {
        *out = 0.0;
    }
    true
}

/// Locate hips and feet, then measure the fold over them.
fn arrange_body<C: SolidProbe>(
    world: &C,
    posture: &Posture,
    rules: ProneRules,
    feet_dist: f32,
) -> Result<Fold, Failure> {
    let ProneRules {
        last_stand,
        zombies,
        diving,
    } = rules;
    let length = match (last_stand, zombies) {
        (false, _) => 1.0,
        (true, false) => LAST_STAND_LENGTH,
        (true, true) => LAST_STAND_LENGTH_ZOMBIES,
    };
    let fold_floor = match (last_stand, zombies) {
        (true, true) => FOLD_MIN_ZOMBIES_LAST_STAND,
        _ => FOLD_MIN,
    };
    let (flat_dir, _) = flat_basis(posture.heading - HALF_TURN);
    let layout = Layout {
        at: posture.at,
        girth: posture.girth,
        grounded: posture.grounded,
        hip_dist: length * HIP_DISTANCE,
        foot_dist: length * feet_dist,
        sweep_z: posture.tall - SWEEP_SPHERE,
    };

    let leg = sweep_legs(world, &layout, [flat_dir[0], flat_dir[1], 0.0])?;
    let hips = drop_hips(world, &layout, leg.heading)?;
    let aimed = aim_feet(world, &layout, &leg, &hips)?;
    let feet = settle_feet(world, &layout, aimed, hips.spot)?;
    Ok(measure_fold(
        world, &layout, hips.spot, feet, fold_floor, last_stand, diving,
    ))
}

/// Package a finished leg sweep.
fn leg_sweep(
    from: [f32; 3],
    to: [f32; 3],
    clear: f32,
    heading: [f32; 3],
    got_to: Option<f32>,
) -> LegSweep {
    LegSweep {
        from,
        to,
        clear,
        heading,
        got_to,
    }
}

/// Drag the ball from the shoulders along the body. A block close to the
/// player rules the body out; one further off is retried on a line lifted at
/// the far end.
fn sweep_legs<C: SolidProbe>(
    world: &C,
    layout: &Layout,
    heading: [f32; 3],
) -> Result<LegSweep, Failure> {
    let run = layout.run();
    let from = raised(layout.at, layout.sweep_z);
    let to = mad(from, run, heading);
    let strike = ball_sweep(world, from, to);
    if strike.fraction >= 1.0 {
        return Ok(leg_sweep(from, to, strike.fraction, heading, None));
    }
    if !layout.grounded {
        return Err(Failure::Refuse);
    }

    let got_to = run * strike.fraction + SWEEP_SPHERE;
    if got_to < layout.girth + NEAR_STOP_PAD {
        return Err(Failure::Refuse);
    }
    let minimum = layout.sweep_z * SHORT_SHARE + layout.hip_dist;
    if got_to < minimum {
        return lifted_retry(world, from, to, minimum);
    }
    Ok(leg_sweep(from, to, strike.fraction, heading, Some(got_to)))
}

/// Re-run a short leg sweep with its far end raised.
fn lifted_retry<C: SolidProbe>(
    world: &C,
    from: [f32; 3],
    mut to: [f32; 3],
    minimum: f32,
) -> Result<LegSweep, Failure> {
    to[2] += SHORT_LIFT;
    let mut heading = sub3(to, from);
    let slant = scale_to_unit(&mut heading);
    let strike = ball_sweep(world, from, to);
    if strike.fraction >= 1.0 {
        return Ok(leg_sweep(from, to, strike.fraction, heading, None));
    }
    let second = strike.fraction * slant + SWEEP_SPHERE;
    if second < minimum {
        return Err(Failure::Refuse);
    }
    Ok(leg_sweep(from, to, strike.fraction, heading, Some(second)))
}

/// Lower the ball onto the floor at the hips.
fn drop_hips<C: SolidProbe>(
    world: &C,
    layout: &Layout,
    heading: [f32; 3],
) -> Result<Hips, Failure> {
    let sizes = if layout.grounded { HIP_DROP_SIZES } else { 1.0 };
    let from = mad(raised(layout.at, layout.sweep_z), layout.hip_dist, heading);
    let fall = sizes * layout.girth + layout.sweep_z - SWEEP_SPHERE;
    let to = [from[0], from[1], from[2] - fall];
    let strike = ball_sweep(world, from, to);
    if strike.fraction == 1.0 {
        return Err(Failure::FlatDown);
    }
    if !strike.walkable {
        return Err(Failure::Refuse);
    }
    let mut spot = lerp(from, to, strike.fraction);
    spot[2] -= SWEEP_SPHERE;
    Ok(Hips {
        from,
        spot,
        extent: fall * strike.fraction + SWEEP_SPHERE,
    })
}

/// Where the feet end up. A sweep nothing stopped finishes at its far end; a
/// stopped one is re-aimed from the hips towards the stopping point, and that
/// second sweep (lifted once if it is blocked too) settles it.
fn aim_feet<C: SolidProbe>(
    world: &C,
    layout: &Layout,
    leg: &LegSweep,
    hips: &Hips,
) -> Result<[f32; 3], Failure> {
    let stop_point = lerp(leg.from, leg.to, leg.clear);
    let Some(got_to) = leg.got_to else {
        return Ok(stop_point);
    };
    if got_to - hips.extent < hips.extent * HIP_FOOT_DISAGREE {
        return Err(Failure::FlatDown);
    }

    let heading = leg.heading;
    let nudge = [
        SWEEP_SPHERE * heading[0],
        SWEEP_SPHERE * heading[1],
        SWEEP_SPHERE * heading[2] + SWEEP_SPHERE,
    ];
    let mut aim = add3(sub3(stop_point, hips.spot), nudge);
    scale_to_unit(&mut aim);

    let run = layout.run();
    let span = run - layout.hip_dist;
    let mut from = hips.from;
    let mut to = mad(from, span, aim);
    to[0] = (run * heading[0] + layout.at[0] + to[0]) * 0.5;
    to[1] = (run * heading[1] + layout.at[1] + to[1]) * 0.5;
    let mut strike = ball_sweep(world, from, to);
    if strike.fraction < 1.0 {
        from = lerp(from, to, strike.fraction);
        from[2] += LEG_RETRY_LIFT;
        to[2] += LEG_RETRY_LIFT;
        strike = ball_sweep(world, from, to);
        if strike.fraction < 1.0 {
            return Err(Failure::FlatDown);
        }
    }
    Ok(lerp(from, to, strike.fraction))
}

/// Lower the ball onto the floor under the feet.
fn settle_feet<C: SolidProbe>(
    world: &C,
    layout: &Layout,
    feet: [f32; 3],
    hip_spot: [f32; 3],
) -> Result<[f32; 3], Failure> {
    let rise = feet[2] - hip_spot[2];
    let fall = rise + rise + layout.girth;
    let to = [feet[0], feet[1], feet[2] - fall];
    let strike = ball_sweep(world, feet, to);
    if strike.fraction == 1.0 {
        return Err(Failure::FlatDown);
    }
    if !strike.walkable {
        return Err(Failure::Refuse);
    }
    let mut spot = lerp(feet, to, strike.fraction);
    spot[2] -= SWEEP_SPHERE;
    Ok(spot)
}

/// Pitch both segments, bound the fold between them, and look along each for
/// obstructions.
fn measure_fold<C: SolidProbe>(
    world: &C,
    layout: &Layout,
    hips: [f32; 3],
    feet: [f32; 3],
    fold_floor: f32,
    last_stand: bool,
    diving: bool,
) -> Fold {
    let shoulders = layout.at;
    let upper_pitch = level(pitch_of(sub3(shoulders, hips)));
    let mut lower_pitch = level(pitch_of(sub3(hips, feet)));

    let fold = degree_gap(upper_pitch, lower_pitch);
    let mut ok = (fold_floor..=FOLD_MAX).contains(&fold);

    if span_blocked(world, shoulders, hips) {
        ok = false;
    }
    if span_blocked(world, hips, feet) {
        // A shallow downhill in the legs may rest on the ground; hips go flat.
        if !(SLOPE_ALLOWANCE < lower_pitch && lower_pitch < 0.0) {
            ok = false;
        }
        lower_pitch = 0.0;
    }

    let (mut upper, mut lower) = (upper_pitch, lower_pitch);
    if last_stand {
        upper = -upper;
        lower = -lower;
    }
    if diving {
        upper = 0.0;
        lower = 0.0;
    }
    Fold { upper, lower, ok }
}

/// Whether anything sits on the thin line between two joints, lifted a bit.
fn span_blocked<C: SolidProbe>(world: &C, a: [f32; 3], b: [f32; 3]) -> bool {
    let nothing = [0.0; 3];
    let strike = world.trace(SweepAsk {
        start: raised(a, SPAN_LIFT),
        end: raised(b, SPAN_LIFT),
        lo_corner: nothing,
        hi_corner: nothing,
        collide_filter: SOLID_MASK,
    });
    strike.fraction < 1.0
}

/// Drag the ball from `start` to `end`.
fn ball_sweep<C: SolidProbe>(world: &C, from_pt: [f32; 3], to_pt: [f32; 3]) -> Trace {
    world.trace(SweepAsk {
        start: from_pt,
        end: to_pt,
        lo_corner: [-SWEEP_SPHERE; 3],
        hi_corner: [SWEEP_SPHERE; 3],
        collide_filter: SOLID_MASK,
    })
}

/// `p` with its height increased by `dz`.
fn raised(p: [f32; 3], dz: f32) -> [f32; 3] {
    [p[0], p[1], p[2] + dz]
}

/// `base + scale * dir`, per component.
fn mad(base: [f32; 3], scale: f32, dir: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| base[i] + scale * dir[i])
}

/// Component-wise `a + b`.
fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| a[i] + b[i])
}

/// Component-wise `a - b`.
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| a[i] - b[i])
}

/// Linear blend from `a` to `b`.
fn lerp(p_from: [f32; 3], p_to: [f32; 3], k: f32) -> [f32; 3] {
    core::array::from_fn(|i| p_from[i] + k * (p_to[i] - p_from[i]))
}

/// Make `v` unit length and return its old length; a zero vector is kept.
fn scale_to_unit(v: &mut [f32; 3]) -> f32 {
    let magnitude = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let scale = 1.0 / if magnitude <= 0.0 { 1.0 } else { magnitude };
    for part in v.iter_mut() {
        *part *= scale;
    }
    magnitude
}

/// Pitch of a direction in degrees, 0 to 360, with up beyond 180.
fn pitch_of(v: [f32; 3]) -> f32 {
    if v[0] == 0.0 && v[1] == 0.0 {
        return if v[2] > 0.0 { 270.0 } else { 90.0 };
    }
    let horizontal = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    let degrees = libm::atan2f(v[2], horizontal) * RADIANS_TO_PITCH;
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees
    }
}

/// Wrap a pitch to the range -180 to 180.
fn level(pitch: f32) -> f32 {
    degree_gap(pitch, 0.0)
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
        sweeps: RefCell<Vec<SweepAsk>>,
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

    impl SolidProbe for Hill {
        fn trace(&self, input: SweepAsk) -> Trace {
            self.sweeps.borrow_mut().push(input);
            let mut strike = Trace::new();
            if input.lo_corner == [0.0; 3] && input.hi_corner == [0.0; 3] {
                // Only the line from the waist to the feet starts behind the
                // player.
                if self.legs_blocked && input.start[0] < -1.0 {
                    strike.fraction = 0.5;
                }
                return strike;
            }
            if self.low_ceiling && input.lo_corner[0] == -15.0 {
                strike.fully_embedded = true;
                return strike;
            }
            let straight_down = input.start[0] == input.end[0] && input.start[1] == input.end[1];
            if !straight_down {
                if input.start[0] > self.wall_x && input.end[0] < self.wall_x {
                    strike.fraction = (input.start[0] - self.wall_x) / (input.start[0] - input.end[0]);
                }
                return strike;
            }
            let ground = self.floor_z + self.slope * input.start[0];
            let bottom_start = input.start[2] + input.lo_corner[2];
            let bottom_end = input.end[2] + input.lo_corner[2];
            if bottom_start >= ground && bottom_end < ground {
                strike.fraction = (bottom_start - ground) / (bottom_start - bottom_end);
                strike.normal = [0.0, 0.0, 1.0];
                strike.walkable = true;
            }
            strike
        }
    }

    const PLAIN: ProneRules = ProneRules {
        last_stand: false,
        zombies: false,
        diving: false,
    };

    /// Lie down at the origin, facing +x: (fits, torso pitch, waist pitch).
    /// A pitch the check never wrote stays NaN.
    fn lie(world: &Hill, rules: ProneRules, grounded_now: bool) -> (bool, f32, f32) {
        let (mut torso, mut waist) = (f32::NAN, f32::NAN);
        let fits = check_prone(
            world,
            [0.0; 3],
            15.0,
            BELLY_CLEARANCE_PROBE,
            0.0,
            false,
            grounded_now,
            true,
            rules,
            BELLY_FOOT_REACH,
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
            .find(|s| s.lo_corner[0] == -SWEEP_SPHERE && s.end[2] < s.start[2])
            .expect("a sweep down for the waist")
            .start[0]
    }

    /// The x the sweep along the body ended at.
    fn feet_x(world: &Hill) -> f32 {
        let sweeps = world.sweeps.borrow();
        sweeps
            .iter()
            .find(|s| s.lo_corner[0] == -SWEEP_SPHERE && s.end[2] == s.start[2])
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
            assert!((feet_x(&world) + (reach * BELLY_FOOT_REACH - 6.0)).abs() < 1e-4);
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

        let was_lying_flat = check_prone(
            &world,
            [0.0; 3],
            15.0,
            BELLY_CLEARANCE_PROBE,
            0.0,
            true,
            true,
            true,
            PLAIN,
            BELLY_FOOT_REACH,
            None,
            None,
        );
        assert!(was_lying_flat);
    }

    #[test]
    fn ground_that_is_too_steep_to_stand_on_refuses_before_any_sweep() {
        let world = Hill::flat();
        let fits = check_prone(
            &world,
            [0.0; 3],
            15.0,
            BELLY_CLEARANCE_PROBE,
            0.0,
            true,
            true,
            false,
            PLAIN,
            BELLY_FOOT_REACH,
            None,
            None,
        );
        assert!(!fits);
        assert!(world.sweeps.borrow().is_empty());
    }

    #[test]
    fn the_rules_come_from_the_player() {
        let mut avatar = AvatarState::ZERO;
        assert!(!ProneRules::for_player(&avatar, true).last_stand);
        avatar.movement_mode = movement_mode::LASTSTAND;
        let rules = ProneRules::for_player(&avatar, true);
        assert!(rules.last_stand && rules.zombies);
    }
}
