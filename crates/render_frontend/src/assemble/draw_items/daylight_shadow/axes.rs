//! The sun's basis, the view matrix built on it, and where things sit in it.

use bevy::math::{Mat4, Vec3, Vec4};

use super::DAYLIGHT_SHADOW_HALF;

/// A sun this close to vertical (`|z|` above it) takes `x` rather than `z` as
/// the helper for building the basis.
pub const DAYLIGHT_AXIS_HELPER_Z_MAX: f32 = 0.9;

/// A length² below this is no direction at all.
const NO_DIRECTION: f32 = 1e-12;

/// Three axes, each `[x, y, z]`.
/// One `[x, y, z]` point or direction.
/// `(nearest, farthest)` along a direction.

fn unit_or(vector: Vec3, otherwise: Vec3) -> Vec3 {
    if vector.length_squared() > NO_DIRECTION {
        vector.normalize()
    } else {
        otherwise
    }
}

/// The direction the sun's light travels, turned round: the way the sun
/// "looks".
pub fn daylight_shadow_forward_from_light_dir(dir: [f32; 3]) -> [f32; 3] {
    (-Vec3::from_array(dir)).to_array()
}

/// `[axis0, axis1, light_direction]`: an orthonormal basis whose third vector
/// is the direction the light travels.
///
/// A light with no direction is taken to shine along `z`.
pub fn daylight_shadow_compact_axes_from_light_dir(travel: Vec3) -> [[f32; 3]; 3] {
    let light = unit_or(travel, Vec3::Z);
    let helper = if light.z.abs() <= DAYLIGHT_AXIS_HELPER_Z_MAX {
        Vec3::Z
    } else {
        Vec3::X
    };
    let axis_a = unit_or(helper.cross(light), Vec3::X);
    let axis_b = light.cross(axis_a);
    [axis_a.to_array(), axis_b.to_array(), light.to_array()]
}

/// The same basis as the engine's view axes, for a light travelling along
/// `travel`: `[forward, right, up]`, `forward` being against the light.
pub fn daylight_axes_from_light_dir(travel: Vec3) -> [[f32; 3]; 3] {
    let [axis_a, axis_b, light] = daylight_shadow_compact_axes_from_light_dir(travel).map(Vec3::from_array);
    [(-light).to_array(), (-axis_a).to_array(), axis_b.to_array()]
}

/// [`daylight_axes_from_light_dir`] for a sun given by the direction it sits in,
/// whose light travels the other way.
pub fn daylight_axes_from_dir(toward_sun: Vec3) -> [[f32; 3]; 3] {
    daylight_axes_from_light_dir(-toward_sun)
}

/// The world-to-sun-view matrix of a cascade whose eye is at `snapped` on the
/// view plane, given the sun's `[forward, right, up]`.
pub fn sun_shadow_view_matrix(sun_axes: [[f32; 3]; 3], snapped: [f32; 2]) -> Mat4 {
    let [forward, right, up] = sun_axes.map(Vec3::from_array);
    let across = -right;
    let view = Mat4::from_cols(
        Vec4::new(across.x, up.x, forward.x, 0.0),
        Vec4::new(across.y, up.y, forward.y, 0.0),
        Vec4::new(across.z, up.z, forward.z, 0.0),
        Vec4::new(-snapped[0], -snapped[1], 0.0, 1.0),
    );
    view
}

/// Where `origin` falls on the sun's view plane, as `[across, up]`.
pub fn eye_org_in_daylight_proj(origin: [f32; 3], daylight_axes: [[f32; 3]; 3]) -> [f32; 2] {
    let origin = Vec3::from_array(origin);
    let [_, right, up] = daylight_axes.map(Vec3::from_array);
    [-origin.dot(right), origin.dot(up)]
}

/// Moves `origin` to the middle of the texel (of `scale` world units) it is
/// in, so a cascade that follows the eye only ever moves a whole texel.
pub fn snap_daylight_proj_origin(origin: [f32; 2], size_k: f32) -> [f32; 2] {
    origin.map(|coordinate| ((coordinate / size_k).floor() + DAYLIGHT_SHADOW_HALF) * size_k)
}

/// How far along `forward` a box centred on `box_centre` with half-sizes `half_size`
/// reaches in each direction: `[nearest, farthest]`.
pub fn stage_extents_along_dir(forward: [f32; 3], box_centre: [f32; 3], half_size: [f32; 3]) -> [f32; 2] {
    let forward = Vec3::from_array(forward);
    let centre = forward.dot(Vec3::from_array(box_centre));
    let reach = forward.abs().dot(Vec3::from_array(half_size));
    [centre - reach, centre + reach]
}

/// The tangents of the half field of view, `(horizontal, vertical)`, read off
/// a perspective matrix. An axis the matrix does not scale gives 1.
pub fn half_fov_tan_of_clip(clip_from_view: Mat4) -> (f32, f32) {
    let tangent = |scale: f32| {
        if scale.abs() > NO_DIRECTION {
            1.0 / scale
        } else {
            1.0
        }
    };
    (
        tangent(clip_from_view.x_axis.x),
        tangent(clip_from_view.y_axis.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(left: [f32; 3], right_v: [f32; 3]) -> bool {
        left.iter().zip(right_v).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn the_forward_is_against_the_light() {
        assert_eq!(
            daylight_shadow_forward_from_light_dir([1.0, -2.0, 0.0]),
            [-1.0, 2.0, -0.0]
        );
    }

    #[test]
    fn the_packed_basis_is_orthonormal_and_ends_with_the_light() {
        for dir in [
            Vec3::new(0.3, -0.4, -0.8),
            Vec3::new(5.0, 0.0, 0.1),
            Vec3::NEG_Z,
            Vec3::new(0.0, 0.01, 0.99),
        ] {
            let [a0, a1, light] = daylight_shadow_compact_axes_from_light_dir(dir).map(Vec3::from_array);
            assert!((a0.length() - 1.0).abs() < 1e-5, "{dir:?}");
            assert!((a1.length() - 1.0).abs() < 1e-5, "{dir:?}");
            assert!((light.length() - 1.0).abs() < 1e-5, "{dir:?}");
            assert!(a0.dot(a1).abs() < 1e-5, "{dir:?}");
            assert!(a0.dot(light).abs() < 1e-5, "{dir:?}");
            assert!(a1.dot(light).abs() < 1e-5, "{dir:?}");
        }
    }

    #[test]
    fn the_light_is_normalised() {
        let [_, _, light] = daylight_shadow_compact_axes_from_light_dir(Vec3::NEG_Z * 10.0);
        assert_eq!(light, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn a_light_with_no_direction_shines_along_z() {
        assert_eq!(
            daylight_shadow_compact_axes_from_light_dir(Vec3::ZERO),
            [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        );
    }

    #[test]
    fn a_vertical_sun_is_built_from_the_x_axis() {
        // |z| = 1 > 0.9, so the helper is x: axis0 = x cross -z = +y.
        let [axis0, ..] = daylight_shadow_compact_axes_from_light_dir(Vec3::NEG_Z);
        assert!(near(axis0, [0.0, 1.0, 0.0]));
        let [horizontal, ..] = daylight_shadow_compact_axes_from_light_dir(Vec3::X);
        // Helper z crossed with +x is +y.
        assert!(near(horizontal, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn the_view_axes_are_the_packed_ones_reordered() {
        let dir = Vec3::new(0.3, -0.4, -0.8);
        let [forward, right, up] = daylight_axes_from_light_dir(dir).map(Vec3::from_array);
        let [a0, a1, light] = daylight_shadow_compact_axes_from_light_dir(dir).map(Vec3::from_array);
        assert_eq!(forward, -light);
        assert_eq!(right, -a0);
        assert_eq!(up, a1);
    }

    #[test]
    fn a_sun_given_by_position_shines_the_other_way() {
        let direction = Vec3::new(0.2, 0.5, 0.7);
        assert_eq!(
            daylight_axes_from_dir(direction),
            daylight_axes_from_light_dir(Vec3::new(-0.2, -0.5, -0.7))
        );
    }

    #[test]
    fn the_view_matrix_puts_the_snapped_eye_at_the_origin() {
        let axes = daylight_axes_from_dir(Vec3::Z);
        let org = eye_org_in_daylight_proj([3.0, 4.0, 5.0], axes);
        let view_pos = sun_shadow_view_matrix(axes, org);
        let eye = view_pos.transform_point3(Vec3::new(3.0, 4.0, 5.0));
        assert!(eye.x.abs() < 1e-5 && eye.y.abs() < 1e-5, "{eye}");
        // Depth is along the sun's forward axis.
        let forward = Vec3::from_array(axes[0]);
        let point = Vec3::new(3.0, 4.0, 5.0) + forward * 7.0;
        assert!((view_pos.transform_point3(point).z - eye.z - 7.0).abs() < 1e-4);
    }

    #[test]
    fn the_eye_on_the_view_plane_is_across_and_up() {
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(eye_org_in_daylight_proj([2.0, 3.0, 4.0], axes), [-3.0, 4.0]);
    }

    #[test]
    fn snapping_lands_in_the_middle_of_a_texel() {
        assert_eq!(snap_daylight_proj_origin([1.1, -0.1], 0.5), [1.25, -0.25]);
        assert_eq!(snap_daylight_proj_origin([0.0, 4.0], 2.0), [1.0, 5.0]);
        // Anything in the same texel snaps to the same place.
        assert_eq!(
            snap_daylight_proj_origin([2.01, 2.99], 1.0),
            snap_daylight_proj_origin([2.5, 2.5], 1.0)
        );
    }

    #[test]
    fn a_box_reaches_its_half_size_either_side_of_its_middle() {
        assert_eq!(
            stage_extents_along_dir([0.0, 0.0, 1.0], [5.0, 5.0, 10.0], [1.0, 1.0, 4.0]),
            [6.0, 14.0]
        );
        // A slanted direction sees the box's slant.
        let [near, far] =
            stage_extents_along_dir([-1.0, 0.0, 0.0], [10.0, 0.0, 0.0], [2.0, 9.0, 9.0]);
        assert_eq!([near, far], [-12.0, -8.0]);
    }

    #[test]
    fn the_field_of_view_is_read_back_from_the_projection() {
        let projection = Mat4::perspective_infinite_reverse_rh(1.0, 1.5, 0.1);
        let (x, y) = half_fov_tan_of_clip(projection);
        assert!((y - (0.5f32).tan()).abs() < 1e-5);
        assert!((x - y * 1.5).abs() < 1e-5);
        assert_eq!(half_fov_tan_of_clip(Mat4::ZERO), (1.0, 1.0));
    }
}
