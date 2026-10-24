//! Trail splitting: how often a moving trail element is cut into a new
//! segment, and where along the move each cut falls.
//!
//! Time, distance and turning each add to a running total. Every time that
//! total passes a whole number the trail is cut once more; the remainder
//! carries to the next update.

use crate::runtime::quat::{axis_to_quat, quat_normalize, unit_quat_to_axis};
use crate::runtime::sort::sort_dist_to_cam_sq;

/// What an update does to a trail's split total.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxTrailSplit {
    /// Not enough has built up for a cut; this is the new total.
    Hold { leftover: f32 },
    /// `extra` cuts, with this much left over.
    Interpolate { leftover: f32, extra: i32 },
}

/// Adds an update's time, distance and turning to the total carried from
/// the last one and says whether it passes a whole number.
pub fn trail_split_window(
    leftover: f32,
    inv_split_time: f32,
    dt_msec: f32,
    inv_split_dist: f32,
    dist_delta: f32,
    inv_split_arc: f32,
    arc_delta: f32,
) -> FxTrailSplit {
    let total = leftover
        + inv_split_time * dt_msec
        + inv_split_dist * dist_delta
        + inv_split_arc * arc_delta;
    if total < 1.0 {
        return FxTrailSplit::Hold { leftover: total };
    }
    let extra = libm::floorf(total) as i32;
    FxTrailSplit::Interpolate {
        leftover: total - extra as f32,
        extra,
    }
}

/// Whether a trail element too far from the camera skips this update's cut.
///
/// The limit is the element's spawn range, stretched by one for each step
/// of its sequence number's lowest set bit, so a sequence that fires less
/// often is allowed to be farther away. A trail on its first sequence, or
/// with no spawn range, never skips.
pub fn trail_split_skips_update(
    sequence: u8,
    spawn_range_base: f32,
    spawn_range_amp: f32,
    camera: [f32; 3],
    origin: [f32; 3],
) -> bool {
    if sequence == 0 {
        return false;
    }
    let range = spawn_range_base + spawn_range_amp;
    if range == 0.0 {
        return false;
    }
    let limit = range * (1.0 + sequence.trailing_zeros() as f32);
    sort_dist_to_cam_sq(camera, origin) > limit * limit
}

/// How far through the update the cut that brings the total to `k` falls.
pub fn trail_split_interpolant_t(k: f32, leftover: f32, total: f32) -> f32 {
    (k - leftover) / (total - leftover)
}

/// The time of a cut `t` of the way from `prev_msec` to `msec_now`.
pub fn trail_split_interpolant_msec(prev_msec: i32, msec_now: i32, t: f32) -> i32 {
    (prev_msec as f32 + t * (msec_now - prev_msec) as f32) as i32
}

/// The point `t` of the way from `begin` to `end`.
pub fn trail_split_lerp_origin(begin: [f32; 3], end: [f32; 3], t: f32) -> [f32; 3] {
    [
        begin[0] + t * (end[0] - begin[0]),
        begin[1] + t * (end[1] - begin[1]),
        begin[2] + t * (end[2] - begin[2]),
    ]
}

/// The orientation `t` of the way from `begin` to `end`, blended as
/// rotations and renormalised.
pub fn trail_split_lerp_axis(begin: [[f32; 3]; 3], end: [[f32; 3]; 3], t: f32) -> [[f32; 3]; 3] {
    let from = axis_to_quat(begin);
    let to = axis_to_quat(end);
    let blended = [
        from[0] + t * (to[0] - from[0]),
        from[1] + t * (to[1] - from[1]),
        from[2] + t * (to[2] - from[2]),
        from[3] + t * (to[3] - from[3]),
    ];
    unit_quat_to_axis(quat_normalize(blended))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn a_small_update_holds_and_carries_the_total() {
        let got = trail_split_window(0.25, 0.01, 10.0, 0.0, 0.0, 0.0, 0.0);
        let FxTrailSplit::Hold { leftover } = got else {
            panic!("{got:?}");
        };
        assert!((leftover - 0.35).abs() < 1e-6);
    }

    #[test]
    fn time_distance_and_turning_all_count_toward_the_total() {
        let got = trail_split_window(0.0, 0.1, 1.0, 0.1, 1.0, 0.1, 1.0);
        let FxTrailSplit::Hold { leftover } = got else {
            panic!("{got:?}");
        };
        assert!((leftover - 0.3).abs() < 1e-6);
    }

    #[test]
    fn passing_a_whole_number_cuts_and_keeps_the_fraction() {
        let got = trail_split_window(0.5, 0.0, 0.0, 1.0, 2.75, 0.0, 0.0);
        let FxTrailSplit::Interpolate { leftover, extra } = got else {
            panic!("{got:?}");
        };
        assert_eq!(extra, 3);
        assert!((leftover - 0.25).abs() < 1e-6);
    }

    #[test]
    fn exactly_one_is_enough_to_cut() {
        assert_eq!(
            trail_split_window(0.0, 0.0, 0.0, 0.5, 2.0, 0.0, 0.0),
            FxTrailSplit::Interpolate {
                leftover: 0.0,
                extra: 1
            }
        );
    }

    #[test]
    fn the_first_sequence_and_unranged_trails_never_skip() {
        let far = [1.0e6, 0.0, 0.0];
        assert!(!trail_split_skips_update(0, 100.0, 0.0, [0.0; 3], far));
        assert!(!trail_split_skips_update(1, 0.0, 0.0, [0.0; 3], far));
    }

    #[test]
    fn a_later_sequence_skips_only_beyond_its_stretched_range() {
        // Sequence 1 has no trailing zeros: the range is 100 as authored.
        assert!(!trail_split_skips_update(
            1,
            60.0,
            40.0,
            [0.0; 3],
            [99.0, 0.0, 0.0]
        ));
        assert!(trail_split_skips_update(
            1,
            60.0,
            40.0,
            [0.0; 3],
            [101.0, 0.0, 0.0]
        ));
        // Sequence 4 has two: three times the range.
        assert!(!trail_split_skips_update(
            4,
            60.0,
            40.0,
            [0.0; 3],
            [299.0, 0.0, 0.0]
        ));
        assert!(trail_split_skips_update(
            4,
            60.0,
            40.0,
            [0.0; 3],
            [301.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn the_interpolant_is_where_the_total_crosses_the_mark() {
        assert_eq!(trail_split_interpolant_t(1.0, 0.5, 1.5), 0.5);
        assert_eq!(trail_split_interpolant_t(2.0, 0.5, 2.5), 0.75);
    }

    #[test]
    fn the_cut_time_is_between_the_two_updates() {
        assert_eq!(trail_split_interpolant_msec(1000, 1100, 0.5), 1050);
        assert_eq!(trail_split_interpolant_msec(1000, 1100, 0.0), 1000);
        assert_eq!(trail_split_interpolant_msec(1000, 1100, 1.0), 1100);
    }

    #[test]
    fn the_cut_origin_lies_on_the_move() {
        assert_eq!(
            trail_split_lerp_origin([0.0, 0.0, 10.0], [10.0, 4.0, 20.0], 0.25),
            [2.5, 1.0, 12.5]
        );
    }

    #[test]
    fn blending_an_orientation_with_itself_changes_nothing() {
        let got = trail_split_lerp_axis(IDENTITY, IDENTITY, 0.4);
        for (row, want) in got.iter().zip(IDENTITY) {
            for (a, b) in row.iter().zip(want) {
                assert!((a - b).abs() < 1e-5, "{got:?}");
            }
        }
    }

    #[test]
    fn blending_halfway_to_a_quarter_turn_gives_an_eighth_turn() {
        // A quarter turn about z.
        let quarter = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let got = trail_split_lerp_axis(IDENTITY, quarter, 0.5);
        let half_root_two = core::f32::consts::FRAC_1_SQRT_2;
        assert!((got[0][0].abs() - half_root_two).abs() < 1e-3, "{got:?}");
        assert!((got[0][1].abs() - half_root_two).abs() < 1e-3, "{got:?}");
    }
}
