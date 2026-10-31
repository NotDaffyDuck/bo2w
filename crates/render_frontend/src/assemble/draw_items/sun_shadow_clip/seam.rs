//! The seam between the cascades.

use bevy::math::Vec2;
use render_frame::SunShadowClipPlanes;

use super::SunShadowClipInput;
use super::plane::{push_finite, unit2};
use super::rays::SunShadowFrustumRays;
use super::rect::{Run, culling_rectangles, ray_run, segment_inside};

/// A ray from the eye and how far it stays inside the rectangles.

/// One plane that cuts the far cascade off where the near one takes over.
///
/// It is laid across the two rays that bound the arc, at the points where they
/// leave the near cascade's rectangles, and pushed out by half a far texel.
/// Nothing is added when the frustum sweeps no arc, when an arc ray never runs
/// inside the rectangles, or when no pair of points on them can be joined
/// without leaving the rectangles.
pub(super) fn add(
    set: &mut SunShadowClipPlanes,
    input: &SunShadowClipInput,
    rays: &SunShadowFrustumRays,
) {
    let Some((first, last)) = rays.bounding_arc else {
        return;
    };
    let (Some(first_direction), Some(last_direction)) = (
        unit2(rays.shadow_rays[first]),
        unit2(rays.shadow_rays[last]),
    ) else {
        return;
    };
    let rects = culling_rectangles(input);
    let origin = Vec2::from_array(input.shadow_origin);
    let (Some(first_run), Some(last_run)) = (
        ray_run(origin, first_direction, &rects),
        ray_run(origin, last_direction, &rects),
    ) else {
        return;
    };
    let reaches = [(first_direction, first_run), (last_direction, last_run)];
    let Some([start, end]) = seam_ends(origin, reaches, &rects) else {
        return;
    };

    let Some(across) = unit2([end.y - start.y, start.x - end.x]) else {
        return;
    };
    let normal = input.axis(0) * across.x + input.axis(1) * across.y;
    let offset = 0.5 * input.probe_size[1] * (across.x.abs() + across.y.abs()) - across.dot(start);
    push_finite(set, normal, offset);
}

/// Where the seam starts and ends.
///
/// Each ray can be taken to where the whole run of rectangles ends, or, when
/// that is further, only to where it leaves the first rectangle. The far ends
/// are tried first; with several rectangles, when the segment between them
/// strays outside, the combinations that use a near end follow, the one that
/// shortens the ray with the larger share of its run first.
fn seam_ends(origin: Vec2, reaches: [(Vec2, Run); 2], rects: &[[f32; 4]]) -> Option<[Vec2; 2]> {
    let point = |ray: usize, near_end: bool| {
        let (direction, run) = reaches[ray];
        origin + direction * if near_end { run.first_exit } else { run.end }
    };
    let far_ends = [point(0, false), point(1, false)];
    if rects.len() == 1 || segment_inside(far_ends, rects) {
        return Some(far_ends);
    }

    let [(_, run0), (_, run1)] = reaches;
    let can_stop_early = [run0.first_exit < run0.end, run1.first_exit < run1.end];
    let first_is_nearer_end = run0.first_exit * run1.end < run1.first_exit * run0.end;
    let combinations: &[(bool, bool)] = match (can_stop_early, first_is_nearer_end) {
        ([false, false], _) => &[],
        ([false, true], _) => &[(false, true)],
        ([true, false], _) => &[(true, false)],
        ([true, true], true) => &[(false, true), (true, false), (true, true)],
        ([true, true], false) => &[(true, false), (false, true), (true, true)],
    };
    combinations
        .iter()
        .map(|&(first_near, last_near)| [point(0, first_near), point(1, last_near)])
        .find(|&ends| segment_inside(ends, rects))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{centred_input, rays_over, with_far_shift};
    use super::*;

    const WEDGE: [[f32; 2]; 4] = [[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]];
    const ROUND_THE_SUN: [[f32; 2]; 4] = [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]];

    fn seam(input: &SunShadowClipInput, outline: [[f32; 2]; 4]) -> Vec<[f32; 4]> {
        let mut set = SunShadowClipPlanes::BLANK;
        add(&mut set, input, &rays_over(outline));
        set.as_slice().to_vec()
    }

    fn near(left: Vec2, right: Vec2) -> bool {
        (left - right).length() < 1e-3
    }

    fn assert_plane(actual: &[f32; 4], expected: [f32; 4]) {
        for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (got - want).abs() < 1e-3,
                "{actual:?} against {expected:?}, at {index}"
            );
        }
    }

    #[test]
    fn the_seam_lies_where_the_arc_leaves_the_near_rectangle() {
        // The arc rays leave the guarded near rectangle (+-28) at x = 28, and
        // the plane is pushed back by half a far texel (2).
        let seam = seam(&centred_input(), WEDGE);
        assert_eq!(seam.len(), 1);
        assert_plane(&seam[0], [1.0, 0.0, 0.0, -26.0]);
    }

    #[test]
    fn a_seam_looks_at_the_far_cascades_texels() {
        let mut input = centred_input();
        input.probe_size[1] = 8.0;
        assert_plane(&seam(&input, WEDGE)[0], [1.0, 0.0, 0.0, -24.0]);
    }

    #[test]
    fn without_an_arc_there_is_no_seam() {
        assert!(seam(&centred_input(), ROUND_THE_SUN).is_empty());
    }

    #[test]
    fn an_eye_outside_the_rectangles_has_no_seam() {
        let mut input = centred_input();
        input.shadow_origin = [100.0, 0.0];
        assert!(seam(&input, WEDGE).is_empty());
    }

    #[test]
    fn a_far_cascade_that_stops_short_moves_the_seam_out() {
        // The extra strip reaches the arc rays at x = 32 instead of 28.
        let seam = seam(&with_far_shift(150.0, 0.0), WEDGE);
        assert_eq!(seam.len(), 1);
        assert_plane(&seam[0], [1.0, 0.0, 0.0, -30.0]);
    }

    #[test]
    fn one_rectangle_needs_no_check() {
        let rect = [[-1.0, -1.0, 1.0, 1.0]];
        let run = |direction| ray_run(Vec2::ZERO, direction, &rect).expect("starts inside");
        let reaches = [(Vec2::X, run(Vec2::X)), (Vec2::Y, run(Vec2::Y))];
        assert_eq!(
            seam_ends(Vec2::ZERO, reaches, &rect),
            Some([Vec2::X, Vec2::Y])
        );
    }

    /// Two strips off a small square, leaving the corner between them empty.
    fn an_l_of_rectangles() -> [[f32; 4]; 3] {
        [
            [-8.0, -8.0, 8.0, 8.0],
            [-12.0, -12.0, 20.0, 12.0],
            [-12.0, -12.0, 12.0, 20.0],
        ]
    }

    #[test]
    fn a_seam_that_leaves_the_rectangles_stops_a_ray_early() {
        let rects = an_l_of_rectangles();
        let first = Vec2::new(2.0, 1.0).normalize();
        let last = Vec2::new(1.0, 2.0).normalize();
        let run = |direction| ray_run(Vec2::ZERO, direction, &rects).expect("starts inside");
        let reaches = [(first, run(first)), (last, run(last))];
        // Far ends: (20, 10) and (10, 20), across the empty corner.
        assert!(near(first * reaches[0].1.end, Vec2::new(20.0, 10.0)));
        assert!(near(last * reaches[1].1.end, Vec2::new(10.0, 20.0)));
        let across_the_corner = [Vec2::new(20.0, 10.0), Vec2::new(10.0, 20.0)];
        assert!(!segment_inside(across_the_corner, &rects));

        let [start, end] = seam_ends(Vec2::ZERO, reaches, &rects).expect("an end can be nearer");
        assert!(segment_inside([start, end], &rects));
        let (near_first, near_last) = (Vec2::new(8.0, 4.0), Vec2::new(4.0, 8.0));
        let stopped_first = near(start, near_first) && near(end, Vec2::new(10.0, 20.0));
        let stopped_last = near(start, Vec2::new(20.0, 10.0)) && near(end, near_last);
        assert!(stopped_first || stopped_last, "{start} {end}");
    }

    #[test]
    fn a_seam_across_a_gap_is_dropped_when_no_end_can_be_nearer() {
        // Two rectangles with empty space between them; the ray that ends at
        // y = 3 lies in that space.
        let rects = [[-1.0, -1.0, 1.0, 1.0], [-1.0, 4.0, 1.0, 5.0]];
        let diagonal = Vec2::new(1.0, 1.0).normalize();
        let run = Run {
            end: 1.4,
            first_exit: 1.4,
        };
        let inside = [(diagonal, run), (Vec2::new(-1.0, 1.0).normalize(), run)];
        assert!(seam_ends(Vec2::ZERO, inside, &rects).is_some());

        let into_the_gap = Run {
            end: 3.0,
            first_exit: 3.0,
        };
        let gap = [(diagonal, run), (Vec2::Y, into_the_gap)];
        assert_eq!(seam_ends(Vec2::ZERO, gap, &rects), None);
    }
}
