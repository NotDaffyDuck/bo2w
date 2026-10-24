//! The caster bound of each sun cascade.
//!
//! A cascade's orthographic matrix says where casters land in the atlas, not
//! which ones are worth drawing: its depth range spans the whole map, so using
//! its six frustum planes as the collection volume admits the entire world
//! swept along the sun. The real bound is narrower and built here from three
//! things: the arc the camera frustum sweeps as the sun sees it, the cascade's
//! own usable rectangle, and, for the far cascade, the seam where the near one
//! ends.

mod arc;
mod boundary;
mod plane;
mod rays;
mod rect;
mod seam;

use bevy::math::Vec3;
use render_frame::SunShadowClipPlanes;

pub use rays::{SunShadowFrustumRays, sun_shadow_frustum_rays};

/// A sine this close to zero is no turn at all, and a normal this close to
/// perpendicular to the sun runs along it.
const ARC_CLASSIFICATION_EPSILON: f32 = 0.001;

/// A cascade side is only worth a plane when what is left of its edge is at
/// least this fraction of the cascade's half extent.
const BOUNDARY_LENGTH_FRACTION: f32 = 0.02;

/// How far a cascade's rectangle is pulled in, as a fraction of its width, so
/// the seam is judged on ground the cascade surely resolves.
const CULLING_RECTANGLE_GUARD_FRACTION: f32 = 0.0625;

/// Two rectangles whose spans overlap or come this close count as touching.
const RECTANGLE_CONNECTION_EPSILON: f32 = 0.01;

/// A far texel is this many near texels wide.
const FAR_PARTITION_SAMPLE_SCALE: f32 = 4.0;

/// Everything the plane set needs from the cascade projection.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowClipInput {
    /// `[axis0, axis1, light_direction]`.
    pub packed_axes: [[f32; 3]; 3],
    pub camera_origin: [f32; 3],
    pub camera_forward: [f32; 3],

    /// Distance from the eye to the near clip plane.
    pub camera_near_distance: f32,

    /// Where the near cascade stops resolving, along the camera's forward axis.
    pub near_shadow_min_distance: f32,

    /// The eye in the sun basis, before snapping.
    pub shadow_origin: [f32; 2],

    /// Where the eye sits inside the cascade rectangle, in texels.
    pub shadow_origin_pixel_center: [f32; 2],
    pub snapped_shadow_origin: [[f32; 2]; 2],
    pub sample_size: [f32; 2],

    /// Width of a cascade rectangle in texels.
    pub useful_size: u32,
}

impl SunShadowClipInput {
    fn axis(&self, index: usize) -> Vec3 {
        Vec3::from_array(self.packed_axes[index])
    }

    /// The cascade's rectangle in the sun basis, as `[min_x, min_y, max_x, max_y]`.
    fn rectangle(&self, partition: usize) -> [f32; 4] {
        let sample = self.sample_size[partition];
        let snapped = self.snapped_shadow_origin[partition];
        let min_x = snapped[0] - sample * self.shadow_origin_pixel_center[0];
        let min_y = snapped[1] - sample * self.shadow_origin_pixel_center[1];
        let span = self.useful_size as f32 * sample;
        [min_x, min_y, min_x + span, min_y + span]
    }
}

/// Builds both cascades' caster bounds as world-space planes, each holding for
/// the points inside it.
///
/// The arc comes first in both, then the near cascade's sides; the far cascade
/// gets the seam before its own sides.
#[must_use]
pub fn sun_shadow_clip_planes(
    input: &SunShadowClipInput,
    rays: &SunShadowFrustumRays,
) -> [SunShadowClipPlanes; 2] {
    let mut sets = [SunShadowClipPlanes::EMPTY; 2];
    arc::add(&mut sets, input, rays);
    boundary::add(&mut sets[0], input, rays, 0);
    seam::add(&mut sets[1], input, rays);
    boundary::add(&mut sets[1], input, rays, 1);
    sets
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const IDENTITY_AXES: [[f32; 3]; 3] =
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    /// An eye at the origin, looking along +x, in the middle of both cascades:
    /// the near one 64 wide (+-32) and the far one 256 wide (+-128).
    pub(super) fn centred_input() -> SunShadowClipInput {
        SunShadowClipInput {
            packed_axes: IDENTITY_AXES,
            camera_origin: [0.0; 3],
            camera_forward: [1.0, 0.0, 0.0],
            camera_near_distance: 1.0,
            near_shadow_min_distance: 16.0,
            shadow_origin: [0.0, 0.0],
            shadow_origin_pixel_center: [32.0, 32.0],
            snapped_shadow_origin: [[0.0; 2]; 2],
            sample_size: [1.0, 4.0],
            useful_size: 64,
        }
    }

    /// The far cascade's rectangle moved to be centred on `[dx, dy]`.
    pub(super) fn with_far_shift(dx: f32, dy: f32) -> SunShadowClipInput {
        let mut input = centred_input();
        input.snapped_shadow_origin[1] = [dx, dy];
        input
    }

    /// Corner rays that, seen along the sun's z axis, outline `outline`.
    pub(super) fn rays_over(outline: [[f32; 2]; 4]) -> SunShadowFrustumRays {
        sun_shadow_frustum_rays(outline.map(|[x, y]| [x, y, 5.0]), IDENTITY_AXES)
    }

    fn has(set: &SunShadowClipPlanes, expected: [f32; 4]) -> bool {
        set.as_slice().iter().any(|actual| {
            actual
                .iter()
                .zip(expected)
                .all(|(got, want)| (got - want).abs() < 1e-3)
        })
    }

    fn planes(input: &SunShadowClipInput, outline: [[f32; 2]; 4]) -> [SunShadowClipPlanes; 2] {
        sun_shadow_clip_planes(input, &rays_over(outline))
    }

    #[test]
    fn the_rectangle_is_laid_out_round_the_eye() {
        let mut input = centred_input();
        assert_eq!(input.rectangle(0), [-32.0, -32.0, 32.0, 32.0]);
        assert_eq!(input.rectangle(1), [-128.0, -128.0, 128.0, 128.0]);

        input.snapped_shadow_origin[0] = [10.0, -5.0];
        input.shadow_origin_pixel_center = [16.0, 48.0];
        assert_eq!(input.rectangle(0), [-6.0, -53.0, 58.0, 11.0]);
    }

    #[test]
    fn a_frustum_beside_the_sun_gets_a_wedge_and_a_seam() {
        let [near, far] = planes(
            &centred_input(),
            [[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]],
        );
        // The two arc sides, then the near cascade's sides that still cut
        // (the +x side lies beyond the wedge's end and is dropped).
        assert_eq!(near.len(), 5);
        assert!(has(&near, [-1.0, 0.0, 0.0, 32.0]));
        assert!(!has(&near, [1.0, 0.0, 0.0, 32.0]));
        // The far cascade also has the seam where the near one ends.
        assert_eq!(far.len(), 6);
        assert!(has(&far, [1.0, 0.0, 0.0, -26.0]));
    }

    #[test]
    fn a_frustum_pointing_away_is_cut_by_depth_and_boxed() {
        let [near, far] = planes(
            &centred_input(),
            [[1.0, 1.0], [1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0]],
        );
        assert_eq!(near.len(), 4);
        assert!(has(&near, [1.0, 0.0, 0.0, -1.0]));
        assert_eq!(far.len(), 4);
        assert!(has(&far, [1.0, 0.0, 0.0, -16.0]));
        assert!(has(&near, [-1.0, 0.0, 0.0, 32.0]));
        assert!(has(&far, [-1.0, 0.0, 0.0, 128.0]));
    }

    #[test]
    fn a_frustum_round_the_sun_is_just_the_cascade_rectangles() {
        let [near, far] = planes(
            &centred_input(),
            [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]],
        );
        assert_eq!(near.len(), 4);
        assert_eq!(far.len(), 4);
        assert_eq!(near.as_slice()[0], [0.0, -1.0, 0.0, 32.0]);
        assert_eq!(far.as_slice()[0], [0.0, -1.0, 0.0, 128.0]);
    }

    #[test]
    fn the_tighter_pair_of_sides_comes_first() {
        let mut input = centred_input();
        input.snapped_shadow_origin[0] = [0.0, 10.0];
        let [near, _] = planes(&input, [[1.0, 1.0], [-1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]]);
        assert_eq!(
            near.as_slice(),
            [
                [0.0, 1.0, 0.0, 22.0],
                [0.0, -1.0, 0.0, 42.0],
                [-1.0, 0.0, 0.0, 32.0],
                [1.0, 0.0, 0.0, 32.0],
            ]
        );
    }

    #[test]
    fn nothing_is_above_the_cap() {
        // Arc (2) + sides (4) + seam (1) never reaches the cap of a set.
        let [near, far] = planes(
            &centred_input(),
            [[1.0, -1.0], [1.0, 1.0], [2.0, 1.0], [2.0, -1.0]],
        );
        assert!(near.len() <= 10 && far.len() <= 10);
    }
}
