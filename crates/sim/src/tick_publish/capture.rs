//! Saving the live area sectors as rows.

use clipmap_t6::WORLD_SECTORS;

use super::area::{ZoneLinkRow, ZoneThingTable, ZoneCellRow};
use crate::sector_links::AreaWorld;

/// The sectors on the free list, head first. Panics if the list loops or leaves the table.
fn free_list(area: &AreaWorld) -> Vec<u16> {
    let mut listed = Vec::new();
    let mut seen = vec![false; WORLD_SECTORS];
    let mut at = area.world.vacant_list_front;
    while at != 0 {
        let slot = usize::from(at);
        assert!(slot < WORLD_SECTORS && !seen[slot], "area free chain");
        seen[slot] = true;
        listed.push(at);
        at = area.world.sectors[slot].parent;
    }
    listed
}

/// Where the unbroken run `n, n + 1, .., last sector` that closes `chain` starts, if it has one.
fn closing_run(chain: &[u16]) -> Option<usize> {
    (0..chain.len()).find(|&start| {
        let first = usize::from(chain[start]);
        chain[start..].iter().map(|&sector| usize::from(sector)).eq(first..WORLD_SECTORS)
    })
}

impl ZoneThingTable {
    pub(crate) fn capture(area: &AreaWorld) -> Self {
        let world = &area.world;
        let chain = free_list(area);
        let (before_run, run_head) = match closing_run(&chain) {
            Some(start) => (chain[..start].to_vec(), chain[start]),
            None => (chain.clone(), 0),
        };
        let mut is_free = vec![false; WORLD_SECTORS];
        for &sector in &chain {
            is_free[usize::from(sector)] = true;
        }

        let sector_rows = world
            .sectors
            .iter()
            .enumerate()
            .skip(1)
            .filter(|&(at, _)| !is_free[at])
            .map(|(at, sector)| ZoneCellRow {
                index: at as u16,
                contents_entities: sector.solid_ents,
                entities: sector.entities,
                dist: sector.dist,
                axis: sector.axis,
                parent: sector.parent,
                child: sector.children,
            })
            .collect();
        let entity_rows = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, filed)| filed.world_sector != 0)
            .map(|(num, filed)| ZoneLinkRow {
                entity_num: num as u16,
                world_sector: filed.world_sector,
                next_entity: filed.following_sector_entity,
                linkcontents: filed.link_contents,
                linkmin: filed.link_min,
                linkmax: filed.link_max,
                contents: area.entities[num].contents,
            })
            .collect();
        Self {
            world_mins: world.mins,
            world_maxs: world.maxs,
            free_prefix: before_run,
            contiguous_free_head: run_head,
            sectors: sector_rows,
            entities: entity_rows,
        }
    }
}
