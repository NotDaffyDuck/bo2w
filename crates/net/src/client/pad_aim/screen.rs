//! Where each target sits on a screen taken as the square -1..1 either way.

use super::view_math::{axes, dot, minus};
use super::{AimTarget, AimView};

/// A target on the screen: the box it covers, how far away its aim point is, and how far from the
/// crosshair its middle is (both squared).
pub(super) struct OnScreen {
    pub(super) target: AimTarget,
    pub(super) min: [f32; 2],
    pub(super) max: [f32; 2],
    pub(super) dist_sqr: f32,
    pub(super) crosshair_sqr: f32,
}

/// The targets whose box shows on the screen, nearest the crosshair first.
pub(super) fn project(view: &AimView, targets: &[AimTarget]) -> Vec<OnScreen> {
    let (ahead, side, above) = axes(view.pitch_yaw);
    let tan_x = (32.5f32.to_radians().tan() * view.view_angle_multiplier).max(1e-3);
    let tan_y = tan_x * 9.0 / 16.0;

    let mut seen: Vec<OnScreen> = targets
        .iter()
        .filter_map(|target| {
            let mut min = [f32::MAX; 2];
            let mut max = [f32::MIN; 2];
            for corner in 0..8u8 {
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        target.box_lo[axis]
                    } else {
                        target.box_hi[axis]
                    }
                };
                let d = minus([pick(0), pick(1), pick(2)], view.eye);
                let depth = dot(d, ahead);
                if depth <= 0.0 {
                    continue;
                }
                let x = dot(d, side) / (depth * tan_x);
                let y = -dot(d, above) / (depth * tan_y);
                min = [min[0].min(x), min[1].min(y)];
                max = [max[0].max(x), max[1].max(y)];
            }
            let empty = max[0] <= min[0] || max[1] <= min[1];
            let off_screen = min[0] > 1.0 || min[1] > 1.0 || max[0] < -1.0 || max[1] < -1.0;
            if empty || off_screen {
                return None;
            }
            let min = [min[0].clamp(-1.0, 1.0), min[1].clamp(-1.0, 1.0)];
            let max = [max[0].clamp(-1.0, 1.0), max[1].clamp(-1.0, 1.0)];
            let middle = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
            let to_aim = minus(target.aim, view.eye);
            Some(OnScreen {
                target: *target,
                min,
                max,
                dist_sqr: dot(to_aim, to_aim),
                crosshair_sqr: middle[0] * middle[0] + middle[1] * middle[1],
            })
        })
        .collect();
    seen.sort_by(|a, b| a.crosshair_sqr.total_cmp(&b.crosshair_sqr));
    seen
}

/// The first of `seen` (the nearest the crosshair) within `range` whose box reaches into the
/// region around the crosshair. The region is given in 640x480 screen pixels, grown by `scale`.
pub(super) fn best(
    seen: &[OnScreen],
    range: f32,
    region: [f32; 2],
    zoom: f32,
) -> Option<&OnScreen> {
    seen.iter().find(|s| s.reaches(range, region, zoom))
}

impl OnScreen {
    /// Whether this target is within `range` and its box reaches into the crosshair region.
    pub(super) fn reaches(&self, range: f32, region: [f32; 2], zoom: f32) -> bool {
        let half = [region[0] / 640.0 * zoom, region[1] / 480.0 * zoom];
        self.dist_sqr <= range * range
            && self.min[0] <= half[0]
            && self.max[0] >= -half[0]
            && self.min[1] <= half[1]
            && self.max[1] >= -half[1]
    }
}

#[cfg(test)]
mod tests {
    use weapon_t6::combat_facts::AimAssistRanges;

    use super::*;

    fn view() -> AimView {
        AimView {
            eye: [0.0; 3],
            pitch_yaw: [0.0; 2],
            drift: [0.0; 3],
            ads_lerp: 0.0,
            view_angle_multiplier: 1.0,
            ranges: AimAssistRanges::OFF,
            dt: 0.01,
        }
    }

    fn boxed(key: u64, centre: [f32; 3], half: f32) -> AimTarget {
        AimTarget {
            key,
            box_lo: centre.map(|c| c - half),
            box_hi: centre.map(|c| c + half),
            aim: centre,
            drift: [0.0; 3],
        }
    }

    #[test]
    fn a_box_ahead_is_seen_and_one_behind_is_not() {
        let seen = project(&view(), &[boxed(1, [500.0, 0.0, 0.0], 20.0), boxed(2, [-500.0, 0.0, 0.0], 20.0)]);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].target.key, 1);
        assert_eq!(seen[0].dist_sqr, 250_000.0);
    }

    #[test]
    fn a_box_far_to_the_side_is_off_the_screen() {
        let seen = project(&view(), &[boxed(1, [100.0, 900.0, 0.0], 20.0)]);
        assert!(seen.is_empty());
    }

    #[test]
    fn the_nearest_to_the_crosshair_comes_first() {
        let seen = project(
            &view(),
            &[boxed(1, [500.0, 80.0, 0.0], 20.0), boxed(2, [500.0, 0.0, 0.0], 20.0)],
        );
        let keys: Vec<u64> = seen.iter().map(|s| s.target.key).collect();
        assert_eq!(keys, [2, 1]);
    }

    #[test]
    fn best_wants_the_target_in_range_and_in_the_region() {
        let seen = project(&view(), &[boxed(1, [500.0, 0.0, 0.0], 20.0)]);
        assert!(best(&seen, 1000.0, [90.0, 90.0], 1.0).is_some());
        assert!(best(&seen, 400.0, [90.0, 90.0], 1.0).is_none());
        // Even an empty region catches a box that covers the crosshair.
        assert!(best(&seen, 1000.0, [0.0, 0.0], 1.0).is_some());
        let aside = project(&view(), &[boxed(1, [500.0, 150.0, 0.0], 20.0)]);
        assert!(best(&aside, 1000.0, [90.0, 90.0], 1.0).is_none());
        assert!(best(&aside, 1000.0, [160.0, 120.0], 2.0).is_some());
    }
}
