//! The client's table of entities: one slot per entity number, each holding the bevy entity that
//! stands for it. The systems here keep the table in step with the snapshot being shown.

mod claims;
mod reconcile;
mod sync;

use bevy::prelude::*;
use playerstate_t6::game::NO_ENTITY_SLOT;
use sim::ClientId;

use crate::identity_slips::LinkIdentityDeficits;
use crate::{ServerStage, ClientStep};

pub(crate) use sync::sync_authority_entities;
pub use sync::mirror_entities;

/// How many entity numbers there are.
pub const ENTITY_NUMBER_COUNT: usize = NO_ENTITY_SLOT as usize;

/// Tags a bevy entity as the table entry for one entity number. The tag is fixed for the entity's
/// life: when the number or the owner changes, the entity is replaced instead.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct ClientEntityRecord {
    slot_no: u16,
    holder: Option<ClientId>,
}

impl ClientEntityRecord {
    pub const fn number(&self) -> u16 {
        self.slot_no
    }

    /// The owning player, when this entity is a player or that player's corpse.
    pub const fn client(&self) -> Option<ClientId> {
        self.holder
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntityTableCell {
    held_entity: Entity,
    holder: Option<ClientId>,
}

/// Maps an entity number to the bevy entity standing for it.
#[derive(Resource, Debug)]
pub struct EntityTableCells {
    cells: Vec<Option<EntityTableCell>>,
}

impl Default for EntityTableCells {
    fn default() -> Self {
        let empty = vec![None; ENTITY_NUMBER_COUNT];
        Self { cells: empty }
    }
}

impl EntityTableCells {
    pub fn object_at_index(&self, slot_no: u16) -> Option<Entity> {
        let held = self.cells.get(slot_no as usize)?.as_ref()?;
        Some(held.held_entity)
    }

    /// The lowest-numbered slot whose owner is `peer`.
    pub fn object_of_receiver(&self, peer: ClientId) -> Option<Entity> {
        self.cells.iter().find_map(|held| match held {
            Some(slot) if slot.holder == Some(peer) => Some(slot.held_entity),
            _ => None,
        })
    }

    /// Occupied cells as `(number, owner, entity)`, lowest number first.
    pub fn each_cell(&self) -> impl Iterator<Item = (u16, Option<ClientId>, Entity)> + '_ {
        self.cells
            .iter()
            .zip(0u16..)
            .filter_map(|(held, at)| held.map(|slot| (at, slot.holder, slot.held_entity)))
    }
}

/// Names the system that filled the table: the authority side or the client side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntitySyncBranch {
    Authority,
    Client,
}

/// Counters for the census dumps: births on each side, and refreshes on the client side only.
#[derive(Resource, Debug, Default, Clone)]
pub struct EntitySpawnTally {
    pub host_spawn_count: u32,
    pub receiver_spawn_count: u32,
    pub applied_by_receiver: u32,
}

impl EntitySpawnTally {
    fn record_spawn(&mut self, arm: EntitySyncBranch) {
        let counter = if arm == EntitySyncBranch::Authority {
            &mut self.host_spawn_count
        } else {
            &mut self.receiver_spawn_count
        };
        *counter = counter.saturating_add(1);
    }

    fn count_refresh(&mut self, arm: EntitySyncBranch) {
        let EntitySyncBranch::Client = arm else {
            return;
        };
        self.applied_by_receiver = self.applied_by_receiver.saturating_add(1);
    }
}

fn init_tables(app: &mut App) {
    app.init_resource::<EntityTableCells>();
    app.init_resource::<LinkIdentityDeficits>();
    app.init_resource::<EntitySpawnTally>();
}

pub(crate) fn enroll_host_objects(app: &mut App) {
    init_tables(app);
    let host_pass = sync_authority_entities.in_set(ServerStage::Fanout);
    app.add_systems(FixedUpdate, host_pass);
}

pub fn enroll_receiver_objects(app: &mut App) {
    init_tables(app);
    let receiver_pass = mirror_entities
        .in_set(ClientStep::Reconcile)
        .after(crate::correct_forecast);
    app.add_systems(Update, receiver_pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a table holding the listed `(number, owner)` pairs, each on a fresh entity.
    fn table_of(pairs: &[(usize, Option<ClientId>)]) -> (EntityTableCells, Vec<Entity>) {
        let mut world = World::new();
        let mut table = EntityTableCells::default();
        let spawned: Vec<Entity> = pairs
            .iter()
            .map(|&(at, who)| {
                let entity = world.spawn_empty().id();
                table.cells[at] = Some(EntityTableCell { held_entity: entity, holder: who });
                entity
            })
            .collect();
        (table, spawned)
    }

    #[test]
    fn an_untouched_table_has_room_for_every_number_and_holds_nothing() {
        let table = EntityTableCells::default();
        assert_eq!(ENTITY_NUMBER_COUNT, table.cells.len());
        assert!(table.each_cell().next().is_none());
        for probe in [0, 1, u16::MAX] {
            assert!(table.object_at_index(probe).is_none());
        }
    }

    #[test]
    fn number_and_owner_lookups_agree_with_what_was_stored() {
        let owner = ClientId(3);
        let (table, made) = table_of(&[(3, Some(owner)), (40, None), (41, Some(owner))]);
        assert_eq!(made[1], table.object_at_index(40).unwrap());
        assert!(table.object_at_index(5).is_none());
        // Two cells share the owner: the lower number is returned.
        assert_eq!(Some(made[0]), table.object_of_receiver(owner));
        assert!(table.object_of_receiver(ClientId(9)).is_none());
    }

    #[test]
    fn iter_walks_occupied_slots_lowest_number_first() {
        let (table, _) = table_of(&[(40, None), (3, Some(ClientId(3)))]);
        let numbers: Vec<u16> = table.each_cell().map(|(number, ..)| number).collect();
        let owners: Vec<Option<ClientId>> = table.each_cell().map(|(_, owner, _)| owner).collect();
        assert_eq!(numbers, vec![3, 40]);
        assert_eq!(owners, vec![Some(ClientId(3)), None]);
    }

    #[test]
    fn births_are_counted_per_side_but_only_client_applies_are_counted() {
        let mut census = EntitySpawnTally::default();
        for arm in [
            EntitySyncBranch::Authority,
            EntitySyncBranch::Client,
            EntitySyncBranch::Client,
        ] {
            census.record_spawn(arm);
        }
        census.count_refresh(EntitySyncBranch::Authority);
        census.count_refresh(EntitySyncBranch::Client);
        assert_eq!(census.host_spawn_count, 1);
        assert_eq!(census.receiver_spawn_count, 2);
        assert_eq!(census.applied_by_receiver, 1);
    }

    #[test]
    fn a_full_counter_saturates() {
        let mut census = EntitySpawnTally::default();
        census.receiver_spawn_count = u32::MAX;
        census.record_spawn(EntitySyncBranch::Client);
        assert_eq!(u32::MAX, census.receiver_spawn_count);
    }
}
