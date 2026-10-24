//! bo2mp: Black Ops II's `BG_CheckProne`, copied from the legacy movement's
//! `check_prone`. The only difference is
//! the waist trace: BO2 always looks down from the waist 2.5 body widths plus
//! the trace height less the capsule (55.5 units for a size of 15), on the
//! ground or in the air. The shared check looks down only one body width (33)
//! in the air. `player_prone_allowed`, the lie-down gate that calls it, is
//! copied too so it uses this one.

use game_math::{angle_subtract, yaw_vectors_2d};
use crate::bridge::{
    CollisionBackend, GroundTraceInput, PRONE_CHECK_HEIGHT, PRONE_FEET_DIST, ProneRules,
};
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, pm_flags};
use trace_t6::Trace;

const PRONE_TRACE_MASK: u32 = 0x0081_0011;

const PRONE_CAPSULE: f32 = 6.0;

const PRONE_FIRST_LIFT: f32 = 10.0;

const PRONE_YAW_FLIP: f32 = 180.0;

const PRONE_SHORT_FEET_FRAC: f32 = 0.699_999_988_079_071;

const PRONE_SHORT_FEET_BUMP: f32 = 22.0;

const PRONE_MIN_FIRST_PAD: f32 = 2.0;

/// BO2 looks down this many body widths from the waist, on the ground or not.
const PRONE_WAIST_SCALE: f32 = 2.5;

const PRONE_WAIST_FEET_REJECT: f32 = -0.75;

const PRONE_POINT_LIFT: f32 = 5.0;

const PRONE_PITCH_DELTA_MIN: f32 = -50.0;
const PRONE_PITCH_DELTA_MAX: f32 = 70.0;

/// Black Ops II puts the waist 12 units along the body, not 18.
const BO2_PRONE_WAIST_ALONG: f32 = 12.0;

/// A blocked far-feet trace is retried this much higher (BO2 keeps 18 here
/// when its waist moves to 12).
const PRONE_FAR_RETRY_LIFT: f32 = 18.0;

/// Black Ops II's `lastStandDistScale`: in last stand the body lies in front
/// of the player, three quarters as long (half in Zombies).
const BO2_LAST_STAND_SCALE: f32 = -0.75;
const BO2_LAST_STAND_SCALE_ZOMBIES: f32 = -0.5;

/// In Zombies last stand the body may bend 65 degrees, not 50.
const BO2_PRONE_PITCH_DELTA_MIN_ZOMBIES: f32 = -65.0;

/// A waist-to-feet line blocked by the ground is forgiven when the legs run
/// downhill by less than this (BO2's `enable_new_prone_check` 0).
const BO2_PRONE_DOWNHILL_FORGIVE: f32 = -20.0;

/// Whether the player may lie down (or stay lying), by the BO2 check. BO2
/// has no lying down in the air: only on the ground, out of water.
#[must_use]
pub(crate) fn player_prone_allowed<C: CollisionBackend>(
    ps: &PlayerState,
    collision: &C,
    f_size: f32,
    weapon_blocks_prone: bool,
    rules: ProneRules,
) -> bool {
    if weapon_blocks_prone {
        return false;
    }
    if (ps.pm_flags & pm_flags::PRONE) != 0 {
        return true;
    }
    if ps.ground_entity_num == ENTITYNUM_NONE {
        return false;
    }
    // Not in water, even ankle deep.
    if ps.waterlevel >= 1 {
        return false;
    }
    check_prone(
        collision,
        ps.origin,
        f_size,
        PRONE_CHECK_HEIGHT,
        ps.viewangles[1],
        false,
        true,
        true,
        rules,
        PRONE_FEET_DIST,
        None,
        None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call boundary for the prone check"
)]
#[must_use]
pub(crate) fn check_prone<C: CollisionBackend>(
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
        let lifted = [origin[0], origin[1], origin[2] + PRONE_FIRST_LIFT];
        let hull = collision.trace(GroundTraceInput {
            start: origin,
            end: lifted,
            mins: [-f_size, -f_size, 0.0],
            maxs: [f_size, f_size, f_height],
            tracemask: PRONE_TRACE_MASK,
        });
        if hull.allsolid {
            return false;
        }
    }
    if on_ground && !ground_walkable {
        return false;
    }

    // Always turned around; last stand flips the body to the front by a
    // negative length instead.
    let ProneRules {
        last_stand,
        zombies,
        diving,
    } = rules;
    let scale = match (last_stand, zombies) {
        (false, _) => 1.0,
        (true, false) => BO2_LAST_STAND_SCALE,
        (true, true) => BO2_LAST_STAND_SCALE_ZOMBIES,
    };
    let yaw = f_yaw - PRONE_YAW_FLIP;
    let waist_along = scale * BO2_PRONE_WAIST_ALONG;
    let prone_feet_dist = scale * prone_feet_dist;
    let pitch_min = if last_stand && zombies {
        BO2_PRONE_PITCH_DELTA_MIN_ZOMBIES
    } else {
        PRONE_PITCH_DELTA_MIN
    };
    let (fwd_xy, _) = yaw_vectors_2d(yaw);
    let mut forward = [fwd_xy[0], fwd_xy[1], 0.0];
    let f_trace_height = f_height - PRONE_CAPSULE;
    let mut start = [origin[0], origin[1], origin[2] + f_trace_height];
    let along = prone_feet_dist - PRONE_CAPSULE;
    let mut end = [
        start[0] + along * forward[0],
        start[1] + along * forward[1],
        start[2] + along * forward[2],
    ];
    let cap_mins = [-PRONE_CAPSULE, -PRONE_CAPSULE, -PRONE_CAPSULE];
    let cap_maxs = [PRONE_CAPSULE, PRONE_CAPSULE, PRONE_CAPSULE];
    let mut trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
    let mut first_hit = false;
    let first_dist = if trace.fraction >= 1.0 {
        prone_feet_dist
    } else {
        if !on_ground {
            return false;
        }
        first_hit = true;
        let dist = along * trace.fraction + PRONE_CAPSULE;
        if dist < f_size + PRONE_MIN_FIRST_PAD {
            return false;
        }
        let short = f_trace_height * PRONE_SHORT_FEET_FRAC + waist_along;
        if dist < short {
            first_hit = false;
            end[2] += PRONE_SHORT_FEET_BUMP;
            let delta = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
            let pitch_len = vec3_normalize_to(delta, &mut forward);
            trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
            if trace.fraction >= 1.0 {
                prone_feet_dist
            } else {
                first_hit = true;
                let retry = trace.fraction * pitch_len + PRONE_CAPSULE;
                if retry < short {
                    return false;
                }
                retry
            }
        } else {
            dist
        }
    };

    let mut feet = vec3_lerp(start, end, trace.fraction);
    start = [
        origin[0] + waist_along * forward[0],
        origin[1] + waist_along * forward[1],
        origin[2] + f_trace_height + waist_along * forward[2],
    ];
    // The one change from the shared check: the waist always looks down
    // 2.5 widths, not one width in the air.
    let down = PRONE_WAIST_SCALE * f_size + f_trace_height - PRONE_CAPSULE;
    end = [start[0], start[1], start[2] - down];
    trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
    if trace.fraction == 1.0 {
        return prone_fail(
            on_ground,
            torso_pitch.as_deref_mut(),
            waist_pitch.as_deref_mut(),
        );
    }
    if !trace.walkable {
        return false;
    }
    let waist_dist = down * trace.fraction + PRONE_CAPSULE;
    let mut waist = vec3_lerp(start, end, trace.fraction);
    waist[2] -= PRONE_CAPSULE;
    if first_hit {
        if first_dist - waist_dist < waist_dist * PRONE_WAIST_FEET_REJECT {
            return prone_fail(
                on_ground,
                torso_pitch.as_deref_mut(),
                waist_pitch.as_deref_mut(),
            );
        }
        let mut delta = [feet[0] - waist[0], feet[1] - waist[1], feet[2] - waist[2]];
        delta[0] += PRONE_CAPSULE * forward[0];
        delta[1] += PRONE_CAPSULE * forward[1];
        delta[2] += PRONE_CAPSULE * forward[2] + PRONE_CAPSULE;
        vec3_normalize(&mut delta);
        let mad_scale = along - waist_along;
        end = [
            start[0] + mad_scale * delta[0],
            start[1] + mad_scale * delta[1],
            start[2] + mad_scale * delta[2],
        ];
        end[0] = (along * forward[0] + origin[0] + end[0]) * 0.5;
        end[1] = (along * forward[1] + origin[1] + end[1]) * 0.5;
        trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
        if trace.fraction < 1.0 {
            start = vec3_lerp(start, end, trace.fraction);
            start[2] += PRONE_FAR_RETRY_LIFT;
            end[2] += PRONE_FAR_RETRY_LIFT;
            trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
            if trace.fraction < 1.0 {
                return prone_fail(
                    on_ground,
                    torso_pitch.as_deref_mut(),
                    waist_pitch.as_deref_mut(),
                );
            }
        }
        feet = vec3_lerp(start, end, trace.fraction);
    }

    start = feet;
    end = [
        feet[0],
        feet[1],
        feet[2] - ((feet[2] - waist[2]) + (feet[2] - waist[2]) + f_size),
    ];
    trace = prone_trace(collision, start, end, cap_mins, cap_maxs);
    if trace.fraction == 1.0 {
        return prone_fail(
            on_ground,
            torso_pitch.as_deref_mut(),
            waist_pitch.as_deref_mut(),
        );
    }
    if !trace.walkable {
        return false;
    }
    feet = vec3_lerp(start, end, trace.fraction);
    feet[2] -= PRONE_CAPSULE;
    let torso = origin;
    let torso_delta = [
        torso[0] - waist[0],
        torso[1] - waist[1],
        torso[2] - waist[2],
    ];
    // Black Ops II keeps the pitches in -180..180.
    let normalize = |pitch: f32| angle_subtract(pitch, 0.0);
    let f_torso = normalize(vec_to_pitch(torso_delta));
    let waist_delta = [waist[0] - feet[0], waist[1] - feet[1], waist[2] - feet[2]];
    let mut f_waist = normalize(vec_to_pitch(waist_delta));
    let pitch_diff = angle_subtract(f_torso, f_waist);
    let mut success = true;
    if !(pitch_min..=PRONE_PITCH_DELTA_MAX).contains(&pitch_diff) {
        success = false;
    }
    let zero = [0.0, 0.0, 0.0];
    trace = prone_trace(
        collision,
        [torso[0], torso[1], torso[2] + PRONE_POINT_LIFT],
        [waist[0], waist[1], waist[2] + PRONE_POINT_LIFT],
        zero,
        zero,
    );
    if trace.fraction < 1.0 {
        success = false;
    }
    trace = prone_trace(
        collision,
        [waist[0], waist[1], waist[2] + PRONE_POINT_LIFT],
        [feet[0], feet[1], feet[2] + PRONE_POINT_LIFT],
        zero,
        zero,
    );
    if trace.fraction < 1.0 {
        // Legs a little downhill may touch the ground; the waist reads flat
        // either way.
        if !(BO2_PRONE_DOWNHILL_FORGIVE < f_waist && f_waist < 0.0) {
            success = false;
        }
        f_waist = 0.0;
    }
    let (mut out_torso, mut out_waist) = (f_torso, f_waist);
    if last_stand {
        out_torso = -out_torso;
        out_waist = -out_waist;
    }
    if diving {
        out_torso = 0.0;
        out_waist = 0.0;
    }
    if let Some(slot) = torso_pitch.as_deref_mut() {
        *slot = out_torso;
    }
    if let Some(slot) = waist_pitch.as_deref_mut() {
        *slot = out_waist;
    }
    if success {
        return true;
    }
    prone_fail(on_ground, torso_pitch, waist_pitch)
}

fn prone_fail(
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

fn prone_trace<C: CollisionBackend>(
    collision: &C,
    start: [f32; 3],
    end: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
) -> Trace {
    collision.trace(GroundTraceInput {
        start,
        end,
        mins,
        maxs,
        tracemask: PRONE_TRACE_MASK,
    })
}

fn vec3_lerp(start: [f32; 3], end: [f32; 3], frac: f32) -> [f32; 3] {
    [
        start[0] + frac * (end[0] - start[0]),
        start[1] + frac * (end[1] - start[1]),
        start[2] + frac * (end[2] - start[2]),
    ]
}

fn vec3_normalize(v: &mut [f32; 3]) -> f32 {
    let len = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let div = if len <= 0.0 { 1.0 } else { len };
    let s = 1.0 / div;
    v[0] *= s;
    v[1] *= s;
    v[2] *= s;
    len
}

fn vec3_normalize_to(src: [f32; 3], dst: &mut [f32; 3]) -> f32 {
    *dst = src;
    vec3_normalize(dst)
}

fn vec_to_pitch(vec: [f32; 3]) -> f32 {
    if vec[0] == 0.0 && vec[1] == 0.0 {
        return if -vec[2] < 0.0 { 270.0 } else { 90.0 };
    }
    let xy = libm::sqrtf(vec[0] * vec[0] + vec[1] * vec[1]);
    let pitch = libm::atan2f(vec[2], xy) * (-180.0 / core::f32::consts::PI);
    if pitch < 0.0 { pitch + 360.0 } else { pitch }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use super::*;

    /// A flat floor at `top`; only straight-down box traces meet it, and the
    /// straight-down box traces are recorded.
    struct Floor {
        top: f32,
        walkable: bool,
        downs: RefCell<Vec<GroundTraceInput>>,
    }

    impl CollisionBackend for Floor {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            let straight_down = input.start[0] == input.end[0]
                && input.start[1] == input.end[1]
                && input.end[2] < input.start[2];
            if input.mins == [0.0; 3] || !straight_down {
                return hit;
            }
            self.downs.borrow_mut().push(input);
            let start = input.start[2] + input.mins[2];
            let end = input.end[2] + input.mins[2];
            if start >= self.top && end < self.top {
                hit.fraction = (start - self.top) / (start - end);
                hit.normal = [0.0, 0.0, 1.0];
                hit.walkable = self.walkable;
            }
            hit
        }
    }

    const BO2: ProneRules = ProneRules {
        last_stand: false,
        zombies: false,
        diving: false,
    };

    /// A body lying in the air (size 15), facing +x: BO2's version, then the
    /// shared one.
    fn lying_in_air(world: &Floor) -> (bool, bool) {
        let origin = [0.0; 3];
        let bo2 = check_prone(
            world,
            origin,
            15.0,
            PRONE_CHECK_HEIGHT,
            0.0,
            true,
            false,
            true,
            BO2,
            PRONE_FEET_DIST,
            None,
            None,
        );
        let shared = crate::bridge::check_prone(
            world,
            origin,
            15.0,
            PRONE_CHECK_HEIGHT,
            0.0,
            true,
            false,
            true,
            BO2,
            PRONE_FEET_DIST,
            None,
            None,
        );
        (bo2, shared)
    }

    /// BO2's waist trace always goes down 55.5 units (size 15), also in the
    /// air where the shared check goes down 33. A steep floor 43 down is
    /// therefore met: BO2 tells the lying body to give up, where the shared
    /// check never sees the floor and lets it lie.
    #[test]
    fn bo2_waist_looks_down_two_and_a_half_widths_in_the_air() {
        let world = Floor {
            top: -25.0,
            walkable: false,
            downs: RefCell::new(Vec::new()),
        };
        let (bo2, shared) = lying_in_air(&world);
        assert!(!bo2, "BO2 reaches the steep floor");
        assert!(shared, "the shared check stops 33 down");

        let downs = world.downs.borrow();
        let reach = |t: &GroundTraceInput| t.start[2] - t.end[2];
        assert!(
            (reach(&downs[0]) - 55.5).abs() < 1e-4,
            "{}",
            reach(&downs[0])
        );
        assert!(
            (reach(&downs[1]) - 33.0).abs() < 1e-4,
            "{}",
            reach(&downs[1])
        );
    }
}
