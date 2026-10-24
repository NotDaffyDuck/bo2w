//! A world of shapes for the collision tests. Its box traces are never used;
//! it remembers which entities a move touched.

use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::bridge::{CollisionBackend, GroundTraceInput};
use trace_t6::{ENTITYNUM_WORLD, Trace};

use super::hull::player_hull;
use super::{Bo2Collision, Geom, IDENTITY, Shape, add};

pub(crate) const MINS: [f32; 3] = [-15.0, -15.0, 0.0];
pub(crate) const MAXS: [f32; 3] = [15.0, 15.0, 70.0];
pub(crate) const MASK: u32 = 1;

pub(crate) struct World {
    pub shapes: Vec<Geom<'static>>,
    pub touched: RefCell<Vec<i32>>,
}

impl World {
    pub(crate) fn new(shapes: Vec<Geom<'static>>) -> Self {
        Self { shapes, touched: RefCell::new(Vec::new()) }
    }
}

impl CollisionBackend for World {
    fn trace(&self, _input: GroundTraceInput) -> Trace {
        unreachable!("mode 1 only")
    }

    fn touch_entity(&self, trace: &Trace, _end_pos: [f32; 3]) {
        let entity = trace_t6::trace_get_entity_hit_id(&trace);
        self.touched.borrow_mut().push(i32::from(entity));
    }
}

impl Bo2Collision for World {
    fn gjk_on(&self) -> bool {
        true
    }

    fn gjk_query<'a>(&'a self, mins: [f32; 3], maxs: [f32; 3], mask: u32, out: &mut Vec<Geom<'a>>) {
        out.extend(
            self.shapes
                .iter()
                .filter(|g| g.contents & mask != 0 && (0..3).all(|a| g.mins[a] <= maxs[a] && mins[a] <= g.maxs[a]))
                .copied(),
        );
    }
}

/// A new key for each shape (tests that check a key set their own, below
/// 1000).
fn next_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A box brush between `lo` and `hi`, world contents and flags 7.
pub(crate) fn block(lo: [f32; 3], hi: [f32; 3]) -> Geom<'static> {
    let center = [0, 1, 2].map(|a| (lo[a] + hi[a]) * 0.5);
    let half = [0, 1, 2].map(|a| (hi[a] - lo[a]) * 0.5);
    Geom {
        shape: Shape::Box { center, half, brush: true },
        axis: IDENTITY,
        origin: [0.0; 3],
        mins: lo,
        maxs: hi,
        contents: MASK,
        surface_flags: 7,
        hit_id: ENTITYNUM_WORLD,
        sentient: false,
        no_push_out: false,
        key: next_key(),
    }
}

/// A floor whose top is z = 0.
pub(crate) fn floor() -> Geom<'static> {
    block([-256.0, -256.0, -16.0], [256.0, 256.0, 0.0])
}

/// Another standing player, number `num`, origin at `origin`.
pub(crate) fn other_player(origin: [f32; 3], num: u16) -> Geom<'static> {
    let hull = player_hull(MINS, MAXS, 0.0);
    let (lo, hi) = hull.calc_aabb();
    Geom {
        shape: Shape::Player { hull },
        axis: IDENTITY,
        origin,
        mins: add(origin, lo),
        maxs: add(origin, hi),
        contents: MASK,
        surface_flags: 0,
        hit_id: num,
        sentient: true,
        no_push_out: true,
        key: next_key(),
    }
}
