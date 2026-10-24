//! View frustum planes, either built from a camera description or read out of
//! a clip matrix. A plane is `[nx, ny, nz, d]`; a point is inside when
//! `n·p + d` is not negative.

use bevy::prelude::*;

/// A perspective camera: where it is, which way it faces, its field of view
/// and its depth range.
#[derive(Clone, Copy, Debug)]
pub struct FrustumSpec {
    pub eye: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fov_y_rad: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

/// The six planes bounding the camera's view, in the order near, far, left,
/// right, bottom, top, all facing inwards.
pub fn perspective_frustum_planes(spec: FrustumSpec) -> [[f32; 4]; 6] {
    let forward = spec.forward.normalize_or_zero();
    let right = spec.right.normalize_or_zero();
    let up = spec.up.normalize_or_zero();
    let tan_y = (spec.fov_y_rad * 0.5).tan();
    let tan_x = tan_y * spec.aspect.max(1e-6);

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

    let mid_depth = (spec.near + spec.far) * 0.5;
    for normal in &mut sides {
        if normal.dot(forward * mid_depth) < 0.0 {
            *normal = -*normal;
        }
    }

    let [left, right_side, bottom, top] = sides;
    [
        plane_through(spec.eye + forward * spec.near, forward),
        plane_through(spec.eye + forward * spec.far, -forward),
        plane_through(spec.eye, left),
        plane_through(spec.eye, right_side),
        plane_through(spec.eye, bottom),
        plane_through(spec.eye, top),
    ]
}

fn plane_through(point: Vec3, normal: Vec3) -> [f32; 4] {
    let n = normal.normalize_or_zero();
    [n.x, n.y, n.z, -n.dot(point)]
}

/// The planes of a clip matrix, normalised, in the order left, right, bottom,
/// top, then the matrix's two depth rows. A degenerate plane comes out all zero:
/// with an infinite projection one depth row is, and accepts everything.
#[must_use]
pub fn clip_from_world_frustum_planes(clip: Mat4) -> [[f32; 4]; 6] {
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

    fn spec() -> FrustumSpec {
        FrustumSpec {
            eye: Vec3::new(10.0, 20.0, 30.0),
            forward: Vec3::X,
            right: Vec3::NEG_Y,
            up: Vec3::Z,
            fov_y_rad: 90f32.to_radians(),
            aspect: 1.0,
            near: 1.0,
            far: 100.0,
        }
    }

    fn inside(planes: &[[f32; 4]], p: Vec3) -> bool {
        planes.iter().all(|plane| distance(plane, p) >= 0.0)
    }

    #[test]
    fn a_point_ahead_is_inside_and_the_rest_are_not() {
        let s = spec();
        let planes = perspective_frustum_planes(s);
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
        let planes = perspective_frustum_planes(s);
        assert!(inside(&planes, s.eye + Vec3::new(10.0, 9.9, 0.0)));
        assert!(inside(&planes, s.eye + Vec3::new(10.0, -9.9, 9.9)));
    }

    #[test]
    fn planes_are_unit_length() {
        for plane in perspective_frustum_planes(spec()) {
            let length = Vec3::new(plane[0], plane[1], plane[2]).length();
            assert!((length - 1.0).abs() < 1e-5, "{plane:?}");
        }
    }

    #[test]
    fn clip_planes_match_the_clip_matrix() {
        let clip = Mat4::perspective_infinite_reverse_rh(90f32.to_radians(), 1.0, 1.0);
        let planes = clip_from_world_frustum_planes(clip);
        assert!(inside(&planes, Vec3::new(0.0, 0.0, -10.0)));
        assert!(inside(&planes, Vec3::new(9.0, -9.0, -10.0)));
        assert!(!inside(&planes, Vec3::new(11.0, 0.0, -10.0)));
        assert!(!inside(&planes, Vec3::new(0.0, 0.0, 10.0)), "behind");
        assert!(!inside(&planes, Vec3::new(0.0, 0.0, -0.5)), "closer than the near distance");
        assert_eq!(planes[4], [0.0; 4], "an infinite projection has no far plane");
    }

    #[test]
    fn a_zero_matrix_gives_zero_planes() {
        assert_eq!(clip_from_world_frustum_planes(Mat4::ZERO), [[0.0; 4]; 6]);
    }
}
