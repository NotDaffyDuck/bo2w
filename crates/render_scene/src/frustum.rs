//! View frustum planes, either built from a camera description or read out of
//! a clip matrix. A plane is `[facing_x, facing_y, facing_z, d]`; a point is inside when
//! `n·p + d` is not negative.

use bevy::prelude::*;

/// A perspective camera: where it is, which way it faces, its field of view
/// and its depth range.
#[derive(Clone, Copy, Debug)]
pub struct FrustumShape {
    pub eye: Vec3,
    pub forward: Vec3,
    pub side_dir: Vec3,
    pub up_dir: Vec3,
    pub vertical_fov_radians: f32,
    pub wide_by_tall: f32,
    pub near_cut: f32,
    pub far_cut: f32,
}

/// Older spelling perspective_frustum_planes still calls; the rename sweep moves it over.
pub use self::view_cone_planes as perspective_frustum_planes;

/// The six planes bounding the camera's view, in the order near, far, left,
/// right, bottom, top, all facing inwards.
pub fn view_cone_planes(spec: FrustumShape) -> [[f32; 4]; 6] {
    let FrustumShape { eye, vertical_fov_radians, wide_by_tall: aspect, near_cut: near, far_cut: far, .. } = spec;
    let [forward, right, up] = [spec.forward, spec.side_dir, spec.up_dir].map(Vec3::normalize_or_zero);
    let tan_y = (vertical_fov_radians * 0.5).tan();
    let tan_x = tan_y * aspect.max(1e-6);

    // Each side plane holds the eye and the edge ray of its side, so its normal is
    // that ray crossed with the in-plane axis; the sign is fixed afterwards.
    let edge = |along: Vec3, amount: f32| (forward + along * amount).normalize_or_zero();
    let mut sides = [
        edge(right, -tan_x).cross(up),
        up.cross(edge(right, tan_x)),
        right.cross(edge(up, -tan_y)),
        edge(up, tan_y).cross(right),
    ]
    .map(Vec3::normalize_or_zero);

    let mid_depth = (near + far) * 0.5;
    for normal in &mut sides {
        if normal.dot(forward * mid_depth) < 0.0 {
            *normal = -*normal;
        }
    }

    let [left, right_side, bottom, top] = sides;
    [
        plane_through(eye + forward * near, forward),
        plane_through(eye + forward * far, -forward),
        plane_through(eye, left),
        plane_through(eye, right_side),
        plane_through(eye, bottom),
        plane_through(eye, top),
    ]
}

fn plane_through(spot: Vec3, facing: Vec3) -> [f32; 4] {
    let unit = facing.normalize_or_zero();
    [unit.x, unit.y, unit.z, -unit.dot(spot)]
}

/// The planes of a clip matrix, normalised, in the order left, right, bottom,
/// top, then the matrix's two depth rows. A degenerate plane comes out all zero:
/// with an infinite projection one depth row is, and accepts everything.
#[must_use]
pub fn frustum_planes_from_clip_matrix(clip: Mat4) -> [[f32; 4]; 6] {
    let [x, y, z, w] = [clip.row(0), clip.row(1), clip.row(2), clip.row(3)];
    [w + x, w - x, w + y, w - y, z, w - z].map(|plane| {
        let length = plane.truncate().length();
        if length > 1e-8 {
            (plane / length).to_array()
        } else {
            [0.0; 4]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(plane: &[f32; 4], p: Vec3) -> f32 {
        plane[0] * p.x + plane[1] * p.y + plane[2] * p.z + plane[3]
    }

    fn spec() -> FrustumShape {
        FrustumShape {
            eye: Vec3::new(10.0, 20.0, 30.0),
            forward: Vec3::X,
            side_dir: Vec3::NEG_Y,
            up_dir: Vec3::Z,
            vertical_fov_radians: 90f32.to_radians(),
            wide_by_tall: 1.0,
            near_cut: 1.0,
            far_cut: 100.0,
        }
    }

    fn inside(planes: &[[f32; 4]], p: Vec3) -> bool {
        planes.iter().all(|plane| distance(plane, p) >= 0.0)
    }

    #[test]
    fn a_point_ahead_is_inside_and_the_rest_are_not() {
        let s = spec();
        let planes = view_cone_planes(s);
        assert!(inside(&planes, s.eye + Vec3::X * 50.0));
        assert!(!inside(&planes, s.eye - Vec3::X * 5.0), "behind");
        assert!(!inside(&planes, s.eye + Vec3::X * 150.0), "past far");
        assert!(!inside(&planes, s.eye + Vec3::X * 0.5), "before near");
        assert!(!inside(&planes, s.eye + Vec3::new(10.0, 20.0, 0.0)), "left of a 45 degree half angle");
        assert!(!inside(&planes, s.eye + Vec3::new(10.0, -20.0, 0.0)), "right of it");
        assert!(!inside(&planes, s.eye + Vec3::new(10.0, 0.0, 20.0)), "above");
        assert!(!inside(&planes, s.eye + Vec3::new(10.0, 0.0, -20.0)), "below");
    }

    #[test]
    fn a_ninety_degree_view_accepts_up_to_its_edge() {
        let s = spec();
        let planes = view_cone_planes(s);
        assert!(inside(&planes, s.eye + Vec3::new(10.0, 9.9, 0.0)));
        assert!(inside(&planes, s.eye + Vec3::new(10.0, -9.9, 9.9)));
    }

    #[test]
    fn planes_are_unit_length() {
        for plane in view_cone_planes(spec()) {
            let length = Vec3::new(plane[0], plane[1], plane[2]).length();
            assert!((length - 1.0).abs() < 1e-5, "{plane:?}");
        }
    }

    #[test]
    fn clip_planes_match_the_clip_matrix() {
        let clip = Mat4::perspective_infinite_reverse_rh(90f32.to_radians(), 1.0, 1.0);
        let planes = frustum_planes_from_clip_matrix(clip);
        let toward = Vec3::new(0.0, 0.0, -10.0);
        assert!(inside(&planes, toward));
        assert!(inside(&planes, Vec3::new(9.0, -9.0, -10.0)));
        assert!(!inside(&planes, Vec3::new(11.0, 0.0, -10.0)));
        assert!(!inside(&planes, -toward), "behind");
        assert!(!inside(&planes, toward * 0.05), "closer than the near distance");
        assert_eq!(planes[4], [0.0; 4], "an infinite projection has no far plane");
    }

    #[test]
    fn a_zero_matrix_gives_zero_planes() {
        assert_eq!(frustum_planes_from_clip_matrix(Mat4::ZERO), [[0.0; 4]; 6]);
    }
}
