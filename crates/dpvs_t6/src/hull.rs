/// One hull-point buffer of the 0x20000-byte pool: 0x200 bytes of vec2.
pub const HULL_POINTS_MAX: usize = 64;

/// A queued portal's hull points, in the portal's hullAxis coordinates.
#[derive(Clone, Copy, Debug)]
pub struct HullPoints {
    pub point_count: [[f32; 2]; HULL_POINTS_MAX],
    pub count: usize,
}

impl HullPoints {
    pub const BLANK: Self = Self {
        point_count: [[0.0; 2]; HULL_POINTS_MAX],
        count: 0,
    };

    /// R_AddVertToPortalHullPoints: a full buffer is first shrunk to its convex hull; when
    /// the hull itself fills it the point is dropped.
    pub fn push_point(&mut self, pt2: [f32; 2]) -> bool {
        if self.count == HULL_POINTS_MAX {
            let mut dst = [[0.0f32; 2]; HULL_POINTS_MAX];
            let n = convex_hull(&self.point_count, &mut dst);
            if n == HULL_POINTS_MAX {
                return false;
            }
            self.point_count[..n].copy_from_slice(&dst[..n]);
            self.count = n;
        }
        self.point_count[self.count] = pt2;
        self.count += 1;
        true
    }
}

/// A point's coordinates along the portal's two hull axes.
#[inline]
#[must_use]
pub fn hull_uv(pos: [f32; 3], ax: [[f32; 3]; 2]) -> [f32; 2] {
    [
        pos[0] * ax[0][0] + pos[1] * ax[0][1] + pos[2] * ax[0][2],
        pos[0] * ax[1][0] + pos[1] * ax[1][1] + pos[2] * ax[1][2],
    ]
}

/// Back from hull coordinates to a point on the portal plane: the plane's point nearest the
/// origin plus the two axes.
#[inline]
#[must_use]
pub fn hull_point(plane: [f32; 4], ax_pair: [[f32; 3]; 2], uv_pt: [f32; 2]) -> [f32; 3] {
    core::array::from_fn(|i| -plane[3] * plane[i] + uv_pt[0] * ax_pair[0][i] + uv_pt[1] * ax_pair[1][i])
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Com_ConvexHull: the convex hull of up to 64 points, counter-clockwise, without collinear
/// points. Fewer than 3 points, or all on one line, give 0.
pub fn convex_hull(points: &[[f32; 2]], dst_buf: &mut [[f32; 2]]) -> usize {
    let nrm = points.len();
    if !(3..=HULL_POINTS_MAX).contains(&nrm) || dst_buf.len() < nrm.min(HULL_POINTS_MAX) {
        return 0;
    }
    let mut sorted = [[0.0f32; 2]; HULL_POINTS_MAX];
    sorted[..nrm].copy_from_slice(points);
    sorted[..nrm].sort_unstable_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));

    let mut hull = [[0.0f32; 2]; HULL_POINTS_MAX * 2];
    let mut k = 0usize;
    for &p in &sorted[..nrm] {
        while k >= 2 && cross(hull[k - 2], hull[k - 1], p) <= 0.0 {
            k -= 1;
        }
        hull[k] = p;
        k += 1;
    }
    let lower = k + 1;
    for &p in sorted[..nrm - 1].iter().rev() {
        while k >= lower && cross(hull[k - 2], hull[k - 1], p) <= 0.0 {
            k -= 1;
        }
        hull[k] = p;
        k += 1;
    }
    let count = k - 1;
    if count < 3 {
        return 0;
    }
    dst_buf[..count].copy_from_slice(&hull[..count]);
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_drops_inner_and_collinear_points() {
        let pts = [
            [0.0, 0.0],
            [2.0, 0.0],
            [1.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
            [1.0, 1.0],
        ];
        let mut out = [[0.0; 2]; 6];
        assert_eq!(convex_hull(&pts, &mut out), 4);
        assert_eq!(&out[..4], &[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]);
    }

    #[test]
    fn line_has_no_hull() {
        let mut out = [[0.0; 2]; 3];
        assert_eq!(convex_hull(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], &mut out), 0);
    }

    #[test]
    fn full_buffer_shrinks_to_its_hull() {
        let mut h = HullPoints::BLANK;
        for i in 0..HULL_POINTS_MAX {
            let corner = i % 4;
            let x = if corner == 1 || corner == 2 { 1.0 } else { 0.0 };
            let y = if corner >= 2 { 1.0 } else { 0.0 };
            assert!(h.push_point([x, y]));
        }
        assert!(h.push_point([0.5, 0.5]));
        assert_eq!(h.count, 5);
    }

    #[test]
    fn hull_points_round_trip_on_the_plane() {
        let plane = [0.0, 0.0, 1.0, -8.0];
        let axis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let v = [3.0, -2.0, 8.0];
        assert_eq!(hull_point(plane, axis, hull_uv(v, axis)), v);
    }
}
