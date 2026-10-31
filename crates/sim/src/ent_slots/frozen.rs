//! The kernel as plain data: what gets saved, sent to a joining client, and read back.

use super::refs::{EntHandle, EntityHandleFault, ThingTies};
use super::{EntityBehaviorClass, PROTECTED_ENTITY_SLOTS};
use trace_t6::WORLD_ENTITY_SLOT;

/// A held slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BayHeldRow {
    pub kind: EntityBehaviorClass, pub linked: bool, pub relations: ThingTies,
    pub next_think_ms: Option<i32>, pub transient_event_time_ms: Option<i32>,
}
/// Older spelling EntityKernelOccupiedSnapshot still calls; the rename sweep moves it over.
pub use self::BayHeldRow as EntityKernelOccupiedSnapshot;

/// One number's record: held or free, and the stamp that tells handles from different lives apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BayFrozenRow {
    pub generation: u32, pub occupied: Option<BayHeldRow>, pub freed_at_ms: i32,
}
/// Older spelling EntityKernelSlotSnapshot still calls; the rename sweep moves it over.
pub use self::BayFrozenRow as EntityKernelSlotSnapshot;

impl BayFrozenRow {
    /// A number nobody has held yet.
    pub(super) const UNUSED: Self = Self { generation: 0, occupied: None, freed_at_ms: i32::MIN };
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrozenSlotTable {
    /// One record per number from the first handed-out one up to `high_water`.
    pub slots: Vec<BayFrozenRow>, pub high_water: i32,
    /// Freed numbers, oldest first: the order they will be reused in.
    pub free_fifo: Vec<i32>, pub level_time_ms: i32, pub frame_serial: u32,
}

/// Older spelling EntityKernelSnapshot still calls; the rename sweep moves it over.
pub use self::FrozenSlotTable as EntityKernelSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityTableSnapshotFault {
    TopMarkOffTable,
    WrongSlotCount,
    FreeQueueBroken,
    BadLink,
}

impl Default for FrozenSlotTable {
    fn default() -> Self {
        let peak_slot = PROTECTED_ENTITY_SLOTS;
        Self {
            high_water: peak_slot,
            slots: Default::default(),
            free_fifo: Default::default(),
            level_time_ms: Default::default(),
            frame_serial: Default::default(),
        }
    }
}

impl FrozenSlotTable {
    /// Where `number` sits in `slots`, if it is a number this snapshot covers.
    fn position(&self, number: i32) -> Option<usize> {
        (PROTECTED_ENTITY_SLOTS..self.high_water)
            .contains(&number)
            .then(|| (number - PROTECTED_ENTITY_SLOTS) as usize)
    }

    pub fn ref_for(&self, number: i32) -> Result<EntHandle, EntityHandleFault> {
        let at = self.position(number).ok_or(EntityHandleFault::NumberNotUsable)?;
        let slot = self.slots.get(at).ok_or(EntityHandleFault::BadSnapshot)?;
        match slot.occupied {
            None => Err(EntityHandleFault::Free),
            Some(_) => Ok(EntHandle::new(number, slot.generation)),
        }
    }

    pub fn holder_class(&self, number: i32) -> Option<EntityBehaviorClass> {
        let index = self.position(number)?;
        self.slots.get(index)?.occupied.map(|held| held.kind)
    }

    /// Checks the snapshot is one a kernel could really have produced: sizes agree, the free list names exactly the
    /// free slots, and every link points at a live entity of the right generation.
    pub fn validate(&self) -> Result<(), EntityTableSnapshotFault> {

        let allowed_marks = PROTECTED_ENTITY_SLOTS..=i32::from(WORLD_ENTITY_SLOT);
        if !allowed_marks.contains(&self.high_water) {
            return Err(EntityTableSnapshotFault::TopMarkOffTable);
        }
        let expected_slots = (self.high_water - PROTECTED_ENTITY_SLOTS) as usize;
        if self.slots.len() != expected_slots {
            return Err(EntityTableSnapshotFault::WrongSlotCount);
        }

        let mut queued = vec![false; self.slots.len()];
        for number in self.free_fifo.iter().copied() {
            let at = self.position(number).ok_or(EntityTableSnapshotFault::FreeQueueBroken)?;
            let reusable = !queued[at] && self.slots[at].occupied.is_none();
            if !reusable {
                return Err(EntityTableSnapshotFault::FreeQueueBroken);
            }
            queued[at] = true;
        }

        for (at, cell) in self.slots.iter().enumerate() {
            if cell.occupied.is_none() != queued[at] {
                return Err(EntityTableSnapshotFault::FreeQueueBroken);
            }
            let Some(held) = &cell.occupied else {
                continue;
            };
            for link in held.relations.links().into_iter().flatten() {
                let target = self.position(link.slot()).and_then(|i| self.slots.get(i));
                let live = target.is_some_and(|t| t.generation == link.generation() && t.occupied.is_some());
                if !live {
                    return Err(EntityTableSnapshotFault::BadLink);
                }
            }
        }
        Ok(())
    }
}
