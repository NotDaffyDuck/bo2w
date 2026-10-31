//! The entity-number table itself.

use std::collections::VecDeque;

use trace_t6::WORLD_ENTITY_SLOT;

use super::refs::{EntitySlotShortage, EntHandle, EntityHandleFault, EntitySlotReadout, ThinkEntryResult};
use super::frozen::{
    BayHeldRow as Held, BayFrozenRow as Slot, FrozenSlotTable,
    EntityTableSnapshotFault,
};
use super::{
    EntityBehaviorClass, PROTECTED_ENTITY_SLOTS as FIRST, SLOT_COOLDOWN_BEFORE_REUSE_MS, EVENT_ENTITY_LIFESPAN_MS,
};

/// One past the last number the kernel can hand out.
const SLOT_CEILING: i32 = WORLD_ENTITY_SLOT as i32;

/// What every number above the high-water mark looks like: never used.
static UNUSED: Slot = Slot::UNUSED;

#[derive(Clone, Debug, PartialEq)]
pub struct EntitySlotArena {
    /// One record per number from `FIRST` up to `high_water`.
    records: Vec<Slot>,
    /// The frame each slot last thought in, in step with `slots`. Not part of a snapshot.
    thought_in: Vec<u32>,
    high_mark: i32,
    reuse_queue: VecDeque<i32>,
    clock_ms: i32,
    frame_no: u32,
}

impl Default for EntitySlotArena {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            thought_in: Vec::new(),
            high_mark: FIRST,
            reuse_queue: VecDeque::new(),
            clock_ms: 0,
            frame_no: 0,
        }
    }
}

/// Where `number` sits in the slot list. Numbers past the high-water mark are allowed here; they just have no record.
fn position(number: i32) -> Result<usize, EntityHandleFault> {
    if (FIRST..SLOT_CEILING).contains(&number) {
        Ok((number - FIRST) as usize)
    } else {
        Err(EntityHandleFault::NumberNotUsable)
    }
}

impl EntitySlotArena {
    pub fn export_snapshot(&self) -> FrozenSlotTable {
        let free_fifo: Vec<i32> = self.reuse_queue.iter().copied().collect();
        FrozenSlotTable {
            slots: self.records.clone(),
            free_fifo,
            high_water: self.high_mark,
            frame_serial: self.frame_no,
            level_time_ms: self.clock_ms,
        }
    }

    pub fn from_world_state(image: &FrozenSlotTable) -> Result<Self, EntityTableSnapshotFault> {
        image.validate()?;
        Ok(Self {
            records: image.slots.clone(),
            thought_in: vec![0; image.slots.len()],
            high_mark: image.high_water,
            reuse_queue: image.free_fifo.iter().copied().collect(),
            clock_ms: image.level_time_ms,
            frame_no: image.frame_serial,
        })
    }

    /// Starts a frame: the clock moves to `clock_millis` and every entity may think once again.
    pub fn open_tick(&mut self, clock_millis: i32) {
        self.clock_ms = clock_millis;
        self.frame_no = self.frame_no.wrapping_add(1);
    }

    pub const fn level_time_ms(&self) -> i32 {
        self.clock_ms
    }

    pub const fn frame_serial(&self) -> u32 {
        self.frame_no
    }

    pub const fn high_water(&self) -> i32 {
        self.high_mark
    }

    fn slot(&self, number: i32) -> Result<&Slot, EntityHandleFault> {
        Ok(self.records.get(position(number)?).unwrap_or(&UNUSED))
    }

    /// The held part of the slot `entity` points at, if the handle is current.
    fn held(&self, entity: EntHandle) -> Result<&Held, EntityHandleFault> {
        let slot = self.slot(entity.slot())?;
        if slot.generation != entity.generation() {
            return Err(EntityHandleFault::Outlived { current: slot.generation });
        }
        slot.occupied.as_ref().ok_or(EntityHandleFault::Free)
    }

    fn held_mut(&mut self, entity: EntHandle) -> Result<&mut Held, EntityHandleFault> {
        self.held(entity)?;
        let at = position(entity.slot())?;
        self.records[at].occupied.as_mut().ok_or(EntityHandleFault::Free)
    }

    pub fn holder_class(&self, number: i32) -> Option<EntityBehaviorClass> {
        self.slot(number).ok()?.occupied.map(|held| held.kind)
    }

    /// Every held number whose kind `wanted` accepts, lowest first.
    pub fn held_slots(&self, mut wanted: impl FnMut(EntityBehaviorClass) -> bool) -> Vec<i32> {
        (FIRST..self.high_mark).filter(|&number| self.holder_class(number).is_some_and(&mut wanted)).collect()
    }

    /// The oldest freed number, if it has sat out its quarantine (or the table is full, so waiting would not help).
    fn reusable(&self) -> Option<i32> {
        let &oldest = self.reuse_queue.front()?;
        let freed_at = self.records[(oldest - FIRST) as usize].freed_at_ms;
        let rested = self.clock_ms.saturating_sub(freed_at) >= SLOT_COOLDOWN_BEFORE_REUSE_MS;
        (rested || self.high_mark >= SLOT_CEILING).then_some(oldest)
    }

    pub fn claim_slot(&mut self, kind: EntityBehaviorClass) -> Result<EntHandle, EntitySlotShortage> {
        let number = match self.reusable() {
            Some(recycled) => {
                self.reuse_queue.pop_front();
                recycled
            }
            None if self.high_mark < SLOT_CEILING => {
                self.records.push(UNUSED);
                self.thought_in.push(0);
                self.high_mark += 1;
                self.high_mark - 1
            }
            None => return Err(EntitySlotShortage::TableFull),
        };
        let at = (number - FIRST) as usize;
        self.thought_in[at] = 0;
        let (unlinked, unscheduled) = (false, None);
        let record = &mut self.records[at];
        record.occupied = Some(Held {
            kind,
            linked: unlinked,
            relations: Default::default(),
            next_think_ms: unscheduled,
            transient_event_time_ms: unscheduled,
        });
        Ok(EntHandle::new(number, record.generation))
    }

    /// A handle to whatever holds `number` right now.
    pub fn ref_for(&self, number: i32) -> Result<EntHandle, EntityHandleFault> {
        let record = self.slot(number)?;
        record.occupied.as_ref().ok_or(EntityHandleFault::Free)?;
        Ok(EntHandle::new(number, record.generation))
    }

    pub fn look_up(&self, entity: EntHandle) -> Result<EntitySlotReadout, EntityHandleFault> {
        self.held(entity).map(|held| EntitySlotReadout {
            handle: entity,
            class: held.kind,
            in_world: held.linked,
            ties: held.relations,
            think_at_ms: held.next_think_ms,
        })
    }

    pub fn mark_linked(&mut self, entity: EntHandle, tied: bool) -> Result<(), EntityHandleFault> {
        self.held_mut(entity)?.linked = tied;
        Ok(())
    }

    /// True once, when the entity's scheduled think time has arrived; the schedule is then cleared. A time of zero
    /// or less means "not scheduled".
    pub fn pop_due_think(&mut self, entity: EntHandle) -> Result<bool, EntityHandleFault> {
        let now = self.clock_ms;
        let held = self.held_mut(entity)?;
        let due = held.next_think_ms.is_some_and(|at| at > 0 && at <= now);
        if due {
            held.next_think_ms = None;
        }
        Ok(due)
    }

    /// Lets the entity in `number` think, at most once per frame.
    pub fn open_think(&mut self, number: i32) -> Result<ThinkEntryResult, EntityHandleFault> {
        Ok(match position(number) {
            Ok(at) => self.enter_think(number, at),
            Err(_) => ThinkEntryResult::Skip,
        })
    }

    /// The think permit for `number` at list position `at`: none unless the number is held and has not thought in
    /// this frame yet.
    fn enter_think(&mut self, slot_no: i32, at: usize) -> ThinkEntryResult {
        let frame = self.frame_no;
        let Some(record) = self.records.get(at) else { return ThinkEntryResult::Skip };
        let Some(held) = &record.occupied else { return ThinkEntryResult::Skip };
        if self.thought_in[at] == frame {
            return ThinkEntryResult::Skip;
        }
        self.thought_in[at] = frame;
        ThinkEntryResult::Run {
            entity: EntHandle::new(slot_no, record.generation),
            kind: held.kind,
            parent: held.relations.parent,
        }
    }

    pub fn mark_transient_event(&mut self, entity: EntHandle, event_time_ms: i32) -> Result<(), EntityHandleFault> {
        self.held_mut(entity)?.transient_event_time_ms = Some(event_time_ms);
        Ok(())
    }

    /// Frees every one-shot event that has outlived its lifetime, and says which ones.
    pub fn drop_old_events(&mut self) -> Vec<EntHandle> {
        let now = self.clock_ms;
        let expired: Vec<EntHandle> = (FIRST..)
            .zip(&self.records)
            .filter_map(|(number, slot)| {
                let stamped = slot.occupied.as_ref()?.transient_event_time_ms?;
                (now.saturating_sub(stamped) > EVENT_ENTITY_LIFESPAN_MS)
                    .then(|| EntHandle::new(number, slot.generation))
            })
            .collect();
        for &entity in &expired {
            self.release_slot(entity).expect("an event found live a moment ago frees cleanly");
        }
        expired
    }

    /// Gives the number back: every link to it is dropped, its generation moves on, and it goes to the back of the
    /// reuse queue.
    pub fn release_slot(&mut self, entity: EntHandle) -> Result<(), EntityHandleFault> {
        self.held(entity)?;
        for held in self.records.iter_mut().filter_map(|slot| slot.occupied.as_mut()) {
            held.relations.forget(entity);
        }
        let released = &mut self.records[(entity.slot() - FIRST) as usize];
        released.occupied = None;
        released.freed_at_ms = self.clock_ms;
        released.generation = released.generation.wrapping_add(1);
        self.reuse_queue.push_back(entity.slot());
        Ok(())
    }
}
