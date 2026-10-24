//! Handles to entities and the answers a lookup can give.

use trace_t6::ENTITYNUM_WORLD;

use super::{EntityRunKind, GENTITY_RESERVED_COUNT};

/// A number plus the generation it had when the handle was made. Once the number is freed its generation moves
/// on, so a handle kept past that point fails to resolve instead of reaching whatever took the number next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityRef {
    number: i32,
    generation: u32,
}

impl EntityRef {
    pub(super) const fn new(number: i32, generation: u32) -> Self {
        Self { number, generation }
    }

    /// Rebuilds a handle from its two parts (the wire form). Reserved and out-of-range numbers are refused.
    pub fn from_parts(number: i32, generation: u32) -> Result<Self, EntityRefError> {
        if !(GENTITY_RESERVED_COUNT..i32::from(ENTITYNUM_WORLD)).contains(&number) {
            return Err(EntityRefError::ReservedOrOutOfRange);
        }
        Ok(Self::new(number, generation))
    }

    pub const fn number(self) -> i32 {
        self.number
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityAllocError {
    CapacityExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityRefError {
    ReservedOrOutOfRange,
    SnapshotMalformed,
    Free,
    StaleGeneration { current: u32 },
    RelationTargetInvalid,
    CyclicParent,
}

/// Who an entity belongs to, rides on, or stands on, plus the offset of its link to a parent's bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityRelations {
    pub owner: Option<EntityRef>,
    pub parent: Option<EntityRef>,
    pub ground: Option<EntityRef>,
    /// The parent's tag this entity is attached to; -1 for none.
    pub parent_tag: i32,
    pub parent_link_axis: [[f32; 3]; 3],
    pub parent_link_origin: [f32; 3],
}

impl Default for EntityRelations {
    fn default() -> Self {
        Self {
            owner: None,
            parent: None,
            ground: None,
            parent_tag: -1,
            parent_link_axis: crate::entity_local::PARENT_LINK_AXIS_IDENTITY,
            parent_link_origin: [0.0; 3],
        }
    }
}

impl EntityRelations {
    /// The three entity links, owner first.
    pub(super) fn links(&self) -> [Option<EntityRef>; 3] {
        [self.owner, self.parent, self.ground]
    }

    /// Drops every link that points at `gone`.
    pub(super) fn forget(&mut self, gone: EntityRef) {
        for link in [&mut self.owner, &mut self.parent, &mut self.ground] {
            if *link == Some(gone) {
                *link = None;
            }
        }
    }
}

/// What a successful lookup shows of a slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySlotView {
    pub entity: EntityRef,
    pub kind: EntityRunKind,
    pub linked: bool,
    pub relations: EntityRelations,
    pub next_think_ms: Option<i32>,
}

/// The answer to "may this entity think now?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkEnter {
    Skip,
    Run { entity: EntityRef, kind: EntityRunKind, parent: Option<EntityRef> },
}
