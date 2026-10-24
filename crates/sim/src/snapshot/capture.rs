//! Saving the live area sectors as rows.

use clipmap_t6::WORLD_SECTORS;

use super::area::{AreaEntityLinkSnapshot, AreaEntityWorldSnapshot, AreaSectorSnapshot};
use crate::area_world::AreaWorld;

/// The sectors on the free list, head first. Panics if the list loops or leaves the table.
fn free_list(area: &AreaWorld) -> Vec<u16> {
    let mut listed = Vec::new();
    let mut seen = vec![false; WORLD_SECTORS];
    let mut at = area.world.free_head;
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

impl AreaEntityWorldSnapshot {
    pub(crate) fn capture(area: &AreaWorld) -> Self {
        let world = &area.world;
        let chain = free_list(area);
        let (free_prefix, contiguous_free_head) = match closing_run(&chain) {
            Some(start) => (chain[..start].to_vec(), chain[start]),
            None => (chain.clone(), 0),
        };
        let mut is_free = vec![false; WORLD_SECTORS];
        for &sector in &chain {
            is_free[usize::from(sector)] = true;
        }

        let mut sectors = Vec::new();
        for (index, sector) in world.sectors.iter().enumerate().skip(1) {
            if is_free[index] {
                continue;
            }
            sectors.push(AreaSectorSnapshot {
                index: index as u16,
                contents_entities: sector.contents_entities,
                entities: sector.entities,
                dist: sector.dist,
                axis: sector.axis,
                parent: sector.parent,
                child: sector.children,
            });
        }
        let mut entities = Vec::new();
        for (entity_num, link) in world.entities.iter().enumerate() {
            if link.world_sector == 0 {
                continue;
            }
            entities.push(AreaEntityLinkSnapshot {
                entity_num: entity_num as u16,
                world_sector: link.world_sector,
                next_entity: link.next_entity_in_world_sector,
                linkcontents: link.link_contents,
                linkmin: link.link_min,
                linkmax: link.link_max,
                contents: area.entities[entity_num].contents,
            });
        }
        Self {
            world_mins: world.mins,
            world_maxs: world.maxs,
            free_prefix,
            contiguous_free_head,
            sectors,
            entities,
        }
    }
}
