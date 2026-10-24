//! The kernel as plain data: what gets saved, sent to a joining client, and read back.

use super::refs::{EntityRef, EntityRefError, EntityRelations};
use super::{EntityRunKind, GENTITY_RESERVED_COUNT};
use trace_t6::ENTITYNUM_WORLD;

/// A held slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityKernelOccupiedSnapshot {
    pub kind: EntityRunKind,
    pub linked: bool,
    pub relations: EntityRelations,
    pub next_think_ms: Option<i32>,
    pub transient_event_time_ms: Option<i32>,
}

/// One number's record: held or free, and the stamp that tells handles from different lives apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityKernelSlotSnapshot {
    pub generation: u32,
    pub occupied: Option<EntityKernelOccupiedSnapshot>,
    pub freed_at_ms: i32,
}

impl EntityKernelSlotSnapshot {
    /// A number nobody has held yet.
    pub(super) const UNUSED: Self = Self { generation: 0, occupied: None, freed_at_ms: i32::MIN };
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityKernelSnapshot {
    /// One record per number from the first handed-out one up to `high_water`.
    pub slots: Vec<EntityKernelSlotSnapshot>,
    pub high_water: i32,
    /// Freed numbers, oldest first: the order they will be reused in.
    pub free_fifo: Vec<i32>,
    pub level_time_ms: i32,
    pub frame_serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKernelSnapshotError {
    HighWaterOutOfRange,
    SlotCountMismatch,
    FreeFifoInvalid,
    RelationInvalid,
}

impl Default for EntityKernelSnapshot {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            high_water: GENTITY_RESERVED_COUNT,
            free_fifo: Vec::new(),
            level_time_ms: 0,
            frame_serial: 0,
        }
    }
}

impl EntityKernelSnapshot {
    /// Where `number` sits in `slots`, if it is a number this snapshot covers.
    fn position(&self, number: i32) -> Option<usize> {
        (GENTITY_RESERVED_COUNT..self.high_water)
            .contains(&number)
            .then(|| (number - GENTITY_RESERVED_COUNT) as usize)
    }

    pub fn current_ref(&self, number: i32) -> Result<EntityRef, EntityRefError> {
        let at = self.position(number).ok_or(EntityRefError::ReservedOrOutOfRange)?;
        let slot = self.slots.get(at).ok_or(EntityRefError::SnapshotMalformed)?;
        if slot.occupied.is_none() {
            return Err(EntityRefError::Free);
        }
        Ok(EntityRef::new(number, slot.generation))
    }

    pub fn occupied_kind(&self, number: i32) -> Option<EntityRunKind> {
        let slot = self.slots.get(self.position(number)?)?;
        slot.occupied.map(|held| held.kind)
    }

    /// Checks the snapshot is one a kernel could really have produced: sizes agree, the free list names exactly the
    /// free slots, and every link points at a live entity of the right generation.
    pub fn validate(&self) -> Result<(), EntityKernelSnapshotError> {
        use EntityKernelSnapshotError as Bad;

        if !(GENTITY_RESERVED_COUNT..=i32::from(ENTITYNUM_WORLD)).contains(&self.high_water) {
            return Err(Bad::HighWaterOutOfRange);
        }
        if self.slots.len() != (self.high_water - GENTITY_RESERVED_COUNT) as usize {
            return Err(Bad::SlotCountMismatch);
        }

        let mut queued = vec![false; self.slots.len()];
        for &number in &self.free_fifo {
            let at = self.position(number).ok_or(Bad::FreeFifoInvalid)?;
            if queued[at] || self.slots[at].occupied.is_some() {
                return Err(Bad::FreeFifoInvalid);
            }
            queued[at] = true;
        }

        for (at, slot) in self.slots.iter().enumerate() {
            if slot.occupied.is_none() != queued[at] {
                return Err(Bad::FreeFifoInvalid);
            }
            let Some(held) = &slot.occupied else {
                continue;
            };
            for link in held.relations.links().into_iter().flatten() {
                let target = self.position(link.number()).and_then(|i| self.slots.get(i));
                let live = target.is_some_and(|t| t.generation == link.generation() && t.occupied.is_some());
                if !live {
                    return Err(Bad::RelationInvalid);
                }
            }
        }
        Ok(())
    }
}
