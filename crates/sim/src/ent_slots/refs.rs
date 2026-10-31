//! Handles to entities and the answers a lookup can give.

use trace_t6::WORLD_ENTITY_SLOT;

use super::{EntityBehaviorClass, PROTECTED_ENTITY_SLOTS};

/// A number plus the generation it had when the handle was made. Once the number is freed its generation moves
/// on, so a handle kept past that point fails to resolve instead of reaching whatever took the number next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntHandle {
    slot: i32, reuse: u32,
}

impl EntHandle {
    pub(super) const fn new(slot: i32, reuse: u32) -> Self {
        Self { slot, reuse }
    }

    /// Rebuilds a handle from its two parts (the wire form). Reserved and out-of-range numbers are refused.
    pub fn from_parts(slot: i32, reuse: u32) -> Result<Self, EntityHandleFault> {
        let usable = PROTECTED_ENTITY_SLOTS..i32::from(WORLD_ENTITY_SLOT);
        if usable.contains(&slot) {
            Ok(Self::new(slot, reuse))
        } else {
            Err(EntityHandleFault::NumberNotUsable)
        }
    }

    pub const fn slot(self) -> i32 {
        self.slot
    }

    pub const fn generation(self) -> u32 {
        self.reuse
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitySlotShortage {
    TableFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityHandleFault {
    NumberNotUsable,
    BadSnapshot,
    Free,
    Outlived { current: u32 },
}

/// Who an entity belongs to, rides on, or stands on, plus the offset of its link to a parent's bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThingTies {
    pub owner: Option<EntHandle>, pub parent: Option<EntHandle>, pub ground: Option<EntHandle>,
    /// The parent's tag this entity is attached to; -1 for none.
    pub parent_tag: i32, pub parent_link_axis: [[f32; 3]; 3], pub parent_link_origin: [f32; 3],
}

/// Older spelling EntityRelations still calls; the rename sweep moves it over.
pub use self::ThingTies as EntityRelations;

const NO_PARENT_TAG: i32 = -1;

impl Default for ThingTies {
    fn default() -> Self {
        let [owner, parent, ground] = [None; 3];
        Self {
            owner,
            parent,
            ground,
            parent_tag: NO_PARENT_TAG,
            parent_link_axis: crate::ent_links::BOUND_FRAME_UNTURNED,
            parent_link_origin: Default::default(),
        }
    }
}

impl ThingTies {
    /// The three entity links, owner first.
    pub(super) fn links(&self) -> [Option<EntHandle>; 3] {
        [self.owner, self.parent, self.ground]
    }

    /// Drops every link that points at `gone`.
    pub(super) fn forget(&mut self, gone: EntHandle) {
        for link in [&mut self.owner, &mut self.parent, &mut self.ground] {
            if *link == Some(gone) {
                *link = None;
            }
        }
    }
}

/// What a successful lookup shows of a slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySlotReadout {
    pub handle: EntHandle, pub class: EntityBehaviorClass, pub in_world: bool, pub ties: ThingTies,
    pub think_at_ms: Option<i32>,
}

/// The answer to "may this entity think now?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkEntryResult {
    Skip,
    Run { entity: EntHandle, kind: EntityBehaviorClass, parent: Option<EntHandle> },
}
