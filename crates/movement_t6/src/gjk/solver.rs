//! Black Ops II's GJK: the distance between the player's shape and one
//! other shape, and the swept version that finds when along a move they
//! first come within reach. Written from the order, tests and numbers of
//! Black Ops II's server (phys_gjk_info and its helpers).
//!
//! Everything is in the player's frame: its axes are the world's, its origin
//! is the player's origin at the start of the move. The other shape's axes
//! are `m` (rows) and its origin `w` in that frame.

use super::hull::Mover;
use super::Shape;
use super::add;
use super::cross;
use super::dot;
use super::magnitude_squared;
use super::scale;
use super::sub;
use super::to_local;
use super::to_world;

/// Closer than this (squared) the two shapes are taken as touching inside.
const PEN_THRESH_SQ: f32 = 0.001_156_000_1;
/// A new support point this close (squared) to one already held: done.
const DUPLICATE_SQ: f32 = 1.000_000_1e-6;
/// Shorter edges (squared) are skipped when reducing the simplex.
const DEGENERATE_EDGE_SQ: f32 = 9.999_999_4e-11;
/// Smaller determinants are skipped when reducing the simplex.
const DEGENERATE_DET: f32 = 1.0e-5;
/// A triangle flatter than this (squared cross product) is solved on its
/// longest edge instead.
const FLAT_TRIANGLE_SQ: f32 = 0.010_000_001;
/// Shorter directions (squared) are no direction at all.
const MIN_DIR_SQ: f32 = 9.999_999e-9;
const CONV_THRESH: f32 = 0.001;
const SEP_CONV_THRESH: f32 = 0.001;
/// The bounds' starting values.
const START_DIST_SQ: f32 = 34.0;
const MAX_ITER: i32 = 29;
/// How many times the swept test may move its time along.
const MAX_CC_RESET_ITERS: u32 = 5;
/// The swept test's smallest reach.
const MIN_MARGIN: f32 = 0.051;
/// The swept test moves its time along only by more than this.
const LAMBDA_STEP: f32 = 0.0001;
/// When the shapes overlap, the other shape is moved this far away to
/// measure how deep they go.
const PULL_APART: f32 = 17.0;

/// A gap past the separation distance ends the test.
const SEPARATION_TEST: u32 = 1;
/// Only "do they touch" was asked (never by the player's traces).
const INTERSECTION_ONLY: u32 = 2;
/// The swept test's time is settled.
const LAMBDA_DONE: u32 = 4;
const CONTINUOUS: u32 = 8;
/// After two time steps the gap test also runs on the closest distance.
const UPPER_SEPARATION: u32 = 0x10;
const SAW_GAP: u32 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Separated,
    Valid,
    Penetrating,
    Invalid,
}

/// The player's shape and one other, placed in the player's frame.
pub(super) struct Pair<'g, 'a> {
    pub cg1: &'g Mover,
    pub cg2: &'g Shape<'a>,
    /// The other shape's axes in the player's frame (rows).
    pub m: [[f32; 3]; 3],
    /// The other shape's origin in the player's frame.
    pub w: [f32; 3],
}

impl Pair<'_, '_> {
    fn support1(&self, d: [f32; 3]) -> [f32; 3] {
        self.cg1.support(d)
    }

    fn support2(&self, d: [f32; 3]) -> [f32; 3] {
        add(to_world(&self.m, self.cg2.support(to_local(&self.m, d))), self.w)
    }
}

/// Where the two shapes come closest, once both skins are on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Contact {
    /// On the player, from its origin (at the contact time).
    pub p1: [f32; 3],
    /// On the other shape, from the player's origin at the start.
    pub p2: [f32; 3],
    /// From the other shape toward the player, unit length.
    pub facing: [f32; 3],
    /// How far along the move (0..1) the contact is.
    pub lambda: f32,
}

/// The swept part of [`do_gjk_collide`]'s input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sweep {
    /// The player's move relative to the other shape.
    pub translation: [f32; 3],
    pub start_time: f32,
    pub finish_stamp: f32,
}

/// BO2's phys_collide_do_gjk_collide: the closest points of the two shapes
/// with their skins (`r1` the player's, `r2` the other's), or None when they
/// stay more than `sep_thresh` apart (over the sweep, when there is one).
pub(super) fn do_gjk_collide(pair: &Pair, r1: f32, r2: f32, sep_thresh: f32, sweep: Option<Sweep>) -> Option<Contact> {
    let mut g = Gjk::new();
    let (start, end) = match sweep {
        Some(s) => {
            g.rel_t = s.translation;
            g.flags |= CONTINUOUS;
            (s.start_time, s.finish_stamp)
        }
        None => (0.0, 1.0),
    };
    g.radii = r1 + r2;
    g.sep = g.radii + sep_thresh;
    g.pen_sq = PEN_THRESH_SQ;
    match g.collide(pair, start, end) {
        Status::Invalid | Status::Separated => return None,
        Status::Valid | Status::Penetrating => {}
    }
    if g.w_set == 0xf {
        return None;
    }
    let (p1, p2) = g.closest_points();
    let v = g.comp_v(g.w_set);
    let length = libm::sqrtf(magnitude_squared(v));
    if 0.0 == length {
        // BO2 divides by this length unguarded; a zero here only comes from
        // a broken simplex, so it counts as no contact.
        return None;
    }
    let normal = scale(v, 1.0 / length);
    Some(Contact {
        p1: sub(p1, scale(normal, r1)),
        p2: add(p2, scale(normal, r2)),
        facing: normal,
        lambda: g.lambda,
    })
}

struct Gjk {
    /// The support points on each shape, and their difference, per slot.
    a: [[f32; 3]; 4],
    b: [[f32; 3]; 4],
    w: [[f32; 3]; 4],
    /// Each slot's weight in the closest point, per subset of slots.
    lambdas: [[f32; 4]; 16],
    w_set: u32,
    last_w_set: u32,
    /// The closest point of the difference so far: the search direction.
    v: [f32; 3],
    pivot: [f32; 3],
    lower: f32,
    upper: f32,
    iter: i32,
    flags: u32,
    radii: f32,
    sep: f32,
    pen_sq: f32,
    rel_t: [f32; 3],
    lambda: f32,
    cc_reset: u32,
}

impl Gjk {
    fn new() -> Self {
        Gjk {
            a: [[0.0; 3]; 4],
            b: [[0.0; 3]; 4],
            w: [[0.0; 3]; 4],
            lambdas: [[0.0; 4]; 16],
            w_set: 0,
            last_w_set: 0,
            v: [0.0; 3],
            pivot: Default::default(),
            lower: -START_DIST_SQ,
            upper: START_DIST_SQ,
            iter: 0,
            flags: 0,
            radii: 0.0,
            sep: 0.0,
            pen_sq: PEN_THRESH_SQ,
            rel_t: [0.0; 3],
            lambda: 0.0,
            cc_reset: 0,
        }
    }

    /// The first free slot.
    fn slot(&self) -> usize {
        if self.w_set & 1 == 0 {
            0
        } else if self.w_set & 2 == 0 {
            1
        } else {
            (((self.w_set & 4) | 8) >> 2) as usize
        }
    }

    /// Fills `slot` with the support points toward -v (player) and v (the
    /// other shape) and returns their difference from the origin.
    fn support(&mut self, pair: &Pair, slot: usize) -> [f32; 3] {
        let v = self.v;
        self.a[slot] = pair.support1(scale(v, -1.0));
        self.b[slot] = pair.support2(v);
        sub(sub(self.a[slot], self.b[slot]), self.pivot)
    }

    /// A new point this close to one the last simplex held.
    fn duplicate(&self, w: [f32; 3]) -> bool {
        (0..4).any(|i| self.last_w_set & (1 << i) != 0 && magnitude_squared(sub(w, self.w[i])) < DUPLICATE_SQ)
    }

    /// Packs the shape points of `set` into the first slots, in order.
    fn compress_verts(&mut self, set: u32) -> usize {
        let mut n = 0;
        for i in 0..4 {
            if set & (1 << i) != 0 {
                self.a[n] = self.a[i];
                self.b[n] = self.b[i];
                n += 1;
            }
        }
        n
    }

    /// Rebuilds the difference points of the first `n` slots from the shape
    /// points and the current origin, then reduces them.
    fn seed_simplex(&mut self, n: usize) -> u32 {
        for i in 0..n {
            self.w[i] = sub(sub(self.a[i], self.b[i]), self.pivot);
        }
        self.w_set = (1 << n) - 1;
        self.backup1(0, true)
    }

    /// Starts a search along `dir`, or (`reuse`) from the points kept from
    /// the last one. Returns the starting iteration count.
    fn init_gjk(&mut self, dir: [f32; 3], reuse: bool) -> i32 {
        let n = if reuse { self.compress_verts(self.w_set) } else { 0 };
        if n != 0 {
            self.w_set = self.seed_simplex(n);
            self.v = self.comp_v(self.w_set);
            self.last_w_set = self.w_set;
            1
        } else {
            self.v = dir;
            self.last_w_set = 0;
            self.w_set = 0;
            0
        }
    }

    /// BO2's backup1: the smallest part of the simplex (with slot `k`, the
    /// newest point) whose closest point to the origin is nearest. Sets that
    /// part's weights and returns its slots, or 0xf when the tetrahedron
    /// holds the origin. `reuse` (a re-seeded simplex) also tries the old
    /// points alone and the edge between slots 1 and 2.
    fn backup1(&mut self, k: usize, reuse: bool) -> u32 {
        let p = self.w[k];
        let mut best = magnitude_squared(p);
        let mut result = 1u32 << k;
        self.lambdas[result as usize][k] = 1.0;

        let mut others = [0usize; 3];
        let mut edges = [[0.0f32; 3]; 3];
        let mut ee = [0.0f32; 3];
        let mut pe = [0.0f32; 3];
        let mut n = 0;
        for i in 0..4 {
            if self.w_set & (1 << i) == 0 || i == k {
                continue;
            }
            others[n] = i;
            let e = sub(self.w[i], p);
            edges[n] = e;
            ee[n] = magnitude_squared(e);
            pe[n] = dot(e, p);
            if DEGENERATE_EDGE_SQ < ee[n] {
                let t = -pe[n] / ee[n];
                if 0.0 < t && 0.0 < 1.0 - t {
                    let d = magnitude_squared(add(p, scale(e, t)));
                    if d < best {
                        result = (1 << k) | (1 << i);
                        self.lambdas[result as usize][i] = t;
                        self.lambdas[result as usize][k] = 1.0 - t;
                        best = d;
                    }
                }
            }
            if reuse {
                let d = magnitude_squared(self.w[i]);
                if d < best {
                    self.lambdas[1 << i][i] = 1.0;
                    result = 1 << i;
                    best = d;
                }
            }
            n += 1;
        }

        if reuse && n == 2 {
            let e = sub(self.w[2], self.w[1]);
            let l = magnitude_squared(e);
            if DEGENERATE_EDGE_SQ < l {
                let t = -dot(e, self.w[1]) / l;
                if 0.0 < t && 0.0 < 1.0 - t {
                    let d = magnitude_squared(add(self.w[1], scale(e, t)));
                    if d < best {
                        result = 6;
                        self.lambdas[6][1] = 1.0 - t;
                        self.lambdas[6][2] = t;
                        best = d;
                    }
                }
            }
        }

        for i in 0..n.saturating_sub(1) {
            for j in i + 1..n {
                let g = dot(edges[i], edges[j]);
                let det = ee[j] * ee[i] - g * g;
                if DEGENERATE_DET < det {
                    let s = (-pe[i] * ee[j] + pe[j] * g) / det;
                    let u = (-ee[i] * pe[j] + g * pe[i]) / det;
                    let r = (1.0 - s) - u;
                    if 0.0 < s && 0.0 < u && 0.0 < r {
                        let d = magnitude_squared(add(add(p, scale(edges[i], s)), scale(edges[j], u)));
                        if d < best {
                            result = (1 << others[i]) | (1 << others[j]) | (1 << k);
                            let slot = &mut self.lambdas[result as usize];
                            slot[others[i]] = s;
                            slot[others[j]] = u;
                            slot[k] = r;
                            best = d;
                        }
                    }
                }
            }
        }

        if n == 3 && self.lower <= 0.0 {
            let c = cross(edges[0], edges[1]);
            let det = dot(c, edges[2]);
            if DEGENERATE_DET < libm::fabsf(det) {
                let t3 = -dot(c, p) / det;
                if t3 >= 0.0 {
                    let q = cross(p, edges[2]);
                    let t1 = dot(q, edges[1]) / det;
                    let t2 = -dot(edges[0], q) / det;
                    if t1 >= 0.0 && t2 >= 0.0 && ((1.0 - t1) - t2) - t3 >= 0.0 {
                        return 0xf;
                    }
                }
            }
        }
        result
    }

    /// The point of `set`'s simplex closest to the origin.
    fn comp_v(&self, set: u32) -> [f32; 3] {
        if set == 0xf {
            return [0.0; 3];
        }
        let mut p = [[0.0f32; 3]; 4];
        let mut n = 0;
        for i in 0..4 {
            if set & (1 << i) != 0 {
                p[n] = self.w[i];
                n += 1;
            }
        }
        match n {
            1 => p[0],
            2 => closest_on_line(p[0], sub(p[1], p[0])),
            3 => {
                let c = cross(sub(p[1], p[0]), sub(p[2], p[0]));
                let cc = magnitude_squared(c);
                if FLAT_TRIANGLE_SQ < cc {
                    return scale(c, dot(c, p[0]) / cc);
                }
                // Too flat: the longest edge.
                let e = [sub(p[1], p[0]), sub(p[2], p[1]), sub(p[0], p[2])];
                let l = [magnitude_squared(e[0]), magnitude_squared(e[1]), magnitude_squared(e[2])];
                let i = if l[0] < l[1] {
                    if l[1] < l[2] { 2 } else { 1 }
                } else if l[0] < l[2] {
                    2
                } else {
                    0
                };
                if l[i] > 0.0 { closest_on_line(p[i], e[i]) } else { p[i] }
            }
            _ => [0.0; 3],
        }
    }

    /// The closest points on the two shapes (no skins), from the simplex's
    /// weights.
    #[allow(clippy::needless_range_loop)] // one index walks the weights and both shapes' points
    fn closest_points(&self) -> ([f32; 3], [f32; 3]) {
        let mut sum = 0.0;
        let mut p1 = [0.0; 3];
        let mut p2 = [0.0; 3];
        let weights = self.lambdas[self.w_set as usize];
        for i in 0..4 {
            if self.w_set & (1 << i) != 0 {
                sum += weights[i];
                p1 = add(p1, scale(self.a[i], weights[i]));
                p2 = add(p2, scale(self.b[i], weights[i]));
            }
        }
        (scale(p1, 1.0 / sum), scale(p2, 1.0 / sum))
    }

    /// BO2's get_initial_support_dir: from the other shape's centre to the
    /// player's, or +x when they coincide.
    fn initial_dir(pair: &Pair) -> [f32; 3] {
        let other = add(to_world(&pair.m, pair.cg2.center()), pair.w);
        let d = sub(pair.cg1.center(), other);
        if MIN_DIR_SQ <= magnitude_squared(d) { d } else { [1.0_f32, 0.0, 0.0] }
    }

    /// The plain distance search from `dir`.
    fn gjk(&mut self, pair: &Pair, dir: [f32; 3], reuse: bool) -> Status {
        self.lower = -START_DIST_SQ;
        self.upper = START_DIST_SQ;
        self.iter = self.init_gjk(dir, reuse);
        loop {
            self.upper = magnitude_squared(self.v);
            if self.iter != 0 && self.upper < self.pen_sq {
                return Status::Penetrating;
            }
            let slot = self.slot();
            let w = self.support(pair, slot);
            let vw = dot(w, self.v);
            if 0.0 < vw && 0.0 < self.upper {
                let t = vw * vw / self.upper;
                if self.lower < t {
                    self.lower = t;
                    if self.flags & SEPARATION_TEST != 0 && self.sep * self.sep < t {
                        return Status::Separated;
                    }
                }
            }
            if self.iter != 0 && 0.0 < self.lower && (conv() * self.upper < self.lower || self.duplicate(w)) {
                return Status::Valid;
            }
            if let Some(found) = self.add_point(slot, w) {
                return found;
            }
            if MAX_ITER < self.iter {
                return Status::Valid;
            }
        }
    }

    /// Adds `w` in `slot`, reduces the simplex and takes its closest point.
    /// Some(Penetrating) when the simplex holds the origin.
    fn add_point(&mut self, idx: usize, w: [f32; 3]) -> Option<Status> {
        self.w[idx] = w;
        self.w_set |= 1 << idx;
        self.last_w_set = self.w_set;
        self.w_set = self.backup1(idx, false);
        if self.w_set == 0xf {
            return Some(Status::Penetrating);
        }
        self.v = self.comp_v(self.w_set);
        self.iter += 1;
        None
    }

    /// The swept search: moves the player along `rel_t` from `start` to the
    /// first time the shapes come within reach.
    fn ray_cast(&mut self, pair: &Pair, dir: [f32; 3], start: f32, end: f32) -> Status {
        self.lambda = start;
        self.pivot = scale(self.rel_t, -start);
        self.lower = -START_DIST_SQ;
        self.upper = START_DIST_SQ;
        self.iter = self.init_gjk(dir, false);
        // The gap tests start off; the time steps turn them on.
        self.flags &= !(SEPARATION_TEST | LAMBDA_DONE | SAW_GAP | UPPER_SEPARATION);
        self.cc_reset = 0;
        let margin = if MIN_MARGIN <= self.radii { self.radii } else { MIN_MARGIN };
        loop {
            self.upper = magnitude_squared(self.v);
            if self.iter != 0 {
                if self.flags & UPPER_SEPARATION != 0 && self.upper < self.sep * self.sep {
                    self.flags |= SEPARATION_TEST;
                }
                if self.upper < self.pen_sq {
                    return Status::Penetrating;
                }
            }
            let slot = self.slot();
            let w = self.support(pair, slot);
            let vw = dot(w, self.v);
            if 0.0 < vw && 0.0 < self.upper {
                self.flags |= SAW_GAP;
                let t = vw * vw / self.upper;
                if self.lower < t {
                    self.lower = t;
                    if self.flags & SEPARATION_TEST != 0 && self.sep * self.sep < t {
                        return Status::Separated;
                    }
                }
            }
            let done = self.iter != 0 && 0.0 < self.lower && (conv() * self.upper < self.lower || self.duplicate(w));

            if self.flags & LAMBDA_DONE == 0 && 0.0 < vw && margin * margin * self.upper <= vw * vw {
                let nv = libm::sqrtf(self.upper);
                let closing = -dot(self.v, self.rel_t);
                let gap = vw - nv * margin;
                if 0.0 < closing {
                    if 0.0 <= gap {
                        let vo = dot(self.v, self.pivot);
                        let mut lam = (vo + gap) / closing;
                        if self.lambda + LAMBDA_STEP < lam {
                            if end < lam {
                                if self.sep * nv < vo + (gap - end * closing) {
                                    return Status::Separated;
                                }
                                lam = end;
                                self.flags |= SEPARATION_TEST | LAMBDA_DONE;
                            }
                            if lam - self.lambda <= LAMBDA_STEP {
                                self.flags |= SEPARATION_TEST | LAMBDA_DONE;
                            }
                            self.lambda = lam;
                            self.pivot = scale(self.rel_t, -lam);
                            self.cc_reset += 1;
                            self.lower = -START_DIST_SQ;
                            self.upper = START_DIST_SQ;
                            if 1 < self.cc_reset {
                                self.flags |= UPPER_SEPARATION;
                            }
                            if MAX_CC_RESET_ITERS <= self.cc_reset {
                                self.flags |= SEPARATION_TEST | LAMBDA_DONE;
                            }
                            if self.w_set == 0 {
                                self.w_set = 1 << slot;
                            }
                            self.init_gjk(self.v, true);
                            self.iter += 1;
                            continue;
                        }
                    }
                } else if 0.0 < gap {
                    // Moving apart with a gap: no contact on this move.
                    if self.sep * self.sep < self.lower {
                        return Status::Separated;
                    }
                    self.flags |= SEPARATION_TEST | LAMBDA_DONE;
                }
            }
            if done {
                return Status::Valid;
            }
            if let Some(found) = self.add_point(slot, w) {
                return found;
            }
            if MAX_ITER < self.iter {
                return if 0.0 < self.lower { Status::Valid } else { Status::Penetrating };
            }
        }
    }

    /// BO2's collide: the search (swept or not), and when the shapes
    /// overlap, how deep: the other shape is moved 17 units out along the
    /// best separating direction and the distance measured again.
    fn collide(&mut self, pair: &Pair, start: f32, end: f32) -> Status {
        self.pivot = [0.0; 3];
        self.flags |= SEPARATION_TEST;
        let d0 = Self::initial_dir(pair);
        let verdict = if self.flags & CONTINUOUS == 0 {
            self.gjk(pair, d0, false)
        } else {
            self.ray_cast(pair, d0, start, end)
        };
        if verdict != Status::Penetrating || self.flags & INTERSECTION_ONLY != 0 {
            return verdict;
        }
        self.lambda = 0.0;
        let mut sd = comp_sep_dir(pair);
        if magnitude_squared(sd) < MIN_DIR_SQ {
            if magnitude_squared(d0) < MIN_DIR_SQ {
                return Status::Invalid;
            }
            sd = d0;
        }
        self.flags &= !SEPARATION_TEST;
        let mut dd = magnitude_squared(sd);
        let mut it = 0;
        let sep_conv = (1.0 - SEP_CONV_THRESH) * (1.0 - SEP_CONV_THRESH);
        loop {
            it += 1;
            let a = pair.support1(scale(sd, -1.0));
            let b = pair.support2(sd);
            let k = -(PULL_APART / libm::sqrtf(dd) - dot(sd, sub(a, b)) / dd);
            self.pivot = scale(sd, k);
            if self.gjk(pair, sd, 1 < it) == Status::Penetrating {
                return Status::Invalid;
            }
            sd = self.comp_v(self.w_set);
            dd = magnitude_squared(sd);
            if !(PULL_APART * PULL_APART <= sep_conv * self.lower && it < 10) {
                break;
            }
        }
        Status::Penetrating
    }
}

/// The point of the line through `p` along `e` closest to the origin.
fn closest_on_line(p: [f32; 3], e: [f32; 3]) -> [f32; 3] {
    sub(p, scale(e, dot(e, p) / magnitude_squared(e)))
}

/// GJK is done when the gap's lower bound is within this share of the
/// upper bound.
fn conv() -> f32 {
    (1.0 - CONV_THRESH) * (1.0 - CONV_THRESH)
}

/// BO2's 12 fixed directions for the separating-direction guess.
const SEP_DIRS: [[f32; 3]; 12] = [
    [-0.515_304, 0.445_123, -0.732_344],
    [-0.222_601, 0.275_979, 0.935_032],
    [0.497_603, 0.665_553, -0.556_264],
    [0.515_302, -0.445_124, 0.732_344],
    [-0.960_414, 0.204_353, 0.189_327],
    [-0.678_502, -0.561_016, -0.474_231],
    [0.960_414, -0.204_353, -0.189_327],
    [-0.497_604, -0.665_553, 0.556_264],
    [0.222_601, -0.275_977, -0.935_032],
    [0.678_504, 0.561_015, 0.474_231],
    [-0.233_54, 0.962_414, 0.138_628],
    [0.233_54, -0.962_414, -0.138_628],
];

/// The 20 faces over those directions' support points.
const SEP_TRIS: [[usize; 3]; 20] = [
    [10, 1, 4],
    [8, 11, 6],
    [0, 10, 4],
    [2, 8, 6],
    [0, 2, 10],
    [2, 0, 8],
    [9, 1, 10],
    [9, 2, 6],
    [2, 9, 10],
    [1, 7, 4],
    [5, 11, 8],
    [0, 5, 8],
    [5, 7, 11],
    [5, 0, 4],
    [7, 5, 4],
    [7, 3, 11],
    [11, 3, 6],
    [3, 9, 6],
    [9, 3, 1],
    [3, 7, 1],
];

/// BO2's comp_sep_dir: the best way out for two overlapping shapes. Builds
/// a 20-face hull of the shapes' difference from 12 fixed directions and
/// returns the inward normal (not unit length) of the face nearest the
/// origin.
fn comp_sep_dir(pair: &Pair) -> [f32; 3] {
    let mut wd = [[0.0f32; 3]; 12];
    let mut a_sum = [0.0; 3];
    let mut b_sum = [0.0; 3];
    let mut w_sum = [0.0; 3];
    for (i, d) in SEP_DIRS.iter().enumerate() {
        let a = pair.support1(*d);
        let b = pair.support2(scale(*d, -1.0));
        wd[i] = sub(a, b);
        a_sum = add(a_sum, a);
        b_sum = add(b_sum, b);
        w_sum = add(w_sum, wd[i]);
    }
    let twelfth = 0.083_333_336;
    let a_avg = scale(a_sum, twelfth);
    let b_avg = scale(b_sum, twelfth);
    let w_avg = scale(w_sum, twelfth);

    let mut best = None::<([f32; 3], f32)>;
    for [i0, i1, i2] in SEP_TRIS {
        let mut n = cross(sub(wd[i1], wd[i0]), sub(wd[i2], wd[i0]));
        if dot(sub(wd[i0], w_avg), n) < 0.0 {
            n = scale(n, -1.0);
        }
        let nn = magnitude_squared(n);
        if MIN_DIR_SQ <= nn {
            let f = dot(scale(wd[i0], -1.0), n);
            let mut score = f * f / nn;
            if f < 0.0 {
                score = -score;
            }
            if best.is_none_or(|(_, s)| s < score) {
                best = Some((n, score));
            }
        }
    }
    if let Some((n, _)) = best {
        return scale(n, -1.0);
    }
    let d = sub(a_avg, b_avg);
    if MIN_DIR_SQ < magnitude_squared(d) { d } else { [1.0_f32, 0.0, 0.0] }
}

#[cfg(test)]
mod tests {
    use super::super::{Hull, IDENTITY, player_hull};
    use super::*;

    fn standing() -> Hull {
        player_hull([-15.0, -15.0, 0.0], [15.0, 15.0, 70.0], 0.0)
    }

    fn floor_box() -> Shape<'static> {
        Shape::Box { center: [0.0, 0.0, -8.0], half: [256.0, 256.0, 8.0], brush: true }
    }

    #[test]
    fn standing_above_a_floor_measures_the_gap() {
        let hull = standing();
        let floor = floor_box();
        // The player's origin 10 above the floor's top.
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, -10.0] };
        let c = do_gjk_collide(&pair, hull.skin(), 0.0, 100.0, None).expect("in reach");
        assert!((c.facing[2] - 1.0).abs() < 1e-4, "{c:?}");
        // The feet's tip is 0.375 above the origin, the skin 0.5 below that.
        let gap = c.p1[2] - c.p2[2];
        assert!((gap - (10.0 + 0.375 - 0.5)).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn out_of_reach_is_none() {
        let hull = standing();
        let floor = floor_box();
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, -50.0] };
        assert!(do_gjk_collide(&pair, hull.skin(), 0.0, 1.0, None).is_none());
    }

    #[test]
    fn a_fall_onto_the_floor_stops_at_the_skin() {
        let hull = standing();
        let floor = floor_box();
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, -10.0] };
        let sweep = Sweep { translation: [0.0, 0.0, -20.0], start_time: 0.0, finish_stamp: 1.0 };
        let c = do_gjk_collide(&pair, hull.skin(), 0.0, 0.125, Some(sweep)).expect("hits");
        // The core gap of 10.375 closes to the 0.5 skin after 9.875 of the
        // 20 down.
        let drop = c.lambda * 20.0;
        assert!(drop > 9.0 && drop < 10.0, "{c:?}");
        assert!((c.facing[2] - 1.0).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn a_move_away_never_hits() {
        let hull = standing();
        let floor = floor_box();
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, -10.0] };
        let sweep = Sweep { translation: [0.0, 0.0, 20.0], start_time: 0.0, finish_stamp: 1.0 };
        assert!(do_gjk_collide(&pair, hull.skin(), 0.0, 0.125, Some(sweep)).is_none());
    }

    #[test]
    fn sunk_into_the_floor_reports_the_depth() {
        let hull = standing();
        let floor = floor_box();
        // The origin 3 below the floor's top.
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, 3.0] };
        let c = do_gjk_collide(&pair, hull.skin(), 0.0, 0.125, None).expect("overlap");
        assert!((c.facing[2] - 1.0).abs() < 1e-3, "{c:?}");
        let depth = dot(c.facing, sub(c.p1, c.p2));
        assert!((depth - (-3.0 + 0.375 - 0.5)).abs() < 1e-2, "{depth} {c:?}");
    }

    #[test]
    fn a_wall_is_met_by_the_twelve_sided_body() {
        let hull = standing();
        let wall = Shape::Box { center: [0.0, 0.0, 0.0], half: [8.0, 256.0, 256.0], brush: true };
        // The wall's near face 40 ahead on +x.
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &wall, m: IDENTITY, w: [48.0, 0.0, 0.0] };
        let c = do_gjk_collide(&pair, hull.skin(), 0.0, 100.0, None).expect("in reach");
        assert!((c.facing[0] + 1.0).abs() < 1e-4, "{c:?}");
        // The ring's corner reaches the polygon radius toward +x.
        let reach = c.p1[0];
        assert!((reach - (hull.polygon_radius + 0.5)).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn sep_dir_points_out_of_a_shallow_overlap() {
        let hull = standing();
        let floor = floor_box();
        let pair = Pair { cg1: &Mover::Body(hull), cg2: &floor, m: IDENTITY, w: [0.0, 0.0, 3.0] };
        let d = comp_sep_dir(&pair);
        // The guess is the nearest of BO2's 20 fixed faces: mostly up.
        assert!(d[2] > 0.0, "{d:?}");
    }
}
