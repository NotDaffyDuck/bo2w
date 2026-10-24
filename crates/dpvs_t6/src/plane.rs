/// Signed distance of `p` from `plane` (`normal . p + w`), the form DpvsPlane coeffs hold.
#[inline]
#[must_use]
pub fn plane_dist(plane: [f32; 4], p: [f32; 3]) -> f32 {
    plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
}

/// The plane through `eye` facing along `forward`, the walk's queue-key plane.
#[inline]
#[must_use]
pub fn view_plane(forward: [f32; 3], eye: [f32; 3]) -> [f32; 4] {
    let d = forward[0] * eye[0] + forward[1] * eye[1] + forward[2] * eye[2];
    [forward[0], forward[1], forward[2], -d]
}

/// The box corner furthest along the plane normal (DpvsPlane.side picks it per axis).
#[inline]
#[must_use]
pub fn positive_vertex(plane: [f32; 4], mins: [f32; 3], maxs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| if plane[i] > 0.0 { maxs[i] } else { mins[i] })
}

/// R_CullBoxDpvs for one plane: culled when even the positive vertex is at or behind it.
#[inline]
#[must_use]
pub fn box_culled_by(plane: [f32; 4], mins: [f32; 3], maxs: [f32; 3]) -> bool {
    plane_dist(plane, positive_vertex(plane, mins, maxs)) <= 0.0
}

/// R_CullBoxDpvs / R_CullBoxLeaf: culled by any plane of the set.
#[must_use]
pub fn box_culled(planes: &[[f32; 4]], mins: [f32; 3], maxs: [f32; 3]) -> bool {
    planes.iter().any(|&p| box_culled_by(p, mins, maxs))
}

/// R_PortalBehindAnyPlane for one plane: every vertex at or behind it.
#[must_use]
pub fn winding_behind_plane(plane: [f32; 4], verts: &[[f32; 3]]) -> bool {
    verts.iter().all(|&v| plane_dist(plane, v) <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_touching_plane_is_culled() {
        let plane = [1.0, 0.0, 0.0, -10.0];
        assert!(box_culled_by(plane, [0.0; 3], [10.0, 1.0, 1.0]));
        assert!(!box_culled_by(plane, [0.0; 3], [10.5, 1.0, 1.0]));
    }

    #[test]
    fn view_plane_passes_through_the_eye() {
        let eye = [3.0, -2.0, 7.0];
        let plane = view_plane([0.0, 1.0, 0.0], eye);
        assert_eq!(plane_dist(plane, eye), 0.0);
        assert_eq!(plane_dist(plane, [3.0, 8.0, 7.0]), 10.0);
    }

    #[test]
    fn positive_vertex_follows_normal_signs() {
        let v = positive_vertex([-1.0, 1.0, 0.0, 0.0], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]);
        assert_eq!(v, [1.0, 5.0, 3.0]);
    }
}
