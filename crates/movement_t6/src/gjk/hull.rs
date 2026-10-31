//! The plain-math parts of Black Ops II's player collision: the player's
//! shape, the plane solver that bends a move along what it touches, and the
//! "can I stand on this" rule. Facts and numbers from Black Ops II's server.

/// How far the shape sits inside the box Black Ops II gives it, before the
/// 0.125 trace margin is taken back off (0.5 - 0.125 = 0.375 per side).
pub const RADIUS_ADJUST: f32 = 0.5;
const TRACE_MARGIN: f32 = 0.125;

/// The 12-sided body's top is a cap this tall; its feet a cone this tall.
const HEAD_OFFSET: f32 = 10.0;
const FOOT_OFFSET: f32 = 2.0;

/// Ground with an up-slope of at least this much is ground you can stand on.
pub const WALKABLE_NORMAL_Z: f32 = 0.7;

/// A contact counts as "on" a face within this share of the face's distance.
const ON_FACE: f32 = 0.999;

/// Black Ops II's plane solver gives up below this (two or three planes that
/// are nearly the same plane).
const SOLVER_DEGENERATE: f32 = 1.0e-5;

/// Most planes one solve takes.
pub const MAX_PLANES: usize = 128;

/// cos and sin of the 12 corners of the body's ring, every 30 degrees.
const RING: [[f32; 2]; 12] = [
    [1.0, 0.0],
    [0.866_025_4, 0.5],
    [0.5, 0.866_025_4],
    [0.0, 1.0],
    [-0.5, 0.866_025_4],
    [-0.866_025_4, 0.5],
    [-1.0, 0.0],
    [-0.866_025_4, -0.5],
    [-0.5, -0.866_025_4],
    [0.0, -1.0],
    [0.5, -0.866_025_4],
    [0.866_025_4, -0.5],
];

/// The player's collision shape. Against brushes it is a 12-sided upright
/// prism, flat sides at `capsule_radius`, with a 10-unit pointed cap on top
/// and a 2-unit cone under the feet; against terrain and entity boxes it is a
/// capsule. Both get a `geom_radius` skin all round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    /// Middle of the shape, from the player's origin.
    pub core_pt: [f32; 3],
    pub half_height: f32,
    pub capsule_radius: f32,
    /// Corner radius of the 12-sided ring: `capsule_radius / cos(15)`.
    pub polygon_radius: f32,
    pub geom_radius: f32,
    pub head_offset: f32,
    pub foot_offset: f32,
    /// true: the capsule shape (terrain, boxes); false: the 12-sided body.
    pub capsule: bool,
}

/// Black Ops II's player shape from the player's box. `adjust_height` is the
/// dvar phys_player_collision_adjust_height (0 by default).
pub fn player_hull(box_lo: [f32; 3], box_hi: [f32; 3], adjust_height: f32) -> Hull {
    let adj = RADIUS_ADJUST - TRACE_MARGIN;
    let low = |a: usize| {
        if box_lo[a] + adj <= box_hi[a] {
            box_lo[a] + adj
        } else {
            box_hi[a]
        }
    };
    let high = |a: usize| {
        if box_lo[a] <= box_hi[a] - adj {
            box_hi[a] - adj
        } else {
            box_lo[a]
        }
    };
    let dim_x = high(0) - low(0);
    let dim_z = high(2) - low(2);
    let mut radius = (if dim_z < dim_x { dim_z } else { dim_x }) * 0.5;
    if radius < 1.0 {
        radius = 1.0;
    }
    let mut half_height = dim_z * 0.5;
    if half_height < 1.0 {
        half_height = 1.0;
    }
    let cos15 = libm::cosf(core::f32::consts::PI / 12.0);
    Hull {
        core_pt: [0.0, 0.0, (high(2) + low(2)) * 0.5 + adjust_height],
        half_height,
        capsule_radius: radius,
        polygon_radius: radius / cos15,
        geom_radius: RADIUS_ADJUST,
        head_offset: if HEAD_OFFSET < half_height {
            HEAD_OFFSET
        } else {
            half_height
        },
        foot_offset: if FOOT_OFFSET < half_height {
            FOOT_OFFSET
        } else {
            half_height
        },
        capsule: false,
    }
}

impl Hull {
    /// The shape's farthest point toward `d` (before the skin), as Black Ops
    /// II picks it: the ring corner nearest d's heading, upper ring when d
    /// points up, lower ring when down, or the cap's tip when that is farther.
    pub fn support(&self, d: [f32; 3]) -> [f32; 3] {
        let c = self.core_pt;
        if self.capsule {
            let reach = self.half_height - self.capsule_radius;
            let z = if d[2] * (2.0 * reach) >= 0.0 {
                c[2] + reach
            } else {
                c[2] - reach
            };
            return [c[0], c[1], z];
        }
        let tau = core::f32::consts::TAU;
        let mut heading = libm::atan2f(d[1], d[0]);
        if heading < 0.0 {
            heading += tau;
        }
        let corner = ((heading * 12.0 * 0.159_154_94 + 0.5) as i32 % 12) as usize;
        let (ring_z, tip_z) = if d[2] < 0.0 {
            (
                c[2] + self.foot_offset - self.half_height,
                c[2] - self.half_height,
            )
        } else {
            (
                c[2] + self.half_height - self.head_offset,
                c[2] + self.half_height,
            )
        };
        let [cos, sin] = RING[corner];
        let ring = [
            c[0] + self.polygon_radius * cos,
            c[1] + self.polygon_radius * sin,
            ring_z,
        ];
        let tip = [c[0], c[1], tip_z];
        if dot(d, minus(ring, tip)) >= 0.0 {
            ring
        } else {
            tip
        }
    }

    /// The skin GJK adds around the shape: just the skin for the 12-sided
    /// body, skin plus the capsule's radius for the capsule.
    pub fn skin(&self) -> f32 {
        if self.capsule {
            self.geom_radius + self.capsule_radius
        } else {
            self.geom_radius
        }
    }

    /// BO2's calc_aabb for the body: the ring's corner radius and the half
    /// height, each with the skin, around the middle.
    pub fn calc_aabb(&self) -> ([f32; 3], [f32; 3]) {
        let r = self.polygon_radius + self.geom_radius;
        let h = self.half_height + self.geom_radius;
        let c = self.core_pt;
        (
            [c[0] - r, c[1] - r, c[2] - h],
            [c[0] + r, c[1] + r, c[2] + h],
        )
    }

    /// Whether a contact `z` (from the player's origin) is low enough to be
    /// the feet: the bottom 2.125 units standing.
    pub fn is_foot(&self, height_z: f32) -> bool {
        let bottom = self.core_pt[2] - self.half_height;
        if self.capsule {
            height_z <= bottom + self.capsule_radius - 1.0
        } else {
            let sin30 = libm::sinf(core::f32::consts::FRAC_PI_6);
            height_z <= bottom + self.foot_offset - sin30 * self.geom_radius
        }
    }
}

/// Most corners a vehicle's moving brush keeps (the Dragonfire's has 16).
pub const MAX_MOVER_VERTS: usize = 32;

/// A vehicle's collision brush as its slide sweeps it: Black Ops II's brush
/// shape for the first piece of the vehicle model's collision map. Its
/// corners are from the vehicle's origin and never turn with the vehicle;
/// it has no skin, and GJK starts from its local origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverBrush {
    vertex_points: [[f32; 3]; MAX_MOVER_VERTS],
    count: usize,
    box_lo: [f32; 3],
    box_hi: [f32; 3],
}

impl MoverBrush {
    /// The brush with these corners (repeats dropped). None when there are
    /// none, or more than [`MAX_MOVER_VERTS`] different ones.
    pub fn new(points: &[[f32; 3]]) -> Option<Self> {
        let mut vertex_points = [[0.0; 3]; MAX_MOVER_VERTS];
        let mut count = 0;
        let mut lowest = [f32::MAX; 3];
        let mut highest = [f32::MIN; 3];
        for &p in points {
            if vertex_points[..count].contains(&p) {
                continue;
            }
            if count == MAX_MOVER_VERTS {
                return None;
            }
            vertex_points[count] = p;
            count += 1;
            lowest = core::array::from_fn(|a| lowest[a].min(p[a]));
            highest = core::array::from_fn(|a| highest[a].max(p[a]));
        }
        (count > 0).then_some(Self {
            vertex_points,
            count,
            box_lo: lowest,
            box_hi: highest,
        })
    }

    /// The corners.
    pub fn vertex_points(&self) -> &[[f32; 3]] {
        &self.vertex_points[..self.count]
    }

    /// The box round the corners, from the vehicle's origin.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        (self.box_lo, self.box_hi)
    }
}

/// What a trace sweeps: the player's body, or a vehicle's brush.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mover {
    Body(Hull),
    Brush(MoverBrush),
}

impl Mover {
    /// The farthest point toward `d` (before the skin).
    pub fn support(&self, d: [f32; 3]) -> [f32; 3] {
        match self {
            Mover::Body(hull) => hull.support(d),
            Mover::Brush(brush) => super::farthest(brush.vertex_points().iter().copied(), d),
        }
    }

    /// Where GJK starts from: the body's middle, the brush's local origin.
    pub fn center(&self) -> [f32; 3] {
        match self {
            Mover::Body(hull) => hull.core_pt,
            Mover::Brush(_) => [0.0; 3],
        }
    }

    /// The skin GJK adds around the shape: a brush has none.
    pub fn skin(&self) -> f32 {
        match self {
            Mover::Body(hull) => hull.skin(),
            Mover::Brush(_) => 0.0,
        }
    }

    /// The box round the shape with its skin (BO2's calc_aabb).
    pub fn calc_aabb(&self) -> ([f32; 3], [f32; 3]) {
        match self {
            Mover::Body(hull) => hull.calc_aabb(),
            Mover::Brush(brush) => brush.bounds(),
        }
    }

    /// Whether a contact `z` is at the feet: only a body has feet.
    pub fn is_foot(&self, height_z: f32) -> bool {
        match self {
            Mover::Body(hull) => hull.is_foot(height_z),
            Mover::Brush(_) => false,
        }
    }

    /// The shape that meets a brush (`brush`) or anything else: the body
    /// meets brushes as its 12-sided self and the rest as the capsule; a
    /// vehicle's brush is always itself.
    pub fn against(&self, brush: bool) -> Mover {
        match *self {
            Mover::Body(hull) => Mover::Body(Hull {
                capsule: !brush,
                ..hull
            }),
            Mover::Brush(b) => Mover::Brush(b),
        }
    }
}

/// One plane for [`project`]: the result must end up with
/// `normal . x >= rs`. Inactive planes are skipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolvePlane {
    pub facing: [f32; 3],
    pub rs: f32,
    pub engaged: bool,
}

/// Black Ops II's plane solver: the smallest change to `start` that clears
/// every active plane (`normal . x >= rs`), working with at most three planes
/// at once. Each round takes the plane the current answer cuts deepest
/// (each plane once), then keeps whichever of "this plane", "this plane and
/// one kept plane" or "this plane and two kept planes" pushes farthest, so a
/// floor, a wall and the crease between them all come out right. It stops
/// when a round would push less than the last one.
pub fn project(start: [f32; 3], planes: &[SolvePlane]) -> [f32; 3] {
    assert!(planes.len() <= MAX_PLANES, "{} planes", planes.len());
    let mut picked = [false; MAX_PLANES];
    let mut p = start;
    let mut prev_score = 0.0f32;
    // The kept planes: four slots, a bit per slot in `set`.
    let mut set = 0u32;
    let mut slot_plane = [0usize; 4];
    let mut slot_e = [0.0f32; 4];
    let mut gram = [[0.0f32; 4]; 4];
    let mut rounds = 0usize;
    while !planes.is_empty() {
        let mut deepest = 1.0e13f32;
        let mut chosen = None;
        for (i, cand) in planes.iter().enumerate() {
            if !cand.engaged || picked[i] {
                continue;
            }
            let cut = dot(cand.facing, p) - cand.rs;
            if cut < deepest {
                deepest = cut;
                chosen = Some(i);
            }
        }
        let Some(new_plane) = chosen else { break };
        if deepest >= 0.0 {
            break;
        }
        picked[new_plane] = true;
        let slot = (0..4).find(|s| set & (1 << s) == 0).expect("a free slot");
        let n_new = planes[new_plane].facing;
        slot_plane[slot] = new_plane;
        slot_e[slot] = planes[new_plane].rs - dot(n_new, start);
        gram[slot][slot] = 1.0;
        for j in (0..4).filter(|j| set & (1 << j) != 0) {
            let g = dot(n_new, planes[slot_plane[j]].facing);
            gram[slot][j] = g;
            gram[j][slot] = g;
        }

        let e = slot_e;
        let mut best = e[slot] * e[slot];
        let mut best_set = 1u32 << slot;
        let mut best_lambda = [0.0f32; 4];
        best_lambda[slot] = e[slot];
        for j in (0..4).filter(|j| set & (1 << j) != 0) {
            let g = gram[slot][j];
            let a = e[slot] - g * e[j];
            let b = e[j] - g * e[slot];
            let det = 1.0 - g * g;
            if a >= 0.0 && b >= 0.0 && det > SOLVER_DEGENERATE {
                let score = (a * a + 2.0 * a * b * g + b * b) / (det * det);
                if score > best {
                    best = score;
                    best_set = (1 << slot) | (1 << j);
                    best_lambda = [0.0; 4];
                    best_lambda[slot] = a / det;
                    best_lambda[j] = b / det;
                }
            }
            for k in (j + 1..4).filter(|k| set & (1 << k) != 0) {
                let idx = [slot, j, k];
                let m = |r: usize, c: usize| gram[idx[r]][idx[c]];
                let rhs = [e[slot], e[j], e[k]];
                // Adjugate of the 3x3 Gram matrix (it is symmetric).
                let adj = [
                    [
                        m(1, 1) * m(2, 2) - m(1, 2) * m(2, 1),
                        m(0, 2) * m(2, 1) - m(0, 1) * m(2, 2),
                        m(0, 1) * m(1, 2) - m(0, 2) * m(1, 1),
                    ],
                    [
                        m(1, 2) * m(2, 0) - m(1, 0) * m(2, 2),
                        m(0, 0) * m(2, 2) - m(0, 2) * m(2, 0),
                        m(0, 2) * m(1, 0) - m(0, 0) * m(1, 2),
                    ],
                    [
                        m(1, 0) * m(2, 1) - m(1, 1) * m(2, 0),
                        m(0, 1) * m(2, 0) - m(0, 0) * m(2, 1),
                        m(0, 0) * m(1, 1) - m(0, 1) * m(1, 0),
                    ],
                ];
                let det = m(0, 0) * adj[0][0] + m(0, 1) * adj[1][0] + m(0, 2) * adj[2][0];
                let num =
                    [0, 1, 2].map(|r| adj[r][0] * rhs[0] + adj[r][1] * rhs[1] + adj[r][2] * rhs[2]);
                if num.iter().all(|&x| x >= 0.0) && det > SOLVER_DEGENERATE {
                    let l = num.map(|x| x / det);
                    // Row by row, left to right: the sum order is kept fixed.
                    let score = (0..9).fold(0.0, |sum, cell| {
                        let (row, col) = (cell / 3, cell % 3);
                        sum + l[row] * m(row, col) * l[col]
                    });
                    if score > best {
                        best = score;
                        best_set = (1 << slot) | (1 << j) | (1 << k);
                        best_lambda = [0.0; 4];
                        idx.iter()
                            .zip(l)
                            .for_each(|(&cand, weight)| best_lambda[cand] = weight);
                    }
                }
            }
        }
        if best < prev_score {
            break;
        }
        prev_score = best;
        set = best_set;
        let lambda = best_lambda;
        p = start;
        for s in (0..4).filter(|s| set & (1 << s) != 0) {
            let n = planes[slot_plane[s]].facing;
            p = [
                p[0] + lambda[s] * n[0],
                p[1] + lambda[s] * n[1],
                p[2] + lambda[s] * n[2],
            ];
        }
        rounds += 1;
        if rounds >= planes.len() {
            break;
        }
    }
    p
}

/// Black Ops II's walkable rule for a contact: never unless it is at the
/// feet; yes if the contact's normal points up at least 0.7; otherwise the
/// thing touched decides (`face_rule`, e.g. [`brush_face_walkable`]).
pub fn is_walkable(at_feet: bool, hit_normal_z: f32, face_rule: impl FnOnce() -> bool) -> bool {
    if !at_feet {
        return false;
    }
    if hit_normal_z >= WALKABLE_NORMAL_Z {
        return true;
    }
    face_rule()
}

/// Whether a point at `dist` along a face's normal sits on a face at `face`.
fn on_face(dist: f32, face: f32) -> bool {
    face * ON_FACE <= dist || face <= dist * ON_FACE
}

/// Picks the walkable answer over a list of faces (normal . point, face
/// distance, normal . up): walkable if the point sits on any face that points
/// up 0.7 or more, else whatever the face the point is farthest out of says.
struct FaceVote {
    nearest: f32,
    nearest_up: f32,
}

impl FaceVote {
    /// Before any face: far outside everything, and not walkable.
    const UNCAST: Self = Self {
        nearest: -100_000.0,
        nearest_up: 0.0,
    };

    /// true: the point is on a walkable face, stop.
    fn face(&mut self, dist: f32, face: f32, up: f32) -> bool {
        if on_face(dist, face) && up >= WALKABLE_NORMAL_Z {
            return true;
        }
        if self.nearest < dist - face {
            self.nearest = dist - face;
            self.nearest_up = up;
        }
        false
    }
}

/// The brush face rule: the brush's six box faces (low then high side, x
/// then y then z), then its other faces (`[facing_x, facing_y, facing_z, dist]`). `point` and
/// `upward` are in the brush's own space.
pub fn brush_face_walkable(
    box_lo: [f32; 3],
    box_hi: [f32; 3],
    sides: &[[f32; 4]],
    probe: [f32; 3],
    upward: [f32; 3],
) -> bool {
    let mut vote = FaceVote::UNCAST;
    let on_box_face = (0..3).any(|a| {
        vote.face(-probe[a], -box_lo[a], -upward[a]) || vote.face(probe[a], box_hi[a], upward[a])
    });
    let on_side_face = on_box_face
        || sides.iter().any(|&[nx, ny, nz, dist]| {
            let normal = [nx, ny, nz];
            vote.face(dot(probe, normal), dist, dot(upward, normal))
        });
    on_side_face || vote.nearest_up >= WALKABLE_NORMAL_Z
}

/// The terrain face rule, over triangles (skipping ones too thin to have a
/// direction). No triangles: not walkable.
pub fn triangles_face_walkable(
    triangle_list: impl IntoIterator<Item = [[f32; 3]; 3]>,
    probe: [f32; 3],
    upward: [f32; 3],
) -> bool {
    let mut vote = FaceVote::UNCAST;
    for [v0, v1, v2] in triangle_list {
        let normal = cross3(minus(v0, v2), minus(v0, v1));
        let length = libm::sqrtf(dot(normal, normal));
        if length <= 0.0001 {
            continue;
        }
        let inv = 1.0 / length;
        let along = |v: [f32; 3]| dot(v, normal) * inv;
        if vote.face(along(probe), along(v0), along(upward)) {
            return true;
        }
    }
    // An empty list leaves the starting vote, which is not walkable.
    vote.nearest_up >= WALKABLE_NORMAL_Z
}

fn dot(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| a[i] - b[i])
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::testkit::{MAXS, MINS};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| close(a[i], b[i]))
    }

    /// Standing: 14.625 flat sides, 34.625 half height, middle 35 up;
    /// lying down: same width, middle 15 up.
    #[test]
    fn the_player_shape_matches_bo2s_numbers() {
        let stand = player_hull(MINS, MAXS, 0.0);
        assert_eq!(stand.core_pt, [0.0, 0.0, 35.0]);
        assert_eq!(stand.half_height, 34.625);
        assert_eq!(stand.capsule_radius, 14.625);
        assert!(close(stand.polygon_radius, 14.625 / 0.965_925_8));
        assert_eq!((stand.head_offset, stand.foot_offset), (10.0, 2.0));
        assert_eq!(stand.skin(), 0.5);
        // Its box: 70 tall plus the skin, the ring's corners plus the skin.
        let (lo, hi) = stand.calc_aabb();
        assert_eq!((lo[2], hi[2]), (-0.125, 70.125));
        assert!(close(hi[0], 14.625 / 0.965_925_8 + 0.5));
        let prone = player_hull(MINS, [15.0, 15.0, 30.0], 0.0);
        assert_eq!(prone.core_pt[2], 15.0);
        assert_eq!((prone.half_height, prone.capsule_radius), (14.625, 14.625));
        // A box only 20 tall: the width follows the height.
        let short = player_hull(MINS, [15.0, 15.0, 20.0], 0.0);
        assert_eq!(short.capsule_radius, (20.0 - 0.75) * 0.5);
        // The height dvar lifts the middle.
        assert_eq!(player_hull(MINS, MAXS, 3.0).core_pt[2], 38.0);
    }

    /// Feet are the bottom 2.125 units standing (2.0 cone minus a 0.25 skin
    /// share); as a capsule, a unit less than its radius.
    #[test]
    fn feet_are_the_bottom_two_and_an_eighth() {
        let mut stand = player_hull(MINS, MAXS, 0.0);
        assert!(stand.is_foot(2.125));
        assert!(!stand.is_foot(2.13));
        stand.capsule = true;
        assert!(stand.is_foot(0.375 + 14.625 - 1.0));
        assert!(!stand.is_foot(0.375 + 14.625 - 0.99));
        assert_eq!(stand.skin(), 15.125);
    }

    /// Sideways the ring corner wins; straight up and down the cap's tip;
    /// a heading between corners rounds to the nearer one.
    #[test]
    fn the_shape_reaches_out_like_bo2s() {
        let h = player_hull(MINS, MAXS, 0.0);
        let r = h.polygon_radius;
        assert!(close3(h.support([1.0, 0.0, 0.1]), [r, 0.0, 69.625 - 10.0]));
        assert!(close3(h.support([1.0, 0.0, -0.1]), [r, 0.0, 0.375 + 2.0]));
        assert!(close3(h.support([0.0, 0.0, 1.0]), [0.0, 0.0, 69.625]));
        assert!(close3(h.support([0.0, 0.0, -1.0]), [0.0, 0.0, 0.375]));
        // 40 degrees rounds to the 30-degree corner, -100 to 270.
        let d40 = [0.766_044_4, 0.642_787_6, 0.0];
        assert!(close3(h.support(d40), [r * 0.866_025_4, r * 0.5, 59.625]));
        let d = [-0.173_648_2, -0.984_807_8, 0.0];
        assert!(close3(h.support(d), [0.0, -r, 59.625]));
        // Steeply down: the cone under the feet is only 2 tall, so the ring
        // wins unless the direction is almost straight down.
        let steep = [0.2, 0.0, -1.0];
        assert!(close3(h.support(steep), [r, 0.0, 2.375]));
        let capsule = Hull { capsule: true, ..h };
        assert!(close3(
            capsule.support([1.0, 0.0, 0.5]),
            [0.0, 0.0, 35.0 + 20.0]
        ));
        assert!(close3(
            capsule.support([1.0, 0.0, -0.5]),
            [0.0, 0.0, 35.0 - 20.0]
        ));
    }

    fn plane(normal: [f32; 3], rs: f32) -> SolvePlane {
        SolvePlane {
            facing: normal,
            rs,
            engaged: true,
        }
    }

    /// Nothing in the way: the move is kept.
    #[test]
    fn a_clear_move_is_kept() {
        let v = [100.0, 50.0, -10.0];
        assert_eq!(project(v, &[plane([0.0, 0.0, 1.0], -20.0)]), v);
        assert_eq!(project(v, &[]), v);
        let mut off = plane([0.0, 0.0, 1.0], 0.0);
        off.engaged = false;
        assert_eq!(project(v, &[off]), v);
    }

    /// Into the floor: the down part goes, the rest is kept.
    #[test]
    fn the_floor_takes_the_down_part() {
        let out = project([100.0, 50.0, -30.0], &[plane([0.0, 0.0, 1.0], 0.0)]);
        assert!(close3(out, [100.0, 50.0, 0.0]), "{out:?}");
    }

    /// Into a wall standing on the floor: slides along both.
    #[test]
    fn the_floor_and_a_wall_slide_along_the_corner() {
        let planes = [plane([0.0, 0.0, 1.0], 0.0), plane([-1.0, 0.0, 0.0], 0.0)];
        let out = project([100.0, 50.0, -30.0], &planes);
        assert!(close3(out, [0.0, 50.0, 0.0]), "{out:?}");
    }

    /// Two walls meeting at 90 degrees, running into the corner: the move
    /// stops, except up the crease.
    #[test]
    fn a_corner_keeps_only_the_crease() {
        let planes = [plane([-1.0, 0.0, 0.0], 0.0), plane([0.0, -1.0, 0.0], 0.0)];
        let out = project([100.0, 80.0, 5.0], &planes);
        assert!(close3(out, [0.0, 0.0, 5.0]), "{out:?}");
    }

    /// A floor and two walls: everything stops.
    #[test]
    fn floor_and_two_walls_stop_everything() {
        let planes = [
            plane([0.0, 0.0, 1.0], 0.0),
            plane([-1.0, 0.0, 0.0], 0.0),
            plane([0.0, -1.0, 0.0], 0.0),
        ];
        let out = project([100.0, 80.0, -30.0], &planes);
        assert!(close3(out, [0.0, 0.0, 0.0]), "{out:?}");
    }

    /// Into a crease of two 45-degree walls (a V): the answer runs along the
    /// crease, the smallest change that clears both.
    #[test]
    fn a_v_crease_is_solved_with_both_planes() {
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let planes = [plane([-s, 0.0, s], 0.0), plane([-s, 0.0, -s], 0.0)];
        let out = project([10.0, 3.0, 0.0], &planes);
        assert!(close3(out, [0.0, 3.0, 0.0]), "{out:?}");
        // Each plane alone would leave the other cut: the pair wins.
        for p in planes {
            assert!(dot(p.facing, out) - p.rs > -1e-4);
        }
    }

    /// A plane the start already clears is never pushed toward.
    #[test]
    fn a_plane_behind_is_left_alone() {
        let planes = [plane([0.0, 0.0, 1.0], 0.0), plane([1.0, 0.0, 0.0], -500.0)];
        let out = project([100.0, 0.0, -30.0], &planes);
        assert!(close3(out, [100.0, 0.0, 0.0]), "{out:?}");
    }

    /// A positive rs pushes the move out by that much (a touch distance).
    #[test]
    fn rs_sets_how_far_out_the_answer_lands() {
        let out = project([0.0, 0.0, -1.0], &[plane([0.0, 0.0, 1.0], 0.25)]);
        assert!(close3(out, [0.0, 0.0, 0.25]), "{out:?}");
    }

    #[test]
    fn steep_contacts_ask_the_face() {
        assert!(!is_walkable(false, 1.0, || true));
        assert!(is_walkable(true, 0.7, || false));
        assert!(!is_walkable(true, 0.69, || false));
        assert!(is_walkable(true, 0.2, || true));
    }

    /// A box brush 64 wide, top at z 32: a contact on the top edge is
    /// walkable even if the contact's normal leans off the side; a contact
    /// halfway down the side is not.
    #[test]
    fn the_brush_face_rule_counts_the_top_edge() {
        let (box_lo, box_hi) = ([-32.0, -32.0, 0.0], [32.0, 32.0, 32.0]);
        let up = [0.0, 0.0, 1.0];
        assert!(brush_face_walkable(box_lo, box_hi, &[], [32.0, 0.0, 32.0], up));
        assert!(!brush_face_walkable(box_lo, box_hi, &[], [32.0, 0.0, 16.0], up));
        // A 60-degree ramp side cut across the box's top corner: on the ramp
        // only (below the top), the steep ramp decides: not walkable.
        let n = [0.866_025_4, 0.0, 0.5];
        let ramp = [n[0], n[1], n[2], dot(n, [28.0, 0.0, 32.0])];
        let on_ramp = [30.0, 0.0, 32.0 - 2.0 * 0.866_025_4 / 0.5];
        assert!(!brush_face_walkable(box_lo, box_hi, &[ramp], on_ramp, up));
        // A gentle 30-degree ramp side: walkable.
        let n = [0.5, 0.0, 0.866_025_4];
        let ramp = [n[0], n[1], n[2], dot(n, [28.0, 0.0, 32.0])];
        let on_ramp = [30.0, 0.0, 32.0 - 2.0 * 0.5 / 0.866_025_4];
        assert!(brush_face_walkable(box_lo, box_hi, &[ramp], on_ramp, up));
    }

    /// Terrain: a flat triangle under the point is walkable, a wall triangle
    /// is not, and no triangles is not.
    #[test]
    fn the_terrain_face_rule() {
        let up = [0.0, 0.0, 1.0];
        // Wound so (v0 - v2) x (v0 - v1) points up.
        let floor = [[0.0, 0.0, 0.0], [0.0, 10.0, 0.0], [10.0, 0.0, 0.0]];
        let wall = [[0.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
        assert!(triangles_face_walkable([floor], [2.0, 2.0, 0.0], up));
        // Wound the other way the triangle faces down: not walkable.
        let down = [floor[0], floor[2], floor[1]];
        assert!(!triangles_face_walkable([down], [2.0, 2.0, 0.0], up));
        assert!(!triangles_face_walkable([wall], [0.0, 2.0, 2.0], up));
        assert!(!triangles_face_walkable(
            core::iter::empty(),
            [0.0, 0.0, 0.0],
            up
        ));
    }
}
