//! A coarse spatial hash of the map's loose pieces, rebuilt every frame, so
//! a bullet, a knife swipe or a blast only tests the pieces near it instead
//! of every dynamic entity in the level.

use std::collections::HashMap;

use bevy::prelude::*;

use super::wakeable;
use crate::adapters::anim::dyn_ent::PieceModels;
use crate::prepare::scene::cull::DynEntModelEntity;
use crate::prepare::scene::world::WorldDynEntInstance;

/// Edge of a grid cell, in map units.
const CELL_EDGE: f32 = 256.0;

/// A lookup spanning more cells than this is not worth walking cell by cell.
const MAX_CELLS_PER_LOOKUP: i64 = 4096;

type CellId = [i32; 3];

#[derive(Resource, Default)]
pub(super) struct WakeGrid {
    /// Every piece seen at the last refill, in query order; buckets store
    /// positions in this list.
    members: Vec<Entity>,
    buckets: HashMap<CellId, Vec<u32>>,
    /// False while some piece could not be placed (a non-finite position),
    /// and before the first refill: then the grid cannot vouch for what it
    /// does not list.
    complete: bool,
    /// Scratch for one lookup: member positions, then the entities.
    found: Vec<u32>,
    answer: Vec<Entity>,
}

impl WakeGrid {
    /// Throws the old contents away and files `pieces` (entity, centre, reach)
    /// under every cell their reach touches.
    pub(super) fn refill(&mut self, pieces: impl IntoIterator<Item = (Entity, Vec3, f32)>) {
        self.members.clear();
        self.buckets.clear();
        self.complete = true;
        for (entity, center, reach) in pieces {
            let slot = self.members.len() as u32;
            self.members.push(entity);
            let spread = Vec3::splat(reach.max(0.0));
            let Some((first, last)) = cell_range(center - spread, center + spread) else {
                self.complete = false;
                continue;
            };
            for cell in cells_between(first, last) {
                self.buckets.entry(cell).or_default().push(slot);
            }
        }
    }

    /// The pieces that may lie inside the box `lo..hi`, each once, in the
    /// order they were filed. When the grid cannot answer (it is incomplete,
    /// the box is not finite, or it spans too many cells) this is every piece
    /// `all` yields instead.
    pub(super) fn near_box(
        &mut self,
        lo: Vec3,
        hi: Vec3,
        all: impl Iterator<Item = Entity>,
    ) -> &[Entity] {
        self.answer.clear();
        let range = cell_range(lo, hi)
            .filter(|&(first, last)| self.complete && cell_total(first, last) <= MAX_CELLS_PER_LOOKUP);
        let Some((first, last)) = range else {
            self.answer.extend(all);
            return &self.answer;
        };
        self.found.clear();
        for cell in cells_between(first, last) {
            if let Some(slots) = self.buckets.get(&cell) {
                self.found.extend_from_slice(slots);
            }
        }
        self.found.sort_unstable();
        self.found.dedup();
        let members = &self.members;
        self.answer
            .extend(self.found.iter().map(|&slot| members[slot as usize]));
        &self.answer
    }

    /// [`Self::near_box`] around the segment `a`..`b`.
    pub(super) fn near_segment(
        &mut self,
        a: Vec3,
        b: Vec3,
        all: impl Iterator<Item = Entity>,
    ) -> &[Entity] {
        self.near_box(a.min(b), a.max(b), all)
    }

    /// [`Self::near_box`] around a sphere.
    pub(super) fn near_sphere(
        &mut self,
        center: Vec3,
        radius: f32,
        all: impl Iterator<Item = Entity>,
    ) -> &[Entity] {
        let spread = Vec3::splat(radius.max(0.0));
        self.near_box(center - spread, center + spread, all)
    }
}

fn cell_of(coordinate: f32) -> Option<i32> {
    coordinate
        .is_finite()
        .then(|| (coordinate / CELL_EDGE).floor() as i32)
}

/// The cells holding the two corners of a box, or `None` if either corner
/// is not a finite point.
fn cell_range(lo: Vec3, hi: Vec3) -> Option<(CellId, CellId)> {
    let corner = |p: Vec3| Some([cell_of(p.x)?, cell_of(p.y)?, cell_of(p.z)?]);
    Some((corner(lo)?, corner(hi)?))
}

fn cell_total(first: CellId, last: CellId) -> i64 {
    (0..3)
        .map(|axis| (i64::from(last[axis]) - i64::from(first[axis]) + 1).max(0))
        .fold(1, i64::saturating_mul)
}

fn cells_between(first: CellId, last: CellId) -> impl Iterator<Item = CellId> {
    (first[2]..=last[2]).flat_map(move |z| {
        (first[1]..=last[1])
            .flat_map(move |y| (first[0]..=last[0]).map(move |x| [x, y, z]))
    })
}

/// Files this frame's pieces. A piece that cannot be knocked loose gets no
/// reach (just its centre), a wakeable one the radius of its model.
pub(super) fn refresh_wake_grid(
    catalog: Option<Res<asset_world::MapXModelSceneCatalog>>,
    t6: Option<Res<assets::PreparedFpvMeshes>>,
    pieces: Query<(Entity, &WorldDynEntInstance, &Transform), With<DynEntModelEntity>>,
    mut grid: ResMut<WakeGrid>,
) {
    let models = PieceModels {
        map: catalog.as_deref(),
        t6: t6.as_deref(),
    };
    grid.refill(pieces.iter().map(|(entity, piece, at)| {
        let reach = match wakeable(piece) {
            Some(_) => models.radius(&piece.current_model).max(1.0),
            None => 0.0,
        };
        (entity, at.translation, reach)
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entities(n: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }

    fn nothing() -> impl Iterator<Item = Entity> {
        std::iter::empty()
    }

    #[test]
    fn cells_floor_toward_negative_infinity() {
        assert_eq!(cell_of(0.0), Some(0));
        assert_eq!(cell_of(255.9), Some(0));
        assert_eq!(cell_of(256.0), Some(1));
        assert_eq!(cell_of(-0.1), Some(-1));
        assert_eq!(cell_of(f32::NAN), None);
        assert_eq!(cell_of(f32::INFINITY), None);
    }

    #[test]
    fn a_box_with_a_bad_corner_has_no_cells() {
        assert!(cell_range(Vec3::ZERO, Vec3::new(1.0, f32::NAN, 1.0)).is_none());
        assert!(cell_range(Vec3::splat(f32::NEG_INFINITY), Vec3::ZERO).is_none());
    }

    #[test]
    fn cell_totals_multiply_and_never_go_negative() {
        assert_eq!(cell_total([0, 0, 0], [0, 0, 0]), 1);
        assert_eq!(cell_total([-1, 0, 2], [1, 3, 2]), 3 * 4);
        assert_eq!(cell_total([5, 0, 0], [4, 0, 0]), 0);
        assert!(cell_total([i32::MIN; 3], [i32::MAX; 3]) > MAX_CELLS_PER_LOOKUP);
    }

    #[test]
    fn cells_between_visits_each_cell_once() {
        let all: Vec<_> = cells_between([0, -1, 0], [1, 0, 0]).collect();
        assert_eq!(all.len(), 4);
        for cell in [[0, -1, 0], [1, -1, 0], [0, 0, 0], [1, 0, 0]] {
            assert!(all.contains(&cell));
        }
    }

    #[test]
    fn a_new_grid_cannot_vouch_for_anything() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        let got = grid.near_box(Vec3::ZERO, Vec3::ONE, ids.iter().copied());
        assert_eq!(got, &ids[..]);
    }

    #[test]
    fn only_pieces_near_the_box_come_back() {
        let ids = entities(3);
        let mut grid = WakeGrid::default();
        grid.refill([
            (ids[0], Vec3::new(10.0, 10.0, 10.0), 5.0),
            (ids[1], Vec3::new(2000.0, 0.0, 0.0), 5.0),
            (ids[2], Vec3::new(100.0, 100.0, 100.0), 5.0),
        ]);
        let got = grid.near_box(Vec3::ZERO, Vec3::splat(200.0), nothing()).to_vec();
        assert_eq!(got, vec![ids[0], ids[2]]);
    }

    #[test]
    fn a_wide_piece_is_found_from_a_cell_it_only_reaches_into() {
        let ids = entities(1);
        let mut grid = WakeGrid::default();
        // Centre in cell 0, but a 300-unit reach spills into cell 1.
        grid.refill([(ids[0], Vec3::new(10.0, 0.0, 0.0), 300.0)]);
        let got = grid
            .near_box(Vec3::new(400.0, 0.0, 0.0), Vec3::new(500.0, 1.0, 1.0), nothing())
            .to_vec();
        assert_eq!(got, ids);
    }

    #[test]
    fn a_piece_in_many_cells_is_reported_once_and_in_filing_order() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        grid.refill([
            (ids[0], Vec3::ZERO, 600.0),
            (ids[1], Vec3::new(1.0, 1.0, 1.0), 0.0),
        ]);
        let got = grid.near_box(Vec3::splat(-700.0), Vec3::splat(700.0), nothing()).to_vec();
        assert_eq!(got, ids);
    }

    #[test]
    fn a_misplaced_piece_sends_every_lookup_to_the_full_list() {
        let ids = entities(3);
        let mut grid = WakeGrid::default();
        grid.refill([
            (ids[0], Vec3::ZERO, 1.0),
            (ids[1], Vec3::new(f32::NAN, 0.0, 0.0), 1.0),
        ]);
        let got = grid.near_box(Vec3::ZERO, Vec3::ONE, ids.iter().copied()).to_vec();
        assert_eq!(got, ids);
    }

    #[test]
    fn a_lookup_over_too_many_cells_uses_the_full_list() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        grid.refill([(ids[0], Vec3::ZERO, 1.0)]);
        let wide = Vec3::splat(CELL_EDGE * 40.0);
        let got = grid.near_box(-wide, wide, ids.iter().copied()).to_vec();
        assert_eq!(got, ids);
    }

    #[test]
    fn refilling_forgets_the_previous_frame() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        grid.refill([(ids[0], Vec3::ZERO, 1.0)]);
        grid.refill([(ids[1], Vec3::ZERO, 1.0)]);
        let got = grid.near_box(Vec3::splat(-1.0), Vec3::ONE, nothing()).to_vec();
        assert_eq!(got, vec![ids[1]]);
    }

    #[test]
    fn a_misplaced_piece_is_forgiven_by_the_next_refill() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        grid.refill([(ids[0], Vec3::splat(f32::INFINITY), 1.0)]);
        grid.refill([(ids[1], Vec3::ZERO, 1.0)]);
        let got = grid.near_box(Vec3::splat(-1.0), Vec3::ONE, nothing()).to_vec();
        assert_eq!(got, vec![ids[1]]);
    }

    #[test]
    fn segment_and_sphere_lookups_cover_their_extent() {
        let ids = entities(2);
        let mut grid = WakeGrid::default();
        grid.refill([
            (ids[0], Vec3::new(1000.0, 0.0, 0.0), 1.0),
            (ids[1], Vec3::new(0.0, 5000.0, 0.0), 1.0),
        ]);
        // Order of the endpoints does not matter.
        let along = grid
            .near_segment(Vec3::new(1100.0, 0.0, 0.0), Vec3::new(900.0, 0.0, 0.0), nothing())
            .to_vec();
        assert_eq!(along, vec![ids[0]]);
        let around = grid.near_sphere(Vec3::new(0.0, 4900.0, 0.0), 200.0, nothing()).to_vec();
        assert_eq!(around, vec![ids[1]]);
        // A negative radius behaves as a point.
        let point = grid.near_sphere(Vec3::new(1000.0, 0.0, 0.0), -50.0, nothing()).to_vec();
        assert_eq!(point, vec![ids[0]]);
    }
}
