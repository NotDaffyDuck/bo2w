use crate::plane::plane_dist;

/// The most planes one portal hands to the cell behind it: 10 sides (or 4 bevels), a near
/// and a far, with room to spare (BO2's child plane count is 16).
pub const MAX_CLIP_PLANES: usize = 16;

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
    /// From BO2's flat row-vector matrices (`m[row * 4 + col]`, as R_SetViewParmsForScene
    /// stores them); no transpose.
    #[must_use]
    pub fn from_row_vector(view_proj: &[f32; 16], inv_view_proj: &[f32; 16]) -> Self {
        let rows = |m: &[f32; 16]| -> [[f32; 4]; 4] {
            core::array::from_fn(|row| core::array::from_fn(|col| m[row * 4 + col]))
        };
        Self {
            view_proj: rows(view_proj),
            inv_view_proj: rows(inv_view_proj),
        }
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len_sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if len_sq == 0.0 {
        return [0.0; 3];
    }
    let inv = 1.0 / libm::sqrtf(len_sq);
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Where side planes are drawn from: the eye, or a direction (BO2's viewOrgIsDir, a view
/// origin with w 0, as the sun-shadow caster walk sets it: the direction back to the sun).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ViewOrigin {
    Point([f32; 3]),
    Dir([f32; 3]),
}

/// The plane through the view origin and the edge from `a` to `b`, pushed out past `a`. For
/// a direction it is the plane through the edge that holds the direction: the limit of the
/// eye plane as the eye moves off along it.
fn side_plane(origin: ViewOrigin, a: [f32; 3], b: [f32; 3]) -> Option<[f32; 4]> {
    let n = match origin {
        ViewOrigin::Point(eye) => normalize(cross(sub(b, eye), sub(a, eye))),
        ViewOrigin::Dir(dir) => normalize(cross(dir, sub(b, a))),
    };
    if n == [0.0; 3] {
        return None;
    }
    Some([n[0], n[1], n[2], CLIP_PLANE_EPSILON - (n[0] * a[0] + n[1] * a[1] + n[2] * a[2])])
}

/// R_AddBevelPlanes unprojects each corner at clip z 1: the near plane under reversed depth.
fn unproject(m: &[[f32; 4]; 4], px: f32, py: f32) -> [f32; 3] {
    let x = m[0][0] * px + m[1][0] * py + m[2][0] + m[3][0];
    let y = m[0][1] * px + m[1][1] * py + m[2][1] + m[3][1];
    let z = m[0][2] * px + m[1][2] * py + m[2][2] + m[3][2];
    let w = m[0][3] * px + m[1][3] * py + m[2][3] + m[3][3];
    if w == 0.0 {
        return [0.0; 3];
    }
    [x / w, y / w, z / w]
}

/// The winding's screen rectangle in clip space; the whole screen when any point is at or
/// behind the eye.
fn screen_rect(view_proj: &[[f32; 4]; 4], winding: &[[f32; 3]]) -> ([f32; 2], [f32; 2]) {
    let m = view_proj;
    let mut mins = [1.0f32, 1.0];
    let mut maxs = [-1.0f32, -1.0];
    for p in winding {
        let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
        if w < PROJECT_W_MIN {
            return ([-1.0, -1.0], [1.0, 1.0]);
        }
        let x = (m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0]) / w;
        let y = (m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1]) / w;
        mins = [mins[0].min(x), mins[1].min(y)];
        maxs = [maxs[0].max(x), maxs[1].max(y)];
    }
    (mins, maxs)
}

/// R_AddBevelPlanes: the 4 planes through the eye and the corners of the winding's screen
/// rectangle. Returns how many were written.
pub fn add_bevel_planes(
    winding: &[[f32; 3]],
    eye: [f32; 3],
    bevels: &Bevels,
    out: &mut [[f32; 4]],
) -> usize {
    let (mins, maxs) = screen_rect(&bevels.view_proj, winding);
    let corners = [
        [mins[0], maxs[1]],
        [mins[0], mins[1]],
        [maxs[0], mins[1]],
        [maxs[0], maxs[1]],
    ]
    .map(|c| unproject(&bevels.inv_view_proj, c[0], c[1]));
    let mut written = 0;
    for i in 0..4 {
        if written == out.len() {
            break;
        }
        if let Some(plane) = side_plane(ViewOrigin::Point(eye), corners[i], corners[(i + 1) % 4])
        {
            out[written] = plane;
            written += 1;
        }
    }
    written
}

/// R_SetupPortalPlanes: the planes the cell behind a portal is seen through. Up to 10
/// winding points give one side plane per edge; more give 4 bevel planes (none without a
/// camera matrix). The near plane is pulled back to the winding's nearest point, the far
/// plane is copied.
pub fn portal_clip_planes(
    winding: &[[f32; 3]],
    eye: [f32; 3],
    near: Option<[f32; 4]>,
    far: Option<[f32; 4]>,
    bevels: Option<&Bevels>,
    out: &mut [[f32; 4]],
) -> usize {
    clip_planes(winding, ViewOrigin::Point(eye), near, far, bevels, out)
}

/// `portal_clip_planes` from either kind of view origin. Bevels need an eye. From a
/// direction the near plane is always moved onto the winding's nearest point, in front or
/// behind (its d of 0 is no place in the map); how BO2 places it there is not read:
/// TODO(fact).
pub(crate) fn clip_planes(
    winding: &[[f32; 3]],
    origin: ViewOrigin,
    near: Option<[f32; 4]>,
    far: Option<[f32; 4]>,
    bevels: Option<&Bevels>,
    out: &mut [[f32; 4]],
) -> usize {
    let n = winding.len();
    let mut written = 0usize;
    if n > CLIP_SIDE_LIMIT {
        if let (Some(b), ViewOrigin::Point(eye)) = (bevels, origin) {
            written = add_bevel_planes(winding, eye, b, out);
        }
    } else {
        for i in 0..n {
            if written == out.len() {
                break;
            }
            if let Some(plane) = side_plane(origin, winding[i], winding[(i + 1) % n]) {
                out[written] = plane;
                written += 1;
            }
        }
    }
    if let Some(mut plane) = near
        && written < out.len()
    {
        let nearest = winding
            .iter()
            .map(|&v| plane_dist(plane, v))
            .fold(f32::INFINITY, f32::min);
        let pull = match origin {
            ViewOrigin::Point(_) => nearest > 0.0,
            ViewOrigin::Dir(_) => true,
        };
        if nearest.is_finite() && pull {
            plane[3] -= nearest;
        }
        out[written] = plane;
        written += 1;
    }
    if let Some(plane) = far
        && written < out.len()
    {
        out[written] = plane;
        written += 1;
    }
    written
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
        let mut out = [[0.0; 4]; MAX_CLIP_PLANES];
        let n = portal_clip_planes(&WINDING, [0.0; 3], None, None, None, &mut out);
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
        let mut out = [[0.0; 4]; MAX_CLIP_PLANES];
        let n = portal_clip_planes(&ring, [0.0; 3], None, None, Some(&b), &mut out);
        assert_eq!(n, 4);
        assert!(out[..n].iter().all(|&p| plane_dist(p, [20.0, 0.0, 0.0]) > 0.0));
        for beside in [[20.0, 5.0, 0.0], [20.0, -5.0, 0.0], [20.0, 0.0, 5.0], [20.0, 0.0, -5.0]] {
            assert!(out[..n].iter().any(|&p| plane_dist(p, beside) < 0.0), "{beside:?}");
        }
    }

    #[test]
    fn near_plane_pulls_back_to_the_winding() {
        let mut out = [[0.0; 4]; MAX_CLIP_PLANES];
        let near = [1.0, 0.0, 0.0, -1.0];
        let n = portal_clip_planes(&WINDING, [0.0; 3], Some(near), None, None, &mut out);
        assert_eq!(n, 5);
        assert_eq!(out[4], [1.0, 0.0, 0.0, -10.0]);
    }

    #[test]
    fn side_planes_from_a_direction_keep_the_slab_behind_the_portal() {
        // The origin is off at -x (the sun at -x, its light running down +x).
        let mut out = [[0.0; 4]; MAX_CLIP_PLANES];
        let near = [1.0, 0.0, 0.0, 0.0];
        let origin = ViewOrigin::Dir([-1.0, 0.0, 0.0]);
        let n = clip_planes(&WINDING, origin, Some(near), None, None, &mut out);
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
        let mut out = [[0.0; 4]; MAX_CLIP_PLANES];
        assert_eq!(portal_clip_planes(&ring, [0.0; 3], None, None, None, &mut out), 0);
    }
}
