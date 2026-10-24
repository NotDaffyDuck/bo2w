use clipmap_t6::WORLD_SECTORS;

use super::{AreaEntityWorldSnapshot, AreaEntityWorldSnapshotError as Bad, Snapshot};
use crate::area_world::{AreaBox, AreaWorld};
use crate::world::Tick;

const CONTENTS: u32 = 0x4000;

fn arena() -> AreaBox {
    AreaBox { mins: [-2048.0; 3], maxs: [2048.0; 3] }
}

fn crate_at(centre: [f32; 3]) -> AreaBox {
    AreaBox::from_mid_half(centre, [16.0; 3]).expect("a finite box")
}

/// An arena with a handful of entities spread out so that the sector tree has to split.
fn busy_area() -> AreaWorld {
    let mut area = AreaWorld::new(arena());
    for num in 0..24usize {
        let step = num as f32;
        let centre = [-1500.0 + step * 130.0, 900.0 - step * 75.0, (step % 5.0) * 40.0];
        area.link(num, CONTENTS, crate_at(centre));
    }
    // Two entities share a spot, so one sector's chain holds both.
    area.link(30, CONTENTS, crate_at([5.0, 5.0, 5.0]));
    area.link(31, CONTENTS, crate_at([6.0, 5.0, 5.0]));
    area
}

#[test]
fn an_empty_area_saves_and_loads_back_unchanged() {
    let area = AreaWorld::new(arena());
    let rows = AreaEntityWorldSnapshot::capture(&area);
    assert!(rows.entities.is_empty());
    assert_eq!(rows.restore().expect("empty rows load"), area);
}

#[test]
fn a_busy_area_survives_the_round_trip_and_still_answers_queries() {
    let area = busy_area();
    let rows = AreaEntityWorldSnapshot::capture(&area);
    assert_eq!(rows.entities.len(), 26);
    assert_eq!(rows.validate(), Ok(()));
    let loaded = rows.restore().expect("rows load");
    assert_eq!(loaded, area);

    let probe = AreaBox { mins: [-100.0; 3], maxs: [100.0; 3] };
    assert_eq!(loaded.query(probe, CONTENTS, 64), area.query(probe, CONTENTS, 64));
    assert_eq!(loaded.bounds(31), area.bounds(31));
}

#[test]
fn unlinking_leaves_a_free_list_that_still_round_trips() {
    let mut area = busy_area();
    for num in [3, 4, 30] {
        assert!(area.unlink(num));
    }
    let rows = AreaEntityWorldSnapshot::capture(&area);
    assert_eq!(rows.entities.len(), 23);
    // Freed sectors and unlinked entities keep stale bytes in the live table that the rows never carry, so the
    // loaded area cannot equal the live one; saving it again must give back the same rows.
    let loaded = rows.restore().expect("rows load");
    assert_eq!(AreaEntityWorldSnapshot::capture(&loaded), rows);
    assert_eq!(loaded.world.free_head, area.world.free_head);
}

#[test]
fn the_closing_run_of_free_sectors_is_stored_as_one_number() {
    let area = AreaWorld::new(arena());
    let rows = AreaEntityWorldSnapshot::capture(&area);
    assert!(rows.contiguous_free_head != 0);
    let first = usize::from(rows.contiguous_free_head);
    // The stored prefix plus the run together name every free sector once.
    let named = rows.free_prefix.len() + (WORLD_SECTORS - first);
    let in_use = rows.sectors.len();
    assert_eq!(named + in_use + 1, WORLD_SECTORS);
}

fn saved() -> AreaEntityWorldSnapshot {
    AreaEntityWorldSnapshot::capture(&busy_area())
}

#[test]
fn bad_world_bounds_are_refused() {
    let mut rows = saved();
    rows.world_mins[1] = rows.world_maxs[1] + 1.0;
    assert_eq!(rows.validate(), Err(Bad::InvalidWorldBounds));
    let mut rows = saved();
    rows.world_maxs[0] = f32::NAN;
    assert_eq!(rows.validate(), Err(Bad::InvalidWorldBounds));
}

#[test]
fn a_broken_free_list_is_refused() {
    let mut rows = saved();
    rows.contiguous_free_head = 1;
    assert_eq!(rows.validate(), Err(Bad::InvalidFreeChain));

    let mut rows = saved();
    rows.contiguous_free_head = WORLD_SECTORS as u16;
    assert_eq!(rows.validate(), Err(Bad::InvalidFreeChain));

    // The same sector twice.
    let mut rows = saved();
    rows.free_prefix = vec![5, 5];
    assert_eq!(rows.validate(), Err(Bad::InvalidFreeChain));

    // The root sector is never free.
    let mut rows = saved();
    rows.free_prefix = vec![1];
    assert_eq!(rows.validate(), Err(Bad::InvalidFreeChain));
}

#[test]
fn sector_rows_must_cover_exactly_the_sectors_not_free() {
    let mut rows = saved();
    rows.sectors.pop();
    assert_eq!(rows.validate(), Err(Bad::InvalidSectorRows));

    let mut rows = saved();
    let duplicate = rows.sectors[0];
    rows.sectors.push(duplicate);
    assert_eq!(rows.validate(), Err(Bad::InvalidSectorRows));

    let mut rows = saved();
    rows.sectors[0].index = 0;
    assert_eq!(rows.validate(), Err(Bad::InvalidSectorRows));
}

#[test]
fn a_sector_may_only_point_at_sectors_in_use() {
    let mut rows = saved();
    rows.sectors[0].parent = WORLD_SECTORS as u16 - 1;
    assert_eq!(rows.validate(), Err(Bad::InvalidSectorRows));

    let mut rows = saved();
    rows.sectors[0].axis = 3;
    assert_eq!(rows.validate(), Err(Bad::InvalidSectorRows));
}

#[test]
fn entity_rows_must_be_filed_in_a_sector_in_use() {
    let mut rows = saved();
    rows.entities[0].world_sector = 0;
    assert_eq!(rows.validate(), Err(Bad::InvalidEntityRows));

    let mut rows = saved();
    rows.entities[0].world_sector = u16::MAX;
    assert_eq!(rows.validate(), Err(Bad::InvalidEntityRows));

    let mut rows = saved();
    let twin = rows.entities[0];
    rows.entities.push(twin);
    assert_eq!(rows.validate(), Err(Bad::InvalidEntityRows));

    let mut rows = saved();
    rows.entities[0].linkmin[2] = f32::INFINITY;
    assert_eq!(rows.validate(), Err(Bad::InvalidEntityRows));
}

#[test]
fn chains_that_miss_or_cross_entities_are_refused() {
    // Drop an entity that its sector's chain still reaches.
    let mut rows = saved();
    rows.entities.pop();
    assert_eq!(rows.validate(), Err(Bad::InvalidRetailRows));

    // Cut a chain short, leaving an entity filed but unreachable.
    let mut rows = saved();
    let head = rows.sectors.iter().position(|sector| sector.entities != 0).expect("a sector holds entities");
    rows.sectors[head].entities = 0;
    assert_eq!(rows.validate(), Err(Bad::InvalidRetailRows));
}

#[test]
fn a_snapshot_for_a_tick_starts_with_nothing_published() {
    let snapshot = Snapshot::unpublished(Tick::default());
    assert_eq!(snapshot.tick, Tick::default());
    assert!(snapshot.players.is_empty() && snapshot.projectiles.is_empty());
    assert_eq!(snapshot, snapshot.clone());
}
