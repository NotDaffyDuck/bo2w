//! Black Ops II's player collision, physics mode 1: its "GJK" character
//! controller. The player is a 12-sided upright body (a capsule against
//! terrain and entity boxes) and every move is a swept GJK test against each
//! brush, box, terrain patch and other player in the way. Written from the
//! facts and numbers in Black Ops II's server; docs/MOVE-T6.md has the map.
//!
//! - [`hull`]: the player's shape, the plane solver, the walkable rules.
//! - `solver`: the GJK distance and swept tests.
//! - `trace`: the player trace and the ground trace.
//! - `slide`: the slide move and the push-out.
//! - `step`: the step-and-slide move.
//!
//! The world supplies the shapes through [`Bo2Collision`]; a world that does
//! not turn it on keeps the box traces. BO2 sweeps the body for the walk
//! and air moves, the ground check and the crouch and stand-up checks
//! ([`PenetrationChecks`]); its prone, lie-down, view-turn, ladder-attach
//! and mantle checks keep the box traces either way.

pub mod hull;
mod slide;
mod solver;
pub(crate) mod step;

#[cfg(test)]
mod testkit;
pub(crate) mod trace;

use alloc::vec::Vec;
use crate::bridge::{SolidProbe, SweepAsk};
use trace_t6::Trace;

pub use hull::{Hull, MAX_MOVER_VERTS, Mover, MoverBrush, player_hull};
pub use slide::vehicle_slide_move;
pub use trace::brush_surface_flags;

/// One thing the player can hit, as the world hands it over.
#[derive(Clone, Copy, Debug)]
pub struct Geom<'a> {
    pub shape: Shape<'a>,
    /// The shape's local axes in the world, one per row. Identity for the
    /// world's brushes and terrain and for players.
    pub axis: [[f32; 3]; 3],
    /// Where the shape's local origin is in the world.
    pub origin: [f32; 3],
    /// The world box around the shape (the broad phase).
    pub lo_corner: [f32; 3],
    pub hi_corner: [f32; 3],
    /// The contents a trace's mask must share to hit it.
    pub contents: u32,
    /// The surface flags a hit reports (brushes: [`brush_surface_flags`];
    /// terrain: its material's; players: 0).
    pub surf_bits: u32,
    /// The entity number a hit reports (the world: `ENTITYNUM_WORLD`).
    pub struck_entity_num: u16,
    /// Another player (or anything alive): the ground trace can be told to
    /// skip these.
    pub sentient: bool,
    /// The slide never pushes the player out of this one (players).
    pub no_push_out: bool,
    /// Unique per shape, so a slide does not hit the same one twice.
    pub key: u64,
}

/// The shape of a [`Geom`], in its own local frame.
#[derive(Clone, Copy, Debug)]
pub enum Shape<'a> {
    /// A brush: its corner points, its box (the six axial faces) and the
    /// planes of its other sides (normal, distance).
    Brush { vertex_points: &'a [[f32; 3]], lo_corner: [f32; 3], hi_corner: [f32; 3], sides: &'a [[f32; 4]] },
    /// A box: centre and half size. `brush` is set when the box is a brush
    /// with only its six axial sides (BO2 builds those as boxes).
    Box { center: [f32; 3], half: [f32; 3], brush: bool },
    /// A terrain patch: a run of triangles (three indices each) into a
    /// vertex table.
    Partition { vertex_points: &'a [[f32; 3]], indices: &'a [u16] },
    /// Another player's body.
    Player { hull: Hull },
}

impl Shape<'_> {
    /// The shape's farthest point toward `d` (local frame, before any skin).
    fn support(&self, d: [f32; 3]) -> [f32; 3] {
        match *self {
            Shape::Brush { vertex_points, .. } => farthest(vertex_points.iter().copied(), d),
            Shape::Box { center, half, .. } => {
                let sign = |a: usize| if d[a] >= 0.0 { half[a] } else { -half[a] };
                [center[0] + sign(0), center[1] + sign(1), center[2] + sign(2)]
            }
            Shape::Partition { vertex_points, indices } => {
                // BO2 walks the patch's unique vertices; the triangles' own
                // indices reach the same points (one past the table is
                // dropped, never a crash).
                farthest(indices.iter().filter_map(|&i| vertex_points.get(usize::from(i)).copied()), d)
            }
            Shape::Player { hull } => hull.support(d),
        }
    }

    /// The point GJK starts from (BO2's get_center): a box's or body's
    /// centre, a brush's or patch's local origin.
    fn center(&self) -> [f32; 3] {
        match *self {
            Shape::Box { center, .. } => center,
            Shape::Player { hull } => hull.core_pt,
            Shape::Brush { .. } | Shape::Partition { .. } => [0.0; 3],
        }
    }

    /// The skin GJK adds around the shape: only bodies have one.
    fn skin(&self) -> f32 {
        match self {
            Shape::Player { hull } => hull.skin(),
            _ => 0.0,
        }
    }

    /// A brush, or a box that is one: the player meets these as its 12-sided
    /// body; everything else meets the capsule.
    fn is_brush(&self) -> bool {
        matches!(*self, Shape::Brush { .. } | Shape::Box { brush: true, .. })
    }
}

/// The first point with the strictly greatest reach toward `d`.
fn farthest(points: impl Iterator<Item = [f32; 3]>, d: [f32; 3]) -> [f32; 3] {
    let mut best = [0.0; 3];
    let mut best_dot = 0.0;
    for (i, p) in points.enumerate() {
        let r = dot(p, d);
        if i == 0 || best_dot < r {
            best = p;
            best_dot = r;
        }
    }
    best
}

/// A world that can run Black Ops II's collision.
pub trait Bo2Collision: SolidProbe {
    /// true: the player moves with BO2's collision (physics mode 1). false:
    /// the box traces of [`SolidQuerySource`].
    fn gjk_on(&self) -> bool {
        false
    }

    /// Every shape whose world box touches `mins..maxs` and whose contents
    /// share `mask`, except the moving player's own body and what it owns.
    fn gjk_query<'a>(&'a self, _mins: [f32; 3], _maxs: [f32; 3], _mask: u32, _out: &mut Vec<Geom<'a>>) {}

    /// Where the moving entity `entnum` is (origin, angles) at `time` (ms),
    /// for the platform carry; `None`: it does not move on a path (or is not
    /// there), and carries nothing.
    fn mover_pose(&self, _entnum: i32, _time: i32) -> Option<([f32; 3], [f32; 3])> {
        None
    }

    /// BO2's CM_GetWaterHeight: the height of the water's surface straight
    /// over or under `origin`, found by a point traced from
    /// [`crate::water::WATER_SEARCH`] above it to as far below it that only
    /// water stops, or [`crate::water::NO_WATER`] when it meets none.
    fn water_height(&self, _origin: [f32; 3]) -> f32 {
        crate::water::NO_WATER
    }
}

/// BO2's is_not_penetrating (PM_CheckDuck, PM_CanStand, PM_CanCrouch):
/// with its collision on, the body placed at a point, and "stuck" is a trace
/// that starts inside something (BO2 reads startsolid there where the box
/// trace reads fully_embedded; the body's trace never sets fully_embedded). Off, the
/// world's box trace. The shared stance checks get this in place of the
/// world.
pub(crate) struct PenetrationChecks<'a, C>(pub &'a C);

impl<C: Bo2Collision> SolidProbe for PenetrationChecks<'_, C> {
    fn trace(&self, input: SweepAsk) -> Trace {
        if !self.0.gjk_on() {
            return self.0.trace(input);
        }
        let SweepAsk { start, end, lo_corner: mins, hi_corner: maxs, collide_filter } = input;
        let mut t = trace::player_trace(self.0, start, end, mins, maxs, collide_filter);
        t.fully_embedded = t.starts_inside;
        t
    }

    fn brush_against(&self, hit: &Trace, finish_location: [f32; 3]) {
        self.0.brush_against(hit, finish_location);
    }
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f32; 3], factor: f32) -> [f32; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(crate) fn magnitude_squared(a: [f32; 3]) -> f32 {
    dot(a, a)
}

/// `axis` (rows) times `p`: a local point's offset in the world.
pub(crate) fn to_world(axis: &[[f32; 3]; 3], p: [f32; 3]) -> [f32; 3] {
    add(add(scale(axis[0], p[0]), scale(axis[1], p[1])), scale(axis[2], p[2]))
}

/// A world direction or offset in `axis`'s local frame.
pub(crate) fn to_local(axis: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(axis[0], v), dot(axis[1], v), dot(axis[2], v)]
}

/// No turn: the world's brushes and terrain, and players.
pub const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
