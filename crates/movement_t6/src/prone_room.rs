//! Lying-down validity test for the BO2 rules. It started from the legacy
//! movement's version and differs in one place: the probe dropped from the
//! hip always reaches 2.5 body widths plus the probe height minus the capsule
//! radius (55.5 units at size 15), grounded or airborne, where the shared
//! version reaches a single body width (33) when airborne.
//! `prone_space_ok`, the gate in front of it, is duplicated so that it
//! calls this variant.

use crate::bridge::{
    SolidProbe, SweepAsk, BELLY_CLEARANCE_PROBE, BELLY_FOOT_REACH, ProneRules,
};
use game_math::degree_gap;
use game_math::flat_basis;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, movement_state_bits};
use trace_t6::Trace;

/// What the body's probes collide with.
const LIE_MASK: u32 = 0x0081_0011;
/// Half-size of the small box every probe sweeps.
const SWEEP_RADIUS: f32 = 6.0;
/// The standing hull is first lifted this far to see that it is not stuck.
const INITIAL_RAISE: f32 = 10.0;
/// The legs lie behind the view.
const HEADING_REVERSE: f32 = 180.0;
/// A leg probe stopped before 0.7 of the probe height (past the hip) is
/// tried again tilted up by 22.
const NEAR_HIT_RATIO: f32 = 0.699_999_988_079_071;
const NEAR_HIT_RAISE: f32 = 22.0;
/// The legs need at least a body width and this much more.
const CLEARANCE_MARGIN: f32 = 2.0;
/// Number of body widths the hip probe descends, grounded or not.
const HIP_DROP_WIDTHS: f32 = 2.5;
/// With blocked legs the feet may lie at most 0.75 hip drops above the hip.
const HIP_FOOT_GAP_LIMIT: f32 = -0.75;
/// The torso and leg rays run this far above the body's points.
const RAY_RAISE: f32 = 5.0;
/// How far the body may bend at the hip.
const BEND_LOWER_LIMIT: f32 = -50.0;
const BEND_UPPER_LIMIT: f32 = 70.0;
/// Distance from the origin to the hip along the body; BO2 uses 12 (not 18).
const HIP_OFFSET_ALONG: f32 = 12.0;
/// How far the blocked far-foot sweep is raised for its second attempt (18
/// stays even though the hip sits at 12).
const SECOND_TRY_RAISE: f32 = 18.0;
/// Factor applied to body length in last stand: the body extends ahead of the
/// player, at three quarters length (half in Zombies).
const LAST_STAND_FACTOR: f32 = -0.75;
const LAST_STAND_FACTOR_ZM: f32 = -0.5;
/// Zombies last stand lets the body bend 65 degrees instead of 50.
const BEND_LOWER_LIMIT_ZM: f32 = -65.0;
/// A hip-to-foot ray that is stopped by terrain is excused if the legs slope
/// down by less than this many degrees.
const SLOPE_EXCUSE: f32 = -20.0;

/// Whether the player may be (or remain) lying down under BO2's test. There is
/// no lying in mid-air; the player must stand on the ground and be dry.
#[must_use]
pub(crate) fn prone_space_ok<C: SolidProbe>(
    avatar: &AvatarState,
    solids: &C,
    breadth: f32,
    gun_forbids_lying: bool,
    rules: ProneRules,
) -> bool {
    let lying = avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0;
    let footing = avatar.standing_on != NO_ENTITY_SLOT && avatar.waterlevel < 1;
    match (gun_forbids_lying, lying) {
        (true, _) => false,
        (false, true) => true,
        (false, false) => {
            footing && check_prone(
                solids, avatar.origin, breadth, BELLY_CLEARANCE_PROBE, avatar.look_angles[1],
                false, true, true, rules, BELLY_FOOT_REACH, None, None,
            )
        }
    }
}

/// How a probe stage ended without a verdict of its own.
enum Halt {
    /// The body does not fit, wherever the player is.
    Refuse,
    /// The probe found nothing to judge: a fail on the ground, a level pass
    /// in the air.
    Inconclusive,
}

/// Where the leg probe ended and how.
struct Legs {
    /// Probe start and end, head height (the end maybe tilted up).
    from: [f32; 3],
    to: [f32; 3],
    reach_frac: f32,
    /// How far along the legs reached.
    length: f32,
    /// Something stood in their way.
    blocked: bool,
}

/// One lying-down probe: the body laid out from `lay_at` along `dir`.
struct Body<'c, C> {
    solids: &'c C,
    lay_at: [f32; 3],
    half_width: f32,
    /// Height of the probes over the feet.
    probe_height: f32,
    /// Along the body to the hip and to the feet (negative in last stand).
    hip_along: f32,
    foot_reach: f32,
    /// The leg probe's own length, short of its box.
    span: f32,
    /// Along the body; tilted up when the legs had to try higher.
    dir: [f32; 3],
    grounded: bool,
}

impl<C: SolidProbe> Body<'_, C> {
    fn sweep(&self, a: [f32; 3], b: [f32; 3]) -> Trace {
        cast_box(self.solids, a, b, [-SWEEP_RADIUS; 3], [SWEEP_RADIUS; 3])
    }

    fn head(&self) -> [f32; 3] {
        let o = self.lay_at;
        [o[0], o[1], o[2] + self.probe_height]
    }

    /// The legs at head height, straight back; stopped short, once more
    /// tilted up when they barely passed the hip.
    fn lay_legs(&mut self) -> Result<Legs, Halt> {
        let from = self.head();
        let mut to = mad(from, self.span, self.dir);
        let first = self.sweep(from, to);
        if first.fraction >= 1.0 {
            return Ok(Legs { from, to, reach_frac: first.fraction, length: self.foot_reach, blocked: false });
        }
        if !self.grounded {
            return Err(Halt::Refuse);
        }
        let reached = self.span * first.fraction + SWEEP_RADIUS;
        if reached < self.half_width + CLEARANCE_MARGIN {
            return Err(Halt::Refuse);
        }
        let near_limit = self.probe_height * NEAR_HIT_RATIO + self.hip_along;
        if reached >= near_limit {
            return Ok(Legs { from, to, reach_frac: first.fraction, length: reached, blocked: true });
        }
        to[2] += NEAR_HIT_RAISE;
        let mut tilted = sub(to, from);
        let tilt_len = unit(&mut tilted);
        self.dir = tilted;
        let second = self.sweep(from, to);
        if second.fraction >= 1.0 {
            return Ok(Legs { from, to, reach_frac: second.fraction, length: self.foot_reach, blocked: false });
        }
        let again = second.fraction * tilt_len + SWEEP_RADIUS;
        match again < near_limit {
            true => Err(Halt::Refuse),
            false => Ok(Legs { from, to, reach_frac: second.fraction, length: again, blocked: true }),
        }
    }

    /// Straight down from the hip: the ground the hip rests on, and how far
    /// down it was.
    fn find_hip(&self) -> Result<([f32; 3], [f32; 3], f32), Halt> {
        let top = mad(self.head(), self.hip_along, self.dir);
        // Differs from the shared check: always 2.5 widths down.
        let drop = HIP_DROP_WIDTHS * self.half_width + self.probe_height - SWEEP_RADIUS;
        let bottom = [top[0], top[1], top[2] - drop];
        let (hip, fraction) = self.ground_below(top, bottom)?;
        Ok((top, hip, drop * fraction + SWEEP_RADIUS))
    }

    /// A downward sweep that must land on walkable ground; the landing point
    /// at the foot of the box.
    fn ground_below(&self, top: [f32; 3], bottom: [f32; 3]) -> Result<([f32; 3], f32), Halt> {
        let strike = self.sweep(top, bottom);
        if strike.fraction == 1.0 {
            return Err(Halt::Inconclusive);
        }
        if !strike.walkable {
            return Err(Halt::Refuse);
        }
        let mut at = lerp(top, bottom, strike.fraction);
        at[2] -= SWEEP_RADIUS;
        Ok((at, strike.fraction))
    }

    /// With the legs blocked: from the hip toward the feet, between the leg
    /// line and the straight body, raised once if it has to be.
    fn reroute_feet(&self, hip_top: [f32; 3], (hip, foot): ([f32; 3], [f32; 3])) -> Result<[f32; 3], Halt> {
        let d = self.dir;
        let mut leg = sub(foot, hip);
        leg[0] += SWEEP_RADIUS * d[0];
        leg[1] += SWEEP_RADIUS * d[1];
        leg[2] += SWEEP_RADIUS * d[2] + SWEEP_RADIUS;
        unit(&mut leg);
        let mut to = mad(hip_top, self.span - self.hip_along, leg);
        to[0] = (self.span * d[0] + self.lay_at[0] + to[0]) * 0.5;
        to[1] = (self.span * d[1] + self.lay_at[1] + to[1]) * 0.5;
        let mut from = hip_top;
        let mut strike = self.sweep(from, to);
        if strike.fraction < 1.0 {
            from = lerp(from, to, strike.fraction);
            from[2] += SECOND_TRY_RAISE;
            to[2] += SECOND_TRY_RAISE;
            strike = self.sweep(from, to);
            if strike.fraction < 1.0 {
                return Err(Halt::Inconclusive);
            }
        }
        Ok(lerp(from, to, strike.fraction))
    }

    /// The hip and feet on the ground, or why not.
    fn lay_out(&mut self) -> Result<([f32; 3], [f32; 3]), Halt> {
        let legs = self.lay_legs()?;
        let mut foot = lerp(legs.from, legs.to, legs.reach_frac);
        let (hip_top, hip, hip_len) = self.find_hip()?;
        if legs.blocked {
            if legs.length - hip_len < hip_len * HIP_FOOT_GAP_LIMIT {
                return Err(Halt::Inconclusive);
            }
            foot = self.reroute_feet(hip_top, (hip, foot))?;
        }
        let rise = foot[2] - hip[2];
        let below = [foot[0], foot[1], foot[2] - (rise + rise + self.half_width)];
        let (feet, _) = self.ground_below(foot, below)?;
        Ok((hip, feet))
    }

    /// A thin ray between two body points, lifted off the ground, is clear.
    fn ray_clear(&self, a: [f32; 3], b: [f32; 3]) -> bool {
        let lift = |p: [f32; 3]| [p[0], p[1], p[2] + RAY_RAISE];
        cast_box(self.solids, lift(a), lift(b), [0.0; 3], [0.0; 3]).fraction >= 1.0
    }
}

/// Writes the torso and waist pitches where the caller asked for them.
fn report(torso_slot: &mut Option<&mut f32>, waist_slot: &mut Option<&mut f32>, torso: f32, waist: f32) {
    if let Some(out) = torso_slot.as_deref_mut() {
        *out = torso;
    }
    if let Some(out) = waist_slot.as_deref_mut() {
        *out = waist;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call boundary for the prone check"
)]
#[must_use]
pub(crate) fn check_prone<C: SolidProbe>(
    solids: &C,
    at_pt: [f32; 3],
    breadth: f32,
    tall_by: f32,
    f_yaw: f32,
    was_lying_flat: bool,
    grounded_now: bool,
    footing_ok: bool,
    rules: ProneRules,
    feet_reach: f32,
    mut chest_tilt: Option<&mut f32>,
    mut hip_tilt: Option<&mut f32>,
) -> bool {
    // In the air a probe with nothing to judge passes, pitches level.
    let inconclusive = |torso: &mut Option<&mut f32>, waist: &mut Option<&mut f32>| {
        if !grounded_now {
            report(torso, waist, 0.0, 0.0);
        }
        !grounded_now
    };
    let stuck = !was_lying_flat && head_room_blocked(solids, at_pt, breadth, tall_by);
    if stuck || (grounded_now && !footing_ok) {
        return false;
    }
    let ProneRules { last_stand, zombies, diving } = rules;
    let (factor, bend_floor) = match (last_stand, zombies) {
        (false, _) => (1.0, BEND_LOWER_LIMIT),
        (true, false) => (LAST_STAND_FACTOR, BEND_LOWER_LIMIT),
        (true, true) => (LAST_STAND_FACTOR_ZM, BEND_LOWER_LIMIT_ZM),
    };
    let (flat, _) = flat_basis(f_yaw - HEADING_REVERSE);
    let foot_reach = factor * feet_reach;
    let mut body = Body {
        solids,
        lay_at: at_pt,
        half_width: breadth,
        probe_height: tall_by - SWEEP_RADIUS,
        hip_along: factor * HIP_OFFSET_ALONG,
        foot_reach,
        span: foot_reach - SWEEP_RADIUS,
        dir: [flat[0], flat[1], 0.0],
        grounded: grounded_now,
    };
    let (hip, feet) = match body.lay_out() {
        Ok(points) => points,
        Err(Halt::Refuse) => return false,
        Err(Halt::Inconclusive) => return inconclusive(&mut chest_tilt, &mut hip_tilt),
    };

    // BO2 keeps pitch within -180..180.
    let torso = degree_gap(pitch_of(sub(at_pt, hip)), 0.0);
    let mut waist = degree_gap(pitch_of(sub(hip, feet)), 0.0);
    let bend = degree_gap(torso, waist);
    let torso_clear = body.ray_clear(at_pt, hip);
    let mut fits = (bend_floor..=BEND_UPPER_LIMIT).contains(&bend) && torso_clear;
    if !body.ray_clear(hip, feet) {
        // Slightly downhill legs may graze the ground; the hip counts as level.
        fits &= SLOPE_EXCUSE < waist && waist < 0.0;
        waist = 0.0;
    }
    let (torso_out, waist_out) = match (diving, last_stand) {
        (true, _) => (0.0, 0.0),
        (false, true) => (-torso, -waist),
        (false, false) => (torso, waist),
    };
    report(&mut chest_tilt, &mut hip_tilt, torso_out, waist_out);
    fits || inconclusive(&mut chest_tilt, &mut hip_tilt)
}

/// True when the hull swept up from the origin starts fully inside solid.
fn head_room_blocked<C: SolidProbe>(solids: &C, at: [f32; 3], half_width: f32, tall: f32) -> bool {
    let lifted = [at[0], at[1], at[2] + INITIAL_RAISE];
    let hull_lo = [-half_width, -half_width, 0.0];
    let hull_hi = [half_width, half_width, tall];
    cast_box(solids, at, lifted, hull_lo, hull_hi).fully_embedded
}

fn cast_box<C: SolidProbe>(solids: &C, src: [f32; 3], dst: [f32; 3], box_lo: [f32; 3], box_hi: [f32; 3]) -> Trace {
    solids.trace(SweepAsk { start: src, end: dst, lo_corner: box_lo, hi_corner: box_hi, collide_filter: LIE_MASK })
}

fn sub(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| lhs[i] - rhs[i])
}

/// `base + k * dir`, component-wise.
fn mad(base: [f32; 3], k: f32, dir: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| base[i] + k * dir[i])
}

/// `a` moved `t` of the way to `b`.
fn lerp(p_from: [f32; 3], p_to: [f32; 3], k: f32) -> [f32; 3] {
    core::array::from_fn(|i| p_from[i] + k * (p_to[i] - p_from[i]))
}

/// Scales `v` to unit length (a zero vector stays zero) and returns its old
/// length.
fn unit(v: &mut [f32; 3]) -> f32 {
    let mag = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let stow_kind = 1.0 / if mag <= 0.0 { 1.0 } else { mag };
    *v = v.map(|c| c * stow_kind);
    mag
}

/// The pitch of a direction in degrees, 0..360, looking down positive.
fn pitch_of(v: [f32; 3]) -> f32 {
    let flat = libm::sqrtf(v[0] * v[0] + v[1] * v[1]);
    if v[0] == 0.0 && v[1] == 0.0 {
        return match -v[2] < 0.0 {
            true => 270.0,
            false => 90.0,
        };
    }
    let deg = libm::atan2f(v[2], flat) * (-180.0 / core::f32::consts::PI);
    if deg < 0.0 { deg + 360.0 } else { deg }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use super::*;

    /// Level ground at height `top`. Only vertical box sweeps can hit it, and
    /// those are logged.
    struct Floor {
        top: f32,
        walkable: bool,
        downs: RefCell<Vec<SweepAsk>>,
    }

    impl SolidProbe for Floor {
        fn trace(&self, input: SweepAsk) -> Trace {
            let (s, e) = (input.start, input.end);
            let straight_down = s[0] == e[0] && s[1] == e[1] && e[2] < s[2];
            let is_box = input.lo_corner != [0.0; 3];
            let mut out = Trace::default();
            out.fraction = 1.0_f32;
            if !(is_box && straight_down) {
                return out;
            }
            self.downs.borrow_mut().push(input);
            let (high, low) = (s[2] + input.lo_corner[2], e[2] + input.lo_corner[2]);
            if high >= self.top && low < self.top {
                out.fraction = (high - self.top) / (high - low);
                out.normal = [0.0, 0.0, 1.0];
                out.walkable = self.walkable;
            }
            out
        }
    }

    /// The hip probe descends 55.5 units here but 33 in the shared check, so a
    /// steep floor 43 below is hit by this one (it refuses) and missed by the
    /// shared one (it allows).
    #[test]
    fn hip_probe_reaches_two_and_a_half_widths_airborne() {
        let world = Floor { top: -25.0, walkable: false, downs: RefCell::new(Vec::new()) };
        let plain = ProneRules { last_stand: false, zombies: false, diving: false };
        // A body floating at size 15 facing +x, judged by this module and
        // then by the shared check.
        let checks: [fn(&Floor, [f32; 3], f32, f32, f32, bool, bool, bool, ProneRules, f32, Option<&mut f32>, Option<&mut f32>) -> bool; 2] = [check_prone::<Floor>, crate::bridge::check_prone::<Floor>];
        let [here, there] = checks.map(|judge| {
            judge(&world, [0.0; 3], 15.0, BELLY_CLEARANCE_PROBE, 0.0, true, false, true, plain, BELLY_FOOT_REACH, None, None)
        });
        assert!(!here, "steep floor should be reached");
        assert!(there, "shared probe ends 33 down");
        let drops: Vec<f32> = world.downs.borrow().iter().map(|t| t.start[2] - t.end[2]).collect();
        assert!(drops.len() >= 2, "{drops:?}");
        for (got, want) in drops.iter().zip([55.5, 33.0]) {
            assert!((got - want).abs() < 1e-4, "{got}");
        }
    }
}
