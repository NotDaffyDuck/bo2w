//! Entity numbers. The kernel hands numbers out and takes them back, stamps each reuse so an old handle is
//! recognised as stale, and keeps the per-frame bookkeeping (who already thought, which events have expired).
//! Beside it: the little state builders for the entity kinds the sim spawns itself, and the use-key edge detector.

mod kernel;
mod glide;
mod presses;
mod refs;
mod frozen;

#[cfg(test)]
mod tests;

use playerstate_t6::game::GENTITY_SPAWN_BASE;

pub use kernel::EntitySlotArena;
pub use glide::{
    MoverSlot, turn_path_of, start_scripted_spin, loot_state_at,
    shell_state_at, build_scripted_mover, move_path_of, spin_rate_pose,
};
pub use presses::{InteractTap, gather_presses};
pub use refs::{
    EntitySlotShortage, EntHandle, EntityHandleFault, ThingTies, EntityRelations, EntitySlotReadout, ThinkEntryResult,
};
pub use frozen::{
    BayHeldRow, EntityKernelOccupiedSnapshot, BayFrozenRow, EntityKernelSlotSnapshot, FrozenSlotTable, EntityKernelSnapshot, EntityTableSnapshotFault,
};

/// Numbers below this belong to clients and corpses; the kernel never hands them out.
pub const PROTECTED_ENTITY_SLOTS: i32 = GENTITY_SPAWN_BASE;
/// A freed number is not given out again until this long after it was freed (unless the table is full).
pub const SLOT_COOLDOWN_BEFORE_REUSE_MS: i32 = 500;
/// How long a one-shot event entity lives before the kernel frees it.
pub const EVENT_ENTITY_LIFESPAN_MS: i32 = 300;

/// The parts of one authority step, in the order they must run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityPassStage {
    TimerStepResult,
    DropOldEvents,
    TakeInput,
    RunAvatars,
    StoreCollision,
    ThinkEntities,
    TouchPass,
    Finalize,
    SendOut,
}

pub const PASS_STAGE_SEQUENCE: &[EntityPassStage] = {
    use EntityPassStage::*;
    &[
        TimerStepResult, DropOldEvents, TakeInput, RunAvatars, StoreCollision, ThinkEntities,
        TouchPass, Finalize, SendOut,
    ]
};

/// What runs in an entity's slot each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityBehaviorClass {
    ScriptedMover,
    Item,
    Missile,
    TempEvent,
    MainLightSource,
    General,
    FallenPlayerBody,
}

/// The first number the kernel hands out (callers outside the sim ask for it by function).
pub fn first_spawnable_slot() -> i32 {
    PROTECTED_ENTITY_SLOTS
}

