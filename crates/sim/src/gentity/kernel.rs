//! The entity-number table itself.

use std::collections::VecDeque;

use trace_t6::ENTITYNUM_WORLD;

use super::refs::{EntityAllocError, EntityRef, EntityRefError, EntityRelations, EntitySlotView, ThinkEnter};
use super::snapshot::{
    EntityKernelOccupiedSnapshot as Held, EntityKernelSlotSnapshot as Slot, EntityKernelSnapshot,
    EntityKernelSnapshotError,
};
use super::{
    EntityRunKind, GENTITY_RESERVED_COUNT as FIRST, GENTITY_REUSE_QUARANTINE_MS, GENTITY_TEMP_EVENT_LIFETIME_MS,
};

/// One past the last number the kernel can hand out.
const LIMIT: i32 = ENTITYNUM_WORLD as i32;

/// What every number above the high-water mark looks like: never used.
static UNUSED: Slot = Slot::UNUSED;

#[derive(Clone, Debug, PartialEq)]
pub struct EntityKernel {
    /// One record per number from `FIRST` up to `high_water`.
    slots: Vec<Slot>,
    /// The frame each slot last thought in, in step with `slots`. Not part of a snapshot.
    thought_in: Vec<u32>,
    high_water: i32,
    free_fifo: VecDeque<i32>,
    level_time_ms: i32,
    frame_serial: u32,
}

impl Default for EntityKernel {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            thought_in: Vec::new(),
            high_water: FIRST,
            free_fifo: VecDeque::new(),
            level_time_ms: 0,
            frame_serial: 0,
        }
    }
}

/// Where `number` sits in the slot list. Numbers past the high-water mark are allowed here; they just have no record.
fn position(number: i32) -> Result<usize, EntityRefError> {
    if (FIRST..LIMIT).contains(&number) {
        Ok((number - FIRST) as usize)
    } else {
        Err(EntityRefError::ReservedOrOutOfRange)
    }
}

impl EntityKernel {
    pub fn to_snapshot(&self) -> EntityKernelSnapshot {
        EntityKernelSnapshot {
            slots: self.slots.clone(),
            high_water: self.high_water,
            free_fifo: self.free_fifo.iter().copied().collect(),
            level_time_ms: self.level_time_ms,
            frame_serial: self.frame_serial,
        }
    }

    pub fn from_snapshot(snapshot: &EntityKernelSnapshot) -> Result<Self, EntityKernelSnapshotError> {
        snapshot.validate()?;
        Ok(Self {
            slots: snapshot.slots.clone(),
            thought_in: vec![0; snapshot.slots.len()],
            high_water: snapshot.high_water,
            free_fifo: snapshot.free_fifo.iter().copied().collect(),
            level_time_ms: snapshot.level_time_ms,
            frame_serial: snapshot.frame_serial,
        })
    }

    /// Starts a frame: the clock moves to `level_time_ms` and every entity may think once again.
    pub fn begin_frame(&mut self, level_time_ms: i32) {
        self.level_time_ms = level_time_ms;
        self.frame_serial = self.frame_serial.wrapping_add(1);
    }

    pub const fn level_time_ms(&self) -> i32 {
        self.level_time_ms
    }

    pub const fn frame_serial(&self) -> u32 {
        self.frame_serial
    }

    pub const fn high_water(&self) -> i32 {
        self.high_water
    }

    fn slot(&self, number: i32) -> Result<&Slot, EntityRefError> {
        Ok(self.slots.get(position(number)?).unwrap_or(&UNUSED))
    }

    /// The held part of the slot `entity` points at, if the handle is current.
    fn held(&self, entity: EntityRef) -> Result<&Held, EntityRefError> {
        let slot = self.slot(entity.number())?;
        if slot.generation != entity.generation() {
            return Err(EntityRefError::StaleGeneration { current: slot.generation });
        }
        slot.occupied.as_ref().ok_or(EntityRefError::Free)
    }

    fn held_mut(&mut self, entity: EntityRef) -> Result<&mut Held, EntityRefError> {
        self.held(entity)?;
        let at = position(entity.number())?;
        self.slots[at].occupied.as_mut().ok_or(EntityRefError::Free)
    }

    pub fn occupied_kind(&self, number: i32) -> Option<EntityRunKind> {
        self.slot(number).ok()?.occupied.map(|held| held.kind)
    }

    /// Every held number whose kind `wanted` accepts, lowest first.
    pub fn occupied_numbers(&self, mut wanted: impl FnMut(EntityRunKind) -> bool) -> Vec<i32> {
        (FIRST..self.high_water).filter(|&number| self.occupied_kind(number).is_some_and(&mut wanted)).collect()
    }

    /// The oldest freed number, if it has sat out its quarantine (or the table is full, so waiting would not help).
    fn reusable(&self) -> Option<i32> {
        let &oldest = self.free_fifo.front()?;
        let freed_at = self.slots[(oldest - FIRST) as usize].freed_at_ms;
        let rested = self.level_time_ms.saturating_sub(freed_at) >= GENTITY_REUSE_QUARANTINE_MS;
        (rested || self.high_water >= LIMIT).then_some(oldest)
    }

    pub fn allocate(&mut self, kind: EntityRunKind) -> Result<EntityRef, EntityAllocError> {
        let number = if let Some(number) = self.reusable() {
            self.free_fifo.pop_front();
            number
        } else if self.high_water < LIMIT {
            self.slots.push(UNUSED);
            self.thought_in.push(0);
            self.high_water += 1;
            self.high_water - 1
        } else {
            return Err(EntityAllocError::CapacityExhausted);
        };
        let at = (number - FIRST) as usize;
        self.thought_in[at] = 0;
        let slot = &mut self.slots[at];
        slot.occupied = Some(Held {
            kind,
            linked: false,
            relations: EntityRelations::default(),
            next_think_ms: None,
            transient_event_time_ms: None,
        });
        Ok(EntityRef::new(number, slot.generation))
    }

    /// A handle to whatever holds `number` right now.
    pub fn current_ref(&self, number: i32) -> Result<EntityRef, EntityRefError> {
        let slot = self.slot(number)?;
        if slot.occupied.is_none() {
            return Err(EntityRefError::Free);
        }
        Ok(EntityRef::new(number, slot.generation))
    }

    pub fn resolve(&self, entity: EntityRef) -> Result<EntitySlotView, EntityRefError> {
        let held = self.held(entity)?;
        Ok(EntitySlotView {
            entity,
            kind: held.kind,
            linked: held.linked,
            relations: held.relations,
            next_think_ms: held.next_think_ms,
        })
    }

    pub fn set_linked(&mut self, entity: EntityRef, linked: bool) -> Result<(), EntityRefError> {
        self.held_mut(entity)?.linked = linked;
        Ok(())
    }

    /// True once, when the entity's scheduled think time has arrived; the schedule is then cleared. A time of zero
    /// or less means "not scheduled".
    pub fn take_due_think(&mut self, entity: EntityRef) -> Result<bool, EntityRefError> {
        let now = self.level_time_ms;
        let held = self.held_mut(entity)?;
        let due = held.next_think_ms.is_some_and(|at| at > 0 && at <= now);
        if due {
            held.next_think_ms = None;
        }
        Ok(due)
    }

    /// Lets the entity in `number` think, at most once per frame.
    pub fn begin_think(&mut self, number: i32) -> Result<ThinkEnter, EntityRefError> {
        let Ok(at) = position(number) else {
            return Ok(ThinkEnter::Skip);
        };
        let frame = self.frame_serial;
        let Some(slot) = self.slots.get(at) else {
            return Ok(ThinkEnter::Skip);
        };
        let Some(held) = &slot.occupied else {
            return Ok(ThinkEnter::Skip);
        };
        if self.thought_in[at] == frame {
            return Ok(ThinkEnter::Skip);
        }
        self.thought_in[at] = frame;
        Ok(ThinkEnter::Run {
            entity: EntityRef::new(number, slot.generation),
            kind: held.kind,
            parent: held.relations.parent,
        })
    }

    pub fn mark_transient_event(&mut self, entity: EntityRef, event_time_ms: i32) -> Result<(), EntityRefError> {
        self.held_mut(entity)?.transient_event_time_ms = Some(event_time_ms);
        Ok(())
    }

    /// Frees every one-shot event that has outlived its lifetime, and says which ones.
    pub fn expire_transient_events(&mut self) -> Vec<EntityRef> {
        let now = self.level_time_ms;
        let expired: Vec<EntityRef> = (FIRST..)
            .zip(&self.slots)
            .filter_map(|(number, slot)| {
                let stamped = slot.occupied.as_ref()?.transient_event_time_ms?;
                (now.saturating_sub(stamped) > GENTITY_TEMP_EVENT_LIFETIME_MS)
                    .then(|| EntityRef::new(number, slot.generation))
            })
            .collect();
        for &entity in &expired {
            self.free(entity).expect("an event found live a moment ago frees cleanly");
        }
        expired
    }

    /// Gives the number back: every link to it is dropped, its generation moves on, and it goes to the back of the
    /// reuse queue.
    pub fn free(&mut self, entity: EntityRef) -> Result<(), EntityRefError> {
        self.held(entity)?;
        for held in self.slots.iter_mut().filter_map(|slot| slot.occupied.as_mut()) {
            held.relations.forget(entity);
        }
        let slot = &mut self.slots[(entity.number() - FIRST) as usize];
        slot.occupied = None;
        slot.freed_at_ms = self.level_time_ms;
        slot.generation = slot.generation.wrapping_add(1);
        self.free_fifo.push_back(entity.number());
        Ok(())
    }
}
