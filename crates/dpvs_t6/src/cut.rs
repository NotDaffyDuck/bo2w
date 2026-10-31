//! Clip planes a portal hands to the cell behind it: one plane per winding
//! edge (or four planes around its screen rectangle for many-sided
//! windings), then the near and far planes.

use crate::plane::plane_dist;

/// The most planes one portal hands to the cell behind it: 10 sides (or 4 bevels), a near
/// and a far, with room to spare (BO2's child plane count is 16).
pub const CLIP_PLANE_CEILING: usize = 16;

/// A winding with more points than this gets 4 screen-space bevel planes instead of sides.
pub const CLIP_SIDE_LIMIT: usize = 10;

/// Each side plane is pushed this far out past its winding edge.
pub const CLIP_PLANE_EPSILON: f32 = 0.001;

/// A winding point with clip-space w below this covers the whole screen.
pub const PROJECT_W_MIN: f32 = 0.125;

/// The camera's view-projection and its inverse, indexed `[row][column]` with points as row
/// vectors (`clip.x = p.x*m[0][0] + p.y*m[1][0] + p.z*m[2][0] + m[3][0]`).
#[derive(Clone, Copy, Debug)]
pub struct Bevels {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
}

impl Bevels {
    /// From two flat 16-float matrices laid out row after row; nothing is transposed.
    #[must_use]
    pub fn from_row_vector(fwd_rows: &[f32; 16], back_rows: &[f32; 16]) -> Self {
        fn fold_rows(flat: &[f32; 16]) -> [[f32; 4]; 4] {
            let mut rows = [[0.0; 4]; 4];
            for (line_ix, piece_ix) in rows.iter_mut().zip(flat.chunks_exact(4)) {
                line_ix.copy_from_slice(piece_ix);
            }
            rows
        }
        Self { view_proj: fold_rows(fwd_rows), inv_view_proj: fold_rows(back_rows) }
    }
}

/// Where side planes are drawn from: the eye, or a direction (BO2's viewOrgIsDir, a view
/// origin with w 0, as the sun-shadow caster walk sets it: the direction back to the sun).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ViewOrigin {
    Point([f32; 3]),
    Dir([f32; 3]),
}

fn minus(lhs: [f32; 3], rhs: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|axis| lhs[axis] - rhs[axis])
}

fn dot(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

/// Right-handed cross product.
fn wedge(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    let [ux, uy, uz] = u;
    let [vx, vy, vz] = v;
    [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx]
}

/// Unit-length copy of `v`; the zero vector stays zero.
fn unit(v: [f32; 3]) -> [f32; 3] {
    let length_sq = dot(v, v);
    if length_sq == 0.0 {
        return v.map(|_| 0.0);
    }
    let inv_length = 1.0 / libm::sqrtf(length_sq);
    v.map(|c| c * inv_length)
}

/// `[x y z 1] * m`: all four clip-space components.
fn row_times(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
    core::array::from_fn(|col| m[0][col] * p[0] + m[1][col] * p[1] + m[2][col] * p[2] + m[3][col])
}

/// The plane holding the edge `from -> to` and the view origin (or, for a
/// direction, the plane holding the edge and running along it), moved out
/// by the epsilon past `from`. None for a degenerate edge.
fn edge_plane(origin: ViewOrigin, from: [f32; 3], to: [f32; 3]) -> Option<[f32; 4]> {
    let normal = unit(match origin {
        ViewOrigin::Point(eye) => wedge(minus(to, eye), minus(from, eye)),
        ViewOrigin::Dir(along) => wedge(along, minus(to, from)),
    });
    if normal.iter().all(|&c| c == 0.0) {
        return None;
    }
    let [nx, ny, nz] = normal;
    Some([nx, ny, nz, CLIP_PLANE_EPSILON - dot(normal, from)])
}

/// A screen position taken back to world space on clip depth 1, which is
/// the near plane with reversed depth.
fn unproject(m: &[[f32; 4]; 4], px: f32, py: f32) -> [f32; 3] {
    let [x, y, z, w] = row_times(m, [px, py, 1.0]);
    if w == 0.0 { [0.0; 3] } else { [x / w, y / w, z / w] }
}

/// Lower-left and upper-right clip-space corners bounding the winding; the
/// whole screen when a point is too near to or behind the eye.
fn screen_rect(view_proj: &[[f32; 4]; 4], winding: &[[f32; 3]]) -> ([f32; 2], [f32; 2]) {
    let whole_screen = ([-1.0; 2], [1.0; 2]);
    let (mut low, mut high) = ([1.0f32; 2], [-1.0f32; 2]);
    for &corner in winding {
        let [x, y, _, w] = row_times(view_proj, corner);
        if w < PROJECT_W_MIN {
            return whole_screen;
        }
        let screen = [x / w, y / w];
        for ax in 0..2 {
            low[ax] = low[ax].min(screen[ax]);
            high[ax] = high[ax].max(screen[ax]);
        }
    }
    (low, high)
}

/// Fills a caller's plane buffer front to back, ignoring planes past its end.
struct PlaneSink<'a> {
    slots: &'a mut [[f32; 4]],
    filled: usize,
}

impl<'a> PlaneSink<'a> {
    fn new(slots: &'a mut [[f32; 4]]) -> Self {
        Self { slots, filled: 0 }
    }

    fn has_room(&self) -> bool {
        self.filled < self.slots.len()
    }

    fn put(&mut self, plane: [f32; 4]) {
        if let Some(slot) = self.slots.get_mut(self.filled) {
            *slot = plane;
            self.filled += 1;
        }
    }

    /// One plane per closed-loop edge of `ring`, until the buffer fills.
    fn push_ring_edges(&mut self, origin: ViewOrigin, ring: &[[f32; 3]]) {
        let next = ring.iter().cycle().skip(1);
        for (&from, &to) in ring.iter().zip(next) {
            if !self.has_room() {
                break;
            }
            if let Some(plane) = edge_plane(origin, from, to) {
                self.put(plane);
            }
        }
    }
}

/// The 4 planes through the eye and the corners of the winding's screen rectangle.
/// Returns how many were written.
pub fn append_edge_bevel_planes(winding: &[[f32; 3]], eye: [f32; 3], bevels: &Bevels, out: &mut [[f32; 4]]) -> usize {
    let (low, high) = screen_rect(&bevels.view_proj, winding);
    // Round the rectangle: top-left, bottom-left, bottom-right, top-right.
    let rect = [(low[0], high[1]), (low[0], low[1]), (high[0], low[1]), (high[0], high[1])];
    let near_corners = rect.map(|(sx, sy)| unproject(&bevels.inv_view_proj, sx, sy));
    let mut sink = PlaneSink::new(out);
    sink.push_ring_edges(ViewOrigin::Point(eye), &near_corners);
    sink.filled
}

/// The planes the cell behind a portal is seen through. Up to 10 winding
/// points give one side plane per edge; more give 4 bevel planes (none
/// without a camera matrix). The near plane is pulled back to the winding's
/// nearest point, the far plane is copied.
pub fn portal_frustum_planes(
    winding: &[[f32; 3]],
    eye: [f32; 3],
    near: Option<[f32; 4]>,
    far: Option<[f32; 4]>,
    bevels: Option<&Bevels>,
    out: &mut [[f32; 4]],
) -> usize {
    frustum_clip_planes_for(winding, ViewOrigin::Point(eye), near, far, bevels, out)
}

/// `portal_frustum_planes` from either kind of view origin. Bevels need an eye. From a
/// direction the near plane is always moved onto the winding's nearest point, in front or
/// behind (its d of 0 is no place in the map); how BO2 places it there is not read:
/// TODO(fact).
pub(crate) fn frustum_clip_planes_for(
    winding: &[[f32; 3]],
    origin: ViewOrigin,
    near: Option<[f32; 4]>,
    far: Option<[f32; 4]>,
    bevels: Option<&Bevels>,
    out: &mut [[f32; 4]],
) -> usize {
    let mut sink = PlaneSink::new(out);
    if winding.len() <= CLIP_SIDE_LIMIT {
        sink.push_ring_edges(origin, winding);
    } else if let (Some(camera), ViewOrigin::Point(eye)) = (bevels, origin) {
        sink.filled = append_edge_bevel_planes(winding, eye, camera, sink.slots);
    }

    if let Some(mut near_plane) = near.filter(|_| sink.has_room()) {
        let closest = winding.iter().fold(f32::INFINITY, |best, &p| best.min(plane_dist(near_plane, p)));
        let move_onto_winding = match origin {
            ViewOrigin::Point(_) => closest > 0.0,
            ViewOrigin::Dir(_) => true,
        };
        if move_onto_winding && closest.is_finite() {
            near_plane[3] -= closest;
        }
        sink.put(near_plane);
    }
    if let Some(far_plane) = far {
        sink.put(far_plane);
    }
    sink.filled
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2x2 portal at x = 10 seen from the origin, wound so its side planes face inward.
    const WINDING: [[f32; 3]; 4] = [
        [10.0, -1.0, -1.0],
        [10.0, -1.0, 1.0],
        [10.0, 1.0, 1.0],
        [10.0, 1.0, -1.0],
    ];

    #[test]
    fn side_planes_keep_the_view_through_the_portal() {
        let mut out = [[0.0; 4]; CLIP_PLANE_CEILING];
        let n = portal_frustum_planes(&WINDING, [0.0; 3], None, None, None, &mut out);
        assert_eq!(n, 4);
        let through = [20.0, 0.0, 0.0];
        let beside = [20.0, 5.0, 0.0];
        let inside = out[..n].iter().all(|&p| plane_dist(p, through) > 0.0);
        let outside = out[..n].iter().any(|&p| plane_dist(p, beside) < 0.0);
        assert!(inside && outside);
    }

    #[test]
    fn row_vector_is_not_transposed() {
        let m: [f32; 16] = core::array::from_fn(|i| i as f32);
        let b = Bevels::from_row_vector(&m, &m);
        assert_eq!(b.view_proj[0], [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(b.inv_view_proj[3][1], 13.0);
    }

    // Eye at the origin looking down +x (left +y, up +z) with a reversed-depth infinite
    // projection: clip = (-y / tan_x, z / tan_y, znear, x).
    const TAN_X: f32 = 1.5;
    const TAN_Y: f32 = 0.75;
    const ZNEAR: f32 = 4.0;

    fn reversed_depth_bevels() -> Bevels {
        let view_proj = [
            0.0, 0.0, 0.0, 1.0, //
            -1.0 / TAN_X, 0.0, 0.0, 0.0, //
            0.0, 1.0 / TAN_Y, 0.0, 0.0, //
            0.0, 0.0, ZNEAR, 0.0,
        ];
        let inv_view_proj = [
            0.0, -TAN_X, 0.0, 0.0, //
            0.0, 0.0, TAN_Y, 0.0, //
            0.0, 0.0, 0.0, 1.0 / ZNEAR, //
            1.0, 0.0, 0.0, 0.0,
        ];
        Bevels::from_row_vector(&view_proj, &inv_view_proj)
    }

    #[test]
    fn corners_unproject_onto_the_near_plane() {
        let b = reversed_depth_bevels();
        assert_eq!(unproject(&b.inv_view_proj, 0.0, 0.0), [ZNEAR, 0.0, 0.0]);
        assert_eq!(
            unproject(&b.inv_view_proj, 1.0, 1.0),
            [ZNEAR, -TAN_X * ZNEAR, TAN_Y * ZNEAR]
        );
    }

    #[test]
    fn bevel_planes_keep_the_view_through_a_round_portal() {
        let mut ring = [[0.0f32; 3]; 12];
        for (i, v) in ring.iter_mut().enumerate() {
            let a = i as f32 * core::f32::consts::TAU / 12.0;
            *v = [10.0, libm::sinf(a), libm::cosf(a)];
        }
        let b = reversed_depth_bevels();
        let mut out = [[0.0; 4]; CLIP_PLANE_CEILING];
        let n = portal_frustum_planes(&ring, [0.0; 3], None, None, Some(&b), &mut out);
        assert_eq!(n, 4);
        assert!(out[..n].iter().all(|&p| plane_dist(p, [20.0, 0.0, 0.0]) > 0.0));
        for beside in [[20.0, 5.0, 0.0], [20.0, -5.0, 0.0], [20.0, 0.0, 5.0], [20.0, 0.0, -5.0]] {
            assert!(out[..n].iter().any(|&p| plane_dist(p, beside) < 0.0), "{beside:?}");
        }
    }

    #[test]
    fn near_plane_pulls_back_to_the_winding() {
        let mut out = [[0.0; 4]; CLIP_PLANE_CEILING];
        let near = [1.0, 0.0, 0.0, -1.0];
        let n = portal_frustum_planes(&WINDING, [0.0; 3], Some(near), None, None, &mut out);
        assert_eq!(n, 5);
        assert_eq!(out[4], [1.0, 0.0, 0.0, -10.0]);
    }

    #[test]
    fn side_planes_from_a_direction_keep_the_slab_behind_the_portal() {
        // The origin is off at -x (the sun at -x, its light running down +x).
        let mut out = [[0.0; 4]; CLIP_PLANE_CEILING];
        let near = [1.0, 0.0, 0.0, 0.0];
        let origin = ViewOrigin::Dir([-1.0, 0.0, 0.0]);
        let n = frustum_clip_planes_for(&WINDING, origin, Some(near), None, None, &mut out);
        assert_eq!(n, 5);
        assert_eq!(out[4], [1.0, 0.0, 0.0, -10.0]);
        for through in [[20.0, 0.0, 0.0], [500.0, 0.9, -0.9]] {
            assert!(out[..n].iter().all(|&p| plane_dist(p, through) > 0.0), "{through:?}");
        }
        for beside in [[20.0, 5.0, 0.0], [500.0, 0.0, -1.5], [5.0, 0.0, 0.0]] {
            assert!(out[..n].iter().any(|&p| plane_dist(p, beside) < 0.0), "{beside:?}");
        }
    }

    #[test]
    fn many_points_without_a_matrix_give_no_sides() {
        let mut ring = [[0.0f32; 3]; 12];
        for (i, v) in ring.iter_mut().enumerate() {
            let a = i as f32 * core::f32::consts::TAU / 12.0;
            *v = [10.0, libm::sinf(a), libm::cosf(a)];
        }
        let mut out = [[0.0; 4]; CLIP_PLANE_CEILING];
        assert_eq!(portal_frustum_planes(&ring, [0.0; 3], None, None, None, &mut out), 0);
    }
}