use playerstate_t6::game::PlayerState;

use crate::ProjectileState;
use crate::match_state::SnapshotMeta;
use crate::world::{ClientId, Tick};
use crate::area_world::{AreaBox, AreaEntity, AreaWorld};
use clipmap_t6::{LINKED_ENTITIES, SvEntity, WORLD_SECTORS, WorldSector};

/// bo2mp: a live sector of BO2's `cm_world_t` (`worldSector_s`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaSectorSnapshot {
    pub index: u16,
    pub contents_entities: u32,
    pub entities: u16,
    pub dist: f32,
    pub axis: u16,
    pub parent: u16,
    pub child: [u16; 2],
}

/// bo2mp: a linked entity: its `svEntity_s` and its `r.contents`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaEntityLinkSnapshot {
    pub entity_num: u16,
    pub world_sector: u16,
    pub next_entity: u16,
    pub linkcontents: u32,
    pub linkmin: [f32; 3],
    pub linkmax: [f32; 3],
    pub contents: u32,
}

/// bo2mp: BO2's area entity sectors: the world's box, the free chain (a
/// listed head, then a run up to the last sector), the live sectors and
/// the linked entities.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaEntityWorldSnapshot {
    pub world_mins: [f32; 3],
    pub world_maxs: [f32; 3],

    pub free_prefix: Vec<u16>,

    pub contiguous_free_head: u16,
    pub sectors: Vec<AreaSectorSnapshot>,
    pub entities: Vec<AreaEntityLinkSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaEntityWorldSnapshotError {
    InvalidWorldBounds,
    InvalidFreeChain,
    InvalidSectorRows,
    InvalidEntityRows,
    InvalidRetailRows,
}

impl AreaEntityWorldSnapshot {
    pub(crate) fn capture(area: &AreaWorld) -> Self {
        let world = &area.world;
        let mut free_order = Vec::new();
        let mut free = vec![false; WORLD_SECTORS];
        let mut cursor = world.free_head;
        while cursor != 0 {
            let index = usize::from(cursor);
            assert!(index < free.len() && !free[index], "area free chain");
            free[index] = true;
            free_order.push(cursor);
            cursor = world.sectors[index].parent;
        }

        let tail_at = free_order.iter().enumerate().find_map(|(offset, &first)| {
            let first = usize::from(first);
            let tail = &free_order[offset..];
            (tail.len() == WORLD_SECTORS.saturating_sub(first)
                && tail
                    .iter()
                    .enumerate()
                    .all(|(delta, &value)| usize::from(value) == first + delta))
            .then_some(offset)
        });
        let (free_prefix, contiguous_free_head) = tail_at.map_or_else(
            || (free_order.clone(), 0),
            |offset| (free_order[..offset].to_vec(), free_order[offset]),
        );

        let sectors = world
            .sectors
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(index, _)| !free[*index])
            .map(|(index, row)| AreaSectorSnapshot {
                index: index as u16,
                contents_entities: row.contents_entities,
                entities: row.entities,
                dist: row.dist,
                axis: row.axis,
                parent: row.parent,
                child: row.children,
            })
            .collect();
        let entities = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, link)| link.world_sector != 0)
            .map(|(entity_num, link)| AreaEntityLinkSnapshot {
                entity_num: entity_num as u16,
                world_sector: link.world_sector,
                next_entity: link.next_entity_in_world_sector,
                linkcontents: link.link_contents,
                linkmin: link.link_min,
                linkmax: link.link_max,
                contents: area.entities[entity_num].contents,
            })
            .collect();
        Self {
            world_mins: world.mins,
            world_maxs: world.maxs,
            free_prefix,
            contiguous_free_head,
            sectors,
            entities,
        }
    }

    pub fn validate(&self) -> Result<(), AreaEntityWorldSnapshotError> {
        self.restore().map(|_| ())
    }

    pub(crate) fn restore(&self) -> Result<AreaWorld, AreaEntityWorldSnapshotError> {
        let bounds = AreaBox {
            mins: self.world_mins,
            maxs: self.world_maxs,
        };
        if !bounds.is_valid() {
            return Err(AreaEntityWorldSnapshotError::InvalidWorldBounds);
        }
        let count = WORLD_SECTORS;
        let mut free_order = self.free_prefix.clone();
        if self.contiguous_free_head != 0 {
            let first = usize::from(self.contiguous_free_head);
            if !(2..count).contains(&first) {
                return Err(AreaEntityWorldSnapshotError::InvalidFreeChain);
            }
            free_order.extend((first..count).map(|index| index as u16));
        }
        let mut free = vec![false; count];
        for &index in &free_order {
            let index = usize::from(index);
            if !(2..count).contains(&index) || free[index] {
                return Err(AreaEntityWorldSnapshotError::InvalidFreeChain);
            }
            free[index] = true;
        }

        let mut area = AreaWorld::new(bounds);
        let world = &mut area.world;
        world.free_head = free_order.first().copied().unwrap_or(0);
        world.sectors = [WorldSector::default(); WORLD_SECTORS];
        for (offset, &index) in free_order.iter().enumerate() {
            world.sectors[usize::from(index)].parent = free_order.get(offset + 1).copied().unwrap_or(0);
        }
        let mut live = vec![false; count];
        for row in &self.sectors {
            let index = usize::from(row.index);
            if index == 0 || index >= count || free[index] || live[index] {
                return Err(AreaEntityWorldSnapshotError::InvalidSectorRows);
            }
            live[index] = true;
        }
        if (1..count).any(|index| free[index] == live[index]) {
            return Err(AreaEntityWorldSnapshotError::InvalidSectorRows);
        }
        let linked = |index: u16| index == 0 || live[usize::from(index)];
        for row in &self.sectors {
            let in_range = usize::from(row.entities) <= LINKED_ENTITIES && row.axis < 3;
            if !in_range || !linked(row.parent) || !row.child.into_iter().all(linked) {
                return Err(AreaEntityWorldSnapshotError::InvalidSectorRows);
            }
            world.sectors[usize::from(row.index)] = WorldSector {
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

        let mut occupied = vec![false; LINKED_ENTITIES];
        for row in &self.entities {
            let index = usize::from(row.entity_num);
            let sector = usize::from(row.world_sector);
            let link = AreaBox {
                mins: row.linkmin,
                maxs: row.linkmax,
            };
            if index >= LINKED_ENTITIES
                || occupied[index]
                || sector == 0
                || !live[sector]
                || usize::from(row.next_entity) > LINKED_ENTITIES
                || !link.is_valid()
            {
                return Err(AreaEntityWorldSnapshotError::InvalidEntityRows);
            }
            occupied[index] = true;
            area.world.entities[index] = SvEntity {
                world_sector: row.world_sector,
                next_entity_in_world_sector: row.next_entity,
                link_contents: row.linkcontents,
                link_min: row.linkmin,
                link_max: row.linkmax,
            };
            area.entities[index] = AreaEntity {
                contents: row.contents,
                absmin: row.linkmin,
                absmax: row.linkmax,
            };
        }

        // Each sector's chain holds exactly the entities filed in it.
        let world = &area.world;
        let mut seen = vec![false; LINKED_ENTITIES];
        let mut chained = 0;
        for row in &self.sectors {
            let mut e = row.entities;
            while e != 0 {
                let i = usize::from(e) - 1;
                if !occupied[i] || seen[i] || usize::from(world.entities[i].world_sector) != usize::from(row.index) {
                    return Err(AreaEntityWorldSnapshotError::InvalidRetailRows);
                }
                seen[i] = true;
                chained += 1;
                e = world.entities[i].next_entity_in_world_sector;
            }
        }
        if chained != self.entities.len() {
            return Err(AreaEntityWorldSnapshotError::InvalidRetailRows);
        }
        Ok(area)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tick: Tick,
    pub players: Vec<(ClientId, PlayerState)>,
    pub projectiles: Vec<ProjectileState>,

    pub meta: SnapshotMeta,
}

impl Snapshot {
    pub fn unpublished(tick: Tick) -> Self {
        Self {
            tick,
            players: Vec::new(),
            projectiles: Vec::new(),
            meta: SnapshotMeta::default(),
        }
    }
}
