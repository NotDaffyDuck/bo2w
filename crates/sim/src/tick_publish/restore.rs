//! Rebuilding the live area sectors from saved rows, refusing rows that do not fit together.

use clipmap_t6::{LINKED_ENTITIES, SvEntity, WORLD_SECTORS, WorldSector};

use super::area::{ZoneThingTable, RegionSnapshotFault};
use crate::sector_links::{AreaBox, AreaEntity, AreaWorld};

impl ZoneThingTable {
    /// The free list the rows describe, head first: the listed part, then the closing run.
    fn free_list(&self) -> Result<Vec<u16>, RegionSnapshotFault> {
        let mut chain = self.free_prefix.clone();
        let run_head = self.contiguous_free_head;
        if run_head != 0 {
            let first = usize::from(run_head);
            if !(2..WORLD_SECTORS).contains(&first) {
                return Err(RegionSnapshotFault::FreeListBroken);
            }
            chain.extend((first..WORLD_SECTORS).map(|sector| sector as u16));
        }
        Ok(chain)
    }

    /// Fills the sector table from the saved sector list. Returns which sectors are in use.
    fn fill_sectors(&self, area: &mut AreaWorld, is_free: &[bool]) -> Result<Vec<bool>, RegionSnapshotFault> {
        let mut in_use = vec![false; WORLD_SECTORS];
        for entry in &self.sectors {
            let slot = usize::from(entry.index);
            if slot == 0 || slot >= WORLD_SECTORS || is_free[slot] || in_use[slot] {
                return Err(RegionSnapshotFault::BadSectorTree);
            }
            in_use[slot] = true;
        }
        // Every sector but the root is either free or in use, never both and never neither.
        if (1..WORLD_SECTORS).any(|slot| is_free[slot] == in_use[slot]) {
            return Err(RegionSnapshotFault::BadSectorTree);
        }
        let connects = |sector: u16| sector == 0 || in_use[usize::from(sector)];
        for entry in &self.sectors {
            let sane = usize::from(entry.entities) <= LINKED_ENTITIES && entry.axis < 3;
            if !sane || !connects(entry.parent) || !entry.child.into_iter().all(connects) {
                return Err(RegionSnapshotFault::BadSectorTree);
            }
            area.world.sectors[usize::from(entry.index)] = WorldSector {
                contents_static_models: 0,
                solid_ents: entry.contents_entities,
                entities: entry.entities,
                prop_set: 0,
                dist: entry.dist,
                axis: entry.axis,
                parent: entry.parent,
                children: entry.child,
            };
        }
        Ok(in_use)
    }

    /// Files the saved entity list into their sectors. Returns which entity numbers are linked.
    fn fill_entities(&self, area: &mut AreaWorld, in_use: &[bool]) -> Result<Vec<bool>, RegionSnapshotFault> {
        let mut tied = vec![false; LINKED_ENTITIES];
        for rec in &self.entities {
            let num = usize::from(rec.entity_num);
            let sector = usize::from(rec.world_sector);
            let bounds = AreaBox { mins: rec.linkmin, maxs: rec.linkmax };
            let misfiled = num >= LINKED_ENTITIES
                || tied[num]
                || sector == 0
                || sector >= WORLD_SECTORS
                || !in_use[sector]
                || usize::from(rec.next_entity) > LINKED_ENTITIES
                || !bounds.is_valid();
            if misfiled {
                return Err(RegionSnapshotFault::BadEntityLinks);
            }
            tied[num] = true;
            area.world.entities[num] = SvEntity {
                world_sector: rec.world_sector,
                following_sector_entity: rec.next_entity,
                link_contents: rec.linkcontents,
                link_min: rec.linkmin,
                link_max: rec.linkmax,
            };
            area.entities[num] = AreaEntity { contents: rec.contents, absmin: rec.linkmin, absmax: rec.linkmax };
        }
        Ok(tied)
    }

    /// Walks every sector's entity chain: each must reach only entities filed in that sector, none twice, and together
    /// they must reach every entity listed.
    fn check_chains(&self, area: &AreaWorld, linked: &[bool]) -> Result<(), RegionSnapshotFault> {
        let mut visited = vec![false; LINKED_ENTITIES];
        let mut reached = 0;
        for sector in &self.sectors {
            let mut link = sector.entities;
            while link != 0 {
                let num = usize::from(link) - 1;
                let filed_here = usize::from(area.world.entities[num].world_sector) == usize::from(sector.index);
                if !linked[num] || visited[num] || !filed_here {
                    return Err(RegionSnapshotFault::SectorChainsBroken);
                }
                visited[num] = true;
                reached += 1;
                link = area.world.entities[num].following_sector_entity;
            }
        }
        if reached == self.entities.len() { Ok(()) } else { Err(RegionSnapshotFault::SectorChainsBroken) }
    }

    pub(crate) fn restore(&self) -> Result<AreaWorld, RegionSnapshotFault> {
        let bounds = AreaBox { mins: self.world_mins, maxs: self.world_maxs };
        if !bounds.is_valid() {
            return Err(RegionSnapshotFault::BadBounds);
        }
        let chain = self.free_list()?;
        let mut is_free = vec![false; WORLD_SECTORS];
        for &sector in &chain {
            let slot = usize::from(sector);
            if !(2..WORLD_SECTORS).contains(&slot) || is_free[slot] {
                return Err(RegionSnapshotFault::FreeListBroken);
            }
            is_free[slot] = true;
        }

        let mut area = AreaWorld::new(bounds);
        area.world.vacant_list_front = chain.first().copied().unwrap_or(0);
        area.world.sectors = [WorldSector::default(); WORLD_SECTORS];
        for (place, &sector) in chain.iter().enumerate() {
            area.world.sectors[usize::from(sector)].parent = chain.get(place + 1).copied().unwrap_or(0);
        }

        let in_use = self.fill_sectors(&mut area, &is_free)?;
        let linked = self.fill_entities(&mut area, &in_use)?;
        self.check_chains(&area, &linked)?;
        Ok(area)
    }
}
