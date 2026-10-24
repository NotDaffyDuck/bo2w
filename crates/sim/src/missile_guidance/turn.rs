//! Heading maths shared by the missile steering modes: bounded turns, and the bank a drone leans into them.

use glam::{EulerRot, Quat, Vec3};

/// `current` swung toward `wanted` by at most `max_radians`; `wanted` itself once it is that close (or when the angle
/// cannot be measured).
pub(crate) fn turn_toward(current: Vec3, wanted: Vec3, max_radians: f32) -> Vec3 {
    let gap = current.angle_between(wanted);
    if gap <= max_radians || gap.is_nan() {
        return wanted;
    }
    let spin = current.cross(wanted);
    // Opposite headings have no unique turning axis; any one perpendicular to the heading will do.
    let spin = if spin.length_squared() < 1e-8 {
        current.any_orthonormal_vector()
    } else {
        spin.normalize()
    };
    Quat::from_axis_angle(spin, max_radians) * current
}

/// The orientation whose forward axis is `direction`, with no roll (yaw about Z, then pitch).
pub(super) fn facing(direction: Vec3) -> Quat {
    let angles = gsc_t6::math::vector_to_angles(direction.to_array());
    Quat::from_euler(EulerRot::ZYX, angles[1].to_radians(), angles[0].to_radians(), 0.0)
}

/// A drone's tuning for how it banks and how sharply it turns.
mod tuning {
    /// Fraction of a small correction applied in one frame.
    pub const SMALL_CORRECTION: f32 = 0.9;
    /// How much of the bank is kept each frame while flying straight.
    pub const LEVEL_OUT: f32 = 0.85;
    /// Largest bank, in degrees, either way.
    pub const MAX_BANK: f32 = 40.0;
    /// Largest change of bank in one frame.
    pub const BANK_STEP: f32 = 3.0;
    /// The turn size (degrees) that earns a full bank.
    pub const FULL_BANK_TURN: f32 = 300.0;
}

/// Where a drone points after one frame of turning, how big a turn it wanted, and how far it banks.
#[derive(Clone, Copy, Debug)]
pub(super) struct Bend {
    pub heading: Vec3,
    /// 0 (dead ahead) to 180 (straight back).
    pub wanted_turn: f32,
    pub bank: f32,
}

/// One frame of drone turning from `current` toward `wanted`, no faster than `max_dps` degrees a second. A small
/// correction is taken 90% of the way at once (or entirely, with `snap_small`); the bank follows the turn's
/// direction, a few degrees a frame.
pub(super) fn bend(current: Vec3, wanted: Vec3, max_dps: f32, snap_small: bool, bank: f32) -> Bend {
    use tuning::*;
    let wanted_turn = (1.0 - (wanted.dot(current) + 1.0) * 0.5) * 180.0;
    if wanted_turn <= 0.1 {
        return Bend {
            heading: wanted,
            wanted_turn,
            bank: bank * LEVEL_OUT,
        };
    }
    let needed = wanted_turn * 20.0;
    let mut share = max_dps / needed;
    if needed < max_dps {
        if snap_small {
            return Bend {
                heading: wanted,
                wanted_turn,
                bank,
            };
        }
        share = SMALL_CORRECTION;
    }
    let heading = (facing(current).slerp(facing(wanted), share).normalize() * Vec3::X).normalize_or_zero();
    let yaw_of = |v: Vec3| v.y.atan2(v.x).to_degrees();
    let mut yaw_change = yaw_of(current) - yaw_of(heading);
    if yaw_change > 180.0 {
        yaw_change -= 360.0;
    } else if yaw_change < -180.0 {
        yaw_change += 360.0;
    }
    let bank_limit = if yaw_change < 0.0 { -MAX_BANK } else { MAX_BANK };
    let mut next_bank = wanted_turn / FULL_BANK_TURN * bank_limit;
    if (next_bank - bank).abs() > BANK_STEP {
        // The step follows the sign of the bank it is heading for, not the sign of the change.
        next_bank = bank + if next_bank < 0.0 { -BANK_STEP } else { BANK_STEP };
    }
    Bend {
        heading,
        wanted_turn,
        bank: next_bank,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AHEAD: Vec3 = Vec3::X;
    const LEFT: Vec3 = Vec3::Y;

    #[test]
    fn a_turn_within_the_limit_arrives_whole() {
        let wanted = Vec3::new(1.0, 0.1, 0.0).normalize();
        assert_eq!(turn_toward(AHEAD, wanted, 0.5), wanted);
    }

    #[test]
    fn a_big_turn_is_cut_to_the_limit() {
        let out = turn_toward(AHEAD, LEFT, 0.25);
        assert!((out.angle_between(AHEAD) - 0.25).abs() < 1e-4);
        assert!(out.y > 0.0);
    }

    #[test]
    fn a_u_turn_still_picks_a_side() {
        let out = turn_toward(AHEAD, -AHEAD, 0.3);
        assert!((out.angle_between(AHEAD) - 0.3).abs() < 1e-4);
        assert!((out.length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn an_unmeasurable_angle_takes_the_goal() {
        let nothing = Vec3::ZERO;
        assert_eq!(turn_toward(nothing, AHEAD, 0.1), AHEAD);
    }

    #[test]
    fn facing_points_forward_along_the_direction() {
        for dir in [Vec3::X, Vec3::Y, -Vec3::X, Vec3::new(1.0, 1.0, 0.5).normalize()] {
            let out = facing(dir) * Vec3::X;
            assert!(out.angle_between(dir) < 1e-3, "{dir:?} -> {out:?}");
        }
    }

    #[test]
    fn straight_ahead_levels_the_bank_out() {
        let out = bend(AHEAD, AHEAD, 360.0, false, 20.0);
        assert_eq!(out.heading, AHEAD);
        assert_eq!(out.wanted_turn, 0.0);
        assert!((out.bank - 17.0).abs() < 1e-4);
    }

    #[test]
    fn a_snapped_small_turn_arrives_at_once_and_keeps_the_bank() {
        let wanted = Vec3::new(1.0, 0.2, 0.0).normalize();
        let out = bend(AHEAD, wanted, 360.0, true, 5.0);
        assert_eq!(out.heading, wanted);
        assert_eq!(out.bank, 5.0);
    }

    #[test]
    fn a_small_turn_that_is_not_snapped_goes_most_of_the_way() {
        let wanted = Vec3::new(1.0, 0.2, 0.0).normalize();
        let out = bend(AHEAD, wanted, 360.0, false, 0.0);
        let covered = out.heading.angle_between(AHEAD) / wanted.angle_between(AHEAD);
        assert!((covered - 0.9).abs() < 0.02, "{covered}");
    }

    #[test]
    fn a_left_turn_banks_negative_a_step_at_a_time() {
        let out = bend(AHEAD, LEFT, 360.0, false, 0.0);
        assert!((out.wanted_turn - 90.0).abs() < 1e-3);
        assert_eq!(out.bank, -3.0);
        // The next frame steps on from there.
        assert_eq!(bend(AHEAD, LEFT, 360.0, false, -3.0).bank, -6.0);
    }

    #[test]
    fn a_right_turn_banks_positive() {
        assert_eq!(bend(AHEAD, -LEFT, 360.0, false, 0.0).bank, 3.0);
    }

    #[test]
    fn a_hard_turn_is_rate_limited() {
        // 90 degrees wanted at 360 deg/s: the share is 360 / (90 * 20) = 0.2 of the way round.
        let out = bend(AHEAD, LEFT, 360.0, false, 0.0);
        let covered = out.heading.angle_between(AHEAD) / LEFT.angle_between(AHEAD);
        assert!((covered - 0.2).abs() < 0.02, "{covered}");
    }
}
