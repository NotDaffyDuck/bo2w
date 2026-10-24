//! Rebuilding the live area sectors from saved rows, refusing rows that do not fit together.

use clipmap_t6::{LINKED_ENTITIES, SvEntity, WORLD_SECTORS, WorldSector};

use super::area::{AreaEntityWorldSnapshot, AreaEntityWorldSnapshotError as Bad};
use crate::area_world::{AreaBox, AreaEntity, AreaWorld};

impl AreaEntityWorldSnapshot {
    /// The free list the rows describe, head first: the listed part, then the closing run.
    fn free_list(&self) -> Result<Vec<u16>, Bad> {
        let mut chain = self.free_prefix.clone();
        if self.contiguous_free_head != 0 {
            let first = usize::from(self.contiguous_free_head);
            if !(2..WORLD_SECTORS).contains(&first) {
                return Err(Bad::InvalidFreeChain);
            }
            chain.extend((first..WORLD_SECTORS).map(|sector| sector as u16));
        }
        Ok(chain)
    }

    /// Fills the sector table from the sector rows. Returns which sectors are in use.
    fn fill_sectors(&self, area: &mut AreaWorld, is_free: &[bool]) -> Result<Vec<bool>, Bad> {
        let mut in_use = vec![false; WORLD_SECTORS];
        for row in &self.sectors {
            let slot = usize::from(row.index);
            if slot == 0 || slot >= WORLD_SECTORS || is_free[slot] || in_use[slot] {
                return Err(Bad::InvalidSectorRows);
            }
            in_use[slot] = true;
        }
        // Every sector but the root is either free or in use, never both and never neither.
        if (1..WORLD_SECTORS).any(|slot| is_free[slot] == in_use[slot]) {
            return Err(Bad::InvalidSectorRows);
        }
        let connects = |sector: u16| sector == 0 || in_use[usize::from(sector)];
        for row in &self.sectors {
            let sane = usize::from(row.entities) <= LINKED_ENTITIES && row.axis < 3;
            if !sane || !connects(row.parent) || !row.child.into_iter().all(connects) {
                return Err(Bad::InvalidSectorRows);
            }
            area.world.sectors[usize::from(row.index)] = WorldSector {
                contents_static_models: 0,
                contents_entities: row.contents_entities,
                entities: row.entities,
                static_models: 0,
                dist: row.dist,
                axis: row.axis,
                parent: row.parent,
                children: row.child,
            };
        }
        Ok(in_use)
    }

    /// Files the entity rows into their sectors. Returns which entity numbers are linked.
    fn fill_entities(&self, area: &mut AreaWorld, in_use: &[bool]) -> Result<Vec<bool>, Bad> {
        let mut linked = vec![false; LINKED_ENTITIES];
        for row in &self.entities {
            let num = usize::from(row.entity_num);
            let sector = usize::from(row.world_sector);
            let bounds = AreaBox { mins: row.linkmin, maxs: row.linkmax };
            let misfiled = num >= LINKED_ENTITIES
                || linked[num]
                || sector == 0
                || sector >= WORLD_SECTORS
                || !in_use[sector]
                || usize::from(row.next_entity) > LINKED_ENTITIES
                || !bounds.is_valid();
            if misfiled {
                return Err(Bad::InvalidEntityRows);
            }
            linked[num] = true;
            area.world.entities[num] = SvEntity {
                world_sector: row.world_sector,
                next_entity_in_world_sector: row.next_entity,
                link_contents: row.linkcontents,
                link_min: row.linkmin,
                link_max: row.linkmax,
            };
            area.entities[num] = AreaEntity { contents: row.contents, absmin: row.linkmin, absmax: row.linkmax };
        }
        Ok(linked)
    }

    /// Walks every sector's entity chain: each must reach only entities filed in that sector, none twice, and together
    /// they must reach every entity listed.
    fn check_chains(&self, area: &AreaWorld, linked: &[bool]) -> Result<(), Bad> {
        let mut visited = vec![false; LINKED_ENTITIES];
        let mut reached = 0;
        for sector in &self.sectors {
            let mut link = sector.entities;
            while link != 0 {
                let num = usize::from(link) - 1;
                let filed_here = usize::from(area.world.entities[num].world_sector) == usize::from(sector.index);
                if !linked[num] || visited[num] || !filed_here {
                    return Err(Bad::InvalidRetailRows);
                }
                visited[num] = true;
                reached += 1;
                link = area.world.entities[num].next_entity_in_world_sector;
            }
        }
        if reached == self.entities.len() { Ok(()) } else { Err(Bad::InvalidRetailRows) }
    }

    pub(crate) fn restore(&self) -> Result<AreaWorld, Bad> {
        let bounds = AreaBox { mins: self.world_mins, maxs: self.world_maxs };
        if !bounds.is_valid() {
            return Err(Bad::InvalidWorldBounds);
        }
        let chain = self.free_list()?;
        let mut is_free = vec![false; WORLD_SECTORS];
        for &sector in &chain {
            let slot = usize::from(sector);
            if !(2..WORLD_SECTORS).contains(&slot) || is_free[slot] {
                return Err(Bad::InvalidFreeChain);
            }
            is_free[slot] = true;
        }

        let mut area = AreaWorld::new(bounds);
        area.world.free_head = chain.first().copied().unwrap_or(0);
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
