//! Rays and segments against the cascades' rectangles in the sun basis.
//!
//! A rectangle is `[min_x, min_y, max_x, max_y]`; a ray is measured in
//! distance from its origin, so its direction must be a unit vector.

use bevy::math::Vec2;

use super::{
    ARC_CLASSIFICATION_EPSILON, CULLING_RECTANGLE_GUARD_FRACTION, RECTANGLE_CONNECTION_EPSILON,
    SunShadowClipInput,
};


/// How far a ray from the eye stays inside the connected run of rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Run {
    /// Where it leaves the run.
    pub end: f32,

    /// Where it leaves the first rectangle, never past `end`.
    pub first_exit: f32,
}

fn inset(rect: [f32; 4], amount: f32) -> [f32; 4] {
    [
        rect[0] + amount,
        rect[1] + amount,
        rect[2] - amount,
        rect[3] - amount,
    ]
}

/// The near rectangle, pulled in by a guard band, plus whatever strips of the
/// full near rectangle the far one, also pulled in, does not reach.
pub(super) fn culling_rectangles(input: &SunShadowClipInput) -> Vec<[f32; 4]> {
    let near = input.rectangle(0);
    let far = input.rectangle(1);
    let useful = input.useful_size as f32;
    let near_inner = inset(
        near,
        useful * input.probe_size[0] * CULLING_RECTANGLE_GUARD_FRACTION,
    );
    let far_inner = inset(
        far,
        useful * input.probe_size[1] * CULLING_RECTANGLE_GUARD_FRACTION,
    );

    let mut rects = vec![near_inner];
    if far_inner[0] > near[0] {
        rects.push([near[0], near[1], far_inner[0], near[3]]);
    } else if far_inner[2] < near[2] {
        rects.push([far_inner[2], near[1], near[2], near[3]]);
    }
    if far_inner[1] > near[1] {
        rects.push([near[0], near[1], near[2], far_inner[1]]);
    } else if far_inner[3] < near[3] {
        rects.push([near[0], far_inner[3], near[2], near[3]]);
    }
    rects
}

/// Entry and exit distance of a ray across one band `low..high` of an axis.
fn band(origin: f32, direction: f32, low: f32, high: f32) -> (f32, f32) {
    if direction < 0.0 {
        ((high - origin) / direction, (low - origin) / direction)
    } else {
        ((low - origin) / direction, (high - origin) / direction)
    }
}

/// Entry and exit distance of a ray through a rectangle.
///
/// A ray running along one axis is judged on that axis alone, as if it moved
/// one unit per unit of distance.
fn interval(origin: Vec2, dir_v: Vec2, rect: [f32; 4]) -> Vec2 {
    let along = |value: f32| if value < 0.0 { -1.0 } else { 1.0 };
    let (enter, exit) = if dir_v.x.abs() < ARC_CLASSIFICATION_EPSILON {
        band(origin.y, along(dir_v.y), rect[1], rect[3])
    } else if dir_v.y.abs() < ARC_CLASSIFICATION_EPSILON {
        band(origin.x, along(dir_v.x), rect[0], rect[2])
    } else {
        let across_x = band(origin.x, dir_v.x, rect[0], rect[2]);
        let across_y = band(origin.y, dir_v.y, rect[1], rect[3]);
        (across_x.0.max(across_y.0), across_x.1.min(across_y.1))
    };
    Vec2::new(enter, exit)
}

/// The interval when it has a length and does not end behind the origin.
fn forward_interval(origin: Vec2, direction: Vec2, rect: [f32; 4]) -> Option<Vec2> {
    let span = interval(origin, direction, rect);
    (span.x < span.y && span.y >= RECTANGLE_CONNECTION_EPSILON).then_some(span)
}

/// How far the touching intervals reach, starting from the one that holds the
/// origin; `None` when none does.
fn connected_end(spans: &mut [Vec2]) -> Option<f32> {
    spans.sort_by(|left, right| left.x.total_cmp(&right.x));
    let (first, rest) = spans.split_first()?;
    if first.x > RECTANGLE_CONNECTION_EPSILON {
        return None;
    }
    let mut end = first.y;
    for span in rest {
        if span.x >= end + RECTANGLE_CONNECTION_EPSILON {
            break;
        }
        end = end.max(span.y);
    }
    Some(end)
}

/// How far the ray from the eye stays inside the rectangles.
pub(super) fn ray_run(origin: Vec2, direction: Vec2, rects: &[[f32; 4]]) -> Option<Run> {
    let mut first_exit = f32::MAX * 0.5;
    let mut spans = Vec::with_capacity(rects.len());
    for (index, rect) in rects.iter().enumerate() {
        let Some(span) = forward_interval(origin, direction, *rect) else {
            continue;
        };
        if index == 0 {
            first_exit = span.y;
        }
        spans.push(span);
    }
    let end = connected_end(&mut spans)?;
    Some(Run {
        end,
        first_exit: first_exit.min(end),
    })
}

/// Whether the whole segment lies within the connected rectangles.
pub(super) fn segment_inside([start, end]: [Vec2; 2], rects: &[[f32; 4]]) -> bool {
    let delta = end - start;
    let length = delta.length();
    if length <= 0.0 || !length.is_finite() {
        return false;
    }
    let direction = delta / length;
    let mut spans: Vec<Vec2> = rects
        .iter()
        .filter_map(|rect| forward_interval(start, direction, *rect))
        .collect();
    connected_end(&mut spans).is_some_and(|reach| length - RECTANGLE_CONNECTION_EPSILON <= reach)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{centred_input, with_far_shift};
    use super::*;

    const SQUARE: [f32; 4] = [-1.0, -1.0, 3.0, 3.0];

    fn unit(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).normalize()
    }

    fn close(left: Vec2, right: Vec2) -> bool {
        (left - right).length() < 1e-5
    }

    #[test]
    fn a_ray_along_an_axis_is_measured_on_that_axis() {
        let rect = [-1.0, -2.0, 3.0, 5.0];
        assert_eq!(interval(Vec2::ZERO, Vec2::X, rect), Vec2::new(-1.0, 3.0));
        assert_eq!(
            interval(Vec2::ZERO, Vec2::NEG_X, rect),
            Vec2::new(-3.0, 1.0)
        );
        assert_eq!(interval(Vec2::ZERO, Vec2::Y, rect), Vec2::new(-2.0, 5.0));
        assert_eq!(
            interval(Vec2::ZERO, Vec2::NEG_Y, rect),
            Vec2::new(-5.0, 2.0)
        );
    }

    #[test]
    fn a_slanted_ray_is_bound_by_whichever_side_it_meets_last() {
        let span = interval(Vec2::ZERO, unit(1.0, 1.0), SQUARE);
        assert!(close(span, Vec2::new(-1.0, 3.0) * std::f32::consts::SQRT_2));

        // Wide rectangle: the ray leaves through the top.
        let span = interval(Vec2::ZERO, unit(1.0, 1.0), [-10.0, -1.0, 10.0, 2.0]);
        assert!(close(span, Vec2::new(-1.0, 2.0) * std::f32::consts::SQRT_2));
    }

    #[test]
    fn a_ray_that_misses_has_an_empty_interval() {
        let span = interval(Vec2::new(10.0, 0.0), unit(1.0, 1.0), SQUARE);
        assert!(span.x >= span.y, "{span}");
        assert_eq!(
            forward_interval(Vec2::new(10.0, 0.0), unit(1.0, 1.0), SQUARE),
            None
        );
    }

    #[test]
    fn an_interval_wholly_behind_the_origin_is_dropped() {
        assert_eq!(forward_interval(Vec2::new(5.0, 0.0), Vec2::X, SQUARE), None);
        assert_eq!(
            forward_interval(Vec2::new(5.0, 0.0), Vec2::NEG_X, SQUARE),
            Some(Vec2::new(2.0, 6.0))
        );
    }

    #[test]
    fn touching_intervals_join_up() {
        let mut spans = [Vec2::new(1.5, 4.0), Vec2::new(-1.0, 2.0)];
        assert_eq!(connected_end(&mut spans), Some(4.0));
    }

    #[test]
    fn a_gap_ends_the_run() {
        let mut spans = [Vec2::new(-1.0, 1.0), Vec2::new(2.0, 3.0)];
        assert_eq!(connected_end(&mut spans), Some(1.0));

        let mut inside = [Vec2::new(-1.0, 5.0), Vec2::new(1.0, 2.0)];
        assert_eq!(connected_end(&mut inside), Some(5.0));
    }

    #[test]
    fn a_run_must_start_at_the_origin() {
        assert_eq!(connected_end(&mut [Vec2::new(0.5, 2.0)]), None);
        assert_eq!(connected_end(&mut [Vec2::new(0.005, 2.0)]), Some(2.0));
        assert_eq!(connected_end(&mut []), None);
    }

    #[test]
    fn a_ray_runs_out_of_the_first_rectangle_before_the_run_ends() {
        let rects = [[-1.0, -1.0, 2.0, 1.0], [2.0, -1.0, 6.0, 1.0]];
        assert_eq!(
            ray_run(Vec2::ZERO, Vec2::X, &rects),
            Some(Run {
                end: 6.0,
                first_exit: 2.0
            })
        );
        // Only the first rectangle: both are the same.
        assert_eq!(
            ray_run(Vec2::ZERO, Vec2::X, &rects[..1]),
            Some(Run {
                end: 2.0,
                first_exit: 2.0
            })
        );
    }

    #[test]
    fn a_ray_that_never_starts_inside_has_no_run() {
        let rects = [[3.0, -1.0, 5.0, 1.0]];
        assert_eq!(ray_run(Vec2::ZERO, Vec2::X, &rects), None);
        assert_eq!(ray_run(Vec2::ZERO, Vec2::X, &[]), None);
    }

    #[test]
    fn a_first_rectangle_behind_the_eye_never_cuts_the_run_short() {
        let rects = [[-5.0, -1.0, -3.0, 1.0], [-1.0, -1.0, 4.0, 1.0]];
        assert_eq!(
            ray_run(Vec2::ZERO, Vec2::X, &rects),
            Some(Run {
                end: 4.0,
                first_exit: 4.0
            })
        );
    }

    #[test]
    fn a_segment_inside_one_rectangle_is_inside() {
        let rects = [SQUARE];
        assert!(segment_inside([Vec2::ZERO, Vec2::new(2.0, 2.0)], &rects));
        assert!(!segment_inside([Vec2::ZERO, Vec2::new(5.0, 0.0)], &rects));
        // A little over the edge is forgiven.
        assert!(segment_inside([Vec2::ZERO, Vec2::new(3.005, 0.0)], &rects));
    }

    #[test]
    fn a_segment_may_cross_from_one_rectangle_into_the_next() {
        let rects = [[-1.0, -1.0, 2.0, 1.0], [2.0, -1.0, 6.0, 1.0]];
        assert!(segment_inside([Vec2::ZERO, Vec2::new(5.0, 0.0)], &rects));
        assert!(!segment_inside([Vec2::ZERO, Vec2::new(7.0, 0.0)], &rects));
    }

    #[test]
    fn a_segment_with_no_length_is_not_inside() {
        assert!(!segment_inside([Vec2::ZERO, Vec2::ZERO], &[SQUARE]));
        assert!(!segment_inside(
            [Vec2::ZERO, Vec2::new(f32::NAN, 0.0)],
            &[SQUARE]
        ));
        assert!(!segment_inside(
            [Vec2::ZERO, Vec2::new(f32::INFINITY, 0.0)],
            &[SQUARE]
        ));
    }

    #[test]
    fn centred_cascades_are_culled_by_the_near_rectangle_alone() {
        // Near +-32, far +-128; guard bands 4 and 16.
        let rects = culling_rectangles(&centred_input());
        assert_eq!(rects, vec![[-28.0, -28.0, 28.0, 28.0]]);
    }

    #[test]
    fn a_far_rectangle_that_stops_short_adds_a_strip_of_the_near_one() {
        // Far shifted right by 150: its inner left edge is 22 + 16 = 38 > -32.
        let input = with_far_shift(150.0, 0.0);
        let rects = culling_rectangles(&input);
        assert_eq!(rects.len(), 2, "{rects:?}");
        assert_eq!(rects[1], [-32.0, -32.0, input.rectangle(1)[0] + 16.0, 32.0]);

        // Shifted left: the strip is on the right.
        let input = with_far_shift(-150.0, 0.0);
        let rects = culling_rectangles(&input);
        assert_eq!(rects.len(), 2, "{rects:?}");
        assert_eq!(rects[1], [input.rectangle(1)[2] - 16.0, -32.0, 32.0, 32.0]);
    }

    #[test]
    fn both_axes_can_add_a_strip() {
        let rects = culling_rectangles(&with_far_shift(150.0, -150.0));
        assert_eq!(rects.len(), 3, "{rects:?}");
    }
}
